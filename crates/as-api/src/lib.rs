//! Types that cross the IPC boundary to the React UI. Everything here derives
//! `specta::Type`, and `app/src/bindings.ts` is generated from it — the UI
//! never hand-writes a backend type.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Static facts about the running app, shown in Settings → About and used by
/// the shell for platform-specific chrome.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub platform: Platform,
    pub arch: String,
    pub data_dir: String,
    pub logs_dir: String,
    pub library_root: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}
