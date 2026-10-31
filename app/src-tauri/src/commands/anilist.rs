//! AniList naming for anime projects (the opt-in in Settings gates all of
//! it): search, confirm a series, match its cast's portraits to the
//! project's characters, apply the suggestions.

use as_api::{AniListMedia, AniListMerge, AniListSuggestion, AniListSuggestions};
use as_core::{CancelToken, ProjectId};
use as_project::anilist::{Media, MergeProposal, NameSuggestion};
use tauri::State;

use crate::error::AppResult;
use crate::state::{AppState, blocking};

fn media_item(m: &Media) -> AniListMedia {
    AniListMedia {
        id: m.id,
        title: m
            .title
            .english
            .clone()
            .or_else(|| m.title.romaji.clone())
            .or_else(|| m.title.native.clone())
            .unwrap_or_default(),
        format: m.format.clone(),
        episodes: m.episodes,
        year: m.season_year,
    }
}

/// The search text the project's title suggests (nothing is sent to
/// AniList by this call).
#[tauri::command]
#[specta::specta]
pub async fn anilist_search_text(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<String> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.anilist_search_text(project_id)?)).await
}

/// Search AniList for `text` — the text the person confirmed is sent as
/// it is. Series the project rejected aren't among the results.
#[tauri::command]
#[specta::specta]
pub async fn anilist_search(
    state: State<'_, AppState>,
    project_id: ProjectId,
    text: String,
) -> AppResult<Vec<AniListMedia>> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.anilist_search(project_id, &text, &CancelToken::new())?))
        .await
        .map(|v| v.iter().map(media_item).collect())
}

/// Confirm series `media` for the project: it and its sequel chain are
/// kept; returns the chain, `media` first.
#[tauri::command]
#[specta::specta]
pub async fn anilist_confirm(
    state: State<'_, AppState>,
    project_id: ProjectId,
    media: u32,
) -> AppResult<Vec<AniListMedia>> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.anilist_confirm(project_id, media, &CancelToken::new())?))
        .await
        .map(|v| v.iter().map(media_item).collect())
}

/// Say series `media` isn't this project's.
#[tauri::command]
#[specta::specta]
pub async fn anilist_reject(state: State<'_, AppState>, project_id: ProjectId, media: u32) -> AppResult<()> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.anilist_reject(project_id, media)?)).await
}

/// Match the confirmed series' cast portraits to the project's
/// characters: every portrait is embedded by the engine (CCIP), then
/// matched against the cast's faces. A portrait the engine fails on is
/// left out (only when it fails on all of them is that an error); an engine
/// that won't start, crashed or was cancelled stops the match at once. When
/// `anilist.auto` is on, the names are given at once.
#[tauri::command]
#[specta::specta]
pub async fn anilist_suggest(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<AniListSuggestions> {
    let lib = state.library.clone();
    let engine = state.engine.clone();
    blocking(move || {
        let cancel = CancelToken::new();
        let cast = lib.anilist_cast(project_id, &cancel)?;
        let mut portraits = Vec::with_capacity(cast.len());
        let mut failed = None;
        for member in cast {
            let Some(image) = &member.image else { continue };
            match engine.embed(image.clone(), &cancel) {
                Ok(emb) => portraits.push((member, emb)),
                Err(e @ as_engine::EngineError::Failed { .. }) => {
                    log::warn!(
                        "anilist: the portrait of character {} wasn't read: {}",
                        member.character.id,
                        e.detail()
                    );
                    failed.get_or_insert(e);
                }
                Err(e) => {
                    return Err(
                        as_project::ProjectError::Invalid(format!("embedding the portraits: {}", e.detail())).into()
                    );
                }
            }
        }
        if let Some(e) = failed.filter(|_| portraits.is_empty()) {
            return Err(as_project::ProjectError::Invalid(format!("embedding the portraits: {}", e.detail())).into());
        }
        let project = lib.project(project_id)?;
        let (rev, names, merges) = project.anilist_suggest(&portraits)?;
        let auto = project.anilist_auto()?;
        if auto {
            project.apply_anilist_suggestions(rev, &names, &merges)?;
        }
        Ok(AniListSuggestions {
            rev,
            auto,
            names: names
                .iter()
                .map(|n: &NameSuggestion| AniListSuggestion {
                    character_id: n.character_id,
                    anilist_id: n.anilist_id,
                    name: n.name.clone(),
                    score: n.score,
                    image: n.image.as_ref().map(|p| p.to_string_lossy().into_owned()),
                })
                .collect(),
            merges: merges
                .iter()
                .map(|m: &MergeProposal| AniListMerge { from: m.from, into: m.into, anilist_id: m.anilist_id })
                .collect(),
        })
    })
    .await
}

/// Apply suggestions made on cast revision `rev` (a regroup or decision
/// since makes them stale — the suggestions are asked for again).
#[tauri::command]
#[specta::specta]
pub async fn anilist_apply(
    state: State<'_, AppState>,
    project_id: ProjectId,
    rev: i64,
    names: Vec<AniListSuggestion>,
    merges: Vec<AniListMerge>,
) -> AppResult<u32> {
    let lib = state.library.clone();
    blocking(move || {
        let names: Vec<NameSuggestion> = names
            .iter()
            .map(|n| NameSuggestion {
                character_id: n.character_id,
                anilist_id: n.anilist_id,
                name: n.name.clone(),
                score: n.score,
                image: n.image.as_ref().map(std::path::PathBuf::from),
            })
            .collect();
        let merges: Vec<MergeProposal> =
            merges.iter().map(|m| MergeProposal { from: m.from, into: m.into, anilist_id: m.anilist_id }).collect();
        Ok(lib.anilist_apply(project_id, rev, &names, &merges)? as u32)
    })
    .await
}

/// Whether names are given without asking (`anilist.auto`), or set it.
#[tauri::command]
#[specta::specta]
pub async fn anilist_auto(state: State<'_, AppState>, project_id: ProjectId, on: Option<bool>) -> AppResult<bool> {
    let lib = state.library.clone();
    blocking(move || {
        let project = lib.project(project_id)?;
        if let Some(on) = on {
            project.set_anilist_auto(on)?;
        }
        Ok(project.anilist_auto()?)
    })
    .await
}
