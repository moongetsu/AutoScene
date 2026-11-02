use std::collections::HashMap;
use std::path::{Path, PathBuf};

use as_api::{AppInfo, MediaToolsStatus, ModelInfo, Platform};
use as_core::AppPaths;
use as_project::Library;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::paths;
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
        extensions_dir: paths.extensions().display().to_string(),
    }
}

/// The media engine (ffmpeg) found on this machine — Settings → About.
#[tauri::command]
#[specta::specta]
pub async fn media_tools(state: State<'_, AppState>) -> AppResult<MediaToolsStatus> {
    let state = state.inner().clone();
    blocking(move || Ok(state.tools_status())).await
}

/// The AI models AutoScene downloads when an analysis first needs them:
/// what each is for, its licence, and whether it is here — Settings → About.
#[tauri::command]
#[specta::specta]
pub async fn models(state: State<'_, AppState>) -> AppResult<Vec<ModelInfo>> {
    let state = state.inner().clone();
    blocking(move || {
        Ok(as_models::manifest()
            .iter()
            .map(|m| ModelInfo {
                id: m.id.clone(),
                purpose: m.purpose.clone(),
                source: m.source.clone(),
                license: m.license.clone(),
                non_commercial: m.non_commercial,
                size_bytes: m.size as f64,
                // Listing doesn't hash: a damaged file is found when it is next used.
                installed: state.models.downloaded(&m.id).is_ok_and(|p| p.is_some()),
            })
            .collect())
    })
    .await
}

/// Show a file or folder in Explorer / Finder: one AutoScene knows (its
/// library, data and logs, a project's folder, an episode's file, an
/// export's folder or files), never just any path. Off the main thread: a
/// share that doesn't answer would freeze the window until Windows gives up.
#[tauri::command]
#[specta::specta]
pub async fn reveal_path(state: State<'_, AppState>, path: String) -> AppResult<()> {
    paths::check(&path)?;
    let lib = state.library.clone();
    blocking(move || {
        let p = PathBuf::from(&path);
        if !known(&lib, &AppPaths::resolve(), &p) {
            log::warn!("not revealing {}, which isn't AutoScene's", paths::name(&p));
            return Err(AppError::new(ErrorCode::Invalid, "AutoScene only shows its own files and folders."));
        }
        if !p.exists() {
            return Err(AppError::new(ErrorCode::NotFound, "That file or folder isn't there any more."));
        }
        tauri_plugin_opener::reveal_item_in_dir(&p).map_err(|e| {
            log::warn!("revealing {}: {e}", paths::name(&p));
            AppError::new(ErrorCode::Io, "That file or folder couldn't be shown.")
        })
    })
    .await
}

/// Whether `path` is one AutoScene keeps things in or was given: the
/// library folder, the app's data and logs, a project's folder (thumbnails,
/// previews), an export's folder and files, an episode's file. Episodes and
/// exports are those of the open projects, which after launch are all of
/// them (each is opened to resume its work).
fn known(lib: &Library, app: &AppPaths, path: &Path) -> bool {
    let folders = [lib.library_root().ok(), Some(app.data.clone()), Some(app.logs())];
    let projects = lib.projects().unwrap_or_default().into_iter().map(|p| Some(PathBuf::from(p.dir)));
    if folders.into_iter().chain(projects).flatten().any(|dir| paths::is_within(path, &dir)) {
        return true;
    }
    lib.open_projects().iter().any(|project| {
        project.exports().unwrap_or_default().iter().any(|e| paths::is_within(path, Path::new(&e.out_dir)))
            || project.media_list_uncounted().unwrap_or_default().iter().any(|m| Path::new(&m.path) == path)
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

/// Open the extensions folder (Settings → Extensions).
#[tauri::command]
#[specta::specta]
pub fn open_extensions_dir() -> AppResult<()> {
    let dir = AppPaths::resolve().extensions();
    std::fs::create_dir_all(&dir)?;
    tauri_plugin_opener::open_path(dir.display().to_string(), None::<&str>).map_err(|e| {
        log::warn!("opening the extensions folder: {e}");
        AppError::new(ErrorCode::Io, "The extensions folder couldn't be opened.")
    })
}

/// List extensions installed in the extensions folder.
#[tauri::command]
#[specta::specta]
pub fn extensions_list() -> Vec<as_api::InstalledExtension> {
    let dir = AppPaths::resolve().extensions();
    let _ = std::fs::create_dir_all(&dir);
    as_export::discover_extensions(&dir)
}

/// Run an extension directly to test its execution.
#[tauri::command]
#[specta::specta]
pub fn extension_test(id: String) -> AppResult<String> {
    let dir = AppPaths::resolve().extensions();
    let installed = as_export::discover_extensions(&dir);
    let ext = installed
        .into_iter()
        .find(|e| e.manifest.id == id)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Extension not found"))?;

    let dummy_payload = as_export::PostExportPayload {
        export_id: "test-run".into(),
        project_id: "manual-test".into(),
        project_title: Some("Manual Test Run".into()),
        out_dir: ext.path.clone(),
        clip_count: 1,
        output_files: vec![],
        completed_at: as_api::now_ms() as u64,
    };

    let cancel = as_core::CancelToken::new();
    let scratch = AppPaths::resolve().cache().join("test_extension");
    as_export::run_hook(&ext.entrypoint_path, &dummy_payload, &scratch, &cancel, Some(std::time::Duration::from_secs(10)))
        .map_err(|e| AppError::new(ErrorCode::Invalid, format!("Extension failed: {e}")))?;

    Ok("Extension test ran successfully!".into())
}

/// Open the logs folder (Settings → About, Help menu on Windows).
#[tauri::command]
#[specta::specta]
pub fn open_logs_dir() -> AppResult<()> {
    let dir = AppPaths::resolve().logs();
    std::fs::create_dir_all(&dir)?;
    tauri_plugin_opener::open_path(dir.display().to_string(), None::<&str>).map_err(|e| {
        log::warn!("opening the logs folder: {e}");
        AppError::new(ErrorCode::Io, "The logs folder couldn't be opened.")
    })
}

/// Put the person's keys (Settings → Shortcuts) on the macOS menu, which
/// answers them there before the page sees them: `keys` maps a menu item's id
/// to its accelerator, `""` for none. Elsewhere there is no menu.
#[tauri::command]
#[specta::specta]
pub fn menu_set_keys(app: AppHandle, keys: HashMap<String, String>) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    crate::mac_menu::set_keys(&app, &keys).map_err(|e| {
        log::warn!("setting the menu's keys: {e}");
        AppError::internal("The menu's keys couldn't be changed.")
    })?;
    #[cfg(not(target_os = "macos"))]
    let _ = (app, keys);
    Ok(())
}

#[cfg(test)]
mod tests {
    use as_api::{AppSettings, ContentKind, NewProject, ProjectKind};

    use super::*;

    #[test]
    fn only_paths_autoscene_knows_are_revealed() {
        let tmp = std::env::temp_dir().join(format!("autoscene-reveal-{}", std::process::id()));
        let (data, root, media) = (tmp.join("data"), tmp.join("Library"), tmp.join("Shows"));
        std::fs::create_dir_all(&media).unwrap();
        let episode = media.join("Show S01E01.mkv");
        std::fs::write(&episode, b"not really a video").unwrap();
        let app = AppPaths::at(&data);
        let lib = Library::open(&app).unwrap();
        let settings = AppSettings { library_root: root.display().to_string(), ..lib.settings().unwrap() };
        lib.update_settings(&settings).unwrap();
        let (summary, ..) = lib
            .create_project(&NewProject {
                title: "Show".into(),
                kind: ProjectKind::Series,
                content: ContentKind::Live,
                files: vec![episode.display().to_string()],
            })
            .unwrap();

        let project = PathBuf::from(&summary.dir);
        for ok in [&root, &root.join("Exports").join("Show"), &project.join("thumbs"), &episode, &app.logs(), &data] {
            assert!(known(&lib, &app, ok), "{}", ok.display());
        }
        for other in [
            media.clone(),
            media.join("Show S01E02.mkv"),
            tmp.join("Library2"),
            root.join("..").join("Shows").join("Show S01E02.mkv"),
            PathBuf::from("Library"),
        ] {
            assert!(!known(&lib, &app, &other), "{}", other.display());
        }
        drop(lib);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
