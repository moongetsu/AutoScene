//! Well-known locations shared by the app, the engine and the CLI.
//!
//! | what | Windows | macOS |
//! |---|---|---|
//! | app data (models, runtimes, library.db, logs) | `%LOCALAPPDATA%\AutoScene` | `~/Library/Application Support/AutoScene` |
//! | default library (project folders) | `Documents\AutoScene Library` | `~/Documents/AutoScene Library` |
//! | updater override | `~/.autoscene/updates.json` | same |
//!
//! `AUTOSCENE_DATA_DIR` overrides the app data root (tests, portable installs).

use std::borrow::Cow;
use std::path::{Path, PathBuf};

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
    /// Character images from AniList, by AniList character id (only while
    /// names from AniList are on).
    pub fn anilist(&self) -> PathBuf {
        self.data.join("anilist")
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

/// Just the file (or folder) name, for the log: full paths name the user
/// (their home folder) and their folders, and logs get shared.
pub fn name(path: &Path) -> Cow<'_, str> {
    path.file_name().map_or(Cow::Borrowed("?"), |n| n.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_gets_only_a_files_name() {
        assert_eq!(name(Path::new("/home/someone/Videos/Show - 01.mkv")), "Show - 01.mkv");
        assert_eq!(name(&Path::new("/data").join("models")), "models");
        assert_eq!(name(Path::new("/")), "?");
        #[cfg(windows)]
        assert_eq!(
            name(Path::new(r"C:\Users\someone\AppData\Local\AutoScene\runtimes\onnxruntime.dll")),
            "onnxruntime.dll"
        );
    }
}
