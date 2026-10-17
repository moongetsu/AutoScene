use std::path::PathBuf;

use as_api::{AppInfo, MediaToolsStatus, Platform};
use as_core::AppPaths;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::{AppState, blocking};

#[tauri::command]
#[specta::specta]
pub fn app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    let paths = AppPaths::resolve();
    AppInfo {
        version: app.package_info().version.to_string(),
        platform: Platform::current(),
        arch: std::env::consts::ARCH.to_string(),
        data_dir: paths.data.display().to_string(),
        logs_dir: paths.logs().display().to_string(),
        library_root: state
            .library
            .library_root()
            .unwrap_or_else(|_| AppPaths::default_library_root())
            .display()
            .to_string(),
    }
}

/// The media engine (ffmpeg) found on this machine — Settings → About.
#[tauri::command]
#[specta::specta]
pub async fn media_tools(state: State<'_, AppState>) -> AppResult<MediaToolsStatus> {
    let state = state.inner().clone();
    blocking(move || Ok(state.tools_status())).await
}

/// Show a file or folder in Explorer / Finder.
#[tauri::command]
#[specta::specta]
pub fn reveal_path(path: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(AppError::new(ErrorCode::NotFound, "That file or folder isn't there any more."));
    }
    tauri_plugin_opener::reveal_item_in_dir(&p).map_err(|e| {
        log::warn!("revealing {path}: {e}");
        AppError::new(ErrorCode::Io, "That file or folder couldn't be shown.")
    })
}

/// Put text on the clipboard. Natively, so it also works from the macOS
/// menu, where the webview's clipboard API refuses (no user gesture).
#[tauri::command]
#[specta::specta]
pub fn clipboard_write(app: AppHandle, text: String) -> AppResult<()> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| {
        log::warn!("clipboard: {e}");
        AppError::new(ErrorCode::Internal, "Couldn't copy to the clipboard.")
    })
}

/// The text on the clipboard, for the text fields' Paste (the webview's own
/// clipboard API would ask the user for permission first). `None` when the
/// clipboard holds no text (a picture, files, nothing). Off the main thread:
/// reading waits for the clipboard's owner, which on Linux can be this app.
#[tauri::command]
#[specta::specta]
pub async fn clipboard_read(app: AppHandle) -> AppResult<Option<String>> {
    blocking(move || {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        Ok(app.clipboard().read_text().inspect_err(|e| log::debug!("clipboard has no text: {e}")).ok())
    })
    .await
}

/// Open the logs folder (Settings → About, Help menu on Windows).
#[tauri::command]
#[specta::specta]
pub fn open_logs_dir() -> AppResult<()> {
    let dir = AppPaths::resolve().logs();
    std::fs::create_dir_all(&dir)?;
    tauri_plugin_opener::open_path(dir.display().to_string(), None::<&str>).map_err(|e| {
        log::warn!("opening {}: {e}", dir.display());
        AppError::new(ErrorCode::Io, "The logs folder couldn't be opened.")
    })
}
