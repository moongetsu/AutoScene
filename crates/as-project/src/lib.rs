//! The library and its projects.
//!
//! A project is a `<Title>.autoscene` folder: `project.db` plus generated
//! files (`thumbs/`, `proxies/`, `tmp/`). Source media are referenced, never
//! copied or modified. Everything here is synchronous and Tauri-free; the app
//! runs the long operations (ingest, proxies, exports) on `as-jobs` workers.

mod error;
mod library;
pub mod naming;
mod project;

pub use error::{ProjectError, ProjectResult};
pub use library::Library;
pub use project::Project;
