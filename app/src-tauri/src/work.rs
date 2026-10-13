//! Background work: ingest, playback proxies and exports, queued on
//! `as-jobs` workers. Each job reports through `job:*` events and announces
//! what it changed (`media:changed`, `library:changed`, `export:changed`) so
//! the UI refetches exactly what moved.

use std::sync::Arc;

use as_api::{JobKind, MediaItem, ProxyState};
use as_core::{ExportId, MediaId};
use as_jobs::{Class, JobError, JobSpec};
use as_project::{Project, ProjectError};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::events::{ExportChanged, LibraryChanged};
use crate::state::AppState;

fn job_error(e: ProjectError) -> JobError {
    match e {
        ProjectError::Cancelled => JobError::Cancelled,
        e => JobError::Failed(e.to_string()),
    }
}

fn media_title(m: &MediaItem) -> String {
    if m.label.is_empty() { m.file_name.clone() } else { format!("{} · {}", m.label, m.file_name) }
}

fn media_changed(app: &AppHandle, project: &Project, media_id: Option<MediaId>) {
    crate::events::emit_media_changed(app, project.id, media_id);
}

/// Refresh the project's Library card after its media changed.
pub fn library_changed(app: &AppHandle, project: &Project) {
    let state = app.state::<AppState>();
    if let Err(e) = state.library.refresh_stats(project) {
        log::warn!("refreshing project stats: {e}");
    }
    let _ = LibraryChanged { project_id: Some(project.id) }.emit(app);
}

/// Index a media (probe, frame index, poster). Queues its preview proxy if it needs one.
pub fn queue_ingest(app: &AppHandle, project: &Arc<Project>, media_id: MediaId) {
    let state = app.state::<AppState>();
    if state.jobs.is_active(|j| j.kind == JobKind::Ingest && j.media_id == Some(media_id)) {
        return;
    }
    let Ok(m) = project.media(media_id) else { return };
    let spec = JobSpec {
        kind: JobKind::Ingest,
        class: Class::Media,
        title: format!("Indexing {}", media_title(&m)),
        subtitle: project.title().ok(),
        project_id: Some(project.id),
        media_id: Some(media_id),
        export_id: None,
    };
    let (app2, project) = (app.clone(), project.clone());
    state.jobs.submit(spec, move |ctx| {
        let state = app2.state::<AppState>();
        let tools = state.tools().map_err(|e| JobError::Failed(e.message))?;
        let result = project.ingest(&tools, media_id, &ctx.cancel, &|stage, p| ctx.progress(stage, p));
        media_changed(&app2, &project, Some(media_id));
        library_changed(&app2, &project);
        let m = result.map_err(job_error)?;
        if let Some(poster) = &m.poster {
            crate::state::allow_asset(&app2, std::path::Path::new(poster));
        }
        if m.proxy == ProxyState::Pending {
            queue_proxy(&app2, &project, media_id);
        }
        Ok(())
    });
}

/// Build the playback proxy for a media whose source the webview can't play frame-accurately.
pub fn queue_proxy(app: &AppHandle, project: &Arc<Project>, media_id: MediaId) {
    let state = app.state::<AppState>();
    // A proxy for a different audio track may still be running: replace it.
    state.jobs.cancel_where(|j| j.kind == JobKind::Proxy && j.media_id == Some(media_id));
    let Ok(m) = project.media(media_id) else { return };
    let spec = JobSpec {
        kind: JobKind::Proxy,
        class: Class::Media,
        title: format!("Preparing preview of {}", media_title(&m)),
        subtitle: project.title().ok(),
        project_id: Some(project.id),
        media_id: Some(media_id),
        export_id: None,
    };
    let (app2, project) = (app.clone(), project.clone());
    state.jobs.submit(spec, move |ctx| {
        let state = app2.state::<AppState>();
        let tools = state.tools().map_err(|e| JobError::Failed(e.message))?;
        ctx.stage("Preparing preview");
        let result = project.build_proxy(&tools, media_id, &ctx.cancel, &|stage, p| ctx.progress(stage, p));
        media_changed(&app2, &project, Some(media_id));
        result.map_err(job_error)
    });
}

/// Render an export created with `Project::create_export`.
pub fn queue_export(app: &AppHandle, project: &Arc<Project>, export_id: ExportId, clips: usize) {
    let state = app.state::<AppState>();
    let spec = JobSpec {
        kind: JobKind::Export,
        class: Class::Export,
        title: if clips == 1 { "Exporting 1 clip".into() } else { format!("Exporting {clips} clips") },
        subtitle: project.title().ok(),
        project_id: Some(project.id),
        media_id: None,
        export_id: Some(export_id),
    };
    let (app2, project) = (app.clone(), project.clone());
    state.jobs.submit(spec, move |ctx| {
        let state = app2.state::<AppState>();
        let notify = || {
            let _ = ExportChanged { project_id: project.id, export_id }.emit(&app2);
        };
        let tools = match state.tools() {
            Ok(t) => t,
            Err(e) => {
                let _ = project.fail_export(export_id, &e.message);
                notify();
                return Err(JobError::Failed(e.message));
            }
        };
        notify();
        let result = project.run_export(&tools, export_id, &ctx.cancel, &|stage, p| ctx.progress(stage, p));
        notify();
        let summary = result.map_err(job_error)?;
        let failed = summary.items.iter().filter(|i| i.error.is_some()).count();
        if failed > 0 {
            return Err(JobError::Failed(if failed == 1 {
                "1 clip failed to export.".into()
            } else {
                format!("{failed} clips failed to export.")
            }));
        }
        Ok(())
    });
}

/// On launch: reopen every project, resume interrupted indexing and pending previews.
pub fn recover_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let projects = match state.library.projects() {
        Ok(p) => p,
        Err(e) => {
            log::error!("listing projects: {e}");
            return;
        }
    };
    for summary in projects {
        let project = match state.library.project(summary.id) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("project '{}' unavailable: {e}", summary.title);
                continue;
            }
        };
        crate::state::allow_asset_dir(app, &project.thumbs_dir());
        match project.recover() {
            Ok((ingest, proxies)) => {
                if !ingest.is_empty() || !proxies.is_empty() {
                    log::info!("resuming '{}': {} to index, {} previews", summary.title, ingest.len(), proxies.len());
                }
                for id in ingest {
                    queue_ingest(app, &project, id);
                }
                for id in proxies {
                    queue_proxy(app, &project, id);
                }
            }
            Err(e) => log::warn!("recovering '{}': {e}", summary.title),
        }
    }
}
