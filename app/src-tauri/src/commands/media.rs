//! Media of a project: listing, adding, audio tracks, and the player's frame map.

use std::path::Path;

use as_api::{FrameMap, JobKind, JobState, MediaItem, MediaStatus, ProxyState};
use as_core::{MediaId, ProjectId};
use tauri::{AppHandle, State};

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
        Ok(p)
    })
    .await?;
    work::library_changed(&app, &project);
    crate::events::emit_media_changed(&app, project_id, Some(media_id));
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
        let pending = matches!(p.media(media_id).map(|m| m.proxy), Ok(ProxyState::Pending));
        Ok((p, map, pending))
    })
    .await?;
    let (project, map, pending) = result;
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
