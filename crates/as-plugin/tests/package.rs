//! The package reader against packages made to do harm (the proofs of the
//! PR #93 review) and against its limits. The packages are built here with
//! ZipWriter; where it won't write one, its headers are rewritten after.
//! Each test works in a folder of its own inside an outer one that holds
//! nothing else, and walks the outer folder to show nothing landed beside it.

use std::fs;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use as_plugin::package::{self, AUTHOR_KEY, LOCK, MAX_ENTRIES, MAX_FILE_BYTES, MAX_PACKAGE_BYTES, RATIO_FROM, SIG};
use as_plugin::{FilesLock, LockedFile, PluginError, PluginResult, Unpacked};
use serde_json::{Value, json};
use zip::write::{FullFileOptions, SimpleFileOptions};
use zip::{CompressionMethod, ZipWriter};

/// A test's folders: `outer` holds only `dir`, the test's own, which holds
/// the package and the folder it unpacks into.
struct Place {
    outer: tempfile::TempDir,
    dir: PathBuf,
}

impl Place {
    fn new() -> Self {
        let outer = tempfile::tempdir().unwrap();
        let dir = outer.path().join("test");
        fs::create_dir(&dir).unwrap();
        Self { outer, dir }
    }

    fn target(&self) -> PathBuf {
        self.dir.join("into")
    }

    fn unpack(&self, zip: &[u8]) -> PluginResult<Unpacked> {
        let package = self.dir.join("plugin.asp");
        fs::write(&package, zip).unwrap();
        package::unpack(&package, &self.target())
    }

    /// Everything below the outer folder: relative paths with `/`, sorted.
    fn left(&self) -> Vec<String> {
        fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = format!("{prefix}{}", entry.file_name().to_string_lossy());
                if entry.file_type().unwrap().is_dir() {
                    walk(&entry.path(), &format!("{path}/"), out);
                }
                out.push(path);
            }
        }
        let mut out = Vec::new();
        walk(self.outer.path(), "", &mut out);
        out.sort();
        out
    }

    /// Unpacks `zip`, which must be refused and leave nothing but itself;
    /// gives the message.
    fn refused(&self, zip: &[u8]) -> String {
        let message = match self.unpack(zip) {
            Err(PluginError::Invalid(message)) => message,
            r => panic!("expected Invalid, got {r:?}"),
        };
        assert_eq!(self.left(), ["test", "test/plugin.asp"], "{message}");
        message
    }
}

type Files = Vec<(String, Vec<u8>)>;

fn plugin() -> Files {
    vec![
        ("plugin.json".into(), br#"{"manifestVersion": 1}"#.to_vec()),
        ("bin/tool".into(), b"#!/bin/sh\necho hello\n".to_vec()),
        ("lib/data.txt".into(), b"hello".to_vec()),
    ]
}

/// The lock of `files`; what lies in bin/ runs.
fn lock_of(files: &Files) -> FilesLock {
    let files = files
        .iter()
        .map(|(path, bytes)| LockedFile {
            path: path.clone(),
            size: bytes.len() as u64,
            sha256: package::sha256_hex(bytes),
            exec: path.starts_with("bin/"),
        })
        .collect();
    FilesLock { lock_version: 1, files }
}

/// `files` and `lock`, as the entries of a package.
fn with_lock_bytes(files: &Files, lock: &[u8]) -> Files {
    let mut entries = files.clone();
    entries.push((LOCK.into(), lock.to_vec()));
    entries
}

/// `files` and their lock, as the entries of a package.
fn with_lock(files: &Files) -> Files {
    with_lock_bytes(files, &serde_json::to_vec(&lock_of(files)).unwrap())
}

fn with(mut entries: Files, path: &str, bytes: &[u8]) -> Files {
    entries.push((path.into(), bytes.to_vec()));
    entries
}

fn deflated() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

fn stored() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
}

fn writer() -> ZipWriter<Cursor<Vec<u8>>> {
    ZipWriter::new(Cursor::new(Vec::new()))
}

fn add<W: Write + Seek>(w: &mut ZipWriter<W>, entries: &Files, options: SimpleFileOptions) {
    for (name, bytes) in entries {
        w.start_file(name.as_str(), options).unwrap();
        w.write_all(bytes).unwrap();
    }
}

fn zip_with(entries: &Files, options: SimpleFileOptions) -> Vec<u8> {
    let mut w = writer();
    add(&mut w, entries, options);
    w.finish().unwrap().into_inner()
}

fn zip_of(entries: &Files) -> Vec<u8> {
    zip_with(entries, deflated())
}

/// A field of a zip header, at its offset in the local and in the central one.
#[derive(Clone, Copy)]
enum Field {
    Flags,
    Method,
    Size,
    Name,
}

impl Field {
    fn offsets(self) -> (usize, usize) {
        match self {
            Self::Flags => (6, 8),
            Self::Method => (8, 10),
            Self::Size => (22, 24),
            Self::Name => (30, 46),
        }
    }
}

/// Rewrites `field` of the entry `name` in both its headers, as ZipWriter
/// won't write it. `zip` is ZipWriter's, without a comment.
fn patch(zip: &mut [u8], name: &str, field: Field, value: &[u8]) {
    patch_in(zip, name, field, value, true);
}

/// Rewrites `field` of the entry `name` in its local header only, which a
/// tool that reads a zip front to back goes by.
fn patch_local(zip: &mut [u8], name: &str, field: Field, value: &[u8]) {
    patch_in(zip, name, field, value, false);
}

fn patch_in(zip: &mut [u8], name: &str, field: Field, value: &[u8], central_too: bool) {
    let u16_at = |zip: &[u8], at: usize| usize::from(u16::from_le_bytes([zip[at], zip[at + 1]]));
    let u32_at = |zip: &[u8], at: usize| u32::from_le_bytes(zip[at..at + 4].try_into().unwrap()) as usize;
    let end = zip.len() - 22;
    let mut central = u32_at(zip, end + 16);
    for _ in 0..u16_at(zip, end + 10) {
        let len = u16_at(zip, central + 28);
        if &zip[central + 46..central + 46 + len] == name.as_bytes() {
            let local = u32_at(zip, central + 42);
            let (in_local, in_central) = field.offsets();
            zip[local + in_local..][..value.len()].copy_from_slice(value);
            if central_too {
                zip[central + in_central..][..value.len()].copy_from_slice(value);
            }
            return;
        }
        central += 46 + len + u16_at(zip, central + 30) + u16_at(zip, central + 32);
    }
    panic!("{name} isn't in the zip");
}

/// `zip` (ZipWriter's, without a comment) ending in a zip64 end, whose
/// record claims `entries` entries and is `record` bytes long after its
/// first 12.
fn zip64(zip: &[u8], entries: u64, record: u64) -> Vec<u8> {
    let end = zip.len() - 22;
    let u32_at = |at: usize| u64::from(u32::from_le_bytes(zip[end + at..end + at + 4].try_into().unwrap()));
    let mut out = zip[..end].to_vec();
    let record_at = out.len() as u64;
    out.extend(b"PK\x06\x06");
    out.extend(record.to_le_bytes());
    // Made by, needed, this disk and the directory's disk.
    out.extend([45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend(entries.to_le_bytes());
    out.extend(entries.to_le_bytes());
    out.extend(u32_at(12).to_le_bytes());
    out.extend(u32_at(16).to_le_bytes());
    out.extend(b"PK\x06\x07\0\0\0\0");
    out.extend(record_at.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    // The end every reader looks for, which says to read the zip64 one.
    out.extend(b"PK\x05\x06\0\0\0\0\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\0\0");
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[test]
fn a_package_unpacks_into_its_folder() {
    let place = Place::new();
    let files = plugin();
    let lock = serde_json::to_vec_pretty(&lock_of(&files)).unwrap();
    let mut w = writer();
    w.add_directory("lib/", deflated()).unwrap();
    add(&mut w, &with_lock_bytes(&files, &lock), deflated());
    let beside =
        vec![(SIG.into(), b"untrusted comment: test\nRWQ\n".to_vec()), (AUTHOR_KEY.into(), b"RWQkey".to_vec())];
    add(&mut w, &beside, stored());
    let unpacked = place.unpack(&w.finish().unwrap().into_inner()).unwrap();

    assert_eq!(unpacked.lock, lock_of(&files));
    assert_eq!(unpacked.lock_bytes, lock);
    assert_eq!(unpacked.signature.as_deref(), Some("untrusted comment: test\nRWQ\n"));
    assert_eq!(unpacked.author_key.as_deref(), Some("RWQkey"));
    for (path, bytes) in files.iter().chain(&beside) {
        assert_eq!(&fs::read(place.target().join(path)).unwrap(), bytes, "{path}");
    }
    assert_eq!(fs::read(place.target().join(LOCK)).unwrap(), lock);
    package::verify_dir(&place.target(), &unpacked.lock).unwrap();
    assert_eq!(
        place.left(),
        [
            "test",
            "test/into",
            "test/into/author.pub",
            "test/into/bin",
            "test/into/bin/tool",
            "test/into/files.lock",
            "test/into/files.lock.minisig",
            "test/into/lib",
            "test/into/lib/data.txt",
            "test/into/plugin.json",
            "test/plugin.asp",
        ]
    );
}

#[test]
fn a_traversing_entry_is_refused() {
    for name in ["../evil", "../../evil", "/abs", "C:x", "C:/x", "a/../b", "a/../../evil"] {
        let place = Place::new();
        let message = place.refused(&zip_of(&with(with_lock(&plugin()), name, b"x")));
        assert!(message.contains("can't be used"), "{name}: {message}");
    }
    // Named only by the lock.
    let place = Place::new();
    let mut lock = lock_of(&plugin());
    lock.files.push(LockedFile { path: "../evil".into(), size: 1, sha256: package::sha256_hex(b"x"), exec: false });
    let message = place.refused(&zip_of(&with_lock_bytes(&plugin(), &serde_json::to_vec(&lock).unwrap())));
    assert_eq!(message, "The plugin names a file that can't be used: ../evil (it can't step into another folder).");
    // Nor does a check of a folder go where such a lock says.
    let Err(PluginError::Invalid(message)) = package::verify_dir(&place.dir, &lock) else { panic!() };
    assert_eq!(message, "The plugin names a file that can't be used: ../evil (it can't step into another folder).");
}

#[test]
fn an_unlisted_entry_is_refused() {
    let place = Place::new();
    let message = place.refused(&zip_of(&with(with_lock(&plugin()), "lib/extra.py", b"print(1)")));
    assert_eq!(message, "The plugin package holds a file its list doesn't name: lib/extra.py.");

    let mut w = writer();
    w.add_directory("empty/", deflated()).unwrap();
    add(&mut w, &with_lock(&plugin()), deflated());
    let message = place.refused(&w.finish().unwrap().into_inner());
    assert_eq!(message, "The plugin package holds a folder its list doesn't name: empty.");
}

#[test]
fn a_listed_file_that_is_missing_is_refused() {
    let place = Place::new();
    let lock = lock_of(&with(plugin(), "lib/gone.txt", b"x"));
    let message = place.refused(&zip_of(&with_lock_bytes(&plugin(), &serde_json::to_vec(&lock).unwrap())));
    assert_eq!(message, "The plugin package is missing lib/gone.txt.");
}

#[test]
fn aliased_entries_are_refused() {
    for name in [
        "plugin.json.",
        "PLUGIN~1.JSO",
        r"bin\x.exe",
        "a/./b",
        "a//b",
        "NUL.txt",
        "lib/con",
        "COM1",
        "x:ads",
        "plugin.json:stream",
        "a b",
        "a.txt ",
        "a\u{7}b",
        "nämlich.txt",
    ] {
        let place = Place::new();
        let message = place.refused(&zip_of(&with(with_lock(&plugin()), name, b"x")));
        assert!(message.contains("can't be used"), "{name}: {message}");
    }

    // A second name in an Info-ZIP Unicode Path field, which zip itself
    // reads instead of the first; other tools may not.
    let place = Place::new();
    let mut w = writer();
    let mut second = vec![1];
    second.extend(crc32(b"lib/data.txt").to_le_bytes());
    second.extend(b"lib/evil.txt");
    let mut options = FullFileOptions::default();
    options.add_extra_field(0x7075, second, false).unwrap();
    w.start_file("lib/data.txt", options).unwrap();
    w.write_all(b"hello").unwrap();
    let rest: Files = with_lock(&plugin()).into_iter().filter(|(path, _)| path != "lib/data.txt").collect();
    add(&mut w, &rest, deflated());
    let message = place.refused(&w.finish().unwrap().into_inner());
    assert!(message.contains("has a second name"), "{message}");
}

#[test]
fn entries_that_differ_only_in_case_are_refused() {
    let place = Place::new();
    let message = place.refused(&zip_of(&with_lock(&with(plugin(), "bin/Tool", b"other"))));
    assert_eq!(message, "The plugin package names bin/tool and bin/Tool, which are one file on Windows and macOS.");

    // Their folders, each entry spelled one way.
    let message = place.refused(&zip_of(&with_lock(&with(plugin(), "Lib/other.txt", b"other"))));
    assert_eq!(message, "The plugin's files.lock names lib and Lib, which are one file on Windows and macOS.");

    // A file and a folder.
    let message = place.refused(&zip_of(&with_lock(&with(with(plugin(), "data", b"a"), "data/b", b"b"))));
    assert_eq!(message, "The plugin's files.lock names data twice.");
}

#[test]
fn two_manifests_are_refused() {
    // Twice under one name, which zip alone would read as one entry.
    let place = Place::new();
    let mut zip = zip_of(&with(with_lock(&plugin()), "plugin.jsoX", br#"{"manifestVersion": 1, "id": "b.b"}"#));
    patch(&mut zip, "plugin.jsoX", Field::Name, b"plugin.json");
    assert_eq!(place.refused(&zip), "The plugin package names one file twice.");

    let message = place.refused(&zip_of(&with(with_lock(&plugin()), "Plugin.json", b"{}")));
    assert_eq!(
        message,
        "The plugin package names plugin.json and Plugin.json, which are one file on Windows and macOS."
    );
}

#[test]
fn a_file_larger_than_its_lock_is_refused() {
    let place = Place::new();
    let mut lock = lock_of(&plugin());
    lock.files[2].size = 3;
    lock.files[2].sha256 = package::sha256_hex(b"hel");
    let message = place.refused(&zip_of(&with_lock_bytes(&plugin(), &serde_json::to_vec(&lock).unwrap())));
    assert_eq!(message, "The plugin package's lib/data.txt isn't the size its list says.");

    // Its header agrees with the lock, while its data unpacks to more. An
    // unpacker that stopped where the header says would find the hash right.
    let big = vec![b'A'; 64 << 10];
    let lock = serde_json::to_vec(&lock_of(&with(plugin(), "lib/big.bin", &big[..1000]))).unwrap();
    let mut zip = zip_of(&with_lock_bytes(&with(plugin(), "lib/big.bin", &big), &lock));
    patch(&mut zip, "lib/big.bin", Field::Size, &1000u32.to_le_bytes());
    assert_eq!(place.refused(&zip), "The plugin package's lib/big.bin is damaged.");
}

#[test]
fn a_local_header_that_disagrees_with_the_directory_is_refused() {
    let place = Place::new();
    for (field, value) in [(Field::Name, b"lib/evil.txt".as_slice()), (Field::Size, &4u32.to_le_bytes())] {
        let mut zip = zip_of(&with_lock(&plugin()));
        patch_local(&mut zip, "lib/data.txt", field, value);
        assert_eq!(place.refused(&zip), "The plugin package is damaged.");
    }
}

#[test]
fn too_many_entries_are_refused() {
    let place = Place::new();
    let mut entries = with_lock(&plugin());
    let listed = entries.len();
    entries.extend((listed..=MAX_ENTRIES).map(|i| (format!("f/{i}"), Vec::new())));
    let message = place.refused(&zip_with(&entries, stored()));
    assert_eq!(message, "The plugin package holds more than 4096 files.");

    // A lock that lists more than fit beside the lock, its signature and the author's key.
    let files: Files =
        plugin().into_iter().chain((3..MAX_ENTRIES - 2).map(|i| (format!("f/{i}"), Vec::new()))).collect();
    let message = place.refused(&zip_with(&with_lock(&files), stored()));
    assert_eq!(message, "The plugin has more than 4093 files.");
}

#[test]
fn a_zip64_end_that_claims_too_much_is_refused() {
    let place = Place::new();
    let entries = with_lock(&plugin());
    let zip = zip_of(&entries);
    // One that tells the truth is read as any other end.
    place.unpack(&zip64(&zip, entries.len() as u64, 44)).unwrap();
    fs::remove_dir_all(place.target()).unwrap();

    // zip makes room for every entry an end claims before it reads one,
    // and for the extra data a zip64 record claims.
    let message = place.refused(&zip64(&zip, MAX_ENTRIES as u64 + 1, 44));
    assert_eq!(message, "The plugin package holds more than 4096 files.");
    let message = place.refused(&zip64(&zip, entries.len() as u64, 1 << 30));
    assert_eq!(message, "The plugin package's list of files is too large.");

    // Ends inside a file of the package aren't its own: where their
    // locators point, past the package or at no zip64 record, zip reads
    // nothing either.
    let mut inner = Vec::new();
    for record_at in [1u64 << 40, 0] {
        inner.extend(b"PK\x06\x07\0\0\0\0");
        inner.extend(record_at.to_le_bytes());
        inner.extend(1u32.to_le_bytes());
        inner.extend(b"PK\x05\x06");
        inner.extend([0xff; 18]);
    }
    place.unpack(&zip_with(&with_lock(&with(plugin(), "lib/inner.zip", &inner)), stored())).unwrap();
}

#[test]
fn a_zip_bomb_stops_at_the_limit() {
    let place = Place::new();
    let zeros = vec![0; 2 << 20];
    let files = with(plugin(), "lib/zeros.bin", &zeros);
    let message = place.refused(&zip_of(&with_lock(&files)));
    assert_eq!(message, "The plugin package's lib/zeros.bin unpacks to over 100 times its packed size.");
    // The same file stored is no bomb.
    place.unpack(&zip_with(&with_lock(&files), stored())).unwrap();
    assert_eq!(fs::read(place.target().join("lib/zeros.bin")).unwrap(), zeros);
    fs::remove_dir_all(place.target()).unwrap();

    // A file up to 1 MB may pack as tightly as it likes, while many of them
    // together may not.
    let files = with(plugin(), "lib/zeros.bin", &zeros[..(RATIO_FROM - 4096) as usize]);
    place.unpack(&zip_of(&with_lock(&files))).unwrap();
    fs::remove_dir_all(place.target()).unwrap();
    let files = (0..3).fold(plugin(), |files, i| with(files, &format!("lib/{i}.bin"), &zeros[..RATIO_FROM as usize]));
    let message = place.refused(&zip_of(&with_lock(&files)));
    assert_eq!(message, "The plugin package unpacks to over 100 times its packed size.");

    // Sizes a lock declares are refused before anything is read.
    let mut lock = lock_of(&plugin());
    for name in ["a", "b", "c"] {
        lock.files.push(LockedFile { path: name.into(), size: MAX_FILE_BYTES, sha256: "0".repeat(64), exec: false });
    }
    let small = with(with(with(plugin(), "a", b"x"), "b", b"x"), "c", b"x");
    let message = place.refused(&zip_of(&with_lock_bytes(&small, &serde_json::to_vec(&lock).unwrap())));
    assert_eq!(message, "The plugin unpacks to more than 2 GB.");
    lock.files[3].size = MAX_FILE_BYTES + 1;
    let message = place.refused(&zip_of(&with_lock_bytes(&small, &serde_json::to_vec(&lock).unwrap())));
    assert_eq!(message, "The plugin's a is larger than 1 GB.");

    // A lock or signature larger than any real one.
    let message = place.refused(&zip_of(&with_lock_bytes(&plugin(), &vec![b' '; (1 << 20) + 1])));
    assert_eq!(message, "The plugin package's files.lock is larger than 1 MB.");
    let message = place.refused(&zip_of(&with(with_lock(&plugin()), SIG, &[b'a'; 5000])));
    assert_eq!(message, "The plugin package's files.lock.minisig is larger than 4 KB.");
    let message = place.refused(&zip_of(&with(with_lock(&plugin()), AUTHOR_KEY, &[b'a'; 1025])));
    assert_eq!(message, "The plugin package's author.pub is larger than 1 KB.");
    let message = place.refused(&zip_of(&with(with_lock(&plugin()), SIG, b"RW\xff")));
    assert_eq!(message, "The plugin package's files.lock.minisig isn't text.");
    let message = place.refused(&zip_of(&with(with_lock(&plugin()), AUTHOR_KEY, b"RW\xff")));
    assert_eq!(message, "The plugin package's author.pub isn't text.");
}

#[test]
fn a_package_larger_than_1_gb_is_refused_before_it_is_read() {
    let place = Place::new();
    let package = place.dir.join("plugin.asp");
    // Sparse where the file system can: the test writes nothing much.
    fs::File::create(&package).unwrap().set_len(MAX_PACKAGE_BYTES + 1).unwrap();
    let message = match package::unpack(&package, &place.target()) {
        Err(PluginError::Invalid(message)) => message,
        r => panic!("expected Invalid, got {r:?}"),
    };
    assert_eq!(message, "The plugin package is larger than 1 GB.");
    assert_eq!(place.left(), ["test", "test/plugin.asp"]);
}

#[test]
fn data_before_the_archive_is_refused() {
    let place = Place::new();
    let zip = zip_of(&with_lock(&plugin()));
    let message = place.refused(&[b"MZ, a program to run first".as_slice(), &zip].concat());
    assert_eq!(message, "The plugin package is damaged.");

    // The same with the archive's offsets moved past it, as `zip -A` does.
    let mut cursor = Cursor::new(b"MZ, a program to run first".to_vec());
    cursor.seek(SeekFrom::End(0)).unwrap();
    let mut w = ZipWriter::new(cursor);
    add(&mut w, &with_lock(&plugin()), deflated());
    let message = place.refused(&w.finish().unwrap().into_inner());
    assert_eq!(message, "The plugin package has data before its first file.");
}

#[test]
fn encrypted_and_unknown_compression_are_refused() {
    let place = Place::new();
    let stored = zip_with(&with_lock(&plugin()), stored());
    let mut zip = stored.clone();
    patch(&mut zip, "lib/data.txt", Field::Flags, &1u16.to_le_bytes());
    assert_eq!(place.refused(&zip), "The plugin package's lib/data.txt is encrypted.");

    // Shrink, implode, deflate64, bzip2, LZMA, zstd, xz, PPMd and AES.
    for method in [1u16, 6, 9, 12, 14, 93, 95, 98, 99] {
        let mut zip = stored.clone();
        patch(&mut zip, "lib/data.txt", Field::Method, &method.to_le_bytes());
        let message = place.refused(&zip);
        assert!(message.contains("lib/data.txt"), "{method}: {message}");
    }
    let mut zip = stored;
    patch(&mut zip, "lib/data.txt", Field::Method, &12u16.to_le_bytes());
    assert_eq!(place.refused(&zip), "The plugin package's lib/data.txt is compressed in a way AutoScene can't unpack.");
}

#[test]
fn symlinks_are_refused() {
    let place = Place::new();
    let files = with(plugin(), "lib/link", b"/etc/passwd");
    let mut w = writer();
    let rest: Files = with_lock(&files).into_iter().filter(|(path, _)| path != "lib/link").collect();
    add(&mut w, &rest, deflated());
    w.add_symlink("lib/link", "/etc/passwd", SimpleFileOptions::default()).unwrap();
    let message = place.refused(&w.finish().unwrap().into_inner());
    assert_eq!(message, "The plugin package's lib/link is a link, which a package can't hold.");

    // `asp pack` refuses a folder with one.
    let src = place.dir.join("src");
    for (path, bytes) in plugin() {
        let file = src.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(place.dir.join("plugin.asp"), src.join("lib/link"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(place.dir.join("plugin.asp"), src.join("lib/link"));
    // Windows makes links only for administrators or in Developer Mode.
    if linked.is_ok() {
        let Err(PluginError::Invalid(message)) = package::lock_folder(&src) else { panic!() };
        assert_eq!(message, "The plugin's lib/link is a link or another special file, which a package can't hold.");

        // Nor does a check of an unpacked folder go through one.
        fs::remove_file(src.join("lib/link")).unwrap();
        let lock = package::lock_folder(&src).unwrap();
        let elsewhere = place.dir.join("elsewhere");
        fs::rename(src.join("lib"), &elsewhere).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elsewhere, src.join("lib")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&elsewhere, src.join("lib")).unwrap();
        let Err(PluginError::Invalid(message)) = package::verify_dir(&src, &lock) else { panic!() };
        assert_eq!(message, "The plugin's lib is missing or is a link.");
        #[cfg(unix)]
        fs::remove_file(src.join("lib")).unwrap();
        #[cfg(windows)]
        fs::remove_dir(src.join("lib")).unwrap();
        fs::rename(&elsewhere, src.join("lib")).unwrap();
        package::verify_dir(&src, &lock).unwrap();

        // A linked file, whose contents are right.
        fs::rename(src.join("plugin.json"), &elsewhere).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elsewhere, src.join("plugin.json")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&elsewhere, src.join("plugin.json")).unwrap();
        let Err(PluginError::Invalid(message)) = package::verify_dir(&src, &lock) else { panic!() };
        assert_eq!(message, "The plugin's plugin.json is missing or is a link.");
    }
    assert!(place.left().iter().all(|p| p.starts_with("test")));
}

#[test]
fn a_changed_file_fails_its_hash() {
    let place = Place::new();
    let lock = serde_json::to_vec(&lock_of(&plugin())).unwrap();
    let mut changed = plugin();
    changed[2].1 = b"jello".to_vec();
    let message = place.refused(&zip_of(&with_lock_bytes(&changed, &lock)));
    assert_eq!(message, "The plugin package's lib/data.txt was changed after the package was made.");

    // And once unpacked.
    let unpacked = place.unpack(&zip_of(&with_lock(&plugin()))).unwrap();
    fs::write(place.target().join("lib/data.txt"), b"jello").unwrap();
    let Err(PluginError::Invalid(message)) = package::verify_dir(&place.target(), &unpacked.lock) else { panic!() };
    assert_eq!(message, "The plugin's lib/data.txt doesn't match its files.lock.");
    fs::remove_file(place.target().join("plugin.json")).unwrap();
    let Err(PluginError::Invalid(message)) = package::verify_dir(&place.target(), &unpacked.lock) else { panic!() };
    assert_eq!(message, "The plugin's plugin.json is missing or is a link.");
}

/// The lock of `plugin()` as JSON, changed by `edit`, in a package.
fn with_lock_json(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut lock = serde_json::to_value(lock_of(&plugin())).unwrap();
    edit(&mut lock);
    zip_of(&with_lock_bytes(&plugin(), &serde_json::to_vec(&lock).unwrap()))
}

#[test]
fn a_lock_that_breaks_its_rules_is_refused() {
    let place = Place::new();
    // A newer lock, also with fields this AutoScene doesn't know.
    let zip = with_lock_json(|l| {
        l["lockVersion"] = json!(2);
        l["signedFor"] = json!("later");
    });
    assert!(matches!(place.unpack(&zip), Err(PluginError::TooNew(field)) if field == "lockVersion"));
    assert_eq!(place.left(), ["test", "test/plugin.asp"]);

    for (edit, wanted) in [
        (json!({"lockVersion": 0}), r#"needs "lockVersion": 1"#),
        (json!({"signedFor": "x"}), "isn't valid (line 1, column"),
        (json!({"files": [{"path": "plugin.json", "size": 1, "sha256": "A".repeat(64)}]}), "64 lowercase hex"),
        (json!({"files": [{"path": "plugin.json", "size": 1, "sha256": "a".repeat(63)}]}), "64 lowercase hex"),
        (json!({"files": [{"path": "files.lock", "size": 1, "sha256": "a".repeat(64)}]}), "can't list files.lock"),
        (json!({"files": [{"path": "Author.pub/x", "size": 1, "sha256": "a".repeat(64)}]}), "can't list Author.pub/x"),
        (json!({"files": [{"path": "lib/data.txt", "size": 5, "sha256": "a".repeat(64)}]}), "doesn't list plugin.json"),
        (json!({"files": "plugin.json"}), "isn't valid (line 1, column"),
    ] {
        let zip = with_lock_json(|l| {
            for (key, value) in edit.as_object().unwrap() {
                l[key] = value.clone();
            }
        });
        let message = place.refused(&zip);
        assert!(message.contains(wanted), "{edit}: {message}");
    }

    let zip = with_lock_json(|l| {
        let tool = l["files"][1].clone();
        l["files"].as_array_mut().unwrap().push(tool);
    });
    assert_eq!(place.refused(&zip), "The plugin's files.lock names bin/tool twice.");

    // A key twice, which JSON readers settle differently.
    let text = String::from_utf8(serde_json::to_vec(&lock_of(&plugin())).unwrap()).unwrap();
    let text = text.replacen(r#""path":"bin/tool""#, r#""path":"bin/tool","path":"lib/data.txt""#, 1);
    let message = place.refused(&zip_of(&with_lock_bytes(&plugin(), text.as_bytes())));
    assert_eq!(message, r#"The plugin's files.lock has "path" twice in one object."#);

    // A lock, or one of its files, written as an array of its values: serde
    // reads them as the objects, while other readers find no files.
    let lock = lock_of(&plugin());
    let values = |f: &LockedFile| json!([f.path, f.size, f.sha256, f.exec]);
    let zip = with_lock_json(|l| l["files"][1] = values(&lock.files[1]));
    let wanted = "The plugin's files.lock must hold an object, with an object for each file.";
    assert_eq!(place.refused(&zip), wanted);
    let whole = json!([1, lock.files.iter().map(values).collect::<Vec<_>>()]);
    assert_eq!(place.refused(&zip_of(&with_lock_bytes(&plugin(), &serde_json::to_vec(&whole).unwrap()))), wanted);

    let message = place.refused(&zip_of(&with_lock_bytes(&plugin(), br#"{"lockVersion": 1,"#)));
    assert!(message.starts_with("The plugin's files.lock isn't valid JSON (line 1, column"), "{message}");
}

#[test]
fn a_folder_that_is_there_is_never_unpacked_into() {
    let place = Place::new();
    fs::create_dir(place.target()).unwrap();
    fs::write(place.target().join("keep.txt"), b"mine").unwrap();
    assert!(matches!(place.unpack(&zip_of(&with_lock(&plugin()))), Err(PluginError::Io(_))));
    assert_eq!(place.left(), ["test", "test/into", "test/into/keep.txt", "test/plugin.asp"]);
    assert_eq!(fs::read(place.target().join("keep.txt")).unwrap(), b"mine");
}

#[test]
fn a_locked_folder_unpacks_as_it_was() {
    let place = Place::new();
    let src = place.dir.join("src");
    for (path, bytes) in plugin() {
        let file = src.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }
    // The package's own files are left out, and so are version control and
    // what Windows and macOS leave in folders.
    fs::write(src.join(LOCK), b"an old lock").unwrap();
    fs::write(src.join(SIG), b"an old signature").unwrap();
    fs::create_dir_all(src.join(".git/refs")).unwrap();
    fs::write(src.join(".git/HEAD"), b"ref: refs/heads/main").unwrap();
    fs::write(src.join("lib/.DS_Store"), b"Bud1").unwrap();
    fs::write(src.join("Thumbs.db"), b"thumbnails").unwrap();
    let lock = package::lock_folder(&src).unwrap();
    let paths: Vec<&str> = lock.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["bin/tool", "lib/data.txt", "plugin.json"]);
    package::verify_dir(&src, &lock).unwrap();

    let entries = with_lock_bytes(&plugin(), &serde_json::to_vec(&lock).unwrap());
    let unpacked = place.unpack(&zip_of(&entries)).unwrap();
    assert_eq!(unpacked.lock, lock);
    package::verify_dir(&place.target(), &lock).unwrap();
    assert!(place.left().iter().all(|p| p.starts_with("test")));

    // What unpack would refuse, pack refuses too.
    fs::write(src.join("lib/a b.txt"), b"x").unwrap();
    let Err(PluginError::Invalid(message)) = package::lock_folder(&src) else { panic!() };
    assert!(message.contains("can't be used"), "{message}");
}

#[cfg(unix)]
fn mode(file: PathBuf) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(file).unwrap().permissions().mode() & 0o7777
}

#[cfg(unix)]
#[test]
fn exec_bits_come_from_the_lock_not_the_zip() {
    let place = Place::new();
    place.unpack(&zip_with(&with_lock(&plugin()), deflated().unix_permissions(0o4777))).unwrap();
    for path in ["plugin.json", "lib/data.txt", LOCK] {
        assert_eq!(mode(place.target().join(path)), 0o644, "{path}");
    }
}

#[cfg(unix)]
#[test]
fn listed_executables_are_executable() {
    let place = Place::new();
    place.unpack(&zip_with(&with_lock(&plugin()), deflated().unix_permissions(0o600))).unwrap();
    assert_eq!(mode(place.target().join("bin/tool")), 0o755);
}
