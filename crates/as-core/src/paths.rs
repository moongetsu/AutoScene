//! Well-known locations shared by the app, the engine and the CLI.
//!
//! | what | Windows | macOS |
//! |---|---|---|
//! | app data (models, runtimes, library.db, logs) | `%LOCALAPPDATA%\AutoScene` | `~/Library/Application Support/AutoScene` |
//! | default library (project folders) | `Documents\AutoScene Library` | `~/Documents/AutoScene Library` |
//! | updater override | `~/.autoscene/updates.json` | same |
//!
//! `AUTOSCENE_DATA_DIR` overrides the app data root (tests, portable installs).

use std::path::PathBuf;

const APP_DIR: &str = "AutoScene";

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Self {
        let data = std::env::var_os("AUTOSCENE_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| {
            let base = if cfg!(target_os = "macos") { dirs::data_dir() } else { dirs::data_local_dir() };
            base.unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR)
        });
        Self { data }
    }

    pub fn at(data: impl Into<PathBuf>) -> Self {
        Self { data: data.into() }
    }

    pub fn library_db(&self) -> PathBuf {
        self.data.join("library.db")
    }
    pub fn models(&self) -> PathBuf {
        self.data.join("models")
    }
    pub fn runtimes(&self) -> PathBuf {
        self.data.join("runtimes")
    }
    pub fn logs(&self) -> PathBuf {
        self.data.join("logs")
    }
    pub fn cache(&self) -> PathBuf {
        self.data.join("cache")
    }

    /// Where new projects go unless the user picks another folder.
    pub fn default_library_root() -> PathBuf {
        dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("AutoScene Library")
    }

    /// Legacy `~/.autoscene` dir — kept only for the updater override file.
    pub fn legacy_dot_dir() -> PathBuf {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".autoscene")
    }
}
