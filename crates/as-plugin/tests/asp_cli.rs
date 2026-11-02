//! The asp tool, run as a plugin author runs it: keys, packages, signatures,
//! and the trust AutoScene would give a package. Each test works in a
//! folder of its own.
#![cfg(feature = "cli")]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use as_plugin::current_platform;
use as_plugin::keys::REVIEW_KEYS;
use as_plugin::manifest;
use as_plugin::package::{self, AUTHOR_KEY, LOCK, SIG};
use as_plugin::trust::{self, Keys, Trust};
use semver::Version;
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive};

/// What a run of asp did.
struct Run {
    code: Option<i32>,
    out: String,
    err: String,
}

impl Run {
    /// Its output, once it succeeded.
    fn ok(self) -> String {
        assert_eq!(self.code, Some(0), "{}", self.err);
        self.out
    }

    /// Its warnings, once it succeeded.
    fn warnings(self) -> String {
        assert_eq!(self.code, Some(0), "{}", self.err);
        self.err
    }

    /// Its errors, once it failed with `code`.
    fn failed(self, code: i32) -> String {
        assert_eq!(self.code, Some(code), "{}{}", self.out, self.err);
        self.err
    }
}

fn asp(args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_asp")).args(args).output().unwrap();
    Run {
        code: output.status.code(),
        out: String::from_utf8(output.stdout).unwrap(),
        err: String::from_utf8(output.stderr).unwrap(),
    }
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// A plugin folder `dir` whose plugin.json starts `command`, with `files`.
fn folder(dir: &Path, command: Value, files: &[(&str, &[u8])]) -> PathBuf {
    let manifest = json!({
        "manifestVersion": 1,
        "id": "dev.autoscene.hello-export",
        "version": "1.2.0",
        "name": "Hello Export",
        "description": "Says hello after an export.",
        "author": {"name": "AutoScene"},
        "engines": {"autoscene": ">=1.0.0, <2", "protocol": 1},
        "runtime": {"type": "process", "command": command}
    });
    let manifest = serde_json::to_vec_pretty(&manifest).unwrap();
    for (path, bytes) in [("plugin.json", manifest.as_slice())].iter().chain(files) {
        let file = dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }
    dir.to_path_buf()
}

/// A plugin that runs a Python script through uv.
fn uv_plugin(dir: &Path) -> PathBuf {
    let command = json!({"*": ["uv", "run", "--script", "--locked", "main.py"]});
    let files: [(&str, &[u8]); 3] =
        [("main.py", b"print('hello')\n"), ("main.py.lock", b"version = 1\n"), ("lib/data.txt", b"hi")];
    folder(dir, command, &files)
}

/// Makes an unencrypted key pair in `dir`; gives its author.pub line.
fn keygen(dir: &Path) -> String {
    asp(&["keygen", text(dir), "--unencrypted"]).ok();
    fs::read_to_string(dir.join(AUTHOR_KEY)).unwrap().trim().to_owned()
}

/// A package's entries, by name.
fn entries(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut entries = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        entries.insert(entry.name().unwrap().into_owned(), bytes);
    }
    entries
}

/// How a package's entries are packed, by name.
fn methods(path: &Path) -> BTreeMap<String, CompressionMethod> {
    let mut zip = ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut methods = BTreeMap::new();
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).unwrap();
        methods.insert(entry.name().unwrap().into_owned(), entry.compression());
    }
    methods
}

/// What lies in `dir`: names, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// `platforms` as asp names them, where this computer's says so.
fn on(platforms: &[&str]) -> String {
    let named = |&p: &&str| if p == current_platform() { format!("{p} (this computer)") } else { p.to_owned() };
    platforms.iter().map(named).collect::<Vec<_>>().join(", ")
}

/// Changes the plugin.json in `dir`.
fn edit_manifest(dir: &Path, edit: impl FnOnce(&mut Value)) {
    let mut manifest: Value = serde_json::from_slice(&fs::read(dir.join("plugin.json")).unwrap()).unwrap();
    edit(&mut manifest);
    fs::write(dir.join("plugin.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

#[test]
fn keygen_pack_sign_verify_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let place = |name: &str| tmp.path().join(name);
    let keys = place("keys");

    // A key pair; another one isn't made over it unless asked.
    let made = asp(&["keygen", text(&keys), "--unencrypted"]).ok();
    let public = fs::read_to_string(keys.join(AUTHOR_KEY)).unwrap();
    let secret = fs::read_to_string(keys.join("asp.key")).unwrap();
    let id = trust::key_id(public.trim()).unwrap();
    assert!(made.contains(&id), "{made}");
    let err = asp(&["keygen", text(&keys), "--unencrypted"]).failed(1);
    assert!(err.contains("is there already; --force replaces it."), "{err}");
    assert_eq!(fs::read_to_string(keys.join("asp.key")).unwrap(), secret);
    let err = asp(&["keygen", text(&keys), "--unencrypted", "--force"]).warnings();
    assert!(err.contains("plugins pinned to the old key"), "{err}");
    let public = fs::read_to_string(keys.join(AUTHOR_KEY)).unwrap();
    assert!(public.ends_with('\n') && !public.trim().contains('\n'), "the bare key line: {public}");
    let id = trust::key_id(public.trim()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(keys.join("asp.key")).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // The same folder gives the same package, byte for byte.
    let src = uv_plugin(&place("hello"));
    let (unsigned, again) = (place("hello.asp"), place("again.asp"));
    let packed = asp(&["pack", text(&src), text(&unsigned)]).ok();
    assert_eq!(packed, format!("Packed dev.autoscene.hello-export 1.2.0 into {}, unsigned.\n", unsigned.display()));
    asp(&["pack", text(&src), text(&again)]).ok();
    assert_eq!(fs::read(&unsigned).unwrap(), fs::read(&again).unwrap());
    let names_in = |path: &Path| entries(path).into_keys().collect::<Vec<_>>();
    assert_eq!(names_in(&unsigned), ["files.lock", "lib/data.txt", "main.py", "main.py.lock", "plugin.json"]);
    assert_eq!(asp(&["verify", text(&unsigned)]).ok(), "dev.autoscene.hello-export 1.2.0: Unreviewed\n");

    // Signed into another file, which leaves the package as it was.
    let signed = place("signed.asp");
    let key = keys.join("asp.key");
    let done = asp(&["sign", text(&unsigned), "--key", text(&key), "--out", text(&signed)]).ok();
    assert!(done.ends_with(&format!(": Pinned (key {id}).\n")), "{done}");
    assert_eq!(fs::read(&unsigned).unwrap(), fs::read(&again).unwrap());
    assert_eq!(asp(&["verify", text(&signed)]).ok(), format!("dev.autoscene.hello-export 1.2.0: Pinned (key {id})\n"));

    // What AutoScene unpacks: the same files and lock, the signature and
    // the key, which judge pins.
    let into = place("unpacked");
    let unpacked = package::unpack(&signed, &into).unwrap();
    let (before, after) = (entries(&unsigned), entries(&signed));
    assert_eq!(unpacked.lock_bytes, before[LOCK]);
    assert!(before.iter().all(|(name, bytes)| after[name] == *bytes));
    assert_eq!(after.len(), before.len() + 2);
    assert_eq!(unpacked.author_key.as_deref(), Some(public.as_str()));
    let app = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
    let m = manifest::parse(&fs::read(into.join("plugin.json")).unwrap(), &app).unwrap().manifest;
    let (sig, author) = (unpacked.signature.as_deref(), unpacked.author_key.as_deref());
    let verdict = trust::judge(&Keys::builtin(), &unpacked.lock_bytes, sig, author, &m, None).unwrap();
    assert_eq!((verdict.trust, verdict.author_key.unwrap().key_id), (Trust::Pinned, id.clone()));

    // Signed in place, also over a signature.
    for _ in 0..2 {
        asp(&["sign", text(&again), "--key", text(&key)]).ok();
        assert_eq!(names_in(&again), names_in(&signed));
        assert_eq!(
            asp(&["verify", text(&again)]).ok(),
            format!("dev.autoscene.hello-export 1.2.0: Pinned (key {id})\n")
        );
    }

    // asp works beside its output and leaves nothing there.
    assert_eq!(names(tmp.path()), ["again.asp", "hello", "hello.asp", "keys", "signed.asp", "unpacked"]);

    // Wrong arguments, and a package in its own folder.
    assert!(asp(&[]).failed(2).contains("usage: asp keygen"));
    assert!(asp(&["pack", text(&src)]).failed(2).contains("expected 2 path(s), got 1"));
    assert!(asp(&["sign", text(&signed)]).failed(2).contains("sign needs --key"));
    assert!(asp(&["verify", text(&signed), "--keys"]).failed(2).contains("unknown option --keys"));
    let err = asp(&["pack", text(&src), text(&src.join("hello.asp"))]).failed(1);
    assert_eq!(err, "asp: Write the package outside the plugin's folder.\n");
    assert_eq!(names(&src), ["lib", "main.py", "main.py.lock", "plugin.json"]);
    // A folder that isn't there is named.
    let missing = place("missing");
    let err = asp(&["pack", text(&missing), text(&place("missing.asp"))]).failed(1);
    assert!(err.starts_with(&format!("asp: {}: ", missing.display())), "{err}");
}

/// Links `link` to the folder `target`: a symlink on Linux and macOS, a
/// junction on Windows, which needs no administrator (its symlinks do).
fn link_folder(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        // mklink takes a "/" in a path for an option.
        let (link, target): (PathBuf, PathBuf) = (link.components().collect(), target.components().collect());
        let made = Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(link).arg(target).output().unwrap();
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    }
}

#[test]
fn pack_refuses_a_folder_with_a_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let src = uv_plugin(&tmp.path().join("hello"));
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    fs::write(elsewhere.join("secret.txt"), b"not the plugin's").unwrap();
    let out = tmp.path().join("hello.asp");

    link_folder(&elsewhere, &src.join("lib/link"));
    let err = asp(&["pack", text(&src), text(&out)]).failed(1);
    assert_eq!(err, "asp: The plugin's lib/link is a link or another special file, which a package can't hold.\n");
    assert_eq!(names(tmp.path()), ["elsewhere", "hello"]);

    // A linked file too, where links to files can be made.
    #[cfg(unix)]
    {
        fs::remove_file(src.join("lib/link")).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("secret.txt"), src.join("lib/link.txt")).unwrap();
        let err = asp(&["pack", text(&src), text(&out)]).failed(1);
        assert!(err.contains("The plugin's lib/link.txt is a link"), "{err}");
        assert_eq!(names(tmp.path()), ["elsewhere", "hello"]);
    }
}

#[test]
fn pack_writes_the_exec_flag_for_listed_programs() {
    let tmp = tempfile::tempdir().unwrap();
    let command = json!({
        "windows-x86_64": ["bin/tool.exe", "--hello"],
        "linux-x86_64": ["bin/tool"],
        "macos-aarch64": ["bin/mac-tool"],
        "*": ["uv", "run", "--script", "--locked", "main.py"]
    });
    let files: [(&str, &[u8]); 5] = [
        ("bin/tool.exe", b"MZ"),
        ("bin/tool", b"\x7fELF"),
        ("bin/helper", b"#!/bin/sh\n"),
        ("main.py", b"print('hello')\n"),
        ("lib/data.txt", b"hi"),
    ];
    let src = folder(&tmp.path().join("hello"), command, &files);
    // Whether a file may run here doesn't make a program, nor keep one from
    // being one.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(src.join("bin/tool"), fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(src.join("bin/helper"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = tmp.path().join("hello.asp");
    let err = asp(&["pack", text(&src), text(&out)]).warnings();

    let lock = package::unpack(&out, &tmp.path().join("unpacked")).unwrap().lock;
    let exec: BTreeMap<&str, bool> = lock.files.iter().map(|f| (f.path.as_str(), f.exec)).collect();
    let helper_runs = cfg!(unix);
    assert_eq!(
        exec,
        BTreeMap::from([
            ("bin/helper", helper_runs),
            ("bin/tool", true),
            ("bin/tool.exe", true),
            ("lib/data.txt", false),
            ("main.py", false),
            ("plugin.json", false),
        ])
    );
    // Each platform falls back to "*" where it has no command of its own.
    let mut warnings = Vec::new();
    if helper_runs {
        warnings.push("warning: bin/helper may run on this computer, so the package marks it to run too.".to_owned());
    }
    let missing = |file: &str| format!("The plugin package is missing {file}, which its command needs.");
    let fallback = on(&["windows-aarch64", "macos-x86_64", "linux-aarch64"]);
    warnings.push(format!("warning: It won't run on {fallback}: {}", missing("main.py.lock")));
    warnings.push(format!("warning: It won't run on {}: {}", on(&["macos-aarch64"]), missing("bin/mac-tool")));
    assert_eq!(err.lines().collect::<Vec<_>>(), warnings);

    // The zip says so too, for tools that unpack it on Linux and macOS.
    let mut zip = ZipArchive::new(fs::File::open(&out).unwrap()).unwrap();
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        let mode = if exec.get(name.as_str()) == Some(&true) { 0o755 } else { 0o644 };
        assert_eq!(entry.unix_mode().map(|m| m & 0o777), Some(mode), "{name}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &str| fs::metadata(tmp.path().join("unpacked").join(path)).unwrap().permissions().mode();
        assert_eq!((mode("bin/tool") & 0o777, mode("lib/data.txt") & 0o777), (0o755, 0o644));
    }
}

#[test]
fn pack_refuses_a_folder_with_a_secret_key() {
    let tmp = tempfile::tempdir().unwrap();
    let place = |name: &str| tmp.path().join(name);
    let src = uv_plugin(&place("hello"));
    let out = place("hello.asp");

    // keygen makes none there.
    let err = asp(&["keygen", text(&src), "--unencrypted"]).failed(1);
    assert_eq!(err, "asp: Make the key outside the plugin's folder, so it never goes into a package.\n");
    assert_eq!(names(&src), ["lib", "main.py", "main.py.lock", "plugin.json"]);

    // A key moved in, by what it holds, and anything named asp.key.
    keygen(&place("keys"));
    fs::copy(place("keys").join("asp.key"), src.join("lib/signing.txt")).unwrap();
    let err = asp(&["pack", text(&src), text(&out)]).failed(1);
    assert_eq!(
        err,
        "asp: The plugin's lib/signing.txt is a secret key, which would let anyone sign as you. \
         Move it out of the plugin's folder.\n"
    );
    fs::remove_file(src.join("lib/signing.txt")).unwrap();
    fs::write(src.join("lib/ASP.KEY"), b"not yet a key").unwrap();
    let err = asp(&["pack", text(&src), text(&out)]).failed(1);
    assert!(err.starts_with("asp: The plugin's lib/ASP.KEY is a secret key"), "{err}");
    assert_eq!(names(tmp.path()), ["hello", "keys"]);

    // A public key is no secret.
    fs::remove_file(src.join("lib/ASP.KEY")).unwrap();
    fs::copy(place("keys").join(AUTHOR_KEY), src.join("lib/author.pub")).unwrap();
    asp(&["pack", text(&src), text(&out)]).ok();
}

#[test]
fn pack_stores_a_file_deflate_would_pack_too_tightly() {
    let tmp = tempfile::tempdir().unwrap();
    let place = |name: &str| tmp.path().join(name);
    let id = trust::key_id(&keygen(&place("keys"))).unwrap();
    let key = place("keys").join("asp.key");

    // Zero-padded weights deflate over a thousand times smaller, which
    // AutoScene takes from a file of up to 1 MB only.
    let src = uv_plugin(&place("model"));
    fs::write(src.join("model.bin"), vec![0; 2 << 20]).unwrap();
    let (unsigned, again, signed) = (place("model.asp"), place("again.asp"), place("signed.asp"));
    asp(&["pack", text(&src), text(&unsigned)]).ok();
    asp(&["pack", text(&src), text(&again)]).ok();
    assert_eq!(fs::read(&unsigned).unwrap(), fs::read(&again).unwrap());
    let stored = |path: &Path| {
        let methods = methods(path).into_iter().filter(|(_, m)| *m != CompressionMethod::Deflated);
        methods.map(|(name, m)| (m == CompressionMethod::Stored).then_some(name)).collect::<Option<Vec<_>>>()
    };
    assert_eq!(stored(&unsigned), Some(vec!["model.bin".to_owned()]));
    asp(&["sign", text(&unsigned), "--key", text(&key), "--out", text(&signed)]).ok();
    assert_eq!(stored(&signed), Some(vec!["model.bin".to_owned()]));
    assert_eq!(asp(&["verify", text(&signed)]).ok(), format!("dev.autoscene.hello-export 1.2.0: Pinned (key {id})\n"));

    // Files of 1 MB may be packed as tightly as deflate likes, but not all
    // of them together: the first of two equal ones is stored.
    let src = uv_plugin(&place("halves"));
    for name in ["a.bin", "b.bin"] {
        fs::write(src.join(name), vec![0; 1 << 20]).unwrap();
    }
    let halves = place("halves.asp");
    asp(&["pack", text(&src), text(&halves)]).ok();
    assert_eq!(stored(&halves), Some(vec!["a.bin".to_owned()]));
    package::unpack(&halves, &place("unpacked")).unwrap();
}

#[test]
fn pack_and_verify_warn_as_autoscene_would() {
    let tmp = tempfile::tempdir().unwrap();
    let place = |name: &str| tmp.path().join(name);
    let files: [(&str, &[u8]); 2] = [("bin/tool", b"#!/bin/sh\n"), ("icon.svg", b"<svg/>")];
    let src = folder(&place("hello"), json!({"*": ["bin/tool"]}), &files);
    // Made for a later AutoScene, with a key this one doesn't know, and a
    // command that runs everywhere but on Windows.
    edit_manifest(&src, |v| {
        v["engines"]["autoscene"] = json!(">=99.0.0");
        v["homepage"] = json!("https://example.com");
        v["icon"] = json!("icon.svg");
    });
    let warnings = [
        "warning: Ignored homepage, which this AutoScene doesn't know.".to_owned(),
        format!(
            r#"warning: AutoScene {} won't install it: its engines.autoscene is ">=99.0.0"."#,
            env!("CARGO_PKG_VERSION")
        ),
        format!(
            r#"warning: It won't run on {}: The plugin's runtime.command["*"] runs on Windows, so its program must be an .exe file."#,
            on(&["windows-x86_64", "windows-aarch64"])
        ),
    ];
    let out = place("hello.asp");
    assert_eq!(asp(&["pack", text(&src), text(&out)]).warnings().lines().collect::<Vec<_>>(), warnings);
    let verified = asp(&["verify", text(&out)]);
    assert_eq!(verified.err.lines().collect::<Vec<_>>(), warnings);
    assert_eq!(verified.ok(), "dev.autoscene.hello-export 1.2.0: Unreviewed\n");

    // A package that runs nowhere, or lacks its icon, isn't written.
    let refused = |edit: fn(&mut Value)| {
        edit_manifest(&src, |v| {
            v["engines"]["autoscene"] = json!(">=1.0.0, <2");
            v.as_object_mut().unwrap().remove("homepage");
            edit(v);
        });
        let err = asp(&["pack", text(&src), text(&place("refused.asp"))]).failed(1);
        assert_eq!(names(tmp.path()), ["hello", "hello.asp"]);
        err
    };
    let windows = r#"The plugin's runtime.command["*"] runs on Windows, so its program must be an .exe file."#;
    assert_eq!(
        refused(|v| v["platforms"] = json!(["windows-x86_64"])),
        format!(
            "warning: It won't run on {}: {windows}\nasp: The plugin runs on none of its platforms.\n",
            on(&["windows-x86_64"])
        )
    );
    let no_command = "The plugin's platforms names it, but runtime.command has no command for it.";
    assert_eq!(
        refused(|v| {
            v["runtime"]["command"] = json!({"windows-x86_64": ["bin/tool.exe"]});
            v["platforms"] = json!(["linux-x86_64"]);
        }),
        format!(
            "warning: It won't run on {}: {no_command}\nasp: The plugin runs on none of its platforms.\n",
            on(&["linux-x86_64"])
        )
    );
    assert_eq!(
        refused(|v| v["platforms"] = json!(["plan9-mips"])),
        "warning: platforms names \"plan9-mips\", which this AutoScene doesn't know.\n\
         asp: The plugin runs on no platform this AutoScene knows.\n"
    );
    assert_eq!(
        refused(|v| {
            v["runtime"]["command"] = json!({"*": ["bin/tool"]});
            v["platforms"] = json!(["linux-x86_64"]);
            v["icon"] = json!("gone.png");
        }),
        "asp: The plugin package is missing its icon, gone.png.\n"
    );
}

/// `package` with `entries` added from `from` (by name), as a zip of its own.
fn grafted(package: &Path, from: &Path, names: &[&str], out: &Path) {
    let (mut entries, from) = (entries(package), entries(from));
    for name in names {
        entries.insert((*name).to_owned(), from[*name].clone());
    }
    let mut zip = zip::ZipWriter::new(fs::File::create(out).unwrap());
    for (name, bytes) in entries {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn verify_names_the_trust_a_package_would_get() {
    let tmp = tempfile::tempdir().unwrap();
    let place = |name: &str| tmp.path().join(name);
    let (author, reviewer) = (keygen(&place("author")), keygen(&place("reviewer")));
    let (author_id, reviewer_id) = (trust::key_id(&author).unwrap(), trust::key_id(&reviewer).unwrap());
    let src = uv_plugin(&place("hello"));
    let unsigned = place("unsigned.asp");
    asp(&["pack", text(&src), text(&unsigned)]).ok();
    let sign = |key: &str, out: &str| {
        let key = place(key).join("asp.key");
        asp(&["sign", text(&unsigned), "--key", text(&key), "--out", text(&place(out))]).ok();
    };
    sign("author", "by-author.asp");
    sign("reviewer", "by-reviewer.asp");
    let trust = |package: &str, review: &[&str]| {
        let package = place(package);
        let mut args = vec!["verify", text(&package)];
        args.extend(review.iter().flat_map(|&key| ["--review-key", key]));
        asp(&args).ok().trim().strip_prefix("dev.autoscene.hello-export 1.2.0: ").unwrap().to_owned()
    };

    assert_eq!(trust("unsigned.asp", &[]), "Unreviewed");
    assert_eq!(trust("unsigned.asp", &[&reviewer]), "Unreviewed");
    assert_eq!(trust("by-author.asp", &[]), format!("Pinned (key {author_id})"));
    assert_eq!(trust("by-author.asp", &[&reviewer]), format!("Pinned (key {author_id})"));
    assert_eq!(trust("by-reviewer.asp", &[&reviewer]), "Reviewed");
    assert_eq!(trust("by-reviewer.asp", &[&author, &reviewer]), "Reviewed");
    // As pasted, or read from a .pub file with Windows line ends.
    assert_eq!(trust("by-reviewer.asp", &[&format!(" {reviewer}\r")]), "Reviewed");
    // AutoScene's own keys are always asked; until the first review there
    // are none, and a review key AutoScene doesn't have reads as an
    // author's.
    if REVIEW_KEYS.is_empty() {
        assert_eq!(trust("by-reviewer.asp", &[]), format!("Pinned (key {reviewer_id})"));
    }

    // Signed by the author's key, but not for this lock: AutoScene refuses it.
    fs::write(src.join("lib/data.txt"), b"changed").unwrap();
    let changed = place("changed.asp");
    asp(&["pack", text(&src), text(&changed)]).ok();
    let forged = place("forged.asp");
    grafted(&changed, &place("by-author.asp"), &[SIG, AUTHOR_KEY], &forged);
    let err = asp(&["verify", text(&forged)]).failed(1);
    assert_eq!(
        err,
        "asp: The package was changed after it was signed. (plugin signature: signed for another plugin or version)\n"
    );

    // Neither a review key nor a package that isn't one.
    let err = asp(&["verify", text(&unsigned), "--review-key", "RWQ not a key"]).failed(1);
    assert_eq!(err, "asp: That isn't a minisign public key.\n");
    let err = asp(&["verify", text(&place("author").join("asp.key"))]).failed(1);
    assert_eq!(err, "asp: The plugin package is damaged.\n");
    let err = asp(&["verify", text(&place("missing.asp"))]).failed(1);
    assert!(err.starts_with(&format!("asp: {}: ", place("missing.asp").display())), "{err}");
}
