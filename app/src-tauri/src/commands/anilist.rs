//! Naming from AniList (anime projects), TVmaze (animated series) and a
//! film's Fandom wiki (animated films; the one opt-in in Settings gates all
//! of it): the look-up the app does by itself
//! once the characters are found ([`look_up`]), and by hand: search, confirm
//! a series, match its cast's portraits to the project's characters, apply
//! the suggestions.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use as_api::{AniListMedia, AniListMerge, AniListNames, AniListSuggestion, AniListSuggestions, CastEdit};
use as_core::{CancelToken, CharacterId, ProjectId};
use as_engine::Engine;
use as_models::ModelStore;
use as_project::anilist::{CastMember, Looks, Media, MergeProposal, NameSuggestion, Source, Suggested};
use as_project::{Library, ProjectError, ProjectResult};
use tauri::{AppHandle, Manager, State};
use tauri_specta::Event;

use crate::error::AppResult;
use crate::events::AniListChanged;
use crate::state::{AppState, blocking};

fn media_item(m: &Media) -> AniListMedia {
    AniListMedia { id: m.id, title: m.name(), format: m.format.clone(), episodes: m.episodes, year: m.season_year }
}

fn suggestion_item(n: &NameSuggestion) -> AniListSuggestion {
    AniListSuggestion {
        character_id: n.character_id,
        anilist_id: n.anilist_id,
        name: n.name.clone(),
        score: n.score,
        image: n.image.as_ref().map(|p| p.to_string_lossy().into_owned()),
    }
}

fn merge_item(m: &MergeProposal) -> AniListMerge {
    AniListMerge { from: m.from, into: m.into, anilist_id: m.anilist_id }
}

/// The confirmed series' cast with their portraits embedded by the engine
/// (CCIP; each once a session). A portrait the engine fails on is left out
/// (only when it fails on all of them is that an error); an engine that
/// won't start, crashed or was cancelled stops at once.
fn portraits(
    lib: &Library,
    engine: &Engine,
    project_id: ProjectId,
    cancel: &CancelToken,
) -> ProjectResult<Vec<(CastMember, Vec<f32>)>> {
    let (source, cast) = lib.anilist_cast(project_id, cancel)?;
    let mut portraits = Vec::with_capacity(cast.len());
    let mut failed = None;
    for member in cast {
        let Some(image) = &member.image else { continue };
        if let Some(emb) = lib.anilist_portrait(source, member.character.id) {
            portraits.push((member, emb));
            continue;
        }
        match embedded(engine, image, cancel)? {
            Ok(emb) => {
                lib.keep_anilist_portrait(source, member.character.id, &emb, cancel);
                portraits.push((member, emb));
            }
            Err(e) => {
                log::warn!("names: the portrait of character {} wasn't read: {}", member.character.id, e.detail());
                failed.get_or_insert(e);
            }
        }
    }
    if let Some(e) = failed.filter(|_| portraits.is_empty()) {
        return Err(ProjectError::Invalid(format!("embedding the portraits: {}", e.detail())));
    }
    Ok(portraits)
}

/// A picture embedded by the engine (CCIP): `Err` inside when the engine
/// couldn't read this one picture; an engine that won't start, crashed or
/// was cancelled stops the caller.
fn embedded(
    engine: &Engine,
    path: &std::path::Path,
    cancel: &CancelToken,
) -> ProjectResult<Result<Vec<f32>, as_engine::EngineError>> {
    match engine.embed(path.to_path_buf(), cancel) {
        Ok(emb) => Ok(Ok(emb)),
        Err(e @ as_engine::EngineError::Failed { .. }) => Ok(Err(e)),
        Err(as_engine::EngineError::Cancelled) => Err(ProjectError::Cancelled),
        Err(e) => Err(ProjectError::Invalid(format!("embedding the portraits: {}", e.detail()))),
    }
}

/// The portraits' embedder (CCIP), downloaded first when it isn't yet: an
/// animation project's face pass never needs it.
fn embedder_ready(models: &ModelStore, cancel: &CancelToken) -> ProjectResult<()> {
    let ccip = as_models::ids::ANIME_CCIP;
    if models.installed(ccip).map_err(|e| ProjectError::Invalid(e.to_string()))?.is_some() {
        return Ok(());
    }
    models.install(ccip, cancel, |_, _| {}).map_err(|e| match e {
        as_models::ModelError::Cancelled => ProjectError::Cancelled,
        e => {
            log::warn!("names: {}", e.detail());
            ProjectError::NotReady(e.to_string())
        }
    })?;
    Ok(())
}

/// The characters' looks as the portraits' embedder sees them, where the
/// project's faces were found by another one (TVmaze and Fandom name
/// animation, whose faces ArcFace or SFace found): each unnamed, shown character's pictures
/// ([`as_project::Project::look_pictures`]) embedded by the engine (each once
/// a session) and averaged. `None` where the faces themselves are the looks
/// (anime), or there are no portraits to match them to.
fn looks(
    lib: &Library,
    engine: &Engine,
    project_id: ProjectId,
    portraits: &[(CastMember, Vec<f32>)],
    cancel: &CancelToken,
) -> ProjectResult<Option<Looks>> {
    if portraits.is_empty() || lib.name_source(project_id)?.faces_are_looks() {
        return Ok(None);
    }
    let (rev, pictures) = lib.project(project_id)?.look_pictures()?;
    let mut looks = HashMap::with_capacity(pictures.len());
    for (character, pictures) in pictures {
        let mut seen = Vec::with_capacity(pictures.len());
        for (path, weight) in pictures {
            if let Some(emb) = lib.look_picture(&path) {
                seen.push((emb, weight));
                continue;
            }
            match embedded(engine, &path, cancel)? {
                Ok(emb) => {
                    lib.keep_look_picture(&path, &emb, cancel);
                    seen.push((emb, weight));
                }
                Err(e) => log::warn!("names: a picture of character {character} wasn't read: {}", e.detail()),
            }
        }
        if let Some(look) = as_project::anilist::look_of(&seen) {
            looks.insert(character, look);
        }
    }
    Ok(Some(Looks { rev, of: looks }))
}

/// A project's look-up this session (see [`look_up`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookUp {
    /// Running; `again` once the cast was found anew meanwhile.
    Running { again: bool },
    /// It failed (AniList unreachable, the engine): the UI doesn't start it
    /// again this session, a new analysis does.
    Failed,
}

/// Look names up on AniList for project `id` by itself, off the caller's
/// thread: its series by its title (or the one confirmed), then that
/// series' cast portraits matched to its characters. The names wait for the
/// person's yes (given at once only when `anilist.auto` is on and the
/// person confirmed the series). Only for a project AniList names
/// ([`Library::anilist_wanted`]); one look-up per project at a time.
/// `rerun`: the characters were found or grouped anew — a look-up running
/// meanwhile runs once more, one that failed is tried again; the UI asking
/// for a project never looked up passes `false`. The UI hears of it through
/// `anilist:changed` (and `cast:changed` when names were given). Switching
/// the opt-in off stops it ([`Library::anilist_token`]).
pub fn look_up(app: &AppHandle, id: ProjectId, rerun: bool) {
    let state = app.state::<AppState>();
    if !matches!(state.library.anilist_wanted(id), Ok(true)) {
        return;
    }
    {
        let mut runs = state.anilist.lock();
        match runs.get_mut(&id) {
            Some(LookUp::Running { again }) => {
                *again |= rerun;
                return;
            }
            Some(LookUp::Failed) if !rerun => return,
            _ => {}
        }
        runs.insert(id, LookUp::Running { again: false });
    }
    let _ = AniListChanged { project_id: id }.emit(app);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let lib = &state.library;
        let mut gave = false;
        loop {
            let done = catch_unwind(AssertUnwindSafe(|| look_up_now(lib, &state.engine, &state.models, id)))
                .unwrap_or_else(|_| Err(ProjectError::Invalid("the look-up stopped unexpectedly".into())));
            gave |= matches!(done, Ok(true));
            let mut runs = state.anilist.lock();
            match (runs.get(&id), &done) {
                (Some(LookUp::Running { again: true }), _) => {
                    runs.insert(id, LookUp::Running { again: false });
                    continue;
                }
                // Switched off meanwhile: not a failure.
                (_, Err(e)) if !matches!(e, ProjectError::Cancelled) && lib.names_from_anilist() => {
                    log::warn!("anilist: names for project {id}: {}", e.detail());
                    runs.insert(id, LookUp::Failed);
                }
                _ => {
                    runs.remove(&id);
                }
            }
            break;
        }
        let _ = AniListChanged { project_id: id }.emit(&app);
        if gave && let Ok(project) = lib.project(id) {
            crate::work::cast_changed(&app, &project);
        }
    });
}

/// One look-up; returns whether names were given at once. The project is
/// held open only while its database is read or written, never while
/// AniList or the engine answers (a project deleted meanwhile goes).
fn look_up_now(lib: &Library, engine: &Engine, models: &ModelStore, id: ProjectId) -> ProjectResult<bool> {
    if !lib.anilist_wanted(id)? {
        return Ok(false);
    }
    let cancel = lib.anilist_token();
    let Some(series) = lib.anilist_find_series(id, &cancel)? else {
        lib.project(id)?.set_anilist_looked()?;
        return Ok(false);
    };
    embedder_ready(models, &cancel)?;
    let portraits = portraits(lib, engine, id, &cancel)?;
    let looks = looks(lib, engine, id, &portraits, &cancel)?;
    if cancel.is_cancelled() {
        return Err(ProjectError::Cancelled);
    }
    let project = lib.project(id)?;
    let (rev, names, merges) = project.anilist_suggest_with(&portraits, looks.as_ref())?;
    log::info!(
        "names: {} names and {} merges from {} portraits of {} ({})",
        names.len(),
        merges.len(),
        portraits.len(),
        series.name,
        project.name_source()?.map_or("no source", Source::name)
    );
    // Another series confirmed meanwhile: what was found is from the wrong
    // cast (the next look-up uses the new one).
    if project.anilist_media()? != series.media {
        return Ok(false);
    }
    // Names from a series the app picked always wait for the person.
    let gave = !series.picked && project.anilist_auto()?;
    if gave {
        project.apply_anilist_suggestions(rev, &names, &merges)?;
    } else {
        let found = Suggested { media: series.media, series: series.name, names, merges };
        let any = !found.names.is_empty() || !found.merges.is_empty();
        project.set_anilist_suggested(any.then_some(&found))?;
    }
    project.set_anilist_looked()?;
    Ok(gave)
}

/// The names from AniList waiting for project `id`'s yes, and whether the
/// look-up runs. A project AniList names but never looked up (made before
/// the look-up, or before the opt-in) is looked up now.
#[tauri::command]
#[specta::specta]
pub async fn anilist_names(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
) -> AppResult<AniListNames> {
    let lib = state.library.clone();
    let runs = state.anilist.clone();
    blocking(move || {
        let wanted = lib.anilist_wanted(project_id)?;
        if wanted {
            let seen = runs.lock().contains_key(&project_id);
            if !seen && !lib.project(project_id)?.anilist_looked()? {
                look_up(&app, project_id, false);
            }
        }
        let looking = matches!(runs.lock().get(&project_id), Some(LookUp::Running { .. }));
        let suggested = if wanted { lib.anilist_suggested(project_id)? } else { None };
        Ok(match suggested {
            Some(s) => AniListNames {
                looking,
                series_id: s.media.first().copied(),
                series: Some(s.series),
                names: s.names.iter().map(suggestion_item).collect(),
                merges: s.merges.iter().map(merge_item).collect(),
            },
            None => AniListNames { looking, series_id: None, series: None, names: vec![], merges: vec![] },
        })
    })
    .await
}

/// Give the names and merges waiting for project `id` that the person saw
/// and kept (`names`, `merges`; only while they still wait as shown). Those
/// left out (`left_out`, `left_out_merges`) are never suggested again.
/// Returns how many were given. Merged characters' faces are placed again,
/// as after any merge.
#[tauri::command]
#[specta::specta]
pub async fn anilist_accept(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    names: Vec<AniListSuggestion>,
    merges: Vec<AniListMerge>,
    left_out: Vec<AniListSuggestion>,
    left_out_merges: Vec<AniListMerge>,
) -> AppResult<u32> {
    let lib = state.library.clone();
    blocking(move || {
        let give: Vec<(CharacterId, u32)> = names.iter().map(|n| (n.character_id, n.anilist_id)).collect();
        let merge: Vec<(CharacterId, CharacterId)> = merges.iter().map(|m| (m.from, m.into)).collect();
        let refuse: Vec<(CharacterId, u32)> = left_out
            .iter()
            .map(|n| (n.character_id, n.anilist_id))
            .chain(left_out_merges.iter().map(|m| (m.from, m.anilist_id)))
            .collect();
        let (merged, named) = lib.anilist_accept(project_id, &give, &merge, &refuse)?;
        let project = lib.project(project_id)?;
        match merges.first() {
            Some(m) if merged > 0 => {
                crate::work::cast_edited(&app, &project, &CastEdit::Merge { from: m.from, into: m.into })
            }
            _ if named > 0 => crate::work::cast_changed(&app, &project),
            _ => {}
        }
        Ok((merged + named) as u32)
    })
    .await
}

/// The person said no to the names waiting for project `id`: they stop
/// waiting (the look-up runs again once the characters are found anew).
#[tauri::command]
#[specta::specta]
pub async fn anilist_dismiss(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<()> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.anilist_dismiss(project_id)?)).await
}

/// The search text the project's title suggests (nothing is sent to
/// AniList by this call).
#[tauri::command]
#[specta::specta]
pub async fn anilist_search_text(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<String> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.anilist_search_text(project_id)?)).await
}

/// Search AniList (anime), TVmaze (animated series) or Wikidata (animated
/// films, with their Fandom wikis) for `text` — the text the person
/// confirmed is sent as it is. Series the project rejected aren't among the
/// results.
#[tauri::command]
#[specta::specta]
pub async fn anilist_search(
    state: State<'_, AppState>,
    project_id: ProjectId,
    text: String,
) -> AppResult<Vec<AniListMedia>> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.anilist_search(project_id, &text, &lib.anilist_token())?))
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
    blocking(move || Ok(lib.anilist_confirm(project_id, media, &lib.anilist_token())?))
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
/// matched against the cast's faces (animation: against pictures of them,
/// see [`looks`]). A portrait the engine fails on is left out (only when it
/// fails on all of them is that an error); an engine that won't start,
/// crashed or was cancelled stops the match at once. When `anilist.auto` is
/// on, the names are given at once.
#[tauri::command]
#[specta::specta]
pub async fn anilist_suggest(state: State<'_, AppState>, project_id: ProjectId) -> AppResult<AniListSuggestions> {
    let lib = state.library.clone();
    let engine = state.engine.clone();
    let models = state.models.clone();
    blocking(move || {
        let cancel = lib.anilist_token();
        embedder_ready(&models, &cancel)?;
        let portraits = portraits(&lib, &engine, project_id, &cancel)?;
        let looks = looks(&lib, &engine, project_id, &portraits, &cancel)?;
        let project = lib.project(project_id)?;
        let (rev, names, merges) = project.anilist_suggest_with(&portraits, looks.as_ref())?;
        let auto = project.anilist_auto()?;
        if auto {
            project.apply_anilist_suggestions(rev, &names, &merges)?;
        }
        Ok(AniListSuggestions {
            rev,
            auto,
            names: names.iter().map(suggestion_item).collect(),
            merges: merges.iter().map(merge_item).collect(),
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
