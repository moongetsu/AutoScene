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
pub mod fandom;
mod library;
pub mod naming;
mod pack;
mod project;
pub mod tvmaze;

pub use analysis::{Pass, ShotsPlan};
pub use bodies::BodiesPlan;
pub use cast::{FACE_CHUNK, FacesPlan, PORTRAIT, Portrait, face_models};
pub use error::{ProjectError, ProjectResult};
pub use library::Library;
pub use project::{Project, Recovery, RelinkedFolder};
