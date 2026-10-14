//! Shared support for synthetic test media.
//!
//! `cargo xtask fixtures` renders videos whose every frame carries a binary
//! **barcode**: the frame's index in the master timeline (16 bits) and the id
//! of the timeline element it belongs to (8 bits). Because the barcode
//! survives lossy encoding, any frame decoded from any variant or export can
//! be identified exactly — which is how frame-exact cutting is proven.

pub mod barcode;
pub mod truth;

pub use truth::{Element, ElementKind, Truth, Variant};

use std::path::PathBuf;

/// `fixtures/out` in the workspace (override with `AUTOSCENE_FIXTURES`).
pub fn fixtures_dir() -> PathBuf {
    std::env::var_os("AUTOSCENE_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/out"))
}

/// Load `truth.json`. `None` (the test skips) only when fixtures were never
/// generated on this machine and `AUTOSCENE_REQUIRE_FIXTURES` is unset; CI
/// sets it, so a missing truth can never make tests pass silently. A truth
/// file that exists but doesn't parse always fails.
pub fn load_truth() -> Option<Truth> {
    let p = fixtures_dir().join("truth.json");
    let required = std::env::var_os("AUTOSCENE_REQUIRE_FIXTURES").is_some_and(|v| v != "0");
    let raw = match std::fs::read_to_string(&p) {
        Ok(raw) => raw,
        Err(e) if !required && e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("fixtures missing; run `cargo xtask fixtures` (skipping)");
            return None;
        }
        Err(e) => panic!("{}: {e} (run `cargo xtask fixtures`)", p.display()),
    };
    match serde_json::from_str(&raw) {
        Ok(t) => Some(t),
        Err(e) => panic!("{} is stale or corrupt ({e}); run `cargo xtask fixtures`", p.display()),
    }
}

/// The ONNX Runtime the app bundles (`cargo xtask fetch-sidecars`), or
/// `AUTOSCENE_ORT_DIR`. `None` (tests skip) when it isn't there and
/// `AUTOSCENE_REQUIRE_MODELS` is unset.
pub fn ort_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("AUTOSCENE_ORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/onnxruntime"));
    let lib = if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };
    if dir.join(lib).is_file() {
        return Some(dir);
    }
    if models_required() {
        panic!("no ONNX Runtime in {} (run `cargo xtask fetch-sidecars`)", dir.display());
    }
    eprintln!("ONNX Runtime missing; run `cargo xtask fetch-sidecars` (skipping)");
    None
}

/// Where tests find AI models: `AUTOSCENE_MODELS_DIR`, else `fixtures/models`
/// (filled by `cargo xtask fetch-models`).
pub fn models_dir() -> PathBuf {
    std::env::var_os("AUTOSCENE_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/models"))
}

/// CI sets `AUTOSCENE_REQUIRE_MODELS` so missing models fail instead of skip.
pub fn models_required() -> bool {
    std::env::var_os("AUTOSCENE_REQUIRE_MODELS").is_some_and(|v| v != "0")
}
