//! `cargo xtask fetch-sidecars`: the pinned ffmpeg/ffprobe build the app
//! ships with, downloaded once and checked against its published SHA256.
//!
//! Lands in `app/src-tauri/ffmpeg/` (git-ignored). Release builds bundle that
//! folder as a resource (`tauri.release.conf.json`), where `as_media::Tools`
//! finds it first; CI puts it on PATH so tests run the exact same build.
//!
//! To move to a newer FFmpeg, update the URLs and hashes below (both
//! publishers list a `.sha256` next to every download) and run the engine
//! test suites.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

struct Archive {
    url: &'static str,
    sha256: &'static str,
    /// Folder inside the archive that holds the binaries ("" = top level).
    bin_dir: &'static str,
}

/// BtbN FFmpeg-Builds, release 9.0.2, GPL, shared libraries (DLLs next to the exes).
#[cfg(windows)]
const ARCHIVES: &[Archive] = &[Archive {
    url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-13-13-05/ffmpeg-n9.0.2-27-g7e93014c5b-win64-gpl-shared-9.0.zip",
    sha256: "73222d3e5e0952033b1140dc771fe00b82a4414c7a6544d5ff2febe7e450850c",
    bin_dir: "ffmpeg-n9.0.2-27-g7e93014c5b-win64-gpl-shared-9.0/bin",
}];

/// martin-riedl.de static arm64 builds of 9.0.2 (with zimg, needed for HDR tone mapping).
#[cfg(target_os = "macos")]
const ARCHIVES: &[Archive] = &[
    Archive {
        url: "https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffmpeg.zip",
        sha256: "c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924",
        bin_dir: "",
    },
    Archive {
        url: "https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffprobe.zip",
        sha256: "fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6",
        bin_dir: "",
    },
];

#[cfg(not(any(windows, target_os = "macos")))]
const ARCHIVES: &[Archive] = &[];

pub fn fetch(dest: &Path) -> Result<()> {
    if ARCHIVES.is_empty() {
        bail!("no pinned ffmpeg build for this platform; install ffmpeg 7+ and put it on PATH");
    }
    let exe = if cfg!(windows) { ".exe" } else { "" };
    if dest.join(format!("ffmpeg{exe}")).is_file() && dest.join(format!("ffprobe{exe}")).is_file() {
        println!("{} already has ffmpeg and ffprobe", dest.display());
        return Ok(());
    }
    let work = dest.with_extension("download");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    fs::create_dir_all(dest)?;
    for (i, a) in ARCHIVES.iter().enumerate() {
        let zip = work.join(format!("part{i}.zip"));
        println!("downloading {}", a.url);
        run(Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error", "--output"])
            .arg(&zip)
            .arg(a.url))?;
        let got = sha256(&zip)?;
        if got != a.sha256 {
            bail!("{}: checksum {got} doesn't match the pinned {}", a.url, a.sha256);
        }
        // An empty, fresh folder per archive; nothing from it runs before the copy below.
        let out = work.join(format!("part{i}"));
        fs::create_dir_all(&out)?;
        // bsdtar reads zip on Windows 10+ and macOS alike.
        run(Command::new("tar").arg("-xf").arg(&zip).arg("-C").arg(&out))?;
        let bin = if a.bin_dir.is_empty() { out.clone() } else { out.join(a.bin_dir) };
        for e in fs::read_dir(&bin).with_context(|| format!("reading {}", bin.display()))? {
            let p = e?.path();
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            // Executables and the libraries they load; ffplay isn't needed.
            if p.is_file() && !name.starts_with("ffplay") {
                fs::copy(&p, dest.join(&name))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(dest.join(&name), fs::Permissions::from_mode(0o755))?;
                }
            }
        }
    }
    fs::remove_dir_all(&work).ok();
    for tool in ["ffmpeg", "ffprobe"] {
        let p: PathBuf = dest.join(format!("{tool}{exe}"));
        if !p.is_file() {
            bail!("{} is missing after extraction", p.display());
        }
    }
    let out = Command::new(dest.join(format!("ffmpeg{exe}"))).arg("-version").output()?;
    println!("{}", String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default());
    println!("sidecars ready in {}", dest.display());
    Ok(())
}

fn sha256(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("spawning {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{:?} failed: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}
