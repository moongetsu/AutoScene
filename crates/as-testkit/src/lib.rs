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

/// Load `truth.json` if fixtures have been generated; `None` lets tests skip
/// gracefully on machines that haven't run `cargo xtask fixtures`.
pub fn load_truth() -> Option<Truth> {
    let p = fixtures_dir().join("truth.json");
    let raw = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&raw).ok()
}
