//! Library: projects, import scanning, settings.

use std::path::Path;

use as_api::{AppSettings, ImportCandidate, NewProject, ProjectKind, ProjectSummary};
use as_core::ProjectId;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::AppResult;
use crate::events::LibraryChanged;
use crate::state::{AppState, allow_asset_dir, blocking};
use crate::work;

#[tauri::command]
#[specta::specta]
pub async fn projects_list(app: AppHandle, state: State<'_, AppState>) -> AppResult<Vec<ProjectSummary>> {
    let lib = state.library.clone();
    let projects = blocking(move || Ok(lib.projects()?)).await?;
    for p in &projects {
        allow_asset_dir(&app, &Path::new(&p.dir).join("thumbs"));
    }
    Ok(projects)
}

#[tauri::command]
#[specta::specta]
pub async fn project_get(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<ProjectSummary> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.summary(project_id)?)).await
}

/// Create a project from video files and start indexing them.
#[tauri::command]
#[specta::specta]
pub async fn project_create(app: AppHandle, state: State<'_, AppState>, req: NewProject) -> AppResult<ProjectSummary> {
    let lib = state.library.clone();
    let (summary, project, added) = blocking(move || Ok(lib.create_project(&req)?)).await?;
    allow_asset_dir(&app, &project.thumbs_dir());
    for id in added {
        work::queue_ingest(&app, &project, id);
    }
    let _ = LibraryChanged { project_id: Some(summary.id) }.emit(&app);
    Ok(summary)
}

#[tauri::command]
#[specta::specta]
pub async fn project_rename(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    title: String,
) -> AppResult<ProjectSummary> {
    let lib = state.library.clone();
    let summary = blocking(move || Ok(lib.rename_project(project_id, &title)?)).await?;
    let _ = LibraryChanged { project_id: Some(project_id) }.emit(&app);
    Ok(summary)
}

/// Remove a project. `delete_files` also deletes its `.autoscene` folder
/// (never the source videos or exported clips).
#[tauri::command]
#[specta::specta]
pub async fn project_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    delete_files: bool,
) -> AppResult<()> {
    state.jobs.cancel_where(|j| j.project_id == Some(project_id));
    // Give cancelled ffmpeg runs a moment to release their files.
    let (lib, jobs) = (state.library.clone(), state.jobs.clone());
    blocking(move || {
        for _ in 0..50 {
            if !jobs.is_active(|j| j.project_id == Some(project_id)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        Ok(lib.delete_project(project_id, delete_files)?)
    })
    .await?;
    let _ = LibraryChanged { project_id: None }.emit(&app);
    Ok(())
}

/// Expand dropped/picked files and folders into videos with parsed episode
/// numbers, in episode order. With `project_id`, marks files already in it.
#[tauri::command]
#[specta::specta]
pub async fn import_scan(
    state: State<'_, AppState>,
    paths: Vec<String>,
    kind: ProjectKind,
    project_id: Option<ProjectId>,
) -> AppResult<Vec<ImportCandidate>> {
    let lib = state.library.clone();
    blocking(move || {
        let project = project_id.map(|id| lib.project(id)).transpose()?;
        Ok(lib.scan(&paths, kind, project.as_deref())?)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_get(state: State<'_, AppState>) -> AppResult<AppSettings> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.settings()?)).await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_update(state: State<'_, AppState>, settings: AppSettings) -> AppResult<AppSettings> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.update_settings(&settings)?)).await
}
