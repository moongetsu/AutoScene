//! Plugin packages (`.asp`): a zip of a plugin's files and `files.lock`,
//! which lists each file with its size, SHA-256 and whether it runs. A
//! signed package also holds the lock's signature (`files.lock.minisig`),
//! and may hold the author's key (`author.pub`). The signature covers the
//! lock, and the lock covers every file.
//!
//! A package may be made to do harm, so [`unpack`] goes by the lock, never
//! by what the zip says of itself. Every name is a [`check_path`] path:
//! one file on every system, and never one outside the plugin's folder. The
//! zip holds exactly the listed files, no two of them one file on Windows or
//! macOS. Each file unpacks to its listed size through SHA-256 and stops
//! there, whatever its header claims, and whether it may run comes from the
//! lock, not the zip. Refused too: encrypted entries, links, compression
//! other than stored or deflate, data before the archive, local headers that
//! disagree with the directory, more than [`MAX_ENTRIES`] entries (also as a
//! zip64 end claims them), files over [`RATIO_FROM`] bytes packed more than
//! [`MAX_RATIO`] times smaller than they unpack (and all files together, when
//! they are over it), and sizes past the limits below. A package unpacks
//! into a new folder; when it is refused there, the folder is removed again.

use std::cell::Cell;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::read::{ArchiveOffset, Config};
use zip::result::ZipError;
use zip::{CompressionMethod, ExtraField, ZipArchive};

use crate::error::{PluginError, PluginResult};
use crate::manifest::{Launch, Twice, invalid, shown};

/// The largest package file.
pub const MAX_PACKAGE_BYTES: u64 = 1 << 30;
/// The most a package's files may unpack to, together.
pub const MAX_UNPACKED_BYTES: u64 = 2 << 30;
/// The largest file in a package.
pub const MAX_FILE_BYTES: u64 = 1 << 30;
/// The most entries (files and folders) a package's zip may hold.
pub const MAX_ENTRIES: usize = 4096;
/// How many times smaller than it unpacks a file over [`RATIO_FROM`] bytes
/// may be packed, and so may a package's files together. Deflate packs real
/// files far less tightly; a zip bomb packs far more.
pub const MAX_RATIO: u64 = 100;
/// Files, and packages, that unpack to at most this may be packed as tightly
/// as they like.
pub const RATIO_FROM: u64 = 1 << 20;
/// The longest path a package may hold, in bytes.
pub const MAX_PATH_BYTES: usize = 200;
/// The most folders deep (counting the file) a package path may go.
pub const MAX_DEPTH: usize = 12;
/// The largest `files.lock`.
pub const MAX_LOCK_BYTES: u64 = 1 << 20;
/// The largest `files.lock.minisig`.
pub const MAX_SIG_BYTES: u64 = 4 << 10;
/// The largest `author.pub`.
pub const MAX_KEY_BYTES: u64 = 1 << 10;
/// The largest `.png` icon.
pub const MAX_ICON_PNG: u64 = 256 << 10;
/// The largest `.svg` icon.
pub const MAX_ICON_SVG: u64 = 64 << 10;
/// The `lockVersion` this AutoScene reads and writes.
pub const LOCK_VERSION: u32 = 1;
pub const LOCK: &str = "files.lock";
pub const SIG: &str = "files.lock.minisig";
pub const AUTHOR_KEY: &str = "author.pub";
pub const MANIFEST: &str = "plugin.json";

/// The files a package holds beside the listed ones.
const BESIDE: [&str; 3] = [LOCK, SIG, AUTHOR_KEY];
/// The most bytes zip may read to list a package (its directory and local
/// headers). A package of [`MAX_ENTRIES`] entries with the longest paths
/// needs about 2 MB.
const MAX_LISTING_BYTES: u64 = 8 << 20;
/// How much of a package's end is searched for zip64 ends: zip looks for the
/// directory's end in the last 64 KB and a few bytes (its comment may be
/// that long).
const TAIL_BYTES: u64 = 128 << 10;
/// What `asp pack` leaves out of a plugin folder, at any depth: version
/// control, and the files Windows and macOS leave in folders.
const LEFT_OUT: [&str; 5] = [".git", ".hg", ".svn", ".DS_Store", "Thumbs.db"];

/// `files.lock`: every file of a plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilesLock {
    pub lock_version: u32,
    pub files: Vec<LockedFile>,
}

/// One file of a plugin: its package path, size in bytes and SHA-256 (64
/// lowercase hex digits), and whether it is a program, which unpacks
/// executable on Linux and macOS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockedFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub exec: bool,
}

/// A package unpacked into its folder, with what its signature check needs:
/// the lock and its exact bytes (which the signature covers), the signature
/// and the author's key.
#[derive(Clone, Debug)]
pub struct Unpacked {
    pub lock: FilesLock,
    pub lock_bytes: Vec<u8>,
    pub signature: Option<String>,
    pub author_key: Option<String>,
}

/// Whether `p` may name a file in a package: relative, split on `/` only,
/// and spelled one way on every system. Each part keeps to ASCII letters,
/// digits and `._@+-`, isn't `.` or `..`, doesn't end in a dot (Windows
/// drops it), and isn't a device name Windows reserves (`NUL.txt` is the
/// console's null device there). That leaves out `\`, drive letters,
/// `~` short names, `:` streams and spaces, so no two such paths can name
/// the same file, nor one outside its folder.
pub fn check_path(p: &str) -> PluginResult<()> {
    let refuse = |why: &str| {
        Err(PluginError::Invalid(format!("The plugin names a file that can't be used: {} ({why}).", shown(p))))
    };
    if p.is_empty() {
        return refuse("it's empty");
    }
    if p.len() > MAX_PATH_BYTES {
        return refuse("the path is too long");
    }
    let parts: Vec<&str> = p.split('/').collect();
    if parts.len() > MAX_DEPTH {
        return refuse("too many folders deep");
    }
    for part in parts {
        if part.is_empty() {
            return refuse("it must be a relative path without empty parts");
        }
        if part == "." || part == ".." {
            return refuse("it can't step into another folder");
        }
        if !part.bytes().all(|b| b.is_ascii_alphanumeric() || b"._@+-".contains(&b)) {
            return refuse("use only letters, digits and . _ @ + -");
        }
        if part.ends_with('.') {
            return refuse("a name can't end with a dot");
        }
        if reserved(part) {
            return refuse("Windows reserves that name");
        }
    }
    Ok(())
}

/// Whether Windows takes a file of this name for a device (`CON`, `NUL`,
/// `COM1`, also with an extension).
pub(crate) fn reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or_default().to_ascii_uppercase();
    match stem.as_bytes() {
        b"CON" | b"PRN" | b"AUX" | b"NUL" => true,
        [b'C', b'O', b'M', d] | [b'L', b'P', b'T', d] => d.is_ascii_digit(),
        _ => false,
    }
}

/// The SHA-256 of `bytes`, as `files.lock` writes it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Unpacks the package `archive` into the new folder `into` (its parent
/// must exist; a folder that is there already is an error and stays as it
/// is). What can be checked before a file is written is checked first, and
/// a package refused later leaves no `into` behind. Gives the lock,
/// signature and author key, for the signature check.
pub fn unpack(archive: &Path, into: &Path) -> PluginResult<Unpacked> {
    let file = File::open(archive)?;
    if file.metadata()?.len() > MAX_PACKAGE_BYTES {
        return Err(invalid("The plugin package is larger than 1 GB."));
    }
    let left = Cell::new(MAX_LISTING_BYTES);
    let mut zip = open(BufReader::new(file), &left)?;
    let listing = listing(&mut zip)?;
    let lock_bytes = read_beside(&mut zip, &listing, LOCK, MAX_LOCK_BYTES)?
        .ok_or_else(|| invalid("The plugin package has no files.lock."))?;
    let lock = read_lock(&lock_bytes)?;
    check_listing(&zip, &listing, &lock)?;
    let signature = read_beside(&mut zip, &listing, SIG, MAX_SIG_BYTES)?.map(|b| text(SIG, b)).transpose()?;
    let author_key =
        read_beside(&mut zip, &listing, AUTHOR_KEY, MAX_KEY_BYTES)?.map(|b| text(AUTHOR_KEY, b)).transpose()?;

    fs::create_dir(into)?;
    let unpacked = Unpacked { lock, lock_bytes, signature, author_key };
    let written = write_files(&mut zip, &listing, &unpacked, into);
    if written.is_err() {
        let _ = fs::remove_dir_all(into);
    }
    written.map(|()| unpacked)
}

/// Checks that `dir` holds each file `lock` lists, of the listed size and
/// SHA-256, as a plain file in plain folders (no link on the way), as
/// before an install commits. Files the lock doesn't list are not looked at.
/// The lock is checked first as [`unpack`] checks it, so none of its paths
/// leads out of `dir`.
pub fn verify_dir(dir: &Path, lock: &FilesLock) -> PluginResult<()> {
    check_lock(lock)?;
    let missing = |path: &str| invalid(format!("The plugin's {} is missing or is a link.", shown(path)));
    let mut folders = HashSet::new();
    for f in &lock.files {
        for folder in folders_of(&f.path) {
            if folders.insert(folder) && !fs::symlink_metadata(file_in(dir, folder)).is_ok_and(|m| m.is_dir()) {
                return Err(missing(folder));
            }
        }
        let path = file_in(dir, &f.path);
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            return Err(missing(&f.path));
        }
        let (size, sha256) = copy_hashed(File::open(&path)?, f.size, &mut io::sink())?;
        if size != f.size || sha256 != f.sha256 {
            return Err(invalid(format!("The plugin's {} doesn't match its files.lock.", shown(&f.path))));
        }
    }
    Ok(())
}

/// Checks that the package of `lock` holds what `launch` starts on
/// `platform` (a key of [`PLATFORMS`](crate::manifest::PLATFORMS)): the
/// program, marked to run unless on Windows, or the script and its lock.
pub fn check_launch(lock: &FilesLock, launch: &Launch, platform: &str) -> PluginResult<()> {
    for file in launch.files() {
        let Some(f) = lock.files.iter().find(|f| f.path == file) else {
            return Err(invalid(format!("The plugin package is missing {}, which its command needs.", shown(&file))));
        };
        if matches!(launch, Launch::Exe { .. }) && !f.exec && !platform.starts_with("windows-") {
            return Err(invalid(format!("The plugin's files.lock doesn't mark {} to run.", shown(&file))));
        }
    }
    Ok(())
}

/// Checks the plugin's icon, the `.png` or `.svg` package path `icon`, in
/// the folder `dir` that holds the files of `lock`: it is listed, no larger
/// than [`MAX_ICON_PNG`] or [`MAX_ICON_SVG`], and starts as its kind does
/// (PNG's signature; SVG text with `<svg` or `<?xml`).
pub fn check_icon(dir: &Path, lock: &FilesLock, icon: &str) -> PluginResult<()> {
    let svg = icon.to_ascii_lowercase().ends_with(".svg");
    let (kind, max) = if svg { ("SVG", MAX_ICON_SVG) } else { ("PNG", MAX_ICON_PNG) };
    let Some(f) = lock.files.iter().find(|f| f.path == icon) else {
        return Err(invalid(format!("The plugin package is missing its icon, {}.", shown(icon))));
    };
    if f.size > max {
        return Err(invalid(format!("The plugin's icon is larger than {}.", size_text(max))));
    }
    let mut start = Vec::new();
    File::open(file_in(dir, icon))?.take(256).read_to_end(&mut start)?;
    let text = start.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&start).trim_ascii_start();
    let fits = if svg {
        text.starts_with(b"<svg") || text.starts_with(b"<?xml")
    } else {
        start.starts_with(b"\x89PNG\r\n\x1a\n")
    };
    if !fits {
        return Err(invalid(format!("The plugin's icon {} isn't a {kind} image.", shown(icon))));
    }
    Ok(())
}

/// Whether `size` bytes packed into `packed` are packed tighter than a
/// package may hold them: more than [`RATIO_FROM`] bytes, over
/// [`MAX_RATIO`] times smaller. It holds for each file, and for all of them.
pub fn packed_too_tightly(size: u64, packed: u64) -> bool {
    size > RATIO_FROM && size / packed.max(1) > MAX_RATIO
}

/// Lists the plugin folder `dir` for its `files.lock`, as `asp pack` does:
/// every file below it, sorted by path, but a lock, signature and author key
/// at its top (the package's own), and version control (`.git`, `.hg`,
/// `.svn`), `.DS_Store` and `Thumbs.db` anywhere. Links and anything else
/// that isn't a plain file or folder are refused, and so is what [`unpack`]
/// would refuse. A file is marked to run when its owner may run it (Linux
/// and macOS); `asp pack` marks the programs the manifest starts too. Its
/// zip holds no folder entries: they would count toward [`MAX_ENTRIES`].
pub fn lock_folder(dir: &Path) -> PluginResult<FilesLock> {
    let mut files = Vec::new();
    list_folder(dir, "", &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let lock = FilesLock { lock_version: LOCK_VERSION, files };
    check_lock(&lock)?;
    Ok(lock)
}

fn list_folder(dir: &Path, prefix: &str, files: &mut Vec<LockedFile>) -> PluginResult<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if LEFT_OUT.iter().any(|n| name.eq_ignore_ascii_case(n)) {
            continue;
        }
        let path = if prefix.is_empty() { name.into_owned() } else { format!("{prefix}/{name}") };
        if prefix.is_empty() && BESIDE.contains(&path.as_str()) {
            continue;
        }
        check_path(&path)?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            list_folder(&entry.path(), &path, files)?;
        } else if kind.is_file() {
            let file = File::open(entry.path())?;
            let meta = file.metadata()?;
            if meta.len() > MAX_FILE_BYTES {
                return Err(invalid(format!("The plugin's {} is larger than 1 GB.", shown(&path))));
            }
            let (size, sha256) = copy_hashed(file, meta.len(), &mut io::sink())?;
            if size != meta.len() {
                return Err(invalid(format!("The plugin's {} changed while it was read.", shown(&path))));
            }
            files.push(LockedFile { path, size, sha256, exec: runs(&meta) });
            if files.len() > MAX_ENTRIES {
                return Err(too_many_files());
            }
        } else {
            return Err(invalid(format!(
                "The plugin's {} is a link or another special file, which a package can't hold.",
                shown(&path)
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn runs(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o100 != 0
}

#[cfg(not(unix))]
fn runs(_: &fs::Metadata) -> bool {
    false
}

/// What a package's zip holds, each entry checked on its own: its files by
/// name, with their index, and its folders.
struct Listing {
    files: BTreeMap<String, usize>,
    folders: Vec<String>,
}

/// Opens a package's zip and checks its listing as a whole: no zip64 end
/// claims more than a package may hold, it starts at the file's first byte,
/// names each entry once, holds at most [`MAX_ENTRIES`], and its local
/// headers agree with its directory (zip reads the directory, other tools
/// may read the local headers).
fn open<R: Read + Seek>(mut reader: R, left: &Cell<u64>) -> PluginResult<ZipArchive<Capped<'_, R>>> {
    check_zip64_ends(&mut reader)?;
    let failed = |e| if left.get() == 0 { listing_too_large() } else { damaged(e) };
    let config = Config { archive_offset: ArchiveOffset::Known(0) };
    let mut zip = ZipArchive::with_config(config, Capped { inner: reader, left }).map_err(failed)?;
    if zip.has_duplicate_names() {
        return Err(invalid("The plugin package names one file twice."));
    }
    if zip.len() > MAX_ENTRIES {
        return Err(too_many_entries());
    }
    if (0..zip.len()).filter_map(|i| zip.by_index_data(i).ok()).map(|e| e.header_start()).min().is_some_and(|s| s > 0) {
        return Err(invalid("The plugin package has data before its first file."));
    }
    zip.validate_header_consistency().map_err(failed)?;
    left.set(u64::MAX);
    Ok(zip)
}

/// Checks each zip64 end in the package's last [`TAIL_BYTES`] before zip
/// reads one. zip makes room for as many entries as a zip64 end claims
/// (some 150 bytes each), and for its record's extra data, before it reads
/// any: 1 GB of zeros ending in one would take gigabytes, which the
/// listing's byte cap doesn't bound. A zip64 end is a locator 20 bytes
/// before a directory end; zip, with the archive at offset 0, reads the
/// zip64 record only where the locator says, so that is where it is checked.
fn check_zip64_ends<R: Read + Seek>(reader: &mut R) -> PluginResult<()> {
    let start = reader.seek(SeekFrom::End(0))?.saturating_sub(TAIL_BYTES);
    reader.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    reader.by_ref().take(TAIL_BYTES).read_to_end(&mut tail)?;
    let u64_at = |bytes: &[u8], at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default());
    for (at, end) in tail.windows(24).enumerate() {
        if !end.starts_with(b"PK\x06\x07") || !end[20..].starts_with(b"PK\x05\x06") {
            continue;
        }
        // The record (56 bytes before its extra data) lies before its
        // locator, or zip doesn't read it.
        let record_at = u64_at(end, 8);
        if record_at.saturating_add(56) > start + at as u64 {
            continue;
        }
        let mut record = [0u8; 56];
        reader.seek(SeekFrom::Start(record_at))?;
        reader.read_exact(&mut record)?;
        if !record.starts_with(b"PK\x06\x06") {
            continue;
        }
        if u64_at(&record, 32) > MAX_ENTRIES as u64 {
            return Err(too_many_entries());
        }
        if u64_at(&record, 4) > MAX_LISTING_BYTES {
            return Err(listing_too_large());
        }
    }
    Ok(())
}

/// Checks each entry on its own: a [`check_path`] name, not encrypted, not
/// a link, stored or deflated, without a second name in an extra field
/// (zip and other tools would unpack it under different names), and no two
/// entries one file on Windows or macOS.
fn listing<R: Read + Seek>(zip: &mut ZipArchive<R>) -> PluginResult<Listing> {
    let mut spelled: HashMap<String, String> = HashMap::new();
    let (mut files, mut folders) = (BTreeMap::new(), Vec::new());
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).map_err(damaged)?;
        let name = String::from_utf8_lossy(entry.name_raw()).into_owned();
        let (path, folder) = match name.strip_suffix('/') {
            Some(path) => (path.to_owned(), true),
            None => (name.clone(), false),
        };
        check_path(&path)?;
        let refuse = |why: &str| Err(invalid(format!("The plugin package's {} {why}.", shown(&name))));
        if entry.encrypted() {
            return refuse("is encrypted");
        }
        if entry.is_symlink() {
            return refuse("is a link, which a package can't hold");
        }
        if !matches!(entry.compression(), CompressionMethod::Stored | CompressionMethod::Deflated) {
            return refuse("is compressed in a way AutoScene can't unpack");
        }
        if entry.extra_data_fields().any(|f| matches!(f, ExtraField::UnicodePath(_))) {
            return refuse("has a second name, which other zip tools would unpack it under");
        }
        match spelled.entry(path.to_ascii_lowercase()) {
            Entry::Occupied(e) => return Err(twice("The plugin package", e.get(), &path)),
            Entry::Vacant(e) => e.insert(path.clone()),
        };
        if folder {
            folders.push(path);
        } else {
            files.insert(path, i);
        }
    }
    Ok(Listing { files, folders })
}

/// Reads a `files.lock`, from a package or back from an installed plugin's
/// folder, and checks it as [`unpack`] does. A newer `lockVersion` is read
/// first, so a field a later AutoScene added never reads as a mistake. A
/// key named twice is refused, and so is a lock or file written as an array
/// of its values: serde reads it as the object, while other readers of a
/// signed lock would find no files in it.
pub fn read_lock(bytes: &[u8]) -> PluginResult<FilesLock> {
    let root: Value = serde_json::from_slice(bytes).map_err(|e| {
        invalid(format!("The plugin's files.lock isn't valid JSON (line {}, column {}).", e.line(), e.column()))
    })?;
    if let Ok(Twice(Some(key))) = serde_json::from_slice(bytes) {
        return Err(invalid(format!("The plugin's files.lock has \"{}\" twice in one object.", shown(&key))));
    }
    if root.get("lockVersion").and_then(Value::as_u64).is_some_and(|v| v > u64::from(LOCK_VERSION)) {
        return Err(PluginError::TooNew("lockVersion".into()));
    }
    let files = root.get("files").and_then(Value::as_array);
    if !root.is_object() || files.is_some_and(|files| !files.iter().all(Value::is_object)) {
        return Err(invalid("The plugin's files.lock must hold an object, with an object for each file."));
    }
    let lock: FilesLock = serde_json::from_slice(bytes).map_err(|e| {
        tracing::warn!(error = %e, "a plugin's files.lock is refused");
        invalid(format!("The plugin's files.lock isn't valid (line {}, column {}).", e.line(), e.column()))
    })?;
    check_lock(&lock)?;
    Ok(lock)
}

/// Checks a lock on its own: each path a [`check_path`] path, listed once,
/// no two paths (nor their folders) one file on Windows or macOS, none of
/// the files beside the listed ones, a SHA-256 of 64 lowercase hex digits,
/// sizes within the limits, and `plugin.json` among them.
fn check_lock(lock: &FilesLock) -> PluginResult<()> {
    if lock.lock_version != LOCK_VERSION {
        return Err(invalid(r#"The plugin's files.lock needs "lockVersion": 1."#));
    }
    // The zip holds the lock, its signature and the author's key too.
    if lock.files.len() > MAX_ENTRIES - BESIDE.len() {
        return Err(too_many_files());
    }
    let mut spelled: HashMap<String, (&str, bool)> = HashMap::new();
    let mut total = 0u64;
    for f in &lock.files {
        check_path(&f.path)?;
        let top = f.path.split('/').next().unwrap_or_default();
        if BESIDE.iter().any(|b| top.eq_ignore_ascii_case(b)) {
            return Err(invalid(format!("The plugin's files.lock can't list {}.", shown(&f.path))));
        }
        if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(invalid(format!(
                "The plugin's files.lock gives {} a SHA-256 that isn't 64 lowercase hex digits.",
                shown(&f.path)
            )));
        }
        if f.size > MAX_FILE_BYTES {
            return Err(invalid(format!("The plugin's {} is larger than 1 GB.", shown(&f.path))));
        }
        total = total.saturating_add(f.size);
        for folder in folders_of(&f.path) {
            match spelled.entry(folder.to_ascii_lowercase()) {
                Entry::Occupied(e) if *e.get() == (folder, true) => {}
                Entry::Occupied(e) => return Err(twice("The plugin's files.lock", e.get().0, folder)),
                Entry::Vacant(e) => {
                    e.insert((folder, true));
                }
            }
        }
        match spelled.entry(f.path.to_ascii_lowercase()) {
            Entry::Occupied(e) => return Err(twice("The plugin's files.lock", e.get().0, &f.path)),
            Entry::Vacant(e) => e.insert((f.path.as_str(), false)),
        };
    }
    if total > MAX_UNPACKED_BYTES {
        return Err(invalid("The plugin unpacks to more than 2 GB."));
    }
    if !lock.files.iter().any(|f| f.path == MANIFEST) {
        return Err(invalid("The plugin's files.lock doesn't list plugin.json."));
    }
    Ok(())
}

/// Checks the zip against its lock: it holds exactly the listed files
/// (beside the lock, its signature and the author's key), folders only on
/// their way, and each file is the listed size. A file over [`RATIO_FROM`]
/// bytes is packed no more than [`MAX_RATIO`] times smaller, and so are all
/// files together when they are over it: many files of just that size could
/// unpack to a thousand times the package.
fn check_listing<R: Read + Seek>(zip: &ZipArchive<R>, listing: &Listing, lock: &FilesLock) -> PluginResult<()> {
    let listed: HashSet<&str> = lock.files.iter().map(|f| f.path.as_str()).collect();
    if let Some(extra) = listing.files.keys().find(|p| !listed.contains(p.as_str()) && !BESIDE.contains(&p.as_str())) {
        return Err(invalid(format!("The plugin package holds a file its list doesn't name: {}.", shown(extra))));
    }
    let on_the_way: HashSet<&str> = lock.files.iter().flat_map(|f| folders_of(&f.path)).collect();
    if let Some(extra) = listing.folders.iter().find(|d| !on_the_way.contains(d.as_str())) {
        return Err(invalid(format!("The plugin package holds a folder its list doesn't name: {}.", shown(extra))));
    }
    let (mut size, mut packed) = (0u64, 0u64);
    for f in &lock.files {
        let Some(&i) = listing.files.get(&f.path) else {
            return Err(invalid(format!("The plugin package is missing {}.", shown(&f.path))));
        };
        let entry = zip.by_index_data(i).map_err(damaged)?;
        if entry.size() != f.size {
            return Err(wrong_size(&f.path));
        }
        if packed_too_tightly(f.size, entry.compressed_size()) {
            return Err(invalid(format!(
                "The plugin package's {} unpacks to over {MAX_RATIO} times its packed size.",
                shown(&f.path)
            )));
        }
        size = size.saturating_add(f.size);
        packed = packed.saturating_add(entry.compressed_size());
    }
    if packed_too_tightly(size, packed) {
        return Err(invalid(format!("The plugin package unpacks to over {MAX_RATIO} times its packed size.")));
    }
    Ok(())
}

/// Reads the zip's `name` (one of the files beside the listed ones) whole,
/// if it holds one; it may be at most `max` bytes.
fn read_beside<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    listing: &Listing,
    name: &str,
    max: u64,
) -> PluginResult<Option<Vec<u8>>> {
    let Some(&i) = listing.files.get(name) else { return Ok(None) };
    let too_large = || invalid(format!("The plugin package's {name} is larger than {}.", size_text(max)));
    let mut bytes = Vec::new();
    zip.by_index(i).map_err(damaged)?.take(max + 1).read_to_end(&mut bytes).map_err(|e| unreadable(name, e))?;
    if bytes.len() as u64 > max {
        return Err(too_large());
    }
    Ok(Some(bytes))
}

/// A limit as a message names it: 4 KB, 1 MB.
fn size_text(bytes: u64) -> String {
    if bytes >= 1 << 20 { format!("{} MB", bytes >> 20) } else { format!("{} KB", bytes >> 10) }
}

fn text(name: &str, bytes: Vec<u8>) -> PluginResult<String> {
    String::from_utf8(bytes).map_err(|_| invalid(format!("The plugin package's {name} isn't text.")))
}

/// Unpacks each listed file into `into`, streamed through SHA-256 and cut
/// off at its listed size, then writes the files beside them.
fn write_files<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    listing: &Listing,
    unpacked: &Unpacked,
    into: &Path,
) -> PluginResult<()> {
    for f in &unpacked.lock.files {
        let target = file_in(into, &f.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = create(&target, f.exec)?;
        let entry = zip.by_index(listing.files[&f.path]).map_err(damaged)?;
        let (size, sha256) = copy_hashed(entry, f.size, &mut out).map_err(|e| unreadable(&f.path, e))?;
        // Its header said the listed size, while its data says another.
        if size != f.size {
            return Err(invalid(format!(
                "The plugin package's {} doesn't unpack to the size its list says.",
                shown(&f.path)
            )));
        }
        if sha256 != f.sha256 {
            return Err(invalid(format!(
                "The plugin package's {} was changed after the package was made.",
                shown(&f.path)
            )));
        }
    }
    create(&into.join(LOCK), false)?.write_all(&unpacked.lock_bytes)?;
    if let Some(signature) = &unpacked.signature {
        create(&into.join(SIG), false)?.write_all(signature.as_bytes())?;
    }
    if let Some(key) = &unpacked.author_key {
        create(&into.join(AUTHOR_KEY), false)?.write_all(key.as_bytes())?;
    }
    Ok(())
}

/// Creates the new file `path`, never one that is there (nor through a
/// link), with the mode a package gives on Unix: 0o755 for a program, 0o644
/// for anything else. The zip's own mode is never used.
fn create(path: &Path, exec: bool) -> io::Result<File> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(if exec { 0o755 } else { 0o644 }))?;
    }
    #[cfg(not(unix))]
    let _ = exec;
    Ok(file)
}

/// Reads at most `size + 1` bytes of `from` and writes the first `size` of
/// them to `to`. Gives how many bytes came, and the SHA-256 of the bytes
/// written.
fn copy_hashed(from: impl Read, size: u64, to: &mut impl Write) -> io::Result<(u64, String)> {
    let mut from = from.take(size.saturating_add(1));
    let (mut hash, mut count, mut buf) = (Sha256::new(), 0u64, vec![0u8; 64 << 10]);
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        count += n as u64;
        if count > size {
            break;
        }
        hash.update(&buf[..n]);
        to.write_all(&buf[..n])?;
    }
    Ok((count, hex(&hash.finalize())))
}

/// The reader under zip while it lists a package: it fails once it has read
/// `left` bytes, so a directory of millions of tiny entries is refused
/// before zip holds them all in memory ([`MAX_ENTRIES`] can only be checked
/// once it has). What zip makes room for before it reads is bounded by
/// [`check_zip64_ends`].
struct Capped<'a, R> {
    inner: R,
    left: &'a Cell<u64>,
}

impl<R: Read> Read for Capped<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.left.get();
        if left == 0 && !buf.is_empty() {
            return Err(io::Error::new(ErrorKind::FileTooLarge, "the package's listing is too large"));
        }
        let max = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..max])?;
        self.left.set(left - n as u64);
        Ok(n)
    }
}

impl<R: Seek> Seek for Capped<'_, R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// The folders on the way to package path `path`, outermost first.
fn folders_of(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(|(end, _)| &path[..end])
}

/// The file of package path `path` (a [`check_path`] path) in `dir`.
pub fn file_in(dir: &Path, path: &str) -> PathBuf {
    path.split('/').fold(dir.to_path_buf(), |file, part| file.join(part))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One name twice in `whose` list, or two that Windows and macOS take for
/// one file (they differ only in case).
fn twice(whose: &str, a: &str, b: &str) -> PluginError {
    if a == b {
        invalid(format!("{whose} names {} twice.", shown(a)))
    } else {
        invalid(format!("{whose} names {} and {}, which are one file on Windows and macOS.", shown(a), shown(b)))
    }
}

fn too_many_entries() -> PluginError {
    invalid(format!("The plugin package holds more than {MAX_ENTRIES} files."))
}

fn listing_too_large() -> PluginError {
    invalid("The plugin package's list of files is too large.")
}

fn too_many_files() -> PluginError {
    invalid(format!("The plugin has more than {} files.", MAX_ENTRIES - BESIDE.len()))
}

fn wrong_size(path: &str) -> PluginError {
    invalid(format!("The plugin package's {} isn't the size its list says.", shown(path)))
}

/// Whether an error reading a zip means it is damaged, rather than that the
/// disk failed.
fn broken(e: &io::Error) -> bool {
    matches!(e.kind(), ErrorKind::InvalidData | ErrorKind::UnexpectedEof | ErrorKind::InvalidInput)
}

fn damaged(e: ZipError) -> PluginError {
    match e {
        ZipError::Io(e) if !broken(&e) => e.into(),
        e => {
            tracing::warn!(error = %e, "a plugin package is damaged");
            invalid("The plugin package is damaged.")
        }
    }
}

/// An error while `path` was unpacked: the zip's (damaged) or the disk's.
fn unreadable(path: &str, e: io::Error) -> PluginError {
    if !broken(&e) {
        return e.into();
    }
    tracing::warn!(file = path, error = %e, "a plugin package's file is damaged");
    invalid(format!("The plugin package's {} is damaged.", shown(path)))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn package_paths_are_relative_and_spelled_one_way() {
        for ok in ["plugin.json", "bin/tool.exe", "lib/a-b_c@2+x/main.py", ".hidden", "CONFIG.txt", "com10"] {
            assert!(check_path(ok).is_ok(), "{ok}");
        }
        let long = "a/".repeat(MAX_DEPTH) + "b";
        let wide = "a".repeat(MAX_PATH_BYTES + 1);
        for bad in [
            "",
            "/abs",
            "C:x",
            "a/../b",
            "../evil",
            "./a",
            "a//b",
            "a/",
            r"bin\x.exe",
            "a b",
            "a.txt ",
            "a\tb",
            "a\0b",
            "x:ads",
            "PLUGIN~1.JSO",
            "plugin.json.",
            "NUL.txt",
            "con",
            "lpt1.log",
            "nämlich.txt",
            &long,
            &wide,
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }

    /// Hands out its bytes two at a time.
    struct Dribble<'a>(&'a [u8]);

    impl Read for Dribble<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.0.len().min(buf.len()).min(2);
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn nothing_past_the_listed_size_is_written() {
        let mut out = Vec::new();
        assert_eq!(copy_hashed(Dribble(b"hello world"), 4, &mut out).unwrap().0, 5);
        assert_eq!(out, b"hell");
        let mut out = Vec::new();
        assert_eq!(copy_hashed(Dribble(b"hello"), 5, &mut out).unwrap(), (5, sha256_hex(b"hello")));
        assert_eq!(out, b"hello");
    }

    fn zip_of(names: impl Iterator<Item = String>) -> Vec<u8> {
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            w.start_file(name, SimpleFileOptions::default()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn the_largest_listing_fits_its_byte_cap() {
        // Paths of the longest length, as many as a package may hold.
        let zip = zip_of((0..MAX_ENTRIES).map(|i| format!("{i:0>width$}", width = MAX_PATH_BYTES)));
        let left = Cell::new(MAX_LISTING_BYTES);
        assert_eq!(open(Cursor::new(zip), &left).unwrap().len(), MAX_ENTRIES);
        assert_eq!(left.get(), u64::MAX, "the cap is lifted for unpacking");
    }

    fn lock_of(files: &[(&str, u64, bool)]) -> FilesLock {
        let files = files.iter().map(|&(path, size, exec)| LockedFile {
            path: path.into(),
            size,
            sha256: sha256_hex(b""),
            exec,
        });
        FilesLock { lock_version: LOCK_VERSION, files: files.collect() }
    }

    #[test]
    fn a_command_needs_its_files_in_the_lock() {
        let lock = lock_of(&[("bin/tool", 2, false), ("bin/tool.exe", 2, true), ("main.py", 2, false)]);
        let exe = |program: &str| Launch::Exe { program: program.into(), args: Vec::new() };
        assert!(check_launch(&lock, &exe("bin/tool.exe"), "linux-x86_64").is_ok());
        // Only Windows runs a program the lock doesn't mark.
        assert!(check_launch(&lock, &exe("bin/tool"), "windows-x86_64").is_ok());
        let Err(PluginError::Invalid(m)) = check_launch(&lock, &exe("bin/tool"), "macos-aarch64") else { panic!() };
        assert_eq!(m, "The plugin's files.lock doesn't mark bin/tool to run.");
        let Err(PluginError::Invalid(m)) = check_launch(&lock, &exe("bin/other"), "windows-x86_64") else { panic!() };
        assert_eq!(m, "The plugin package is missing bin/other, which its command needs.");
        let uv = Launch::UvScript { script: "main.py".into() };
        let Err(PluginError::Invalid(m)) = check_launch(&lock, &uv, "linux-x86_64") else { panic!() };
        assert_eq!(m, "The plugin package is missing main.py.lock, which its command needs.");
    }

    #[test]
    fn an_icon_is_a_small_png_or_svg() {
        let dir = tempfile::tempdir().unwrap();
        let icons: [(&str, &[u8]); 4] = [
            ("icon.png", b"\x89PNG\r\n\x1a\n...."),
            ("icon.svg", b"\xEF\xBB\xBF\n  <svg xmlns='http://www.w3.org/2000/svg'/>"),
            ("plain.svg", b"<?xml version='1.0'?><svg/>"),
            ("fake.png", b"<svg/>"),
        ];
        for (name, bytes) in icons {
            fs::write(dir.path().join(name), bytes).unwrap();
        }
        let lock = lock_of(&[
            ("icon.png", 12, false),
            ("icon.svg", 50, false),
            ("plain.svg", 27, false),
            ("fake.png", 6, false),
        ]);
        let check = |icon: &str| match check_icon(dir.path(), &lock, icon) {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        };
        assert_eq!((check("icon.png"), check("icon.svg"), check("plain.svg")), Default::default());
        assert_eq!(check("fake.png"), "The plugin's icon fake.png isn't a PNG image.");
        assert_eq!(check("gone.svg"), "The plugin package is missing its icon, gone.svg.");
        let lock = lock_of(&[("icon.svg", MAX_ICON_SVG + 1, false)]);
        let Err(e) = check_icon(dir.path(), &lock, "icon.svg") else { panic!() };
        assert_eq!(e.to_string(), "The plugin's icon is larger than 64 KB.");
    }

    #[test]
    fn a_listing_past_its_byte_cap_is_refused() {
        let zip = zip_of((0..100).map(|i| format!("file{i}")));
        let Err(PluginError::Invalid(message)) = open(Cursor::new(zip), &Cell::new(2000)) else { panic!() };
        assert_eq!(message, "The plugin package's list of files is too large.");
    }
}
