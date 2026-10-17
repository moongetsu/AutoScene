//! Media of a project: listing, adding, audio tracks, and the player's frame map.

use std::path::Path;

use as_api::{
    AnalysisStage, FrameMap, JobKind, JobState, MediaItem, MediaStatus, ProxyState, Relink, RelinkSummary, Segment,
    StageState,
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
    let _one_at_a_time = RELINKING.lock().await;
    let interrupted = stop_work_on(&state, &[media_id]).await?;
    let (project, relink) = blocking(move || {
        let p = lib.project(project_id)?;
        let r = p.relink(&tools, media_id, Path::new(&path), force);
        // Whatever the answer: what the relink stopped goes on.
        resume_interrupted(&p, &interrupted);
        Ok((p, r?))
    })
    .await?;
    match relink {
        Relink::Different { .. } => return Ok(relink),
        // Indexed again (the ingest queues the analysis).
        Relink::Reindex => work::queue_ingest(&app, &project, media_id),
        Relink::Same => found_as_it_was(&app, &project, media_id),
    }
    work::library_changed(&app, &project);
    crate::events::emit_media_changed(&app, project_id, Some(media_id));
    // Its faces can be shown and made into portraits again, or went with
    // an analysis the relink reset.
    work::cast_changed(&app, &project);
    Ok(relink)
}

/// How long a relink waits for the work on the old path to stop.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// One relink (of a file or a folder) at a time: one deciding which files
/// are missing while another is queueing work for one it found would stop
/// that work again.
static RELINKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Stop every job of `media` and wait until they have ended: nothing may go
/// on reading the old path, or record its outcome over what the relink sets.
/// Work queued for them meanwhile (a restart the stopped analysis owed, a
/// new face model's pass) is stopped too. Returns the media whose analysis
/// was under way (see `resume_interrupted`).
async fn stop_work_on(state: &AppState, media: &[MediaId]) -> AppResult<Vec<MediaId>> {
    let of = |j: &as_api::JobInfo| j.media_id.is_some_and(|m| media.contains(&m));
    let interrupted: Vec<MediaId> = media
        .iter()
        .copied()
        .filter(|&m| state.jobs.is_active(|j| j.kind == JobKind::Analyze && j.media_id == Some(m)))
        .collect();
    let until = std::time::Instant::now() + STOP_WAIT;
    loop {
        state.jobs.cancel_where(of);
        let running = state.jobs.is_active(of) || media.iter().any(|m| state.analysis.lock().running.contains(m));
        if !running {
            break;
        }
        if std::time::Instant::now() > until {
            return Err(AppError::new(
                ErrorCode::NotReady,
                "The work on this episode is still stopping. Try again in a moment.",
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Ok(interrupted)
}

/// Analysis the relink stopped is pending again, so it carries on from where
/// it was (now, or at the next launch once its file is there).
fn resume_interrupted(project: &as_project::Project, media: &[MediaId]) {
    for &id in media {
        let Ok(m) = project.media(id) else { continue };
        let resumed = (|| {
            if m.shots == StageState::Stopped {
                project.set_shots_state(id, StageState::Pending, None)?;
            }
            if m.faces == StageState::Stopped {
                project.set_faces_state(id, StageState::Pending, None)?;
            }
            Ok::<_, as_project::ProjectError>(())
        })();
        if let Err(e) = resumed {
            log::warn!("resuming the analysis of a file found again: {}", e.detail());
        }
    }
}

/// A file found as it was: its preview and analysis go on where they were.
fn found_as_it_was(app: &AppHandle, project: &std::sync::Arc<as_project::Project>, media_id: MediaId) {
    if project.media(media_id).is_ok_and(|m| m.proxy == ProxyState::Pending) {
        work::queue_proxy(app, project, media_id);
    }
    work::queue_analysis(app, project, media_id);
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
    let _one_at_a_time = RELINKING.lock().await;
    let missing: Vec<MediaId> = {
        let lib = state.library.clone();
        blocking(move || {
            let p = lib.project(project_id)?;
            Ok(p.media_list()?.into_iter().filter(|m| m.status == MediaStatus::Missing).map(|m| m.id).collect())
        })
        .await?
    };
    let interrupted = stop_work_on(&state, &missing).await?;
    let (project, found) = blocking(move || {
        let p = lib.project(project_id)?;
        let found = p.relink_folder(&tools, Path::new(&dir));
        resume_interrupted(&p, &interrupted);
        Ok((p, found?))
    })
    .await?;
    for &id in &found.reindex {
        work::queue_ingest(&app, &project, id);
    }
    // Those found as they were: what waited for them can run.
    for &id in &found.same {
        found_as_it_was(&app, &project, id);
    }
    if !found.same.is_empty() || !found.reindex.is_empty() {
        work::library_changed(&app, &project);
        crate::events::emit_media_changed(&app, project_id, None);
        work::cast_changed(&app, &project);
    }
    Ok(found.summary)
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
                } else if p.finds_bodies() && m.bodies != as_api::StageState::Ready {
                    p.set_bodies_state(media_id, as_api::StageState::Pending, None)?;
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
