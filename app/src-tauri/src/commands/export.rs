//! Exports: presets, starting an export, history.

use std::path::PathBuf;

use as_api::{ExportPreset, ExportRequest, ExportSummary};
use as_core::ProjectId;
use as_export::builtin_presets;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::ExportChanged;
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
    let lib = state.library.clone();
    let (project, summary) = blocking(move || {
        let project = lib.project(project_id)?;
        let out_dir = match req.out_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) => PathBuf::from(d),
            None => lib.default_export_dir(&lib.summary(project_id)?)?,
        };
        let summary = project.create_export(&req, preset, &out_dir)?;
        Ok((project, summary))
    })
    .await?;
    work::queue_export(&app, &project, summary.id, summary.items.len());
    let _ = ExportChanged { project_id, export_id: summary.id }.emit(&app);
    Ok(summary)
}

/// Newest first.
#[tauri::command]
#[specta::specta]
pub async fn exports_list(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<Vec<ExportSummary>> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.project(project_id)?.exports()?)).await
}
