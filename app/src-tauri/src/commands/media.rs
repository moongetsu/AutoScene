//! Media of a project: listing, adding, audio tracks, and the player's frame map.

use std::path::Path;

use as_api::{
    AnalysisStage, FrameMap, JobKind, JobState, MediaItem, MediaStatus, ProxyState, Relink, RelinkSummary, Segment,
};
use as_core::{MediaId, ProjectId};
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::{AppState, allow_asset, allow_asset_dir, blocking};
use crate::work;

#[tauri::command]
#[specta::specta]
pub async fn media_list(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> AppResult<Vec<MediaItem>> {
    let lib = state.library.clone();
    let (list, thumbs) = blocking(move || {
        let p = lib.project(project_id)?;
        Ok((p.media_list()?, p.thumbs_dir()))
    })
    .await?;
    allow_asset_dir(&app, &thumbs);
    Ok(list)
}

#[tauri::command]
#[specta::specta]
pub async fn media_get(state: State<'_, AppState>, project_id: ProjectId, media_id: MediaId) -> AppResult<MediaItem> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.project(project_id)?.media(media_id)?)).await
}

/// Add videos to a project and start indexing them. Returns how many were new.
#[tauri::command]
#[specta::specta]
pub async fn media_add(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    files: Vec<String>,
) -> AppResult<u32> {
    let lib = state.library.clone();
    let (project, added) = blocking(move || {
        let p = lib.project(project_id)?;
        let kind = lib.summary(project_id)?.kind;
        let added = lib.add_media(&p, &files, kind)?;
        Ok((p, added))
    })
    .await?;
    for &id in &added {
        work::queue_ingest(&app, &project, id);
    }
    work::library_changed(&app, &project);
    crate::events::emit_media_changed(&app, project_id, None);
    Ok(added.len() as u32)
}

#[tauri::command]
#[specta::specta]
pub async fn media_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
) -> AppResult<()> {
    state.jobs.cancel_where(|j| j.media_id == Some(media_id));
    let lib = state.library.clone();
    let project = blocking(move || {
        let p = lib.project(project_id)?;
        p.remove_media(media_id)?;
        // Its faces are gone: characters only it had go too.
        if let Err(e) = p.recluster() {
            log::warn!("grouping the cast after removing a media: {}", e.detail());
        }
        Ok(p)
    })
    .await?;
    work::library_changed(&app, &project);
    crate::events::emit_media_changed(&app, project_id, Some(media_id));
    work::cast_changed(&app, &project);
    Ok(())
}

/// Index a media again (after a failure, or once a missing file is back).
#[tauri::command]
#[specta::specta]
pub async fn media_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
) -> AppResult<()> {
    let lib = state.library.clone();
    let project = blocking(move || {
        let p = lib.project(project_id)?;
        p.set_media_queued(media_id)?;
        Ok(p)
    })
    .await?;
    work::queue_ingest(&app, &project, media_id);
    crate::events::emit_media_changed(&app, project_id, Some(media_id));
    Ok(())
}

/// Point an episode whose file moved at `path`. The same file keeps
/// everything; another copy is indexed again (and keeps its analysis when
/// its frames are the same); another video comes back as `different`
/// unless `force`.
#[tauri::command]
#[specta::specta]
pub async fn media_relink(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
    path: String,
    force: bool,
) -> AppResult<Relink> {
    let lib = state.library.clone();
    let tools = state.tools()?;
    let (project, relink) = blocking(move || {
        let p = lib.project(project_id)?;
        let r = p.relink(&tools, media_id, Path::new(&path), force)?;
        Ok((p, r))
    })
    .await?;
    match relink {
        Relink::Different { .. } => return Ok(relink),
        Relink::Reindex => work::queue_ingest(&app, &project, media_id),
        Relink::Same => {}
    }
    work::library_changed(&app, &project);
    crate::events::emit_media_changed(&app, project_id, Some(media_id));
    // Its faces can be shown and made into portraits again.
    work::cast_changed(&app, &project);
    Ok(relink)
}

/// Look for every missing episode of a project in `dir` and its folders.
#[tauri::command]
#[specta::specta]
pub async fn media_relink_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    dir: String,
) -> AppResult<RelinkSummary> {
    let lib = state.library.clone();
    let tools = state.tools()?;
    let (project, summary, reindex) = blocking(move || {
        let p = lib.project(project_id)?;
        let (summary, reindex) = p.relink_folder(&tools, Path::new(&dir))?;
        Ok((p, summary, reindex))
    })
    .await?;
    for id in reindex {
        work::queue_ingest(&app, &project, id);
    }
    if summary.same + summary.reindex > 0 {
        work::library_changed(&app, &project);
        crate::events::emit_media_changed(&app, project_id, None);
        work::cast_changed(&app, &project);
    }
    Ok(summary)
}

/// Build (or rebuild) the preview of a media that needs one.
#[tauri::command]
#[specta::specta]
pub async fn media_prepare_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
) -> AppResult<()> {
    let lib = state.library.clone();
    let (project, m) = blocking(move || {
        let p = lib.project(project_id)?;
        let m = p.media(media_id)?;
        Ok((p, m))
    })
    .await?;
    if m.status != MediaStatus::Ready || m.proxy == ProxyState::NotNeeded {
        return Err(AppError::new(ErrorCode::Invalid, "This file doesn't need a separate preview."));
    }
    if !state.jobs.is_active(|j| j.kind == JobKind::Proxy && j.media_id == Some(media_id)) {
        work::queue_proxy(&app, &project, media_id);
    }
    Ok(())
}

/// Choose the audio track for playback and export (`None` = the file's default).
#[tauri::command]
#[specta::specta]
pub async fn media_set_audio(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
    ordinal: Option<u32>,
) -> AppResult<MediaItem> {
    let lib = state.library.clone();
    let (project, needs_proxy, m) = blocking(move || {
        let p = lib.project(project_id)?;
        let needs = p.set_audio(media_id, ordinal)?;
        let m = p.media(media_id)?;
        Ok((p, needs, m))
    })
    .await?;
    if needs_proxy {
        work::queue_proxy(&app, &project, media_id);
    } else {
        // The new track plays straight from the file: a preview still being
        // built for the old one is pointless.
        state.jobs.cancel_where(|j| j.kind == JobKind::Proxy && j.media_id == Some(media_id));
    }
    crate::events::emit_media_changed(&app, project_id, Some(media_id));
    Ok(m)
}

/// Everything the player needs to show frame `f` exactly. Grants the webview
/// access to the file it should play.
#[tauri::command]
#[specta::specta]
pub async fn media_frame_map(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
) -> AppResult<FrameMap> {
    let lib = state.library.clone();
    let result = blocking(move || {
        let p = lib.project(project_id)?;
        let map = p.frame_map(media_id);
        let media = p.media(media_id);
        let pending = matches!(media.as_ref().map(|m| m.proxy), Ok(ProxyState::Pending));
        // The player asks only for media it thinks are ready. A damaged index
        // marks its media failed (here or in an export): tell the views, so
        // they offer "Index again".
        let damaged = map.is_err() && matches!(media.map(|m| m.status), Ok(MediaStatus::Failed));
        Ok((p, map, pending, damaged))
    })
    .await?;
    let (project, map, pending, damaged) = result;
    if damaged {
        work::library_changed(&app, &project);
        crate::events::emit_media_changed(&app, project_id, Some(media_id));
    }
    // Build a missing preview on demand — unless someone stopped the last
    // build: then the player offers "Prepare preview" instead of restarting
    // it on every refresh.
    let jobs = state.jobs.jobs();
    let proxy_jobs = || jobs.iter().filter(|j| j.kind == JobKind::Proxy && j.media_id == Some(media_id));
    let running = proxy_jobs().any(|j| !j.state.is_finished());
    let stopped = proxy_jobs().max_by_key(|j| (j.created_at, j.rev)).is_some_and(|j| j.state == JobState::Cancelled);
    if pending && !running && !stopped {
        work::queue_proxy(&app, &project, media_id);
    }
    let map = map?;
    allow_asset(&app, Path::new(&map.src));
    Ok(map)
}

/// Shots, transitions and black/flash frames of a media (empty until its
/// shots are detected).
#[tauri::command]
#[specta::specta]
pub async fn media_segments(
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
) -> AppResult<Vec<Segment>> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.project(project_id)?.segments(media_id)?)).await
}

/// Run the analysis of a media again. `redo` clears results from that stage
/// on first (a running analysis of the media is stopped and restarted once
/// it has stopped); `None` continues a stopped or failed run.
#[tauri::command]
#[specta::specta]
pub async fn media_analyze(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
    redo: Option<AnalysisStage>,
) -> AppResult<()> {
    let lib = state.library.clone();
    let project = blocking(move || {
        let p = lib.project(project_id)?;
        let m = p.media(media_id)?;
        if m.status != MediaStatus::Ready {
            return Err(AppError::new(ErrorCode::NotReady, format!("{} isn't indexed yet.", m.file_name)));
        }
        Ok(p)
    })
    .await?;
    let analysis = |j: &as_api::JobInfo| j.kind == JobKind::Analyze && j.media_id == Some(media_id);
    match redo {
        Some(stage) => {
            // A running job restarts the analysis once it has stopped; with
            // none running, a queued one is dropped and the restart made here.
            let handed_over = {
                let mut runs = state.analysis.lock();
                if runs.running.contains(&media_id) {
                    let merged = match runs.redo.get(&media_id) {
                        Some(AnalysisStage::Shots) => AnalysisStage::Shots,
                        _ => stage,
                    };
                    runs.redo.insert(media_id, merged);
                    true
                } else {
                    false
                }
            };
            state.jobs.cancel_where(analysis);
            if !handed_over {
                let app2 = app.clone();
                blocking(move || {
                    work::restart_analysis(&app2, &project, media_id, stage);
                    Ok(())
                })
                .await?;
            }
        }
        None => {
            let p = project.clone();
            blocking(move || {
                let m = p.media(media_id)?;
                if m.shots != as_api::StageState::Ready {
                    p.set_shots_state(media_id, as_api::StageState::Pending, None)?;
                } else if m.faces != as_api::StageState::Ready {
                    p.set_faces_state(media_id, as_api::StageState::Pending, None)?;
                }
                Ok(())
            })
            .await?;
            work::queue_analysis(&app, &project, media_id);
        }
    }
    crate::events::emit_media_changed(&app, project_id, Some(media_id));
    let _ = crate::events::AnalysisChanged { project_id, media_id }.emit(&app);
    Ok(())
}
