//! Library: projects, import scanning, settings.

use std::path::Path;

use as_api::{AppSettings, ContentKind, ImportCandidate, JobInfo, NewProject, ProjectKind, ProjectSummary};
use as_core::ProjectId;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::commands::media::stop_jobs;
use crate::error::AppResult;
use crate::events::LibraryChanged;
use crate::paths;
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
    paths::check_all(&req.files)?;
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

/// Say what a project's footage is (live action, 3D animation, mixed; a
/// project never moves to or from anime). Its cast is grouped again for it,
/// and its faces found again when it moves to or from 3D animation.
#[tauri::command]
#[specta::specta]
pub async fn project_set_content(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    content: ContentKind,
) -> AppResult<ProjectSummary> {
    let lib = state.library.clone();
    let (summary, regroup) = blocking(move || {
        let (summary, regroup) = lib.set_project_content(project_id, content)?;
        Ok((summary, regroup.then(|| lib.project(project_id)).transpose()?))
    })
    .await?;
    if let Some(project) = regroup {
        work::queue_regroup(&app, &project);
        work::queue_outdated_faces(&app, &project);
    }
    let _ = LibraryChanged { project_id: Some(project_id) }.emit(&app);
    Ok(summary)
}

/// Say whether a project's faces are found with Fast Scan (the way the
/// Scener app finds them). Faces found the other way are found again, the
/// user's decisions on them kept, and the cast is grouped again.
#[tauri::command]
#[specta::specta]
pub async fn project_set_fast_scan(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    fast_scan: bool,
) -> AppResult<ProjectSummary> {
    let lib = state.library.clone();
    let (summary, changed) = blocking(move || {
        let (summary, changed) = lib.set_project_fast_scan(project_id, fast_scan)?;
        Ok((summary, changed.then(|| lib.project(project_id)).transpose()?))
    })
    .await?;
    if let Some(project) = changed {
        work::queue_regroup(&app, &project);
        work::queue_outdated_faces(&app, &project);
    }
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
    // Its work stops first, also what it queues while stopping: nothing may
    // go on writing into its folder or hold its database open. When that
    // takes too long, the project stays as it was.
    let of = move |j: &JobInfo| j.project_id == Some(project_id);
    let still = "The work on this project is still stopping. Try again in a moment.";
    stop_jobs(&state, of, || false, still).await?;
    let lib = state.library.clone();
    let result = blocking(move || Ok(lib.delete_project(project_id, delete_files)?)).await;
    // Even when removing the folder failed half-way, the list may have changed.
    let _ = LibraryChanged { project_id: None }.emit(&app);
    result
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
    crate::paths::check_all(&paths)?;
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
pub async fn settings_update(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> AppResult<AppSettings> {
    let lib = state.library.clone();
    let before = lib.finds_bodies();
    let saved = blocking(move || {
        // Only a new library folder is checked: every other change sends
        // back the one in use.
        if settings.library_root.trim() != lib.library_root()?.display().to_string() {
            paths::check(settings.library_root.trim())?;
        }
        Ok(lib.update_settings(&settings)?)
    })
    .await?;
    // Finding people by their bodies changes every cast.
    if saved.find_bodies != before {
        work::bodies_setting_changed(&app, saved.find_bodies);
    }
    Ok(saved)
}
