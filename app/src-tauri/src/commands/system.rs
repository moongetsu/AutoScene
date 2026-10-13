use std::path::PathBuf;

use as_api::{AppInfo, Platform};
use as_core::AppPaths;
use tauri::AppHandle;

use crate::error::{AppError, AppResult, ErrorCode};

#[tauri::command]
#[specta::specta]
pub fn app_info(app: AppHandle) -> AppInfo {
    let paths = AppPaths::resolve();
    AppInfo {
        version: app.package_info().version.to_string(),
        platform: Platform::current(),
        arch: std::env::consts::ARCH.to_string(),
        data_dir: paths.data.display().to_string(),
        logs_dir: paths.logs().display().to_string(),
        library_root: AppPaths::default_library_root().display().to_string(),
    }
}

/// Show a file or folder in Explorer / Finder.
#[tauri::command]
#[specta::specta]
pub fn reveal_path(path: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(AppError::new(ErrorCode::NotFound, format!("{path} does not exist")));
    }
    tauri_plugin_opener::reveal_item_in_dir(&p).map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))
}

/// Open the logs folder (Settings → About, Help menu on Windows).
#[tauri::command]
#[specta::specta]
pub fn open_logs_dir() -> AppResult<()> {
    let dir = AppPaths::resolve().logs();
    std::fs::create_dir_all(&dir)?;
    tauri_plugin_opener::open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| AppError::new(ErrorCode::Io, e.to_string()))
}
