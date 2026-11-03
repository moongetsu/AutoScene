//! Exports: presets, starting an export, history.

use std::path::PathBuf;

use as_api::{ClipRequest, ExportPlan, ExportPreset, ExportRequest, ExportSummary, ListFormat, SavedList};
use as_core::{ExportId, ProjectId};
use as_export::builtin_presets;
use as_media::Caps;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::ExportChanged;
use crate::paths;
use crate::state::{AppState, blocking};
use crate::work;

#[tauri::command]
#[specta::specta]
pub fn export_presets() -> Vec<ExportPreset> {
    builtin_presets()
}

/// Folder exports of this project go to unless the user picks another.
#[tauri::command]
#[specta::specta]
pub async fn export_default_dir(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<String> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.default_export_dir(&lib.summary(project_id)?)?.display().to_string())).await
}

/// Validate, record and queue an export. Progress arrives as `job:*` events,
/// results as `export:changed`.
#[tauri::command]
#[specta::specta]
pub async fn export_start(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    req: ExportRequest,
) -> AppResult<ExportSummary> {
    let preset = builtin_presets()
        .into_iter()
        .find(|p| p.id == req.preset_id)
        .ok_or_else(|| AppError::new(ErrorCode::Invalid, "Unknown export preset."))?;
    // The folder picked for it; none: the project's own in the library.
    let chosen = req.out_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()).map(PathBuf::from);
    if let Some(dir) = &chosen {
        paths::check(dir)?;
    }
    let lib = state.library.clone();
    let (project, summary) = blocking(move || {
        let project = lib.project(project_id)?;
        let out_dir = match chosen {
            Some(d) => d,
            None => lib.default_export_dir(&lib.summary(project_id)?)?,
        };
        let summary = project.create_export(&req, preset, &out_dir)?;
        Ok((project, summary))
    })
    .await?;
    work::queue_export(&app, &project, &summary);
    let _ = ExportChanged { project_id, export_id: summary.id }.emit(&app);
    Ok(summary)
}

/// What an export of `clips` would make before it starts: the file names
/// and roughly how big it gets with each preset.
#[tauri::command]
#[specta::specta]
pub async fn export_plan(
    state: State<'_, AppState>,
    project_id: ProjectId,
    clips: Vec<ClipRequest>,
) -> AppResult<ExportPlan> {
    let (lib, tools) = (state.library.clone(), state.tools().ok());
    blocking(move || {
        // Sized for the encoders an export would use (found once, in about a second).
        let none = Caps::default();
        let caps = tools.as_deref().map_or(&none, |t| t.caps());
        Ok(lib.project(project_id)?.export_plan(&clips, &builtin_presets(), caps)?)
    })
    .await
}

/// Newest first.
#[tauri::command]
#[specta::specta]
pub async fn exports_list(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<Vec<ExportSummary>> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.project(project_id)?.exports()?)).await
}

/// Save an export's clips as an EDL or a cut list (CSV) at `path` (see
/// `list_path`, `Project::export_list_file`).
#[tauri::command]
#[specta::specta]
pub async fn export_save_list(
    state: State<'_, AppState>,
    project_id: ProjectId,
    export_id: ExportId,
    format: ListFormat,
    path: String,
) -> AppResult<SavedList> {
    let path = list_path(&path, format)?;
    let (lib, tools, saved) = (state.library.clone(), state.tools().ok(), state.saved_lists.clone());
    blocking(move || {
        let (text, left_out) = lib.project(project_id)?.export_list_file(export_id, format, tools.as_deref())?;
        std::fs::write(&path, text)?;
        // Its toast offers to show it, wherever it went.
        saved.lock().insert(path.clone());
        Ok(SavedList { path: path.display().to_string(), left_out })
    })
    .await
}

/// Where to save a list: `path` as the save dialog gave it, which ends in
/// `.edl` or `.csv` as the format says, so the page can't write anything
/// else. A name without an extension gets it (the dialogs on Linux don't add
/// it), unless a file of that name is there: the dialog didn't ask before
/// replacing that one.
fn list_path(path: &str, format: ListFormat) -> AppResult<PathBuf> {
    let mut path = PathBuf::from(path);
    paths::check(&path)?;
    let ext = match format {
        ListFormat::Edl => "edl",
        ListFormat::Csv => "csv",
    };
    match path.extension() {
        Some(e) if e.eq_ignore_ascii_case(ext) => return Ok(path),
        None if !path.with_extension(ext).exists() => {
            path.set_extension(ext);
            return Ok(path);
        }
        _ => {}
    }
    Err(AppError::new(ErrorCode::Invalid, format!("Save it as a .{ext} file.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_saved_only_as_their_kind_of_file() {
        let tmp = std::env::temp_dir().join(format!("autoscene-lists-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let at = |name: &str| tmp.join(name).display().to_string();
        let edl = |path: &str| list_path(path, ListFormat::Edl).map_err(|e| e.message);
        assert_eq!(edl(&at("Walter.edl")), Ok(tmp.join("Walter.edl")));
        assert_eq!(edl(&at("Walter.EDL")), Ok(tmp.join("Walter.EDL")));
        assert_eq!(list_path(&at("Walter.csv"), ListFormat::Csv).ok(), Some(tmp.join("Walter.csv")));
        for other in ["Walter.txt", "Walter.csv", "Walter.edl.exe", "Walter v1.2"] {
            assert_eq!(edl(&at(other)), Err("Save it as a .edl file.".to_owned()), "{other}");
        }
        assert!(list_path(&at("Walter.edl"), ListFormat::Csv).is_err());
        // Typed without one, it gets it; unless that would replace a file the dialog didn't ask about.
        assert_eq!(edl(&at("Walter")), Ok(tmp.join("Walter.edl")));
        std::fs::write(tmp.join("Walter.edl"), b"").unwrap();
        assert_eq!(edl(&at("Walter")), Err("Save it as a .edl file.".to_owned()));
        // Only a plain place.
        for bad in ["Walter.edl", "../Walter.edl"] {
            assert!(edl(bad).unwrap_err().starts_with("AutoScene can't use that location."), "{bad}");
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
