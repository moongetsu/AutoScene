//! The library and its projects.
//!
//! A project is a `<Title>.autoscene` folder: `project.db` plus generated
//! files (`thumbs/`, `faces/`, `proxies/`, `tmp/`). Source media are referenced, never
//! copied or modified. Everything here is synchronous and Tauri-free; the app
//! runs the long operations (ingest, proxies, exports) on `as-jobs` workers.

mod analysis;
pub mod anilist;
mod bodies;
mod cast;
mod error;
mod library;
pub mod naming;
mod pack;
mod project;

pub use analysis::{Pass, ShotsPlan};
pub use bodies::BodiesPlan;
pub use cast::{FACE_CHUNK, FacesPlan, PORTRAIT, Portrait, face_models};
pub use error::{ProjectError, ProjectResult};
pub use library::Library;
pub use project::{Project, Recovery, RelinkedFolder};

/// Whether the anime character pipeline is switched on for this process —
/// `AUTOSCENE_ANIME_FACES` set to anything but empty or "0". It stays
/// behind the switch until the anime benchmark's sealed test releases the
/// feature (see bench/anime); the setting can't turn it on.
pub fn anime_faces_enabled() -> bool {
    std::env::var_os("AUTOSCENE_ANIME_FACES").is_some_and(|v| !v.is_empty() && v != "0")
}
