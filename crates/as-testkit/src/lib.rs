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
