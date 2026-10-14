//! `cargo xtask fetch-sidecars`: the pinned native tools the app ships with,
//! downloaded once and checked against their published SHA256:
//!
//! - `ffmpeg/`: ffmpeg + ffprobe (media layer, exports);
//! - `onnxruntime/`: ONNX Runtime for the analysis engine (DirectML on
//!   Windows, CoreML via the stock build on macOS). On Windows the folder also
//!   gets the Visual C++ runtime DLLs ONNX Runtime links against, app-local
//!   (Microsoft allows redistributing them this way), so the engine starts on
//!   PCs that never installed the VC++ Redistributable.
//!
//! Both land in `app/src-tauri/` (git-ignored). Release builds bundle them as
//! resources (`tauri.release.conf.json`); dev builds and tests find them there
//! too, and CI puts ffmpeg on PATH so tests run the exact same build.
//!
//! To move to a newer version, update the URLs and hashes below (FFmpeg
//! publishers list a `.sha256` next to every download, GitHub release assets
//! carry a digest) and run the engine test suites.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// What to take out of an archive.
enum Pick {
    /// Every file in this folder ("" = top level), except ffplay.
    Dir(&'static str),
    /// These files: (path inside the archive, name in the bundle folder).
    Files(&'static [(&'static str, &'static str)]),
}

struct Archive {
    url: &'static str,
    sha256: &'static str,
    pick: Pick,
}

/// A folder of native files assembled from one or more archives.
struct Bundle {
    dir: &'static str,
    archives: &'static [Archive],
    /// Files that must be there; `true` = must also start with `-version`.
    required: &'static [(&'static str, bool)],
    /// Copy the Visual C++ runtime DLLs in (Windows; from the local Visual Studio).
    vc_runtime: bool,
}

/// What ONNX Runtime imports from the Visual C++ runtime.
#[cfg(windows)]
const VC_RUNTIME: &[&str] = &["msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll", "vcruntime140_1.dll"];

#[cfg(windows)]
const BUNDLES: &[Bundle] = &[
    Bundle {
        dir: "ffmpeg",
        // BtbN FFmpeg-Builds, release 9.0.2, GPL, shared libraries (DLLs next to the exes).
        archives: &[Archive {
            url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-13-13-05/ffmpeg-n9.0.2-27-g7e93014c5b-win64-gpl-shared-9.0.zip",
            sha256: "73222d3e5e0952033b1140dc771fe00b82a4414c7a6544d5ff2febe7e450850c",
            pick: Pick::Dir("ffmpeg-n9.0.2-27-g7e93014c5b-win64-gpl-shared-9.0/bin"),
        }],
        required: &[("ffmpeg.exe", true), ("ffprobe.exe", true)],
        vc_runtime: false,
    },
    Bundle {
        dir: "onnxruntime",
        // ONNX Runtime 1.24.4 with the DirectML EP (its last DirectML release)
        // and the DirectML 1.15.4 redistributable it was built against.
        archives: &[
            Archive {
                url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.4/microsoft.ml.onnxruntime.directml.1.24.4.nupkg",
                sha256: "57e9f11b73437bef7a309496135d4c1f96b1a8e9ddba60013fa27bfc1d788681",
                pick: Pick::Files(&[
                    ("runtimes/win-x64/native/onnxruntime.dll", "onnxruntime.dll"),
                    ("runtimes/win-x64/native/onnxruntime_providers_shared.dll", "onnxruntime_providers_shared.dll"),
                ]),
            },
            Archive {
                url: "https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg",
                sha256: "4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9",
                pick: Pick::Files(&[("bin/x64-win/DirectML.dll", "DirectML.dll")]),
            },
        ],
        required: &[
            ("onnxruntime.dll", false),
            ("DirectML.dll", false),
            ("msvcp140.dll", false),
            ("msvcp140_1.dll", false),
            ("vcruntime140.dll", false),
            ("vcruntime140_1.dll", false),
        ],
        vc_runtime: true,
    },
];

#[cfg(target_os = "macos")]
const BUNDLES: &[Bundle] = &[
    Bundle {
        dir: "ffmpeg",
        // martin-riedl.de static arm64 builds of 9.0.2 (with zimg, needed for HDR tone mapping).
        archives: &[
            Archive {
                url: "https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffmpeg.zip",
                sha256: "c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924",
                pick: Pick::Dir(""),
            },
            Archive {
                url: "https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffprobe.zip",
                sha256: "fcbe839537485eaee7a7a8bc5cbc0f90d53617e80943e8a5b2e31cb851197ea6",
                pick: Pick::Dir(""),
            },
        ],
        required: &[("ffmpeg", true), ("ffprobe", true)],
        vc_runtime: false,
    },
    Bundle {
        dir: "onnxruntime",
        // Microsoft's osx-arm64 build (CPU + CoreML EPs).
        archives: &[Archive {
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-osx-arm64-1.31.0.tgz",
            sha256: "1031b36dd3bdaa8d976a6130d9cf2a05e9be320b93ae804f8427b91dc3432afc",
            pick: Pick::Files(&[(
                "onnxruntime-osx-arm64-1.31.0/lib/libonnxruntime.1.31.0.dylib",
                "libonnxruntime.dylib",
            )]),
        }],
        required: &[("libonnxruntime.dylib", false)],
        vc_runtime: false,
    },
];

#[cfg(not(any(windows, target_os = "macos")))]
const BUNDLES: &[Bundle] = &[];

/// Written into each folder last; lists the archive hashes it was built from.
const STAMP: &str = ".autoscene-sidecars";

/// Assemble every bundle under `root` (normally `app/src-tauri`).
pub fn fetch(root: &Path) -> Result<()> {
    if BUNDLES.is_empty() {
        bail!("no pinned native tools for this platform; install ffmpeg 7+ and put it on PATH");
    }
    for b in BUNDLES {
        fetch_bundle(b, &root.join(b.dir)).with_context(|| format!("fetching {}", b.dir))?;
    }
    Ok(())
}

fn fetch_bundle(b: &Bundle, dest: &Path) -> Result<()> {
    let vc = if b.vc_runtime { Some(vc_runtime_dir()?) } else { None };
    // Up to date only when the folder was completed from exactly these archives
    // (and this Visual C++ runtime).
    let mut want: String = b.archives.iter().map(|a| format!("{}\n", a.sha256)).collect();
    if let Some(dir) = &vc {
        want.push_str(&format!("vc-runtime {}\n", dir.display()));
    }
    if fs::read_to_string(dest.join(STAMP)).is_ok_and(|s| s == want) {
        println!("{} is up to date", dest.display());
        return Ok(());
    }
    // Everything is assembled and checked in a staging folder first and only
    // then swapped in, so an interrupted run never leaves a folder that looks
    // complete (ffmpeg present, a DLL missing) for the next run or a release.
    let work = dest.with_extension("download");
    let _ = fs::remove_dir_all(&work);
    let staging = work.join("staging");
    fs::create_dir_all(&staging)?;
    for (i, a) in b.archives.iter().enumerate() {
        let archive = work.join(format!("part{i}"));
        println!("downloading {}", a.url);
        run(Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error", "--output"])
            .arg(&archive)
            .arg(a.url))?;
        let got = sha256(&archive)?;
        if got != a.sha256 {
            bail!("{}: checksum {got} doesn't match the pinned {}", a.url, a.sha256);
        }
        // An empty, fresh folder per archive; nothing from it runs before the copy below.
        let out = work.join(format!("part{i}.x"));
        fs::create_dir_all(&out)?;
        // bsdtar reads zip (NuGet packages are zips) and tgz on Windows 10+ and macOS.
        run(Command::new("tar").arg("-xf").arg(&archive).arg("-C").arg(&out))?;
        let copy = |from: &Path, name: &str| -> Result<()> {
            fs::copy(from, staging.join(name)).with_context(|| format!("copying {}", from.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(staging.join(name), fs::Permissions::from_mode(0o755))?;
            }
            Ok(())
        };
        match &a.pick {
            Pick::Dir(sub) => {
                let bin = if sub.is_empty() { out.clone() } else { out.join(sub) };
                for e in fs::read_dir(&bin).with_context(|| format!("reading {}", bin.display()))? {
                    let p = e?.path();
                    let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    // Executables and the libraries they load; ffplay isn't needed.
                    if p.is_file() && !name.starts_with("ffplay") {
                        copy(&p, &name)?;
                    }
                }
            }
            Pick::Files(files) => {
                for (inside, name) in *files {
                    copy(&out.join(inside), name)?;
                }
            }
        }
    }
    #[cfg(windows)]
    if let Some(dir) = &vc {
        for name in VC_RUNTIME {
            fs::copy(dir.join(name), staging.join(name))
                .with_context(|| format!("copying {name} from {}", dir.display()))?;
        }
        println!("Visual C++ runtime from {}", dir.display());
    }
    for (name, runs) in b.required {
        let p: PathBuf = staging.join(name);
        if !p.is_file() {
            bail!("{} is missing after extraction", p.display());
        }
        if *runs {
            // Starting the tool proves its libraries are all there.
            let out = Command::new(&p).arg("-version").output().with_context(|| format!("running {}", p.display()))?;
            if !out.status.success() {
                bail!("{} doesn't start: {}", p.display(), String::from_utf8_lossy(&out.stderr));
            }
            println!("{}", String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default());
        }
    }
    fs::write(staging.join(STAMP), &want)?;
    if dest.exists() {
        fs::remove_dir_all(dest)
            .with_context(|| format!("replacing {} (is the app or a test still using it?)", dest.display()))?;
    }
    fs::rename(&staging, dest)?;
    fs::remove_dir_all(&work).ok();
    println!("ready: {}", dest.display());
    Ok(())
}

/// The newest x64 Visual C++ runtime redistributable folder
/// (`…\VC\Redist\MSVC\<version>\x64\Microsoft.VC14x.CRT`): from a Developer
/// prompt's `VCToolsRedistDir`, else from the newest Visual Studio with C++ tools.
#[cfg(windows)]
fn vc_runtime_dir() -> Result<PathBuf> {
    let crt_in = |redist: &Path| -> Option<PathBuf> {
        let mut crt: Vec<PathBuf> = fs::read_dir(redist.join("x64"))
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().unwrap_or_default().to_string_lossy();
                n.starts_with("Microsoft.VC14") && n.ends_with(".CRT") && VC_RUNTIME.iter().all(|f| p.join(f).is_file())
            })
            .collect();
        crt.sort();
        crt.pop()
    };
    if let Some(d) = std::env::var_os("VCToolsRedistDir").map(PathBuf::from)
        && let Some(crt) = crt_in(&d)
    {
        return Ok(crt);
    }
    let program_files =
        std::env::var_os("ProgramFiles(x86)").map_or_else(|| PathBuf::from(r"C:\Program Files (x86)"), PathBuf::from);
    let vswhere = program_files.join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    let out = Command::new(&vswhere)
        .args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"])
        .args(["-property", "installationPath"])
        .output()
        .with_context(|| format!("running {} (Visual Studio with C++ tools is needed)", vswhere.display()))?;
    let msvc = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join(r"VC\Redist\MSVC");
    let mut versions: Vec<PathBuf> = fs::read_dir(&msvc)
        .with_context(|| format!("reading {}", msvc.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("14.")))
        .collect();
    // Numeric order: 14.51.36231 after 14.50.35710.
    versions.sort_by_key(|p| {
        let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
        name.split('.').map(|x| x.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>()
    });
    versions
        .iter()
        .rev()
        .find_map(|v| crt_in(v))
        .with_context(|| format!("no x64 Visual C++ runtime under {}", msvc.display()))
}

#[cfg(not(windows))]
fn vc_runtime_dir() -> Result<PathBuf> {
    bail!("the Visual C++ runtime is only bundled on Windows")
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
