//! The cast: characters found across a project's media, their face tracks,
//! and the user's decisions about them.

use as_api::{Cast, CastEdit, CharacterShot, FaceTrackItem, Presence, TrackFilter};
use as_core::{CancelToken, CharacterId, MediaId, ProjectId};
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::{AppState, allow_asset_tree, blocking};
use crate::work;

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
    blocking(move || {
        let p = lib.project(project_id)?;
        let created = p.cast_edit(&edit)?;
        work::cast_edited(&app, &p, &edit);
        Ok(created)
    })
    .await
}

/// A face's portrait (3:4, sharp where a face is shown big), made now unless
/// it exists. `None` when it can't be made now (the media is missing or being
/// indexed): the thumbnail stands in.
#[tauri::command]
#[specta::specta]
pub async fn cast_portrait(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    track_id: u32,
) -> AppResult<Option<String>> {
    let lib = state.library.clone();
    let tools = state.tools()?;
    let (portrait, faces) = blocking(move || {
        let p = lib.project(project_id)?;
        let portrait = p.portrait(&tools, i64::from(track_id), &CancelToken::new())?;
        Ok((portrait, p.faces_dir()))
    })
    .await?;
    allow_asset_tree(&app, &faces);
    Ok(portrait.path().map(|p| p.display().to_string()))
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

/// Every shot a character is seen in, with its sprite and the character's
/// clearest face there.
#[tauri::command]
#[specta::specta]
pub async fn character_shots(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    character_id: CharacterId,
) -> AppResult<Vec<CharacterShot>> {
    let lib = state.library.clone();
    let (shots, faces) = blocking(move || {
        let p = lib.project(project_id)?;
        Ok((p.character_shots(character_id)?, p.faces_dir()))
    })
    .await?;
    allow_asset_tree(&app, &faces);
    Ok(shots)
}
