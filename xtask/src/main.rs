//! Dev tasks. `cargo xtask <task>`:
//!
//! - `fixtures [--out DIR] [--ffmpeg PATH]` — render synthetic test media with
//!   frame barcodes and a truth JSON (see `fixtures.rs`).
//! - `fetch-sidecars [--out DIR]` — download the pinned, checksum-verified
//!   ffmpeg/ffprobe build the app bundles (see `sidecars.rs`).

mod fixtures;
mod sidecars;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let task = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();
    match task.as_str() {
        "fixtures" => {
            let out = flag(&rest, "--out").map(PathBuf::from).unwrap_or_else(|| workspace_root().join("fixtures/out"));
            let ffmpeg = flag(&rest, "--ffmpeg").unwrap_or_else(|| "ffmpeg".into());
            fixtures::generate(&out, &ffmpeg).context("generating fixtures")
        }
        "fetch-sidecars" => {
            let out = flag(&rest, "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| workspace_root().join("app/src-tauri/ffmpeg"));
            sidecars::fetch(&out).context("fetching ffmpeg")
        }
        "" | "help" | "--help" => {
            println!("cargo xtask fixtures [--out DIR] [--ffmpeg PATH]");
            println!("cargo xtask fetch-sidecars [--out DIR]");
            Ok(())
        }
        other => bail!("unknown task '{other}' (try `cargo xtask help`)"),
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace").to_path_buf()
}
