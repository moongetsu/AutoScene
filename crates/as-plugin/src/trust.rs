//! Who stands behind a plugin package. A package may hold a minisign
//! signature of its `files.lock`, which hashes every file, and the public key
//! of the author who signed it (`author.pub`). [`judge`] gives it one of
//! three trusts:
//!
//! - Reviewed: an AutoScene Plugins key compiled into the app
//!   ([`crate::keys::REVIEW_KEYS`]) signed it.
//! - Pinned: its author signed it, with the key in its `author.pub`. The
//!   first install pins that key, and an update must be signed by the same
//!   one. A package never becomes Reviewed by a key it brings along.
//! - Unreviewed: nobody signed it, or a key AutoScene doesn't know did.
//!
//! The signature's trusted comment names the plugin, its version and the
//! lock's SHA-256 ([`trusted_comment`]), so a signature for one plugin or
//! version is refused on another. Only prehashed signatures are taken (what
//! minisign makes by default), never legacy ones. A signature that fails
//! under a key AutoScene knows (a review key, the package's author key or the
//! pinned one) refuses the package: it is never taken for an unsigned one.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use minisign_verify::{Error as SignatureError, PublicKey, Signature};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{PluginError, PluginResult};
use crate::keys::REVIEW_KEYS;
use crate::manifest::{Manifest, PluginId, invalid};
use crate::package::sha256_hex;

/// Who stands behind a plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// AutoScene reviewed it: an AutoScene Plugins key signed it.
    Reviewed,
    /// Its author signed it, with the key its first install pinned.
    Pinned,
    /// Nobody AutoScene knows signed it.
    Unreviewed,
}

/// The review keys a package's signature is checked against.
#[derive(Clone, Debug)]
pub struct Keys {
    review: Vec<PublicKey>,
}

impl Keys {
    /// The AutoScene Plugins keys compiled into the app ([`REVIEW_KEYS`]).
    pub fn builtin() -> Self {
        Self::new(REVIEW_KEYS).expect("keys::REVIEW_KEYS are minisign public keys")
    }

    /// Review keys of one's own (tests, `asp verify --review-key`), each the
    /// base64 line of a minisign `.pub` file.
    pub fn new(review: &[&str]) -> PluginResult<Self> {
        let review = review.iter().map(|key| read_key(key).map(|(key, _)| key)).collect::<PluginResult<_>>()?;
        Ok(Self { review })
    }
}

/// An author's minisign public key: the base64 line of its `.pub` file (the
/// one spelling of the key there is, so two keys are one when their text is),
/// and its key id as minisign shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AuthorKey {
    pub base64: String,
    pub key_id: String,
}

/// A package's trust, and the author key its plugin is pinned to after it:
/// the key that signed a Pinned package, or the pin a Reviewed one keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub trust: Trust,
    pub author_key: Option<AuthorKey>,
}

/// The trusted comment a package's signature carries:
/// `autoscene-plugin;id=<id>;version=<version>;lock=<SHA-256 of files.lock>`.
pub fn trusted_comment(id: &PluginId, version: &str, lock_sha256: &str) -> String {
    format!("autoscene-plugin;id={id};version={version};lock={lock_sha256}")
}

/// The key id of the minisign public key `base64` (the line a `.pub` file
/// holds), as minisign shows it: 16 uppercase hex digits.
pub fn key_id(base64: &str) -> PluginResult<String> {
    let raw = STANDARD.decode(base64).ok().filter(|raw| raw.len() == 42 && raw.starts_with(b"Ed"));
    let raw = raw.ok_or_else(|| invalid("That isn't a minisign public key."))?;
    Ok(key_id_of(&raw))
}

/// The key id in `raw`, a decoded minisign key or signature (its algorithm,
/// then the id's 8 bytes, little-endian), as minisign shows it.
fn key_id_of(raw: &[u8]) -> String {
    let mut id = [0u8; 8];
    id.copy_from_slice(&raw[2..10]);
    format!("{:016X}", u64::from_le_bytes(id))
}

/// Judges a package by the signature of its lock `lock_bytes` (from
/// `files.lock.minisig`), the author key beside it (from `author.pub`) and
/// the author key its plugin is pinned to, if it is installed. `m` is its
/// manifest, whose id and version the signature must name.
///
/// A package of a pinned plugin must be Reviewed, or signed by the pinned key
/// (`author.pub` may then be left out); one that is unsigned or signed by
/// another key is [`PluginError::KeyChanged`]. A signature that can't be read
/// or names another plugin, version or lock is
/// [`PluginError::BadSignature`], and so is one that is legacy or fails under
/// a key AutoScene knows (a review key, the package's `author.pub` or the
/// pinned key). One by a key AutoScene doesn't know, with no `author.pub` and
/// no pin, counts as unsigned.
pub fn judge(
    keys: &Keys,
    lock_bytes: &[u8],
    sig: Option<&str>,
    author_key: Option<&str>,
    m: &Manifest,
    pinned: Option<&AuthorKey>,
) -> PluginResult<Verdict> {
    let unreviewed = || Verdict { trust: Trust::Unreviewed, author_key: None };
    let Some(sig) = sig else {
        return match pinned {
            Some(pin) => Err(key_changed(pin, "none")),
            None => Ok(unreviewed()),
        };
    };
    let signature = Signature::decode(sig).map_err(|e| bad(format!("the signature can't be read: {e}")))?;
    if signature.trusted_comment() != trusted_comment(&m.id, &m.version, &sha256_hex(lock_bytes)) {
        return Err(bad("signed for another plugin or version"));
    }
    for key in &keys.review {
        match key.verify(lock_bytes, &signature, false) {
            // author.pub lies outside the lock, so the review doesn't cover
            // it: it makes no pin, and an existing one stays.
            Ok(()) => return Ok(Verdict { trust: Trust::Reviewed, author_key: pinned.cloned() }),
            Err(SignatureError::UnexpectedKeyId) => {}
            Err(e) => return Err(bad(format!("review key: {e}"))),
        }
    }
    let Some(author_key) = author_key else {
        let Some(pin) = pinned else { return Ok(unreviewed()) };
        // asp sign always adds author.pub, but the pinned key can check the
        // signature without it. A pin that doesn't read matches no key.
        let (key, author) = read_key(&pin.base64).map_err(|_| key_changed(pin, &signer_id(sig)))?;
        return match key.verify(lock_bytes, &signature, false) {
            Ok(()) => Ok(Verdict { trust: Trust::Pinned, author_key: Some(author) }),
            Err(SignatureError::UnexpectedKeyId) => Err(key_changed(pin, &signer_id(sig))),
            Err(e) => Err(bad(format!("pinned key {}: {e}", pin.key_id))),
        };
    };
    let (key, author) = read_key(author_key.trim()).map_err(|_| bad("author.pub isn't a minisign public key"))?;
    key.verify(lock_bytes, &signature, false).map_err(|e| bad(format!("author key {}: {e}", author.key_id)))?;
    match pinned {
        Some(pin) if pin.base64 != author.base64 => Err(key_changed(pin, &author.key_id)),
        _ => Ok(Verdict { trust: Trust::Pinned, author_key: Some(author) }),
    }
}

/// Reads a minisign public key (the base64 line of a `.pub` file), in the one
/// spelling base64 has for it.
fn read_key(base64: &str) -> PluginResult<(PublicKey, AuthorKey)> {
    let key_id = key_id(base64)?;
    let key = PublicKey::from_base64(base64).map_err(|_| invalid("That isn't a minisign public key."))?;
    Ok((key, AuthorKey { base64: base64.to_owned(), key_id }))
}

/// The key id of the key that made `sig`, a signature [`Signature::decode`]
/// reads, as minisign shows it.
fn signer_id(sig: &str) -> String {
    let raw = sig.lines().nth(1).and_then(|line| STANDARD.decode(line.trim()).ok()).filter(|raw| raw.len() == 74);
    raw.map_or_else(|| "unknown".into(), |raw| key_id_of(&raw))
}

fn key_changed(pin: &AuthorKey, got: &str) -> PluginError {
    PluginError::KeyChanged { pinned: pin.key_id.clone(), got: got.to_owned() }
}

fn bad(why: impl Into<String>) -> PluginError {
    PluginError::BadSignature(why.into())
}

#[cfg(test)]
mod tests {
    use minisign::KeyPair;
    use semver::Version;
    use serde_json::json;

    use super::*;
    use crate::manifest;

    /// A files.lock (the signature check only hashes it).
    const LOCK: &[u8] = concat!(
        r#"{"lockVersion":1,"files":[{"path":"plugin.json","size":2,"#,
        r#""sha256":"44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","exec":false}]}"#
    )
    .as_bytes();
    /// The BLAKE2b-512 hash of [`LOCK`], which a minisign signature signs.
    const LOCK_BLAKE2B: &str = concat!(
        "a3e5bfc5670ceda082552d2aa81333959955d650faf3cad26fedfb7366f92860",
        "94b2d8005d594f9f6ed6c32d629e7d96f6e5f153f5d2df2aff55ba8990ffb070"
    );

    fn plugin(id: &str, version: &str) -> Manifest {
        let json = json!({
            "manifestVersion": 1,
            "id": id,
            "version": version,
            "name": "Hello Export",
            "description": "Says hello after an export.",
            "author": {"name": "AutoScene"},
            "engines": {"autoscene": ">=1.0.0, <2", "protocol": 1},
            "runtime": {"type": "process", "command": {"*": ["uv", "run", "--script", "--locked", "main.py"]}}
        });
        let app = Version::parse("1.0.3-BETA").unwrap();
        manifest::parse(&serde_json::to_vec(&json).unwrap(), &app).unwrap().manifest
    }

    fn hello(version: &str) -> Manifest {
        plugin("dev.autoscene.hello-export", version)
    }

    /// A minisign key pair, as `asp keygen` makes one.
    struct Signer(KeyPair);

    impl Signer {
        fn new() -> Self {
            Self(KeyPair::generate_unencrypted_keypair().unwrap())
        }

        /// Its `author.pub`.
        fn public(&self) -> String {
            self.0.pk.to_base64()
        }

        fn author_key(&self) -> AuthorKey {
            AuthorKey { base64: self.public(), key_id: key_id(&self.public()).unwrap() }
        }

        /// Its signature of `lock` for `m`, as `asp sign` makes it.
        fn sign(&self, lock: &[u8], m: &Manifest) -> String {
            self.sign_as(lock, &trusted_comment(&m.id, &m.version, &sha256_hex(lock)))
        }

        /// Its signature of `lock` with the trusted comment `comment`.
        fn sign_as(&self, lock: &[u8], comment: &str) -> String {
            minisign::sign(Some(&self.0.pk), &self.0.sk, lock, Some(comment), None).unwrap().into_string()
        }
    }

    fn reviewed_by(reviewer: &Signer) -> Keys {
        Keys::new(&[&reviewer.public()]).unwrap()
    }

    /// The base64 `text` with the bytes at `at` replaced by `bytes`.
    fn rebytes(text: &str, at: usize, bytes: &[u8]) -> String {
        let mut raw = STANDARD.decode(text).unwrap();
        raw[at..at + bytes.len()].copy_from_slice(bytes);
        STANDARD.encode(raw)
    }

    /// `sig` with the bytes at `at` of its signature line replaced: its
    /// algorithm (0..2) or key id (2..10).
    fn with_sig_bytes(sig: &str, at: usize, bytes: &[u8]) -> String {
        let mut lines: Vec<String> = sig.lines().map(str::to_owned).collect();
        lines[1] = rebytes(&lines[1], at, bytes);
        lines.join("\n") + "\n"
    }

    fn bad_signature(r: PluginResult<Verdict>) -> String {
        match r {
            Err(PluginError::BadSignature(why)) => why,
            r => panic!("expected BadSignature, got {r:?}"),
        }
    }

    fn key_changed(r: PluginResult<Verdict>) -> (String, String) {
        match r {
            Err(PluginError::KeyChanged { pinned, got }) => (pinned, got),
            r => panic!("expected KeyChanged, got {r:?}"),
        }
    }

    #[test]
    fn a_package_signed_by_a_review_key_is_reviewed() {
        let (current, next, author) = (Signer::new(), Signer::new(), Signer::new());
        let keys = Keys::new(&[&current.public(), &next.public()]).unwrap();
        let m = hello("1.2.0");
        let reviewed = Verdict { trust: Trust::Reviewed, author_key: None };
        for reviewer in [&current, &next] {
            assert_eq!(judge(&keys, LOCK, Some(&reviewer.sign(LOCK, &m)), None, &m, None).unwrap(), reviewed);
        }
        // The author's key beside it makes no pin, and an existing pin is
        // kept: the review doesn't cover author.pub.
        let sig = current.sign(LOCK, &m);
        assert_eq!(judge(&keys, LOCK, Some(&sig), Some(&author.public()), &m, None).unwrap(), reviewed);
        let pin = author.author_key();
        assert_eq!(
            judge(&keys, LOCK, Some(&sig), Some(&Signer::new().public()), &m, Some(&pin)).unwrap(),
            Verdict { trust: Trust::Reviewed, author_key: Some(pin) }
        );
    }

    #[test]
    fn an_author_signed_package_is_pinned_to_its_key() {
        let (author, reviewer) = (Signer::new(), Signer::new());
        let m = hello("1.2.0");
        let sig = author.sign(LOCK, &m);
        let pinned = Verdict { trust: Trust::Pinned, author_key: Some(author.author_key()) };
        // Review keys that didn't sign it are passed over.
        for keys in [Keys::builtin(), reviewed_by(&reviewer)] {
            assert_eq!(judge(&keys, LOCK, Some(&sig), Some(&author.public()), &m, None).unwrap(), pinned);
        }
        // An update signed with the pinned key keeps it; author.pub may end
        // in a line break, or be left out: the pin checks the signature.
        let update = hello("1.3.0");
        let sig = author.sign(LOCK, &update);
        let file = format!("{}\r\n", author.public());
        let pin = author.author_key();
        for author_key in [Some(file.as_str()), None] {
            assert_eq!(judge(&Keys::builtin(), LOCK, Some(&sig), author_key, &update, Some(&pin)).unwrap(), pinned);
        }
    }

    #[test]
    fn an_update_signed_by_another_author_is_refused() {
        let (author, other) = (Signer::new(), Signer::new());
        let m = hello("1.3.0");
        let keys = Keys::builtin();
        let pin = author.author_key();
        let other_id = other.author_key().key_id;
        // With the other key beside it, and without.
        let sig = other.sign(LOCK, &m);
        for author_key in [Some(other.public()), None] {
            let r = judge(&keys, LOCK, Some(&sig), author_key.as_deref(), &m, Some(&pin));
            assert_eq!(key_changed(r), (pin.key_id.clone(), other_id.clone()));
        }
        // Beside the pinned key, which didn't make it.
        let r = judge(&keys, LOCK, Some(&sig), Some(&author.public()), &m, Some(&pin));
        assert!(bad_signature(r).starts_with(&format!("author key {}:", pin.key_id)));
        // Another key that took the pinned key's id is still another key.
        let id = &STANDARD.decode(&pin.base64).unwrap()[2..10];
        let lookalike = rebytes(&other.public(), 2, id);
        let sig = with_sig_bytes(&sig, 2, id);
        let r = judge(&keys, LOCK, Some(&sig), Some(&lookalike), &m, Some(&pin));
        assert_eq!(key_changed(r), (pin.key_id.clone(), pin.key_id.clone()));
        // Without that key beside it, the pinned key finds it didn't make it.
        let r = judge(&keys, LOCK, Some(&sig), None, &m, Some(&pin));
        assert!(bad_signature(r).starts_with(&format!("pinned key {}:", pin.key_id)));
        // A pin that doesn't read is no key, so even the pinned author's own
        // signature doesn't match it.
        let broken = AuthorKey { base64: "RWQ not a key".into(), ..pin.clone() };
        let r = judge(&keys, LOCK, Some(&author.sign(LOCK, &m)), None, &m, Some(&broken));
        assert_eq!(key_changed(r), (pin.key_id.clone(), pin.key_id.clone()));
    }

    #[test]
    fn an_unsigned_update_of_a_pinned_plugin_is_refused() {
        let author = Signer::new();
        let m = hello("1.3.0");
        let pin = author.author_key();
        for author_key in [None, Some(author.public())] {
            let r = judge(&Keys::builtin(), LOCK, None, author_key.as_deref(), &m, Some(&pin));
            assert_eq!(key_changed(r), (pin.key_id.clone(), "none".to_owned()));
        }
    }

    #[test]
    fn a_signature_for_another_version_is_refused() {
        let (reviewer, author) = (Signer::new(), Signer::new());
        let keys = reviewed_by(&reviewer);
        let m = hello("1.2.0");
        let lock = sha256_hex(LOCK);
        let expected = trusted_comment(&m.id, "1.2.0", &lock);
        let comments = [
            trusted_comment(&m.id, "1.1.0", &lock),
            trusted_comment(&plugin("dev.example.other", "1.2.0").id, "1.2.0", &lock),
            trusted_comment(&m.id, "1.2.0", &sha256_hex(b"another lock")),
            // The right one with more after it.
            format!("{expected};x"),
            format!("{expected} "),
            // minisign's own.
            "timestamp:1762041600\tfile:files.lock\thashed".to_owned(),
        ];
        for comment in &comments {
            for signer in [&reviewer, &author] {
                let r = judge(&keys, LOCK, Some(&signer.sign_as(LOCK, comment)), Some(&author.public()), &m, None);
                assert_eq!(bad_signature(r), "signed for another plugin or version", "{comment}");
            }
        }
        // A signature for 1.2.0 whose trusted comment was rewritten to name
        // 1.3.0: the comment has a signature of its own, under every key.
        let (update, pin) = (hello("1.3.0"), author.author_key());
        for (signer, author_key, pinned) in
            [(&reviewer, None, None), (&author, Some(author.public()), None), (&author, None, Some(&pin))]
        {
            let author_key = author_key.as_deref();
            let sig = signer.sign(LOCK, &m);
            assert!(judge(&keys, LOCK, Some(&sig), author_key, &m, pinned).is_ok());
            let rewritten = sig.replace(&expected, &trusted_comment(&m.id, "1.3.0", &lock));
            assert_ne!(rewritten, sig);
            let r = judge(&keys, LOCK, Some(&rewritten), author_key, &update, pinned);
            assert!(bad_signature(r).ends_with(&SignatureError::InvalidSignature.to_string()));
        }
    }

    #[test]
    fn a_changed_lock_fails_its_signature() {
        let (reviewer, author) = (Signer::new(), Signer::new());
        let keys = reviewed_by(&reviewer);
        let m = hello("1.2.0");
        let changed = String::from_utf8(LOCK.to_vec()).unwrap().replace(r#""exec":false"#, r#""exec":true"#);
        let changed = changed.as_bytes();
        let names_changed = trusted_comment(&m.id, &m.version, &sha256_hex(changed));
        for (signer, author_key) in [(&reviewer, None), (&author, Some(author.public()))] {
            let author_key = author_key.as_deref();
            let sig = signer.sign(LOCK, &m);
            assert!(judge(&keys, LOCK, Some(&sig), author_key, &m, None).is_ok());
            // The trusted comment names the lock it was signed for.
            bad_signature(judge(&keys, changed, Some(&sig), author_key, &m, None));
            // The trusted comment rewritten to name the changed lock.
            let rewritten = sig.replace(&trusted_comment(&m.id, &m.version, &sha256_hex(LOCK)), &names_changed);
            assert_ne!(rewritten, sig);
            bad_signature(judge(&keys, changed, Some(&rewritten), author_key, &m, None));
            // Signed with a trusted comment for the changed lock.
            let sig = signer.sign_as(LOCK, &names_changed);
            bad_signature(judge(&keys, changed, Some(&sig), author_key, &m, None));
        }
        let r = judge(&keys, LOCK, Some("untrusted comment: hello\nRWQ=\n"), None, &m, None);
        assert!(bad_signature(r).starts_with("the signature can't be read"));
    }

    #[test]
    fn legacy_signatures_are_refused() {
        // A minisign signature signs the lock's BLAKE2b-512 hash. Marked
        // legacy (`Ed`), it is a legacy signature of that hash, taken as the
        // lock.
        let hash: Vec<u8> = (0..LOCK_BLAKE2B.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&LOCK_BLAKE2B[i..i + 2], 16).unwrap())
            .collect();
        let (reviewer, author) = (Signer::new(), Signer::new());
        let keys = reviewed_by(&reviewer);
        let m = hello("1.2.0");
        let comment = trusted_comment(&m.id, &m.version, &sha256_hex(&hash));
        let pin = author.author_key();
        for (signer, author_key, pinned) in
            [(&reviewer, None, None), (&author, Some(author.public()), None), (&author, None, Some(&pin))]
        {
            let legacy = with_sig_bytes(&signer.sign_as(LOCK, &comment), 0, b"Ed");
            let key = PublicKey::from_base64(&signer.public()).unwrap();
            assert!(key.verify(&hash, &Signature::decode(&legacy).unwrap(), true).is_ok(), "a good legacy signature");
            bad_signature(judge(&keys, &hash, Some(&legacy), author_key.as_deref(), &m, pinned));
        }
    }

    #[test]
    fn unsigned_and_unknown_key_packages_are_unreviewed() {
        let (stranger, reviewer) = (Signer::new(), Signer::new());
        let keys = reviewed_by(&reviewer);
        let m = hello("1.2.0");
        let unreviewed = Verdict { trust: Trust::Unreviewed, author_key: None };
        // Unsigned, also with a key beside it, which signed nothing.
        assert_eq!(judge(&keys, LOCK, None, None, &m, None).unwrap(), unreviewed);
        assert_eq!(judge(&keys, LOCK, None, Some(&stranger.public()), &m, None).unwrap(), unreviewed);
        // Signed by a key it doesn't bring along, and by a review key this
        // AutoScene doesn't have.
        let sig = stranger.sign(LOCK, &m);
        assert_eq!(judge(&keys, LOCK, Some(&sig), None, &m, None).unwrap(), unreviewed);
        let sig = reviewer.sign(LOCK, &m);
        assert_eq!(judge(&Keys::builtin(), LOCK, Some(&sig), None, &m, None).unwrap(), unreviewed);
        // A key beside it that can't be read doesn't make it unsigned.
        let r = judge(&keys, LOCK, Some(&stranger.sign(LOCK, &m)), Some("RWQ not a key"), &m, None);
        assert_eq!(bad_signature(r), "author.pub isn't a minisign public key");
    }

    #[test]
    fn key_ids_read_like_minisign() {
        // minisign-verify's example key, which minisign shows as E7620F1842B4E81F.
        let example = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        assert_eq!(key_id(example).unwrap(), "E7620F1842B4E81F");
        let pair = KeyPair::generate_unencrypted_keypair().unwrap();
        let file = pair.pk.to_box().unwrap().into_string();
        let id = key_id(&pair.pk.to_base64()).unwrap();
        assert_eq!(file.lines().next().unwrap(), format!("untrusted comment: minisign public key: {id}"));
        // Not public keys: cut short (also to its algorithm alone, or by a
        // byte), a byte too long, padded, spaced, with another algorithm, a
        // whole .pub file.
        let raw = STANDARD.decode(example).unwrap();
        for bad in [
            "",
            "RWQ=",
            &example[..54],
            STANDARD.encode(&raw[..41]).as_str(),
            STANDARD.encode([&raw[..], &[0]].concat()).as_str(),
            format!("{example}==").as_str(),
            format!(" {example}").as_str(),
            rebytes(example, 0, b"ED").as_str(),
            file.as_str(),
        ] {
            assert!(key_id(bad).is_err(), "{bad}");
            assert!(Keys::new(&[bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_review_keys_are_minisign_keys() {
        assert_eq!(Keys::builtin().review.len(), REVIEW_KEYS.len());
    }
}
