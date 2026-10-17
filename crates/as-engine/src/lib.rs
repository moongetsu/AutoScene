//! The analysis engine: ONNX Runtime and ffmpeg pipelines in a process of
//! their own.
//!
//! - A GPU or driver crash never takes the UI down; the job resumes from the
//!   app's checkpoints in a fresh process.
//! - `ort` binds one runtime per process (`load-dynamic`), so switching
//!   runtimes (DirectML ↔ CUDA speed pack) is a restart.
//! - VRAM is freed completely when the engine exits after being idle.
//!
//! The app starts its own executable with `--engine` (see [`main_from_args`]);
//! the `autoscene-engine` binary is the same engine for headless use.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::{Engine, EngineError, FacesEvent, FacesInput, Launch, ShotsEvent};
pub use server::{Config, serve};

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

/// Idle time after which a serving engine exits on its own.
pub const IDLE_EXIT: Duration = Duration::from_secs(120);

/// Arguments for [`main_from_args`].
pub fn args(cfg: &Config) -> Vec<OsString> {
    let mut a: Vec<OsString> =
        vec!["--ort".into(), cfg.ort_dir.clone().into(), "--models".into(), cfg.models_dir.clone().into()];
    if let Some(d) = &cfg.ffmpeg_dir {
        a.extend(["--ffmpeg".into(), d.clone().into()]);
    }
    if !cfg.gpu {
        a.push("--cpu".into());
    }
    if let Some(idle) = cfg.idle_exit {
        a.extend(["--idle".into(), idle.as_secs().to_string().into()]);
    }
    a
}

/// Serve on stdin/stdout. `args`: `--ort DIR --models DIR [--ffmpeg DIR]
/// [--cpu] [--idle SECS]`. Logs go to stderr. Returns the exit code.
pub fn main_from_args(args: impl IntoIterator<Item = OsString>) -> i32 {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .without_time()
        .init();
    let mut ort_dir = None;
    let mut models_dir = None;
    let mut ffmpeg_dir = None;
    let mut gpu = true;
    let mut idle_exit = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--ort") => ort_dir = it.next().map(PathBuf::from),
            Some("--models") => models_dir = it.next().map(PathBuf::from),
            Some("--ffmpeg") => ffmpeg_dir = it.next().map(PathBuf::from),
            Some("--cpu") => gpu = false,
            Some("--idle") => idle_exit = it.next().and_then(|s| s.to_str()?.parse().ok()).map(Duration::from_secs),
            _ => {
                eprintln!("unknown argument {a:?}");
                return 2;
            }
        }
    }
    let (Some(ort_dir), Some(models_dir)) = (ort_dir, models_dir) else {
        eprintln!("usage: autoscene-engine --ort DIR --models DIR [--ffmpeg DIR] [--cpu] [--idle SECS]");
        return 2;
    };
    let cfg = Config { ort_dir, models_dir, ffmpeg_dir, gpu, idle_exit };
    serve(cfg, std::io::BufReader::new(std::io::stdin()), std::io::stdout())
}
