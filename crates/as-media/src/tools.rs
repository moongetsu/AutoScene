//! Locating and launching ffmpeg/ffprobe.
//!
//! Resolution order: `AUTOSCENE_FFMPEG_DIR` → the app's bundled `ffmpeg`
//! resource folder → `PATH`. Every process is spawned without a console
//! window (a GUI app spawning console tools would otherwise flash windows on
//! Windows).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::caps::Caps;
use crate::{MediaError, MediaResult};

#[derive(Debug)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub version: String,
    caps: OnceLock<Caps>,
}

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

impl Tools {
    /// Find ffmpeg + ffprobe. `bundled` is the app's resource folder (if any).
    pub fn locate(bundled: Option<&Path>) -> MediaResult<Self> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os("AUTOSCENE_FFMPEG_DIR") {
            dirs.push(d.into());
        }
        if let Some(b) = bundled {
            dirs.push(b.to_path_buf());
        }
        for d in &dirs {
            let (ff, fp) = (d.join(format!("ffmpeg{EXE}")), d.join(format!("ffprobe{EXE}")));
            if ff.is_file() && fp.is_file() {
                return Self::checked(ff, fp);
            }
        }
        let on_path = |name: &str| -> Option<PathBuf> {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths).map(|p| p.join(format!("{name}{EXE}"))).find(|p| p.is_file())
            })
        };
        match (on_path("ffmpeg"), on_path("ffprobe")) {
            (Some(ff), Some(fp)) => Self::checked(ff, fp),
            _ => Err(MediaError::ToolsMissing),
        }
    }

    fn checked(ffmpeg: PathBuf, ffprobe: PathBuf) -> MediaResult<Self> {
        let out = crate::proc::output(no_window(Command::new(&ffmpeg)).arg("-version"))
            .map_err(|_| MediaError::ToolsMissing)?;
        let first = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string();
        // "ffmpeg version 8.0.1-full_build-www.gyan.dev Copyright …" / "ffmpeg version n7.1 …"
        let version = first.split_whitespace().nth(2).unwrap_or("unknown").trim_start_matches('n').to_string();
        let major: u32 = version.split(['.', '-']).next().and_then(|m| m.parse().ok()).unwrap_or(0);
        // -stats_enc_pre {ptsi} and -enc_time_base need FFmpeg 7.
        if major != 0 && major < 7 {
            return Err(MediaError::ToolsTooOld(version));
        }
        tracing::info!(ffmpeg = %ffmpeg.display(), %version, "media tools ready");
        Ok(Self { ffmpeg, ffprobe, version, caps: OnceLock::new() })
    }

    /// `ffmpeg -hide_banner -nostdin` with no console window.
    pub fn ffmpeg(&self) -> Command {
        let mut c = no_window(Command::new(&self.ffmpeg));
        c.args(["-hide_banner", "-nostdin"]);
        c
    }

    /// `ffprobe -v error` with no console window.
    pub fn ffprobe(&self) -> Command {
        let mut c = no_window(Command::new(&self.ffprobe));
        c.args(["-hide_banner", "-v", "error"]);
        c
    }

    /// Hardware capabilities, detected once (encoder smoke tests take ~1 s).
    pub fn caps(&self) -> &Caps {
        self.caps.get_or_init(|| Caps::detect(self))
    }
}

#[cfg(windows)]
fn no_window(mut c: Command) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

#[cfg(not(windows))]
fn no_window(c: Command) -> Command {
    c
}
