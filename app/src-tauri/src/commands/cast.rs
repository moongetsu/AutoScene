//! The cast: characters found across a project's media, their face tracks,
//! and the user's decisions about them.

use as_api::{Cast, CastEdit, ClipRequest, FaceTrackItem, Presence, TrackFilter};
use as_core::{CharacterId, MediaId, ProjectId};
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::AppResult;
use crate::events::CastChanged;
use crate::state::{AppState, allow_asset_tree, blocking};

#[tauri::command]
#[specta::specta]
pub async fn cast_get(app: AppHandle, state: State<'_, AppState>, project_id: ProjectId) -> AppResult<Cast> {
    let lib = state.library.clone();
    let (cast, faces) = blocking(move || {
        let p = lib.project(project_id)?;
        Ok((p.cast()?, p.faces_dir()))
    })
    .await?;
    allow_asset_tree(&app, &faces);
    Ok(cast)
}

#[tauri::command]
#[specta::specta]
pub async fn cast_tracks(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    filter: TrackFilter,
) -> AppResult<Vec<FaceTrackItem>> {
    let lib = state.library.clone();
    let (tracks, faces) = blocking(move || {
        let p = lib.project(project_id)?;
        Ok((p.cast_tracks(filter)?, p.faces_dir()))
    })
    .await?;
    allow_asset_tree(&app, &faces);
    Ok(tracks)
}

/// Apply a decision about the cast. Returns the character it created, if any.
#[tauri::command]
#[specta::specta]
pub async fn cast_edit(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    edit: CastEdit,
) -> AppResult<Option<CharacterId>> {
    let lib = state.library.clone();
    let created = blocking(move || Ok(lib.project(project_id)?.cast_edit(&edit)?)).await?;
    let _ = CastChanged { project_id }.emit(&app);
    Ok(created)
}

/// Where characters are on screen in one media (for the timeline).
#[tauri::command]
#[specta::specta]
pub async fn media_presence(
    state: State<'_, AppState>,
    project_id: ProjectId,
    media_id: MediaId,
) -> AppResult<Vec<Presence>> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.project(project_id)?.presence(media_id)?)).await
}

/// Every shot a character is seen in, as clips ready to export.
#[tauri::command]
#[specta::specta]
pub async fn character_clips(
    state: State<'_, AppState>,
    project_id: ProjectId,
    character_id: CharacterId,
) -> AppResult<Vec<ClipRequest>> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.project(project_id)?.character_clips(character_id)?)).await
}
