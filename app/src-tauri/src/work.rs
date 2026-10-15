//! Background work: ingest, playback proxies, analysis, exports and upkeep,
//! queued on `as-jobs` workers. Each job reports through `job:*` events and announces
//! what it changed (`media:changed`, `analysis:changed`, `library:changed`,
//! `export:changed`) so the UI refetches exactly what moved.

use std::sync::Arc;

use as_api::{AnalysisStage, ContentKind, JobKind, MediaItem, ProxyState, StageState};
use as_core::{ExportId, MediaId};
use as_engine::{EngineError, FacesEvent, ShotsEvent};
use as_jobs::{Class, JobCtx, JobError, JobSpec};
use as_models::ModelError;
use as_project::{Project, ProjectError};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::events::{AnalysisChanged, CastChanged, ExportChanged, LibraryChanged};
use crate::state::AppState;

fn job_error(e: ProjectError) -> JobError {
    match e {
        ProjectError::Cancelled => JobError::Cancelled,
        e => {
            log::warn!("{}", e.detail());
            JobError::Failed(e.to_string())
        }
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
        let language = state.library.audio_language().unwrap_or_default();
        let result = project.ingest(&tools, media_id, &language, &ctx.cancel, &|stage, p| ctx.progress(stage, p));
        if matches!(result, Err(ProjectError::Cancelled)) && !state.jobs.is_shutting_down() {
            let _ = project.set_media_failed(media_id, "Indexing was cancelled.");
        }
        media_changed(&app2, &project, Some(media_id));
        library_changed(&app2, &project);
        let m = result.map_err(job_error)?;
        for poster in [&m.poster, &m.poster_small].into_iter().flatten() {
            crate::state::allow_asset(&app2, std::path::Path::new(poster));
        }
        if m.proxy == ProxyState::Pending {
            queue_proxy(&app2, &project, media_id);
        }
        queue_analysis(&app2, &project, media_id);
        Ok(())
    });
}

/// Why an analysis stage stopped.
enum StageError {
    Cancelled,
    /// Something about this media or the analysis itself.
    Failed(String),
    /// The environment (offline model download, an engine that can't start):
    /// the stage waits as pending and runs again on the next launch.
    Later(String),
}

impl From<StageError> for JobError {
    fn from(e: StageError) -> Self {
        match e {
            StageError::Cancelled => JobError::Cancelled,
            StageError::Failed(m) | StageError::Later(m) => JobError::Failed(m),
        }
    }
}

fn stage_error(e: ProjectError) -> StageError {
    match job_error(e) {
        JobError::Cancelled => StageError::Cancelled,
        JobError::Failed(m) => StageError::Failed(m),
    }
}

fn engine_error(e: EngineError) -> StageError {
    match e {
        EngineError::Cancelled => StageError::Cancelled,
        e @ EngineError::Start { .. } => {
            log::warn!("analysis: {}", e.detail());
            StageError::Later(e.to_string())
        }
        e => {
            log::warn!("analysis: {}", e.detail());
            StageError::Failed(e.to_string())
        }
    }
}

/// Record how a stage ended (`set(state, error)`); a quit leaves it
/// `running` so it resumes on the next launch.
fn record_stage(
    state: &AppState,
    r: &Result<(), StageError>,
    set: impl FnOnce(StageState, Option<&str>) -> as_project::ProjectResult<()>,
) {
    let outcome = match r {
        Ok(()) => return,
        Err(StageError::Cancelled) if state.jobs.is_shutting_down() => return,
        // Stopped by the user: stays stopped (not resumed on the next launch).
        Err(StageError::Cancelled) => set(StageState::Stopped, None),
        Err(StageError::Later(msg)) => set(StageState::Pending, Some(msg)),
        Err(StageError::Failed(msg)) => set(StageState::Failed, Some(msg)),
    };
    if let Err(e) = outcome {
        log::warn!("recording the analysis state: {e}");
    }
}

/// Faces are found in live action; anime gets its own detectors later.
fn finds_faces(project: &Project) -> bool {
    !matches!(project.content(), Ok(ContentKind::Anime))
}

/// Media whose faces another embedder found are found again: embeddings of
/// two models can't be grouped together. Their decisions wait in
/// `track_edits`. Returns them (pending now, not queued). Only call it once
/// the current embedder is installed, so the old faces stay until then.
fn refresh_outdated_faces(app: &AppHandle, project: &Project) -> as_project::ProjectResult<Vec<MediaId>> {
    let ids = project.refresh_outdated_faces(as_models::ids::ARCFACE)?;
    if !ids.is_empty() {
        let title = project.title().unwrap_or_default();
        log::info!("'{title}': {} media get their faces found with the new model", ids.len());
        media_changed(app, project, None);
        library_changed(app, project);
        let _ = CastChanged { project_id: project.id }.emit(app);
    }
    Ok(ids)
}

/// Analyse a media in the engine: shots, then faces, then the project's
/// cast is clustered again. Resumes from checkpoints; downloads the models
/// it needs first. Nothing happens when its analysis is already queued or
/// running.
pub fn queue_analysis(app: &AppHandle, project: &Arc<Project>, media_id: MediaId) {
    let state = app.state::<AppState>();
    if state.jobs.is_active(|j| j.kind == JobKind::Analyze && j.media_id == Some(media_id)) {
        return;
    }
    submit_analysis(app, project, media_id);
}

/// Unregisters a running analysis however its job ends (a panic drops the
/// restart it owed: the stage stays as it was and can be retried).
struct Running<'a> {
    state: &'a AppState,
    media: MediaId,
}

impl Running<'_> {
    /// The job is done writing: the restart asked for meanwhile, if any.
    fn finish(self) -> Option<AnalysisStage> {
        let mut runs = self.state.analysis.lock();
        runs.running.remove(&self.media);
        let redo = runs.redo.remove(&self.media);
        drop(runs);
        std::mem::forget(self);
        redo
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let mut runs = self.state.analysis.lock();
        runs.running.remove(&self.media);
        runs.redo.remove(&self.media);
    }
}

/// Queue an analysis job for a media unconditionally (a duplicate finds
/// its stages done and ends at once: one analysis runs at a time).
fn submit_analysis(app: &AppHandle, project: &Arc<Project>, media_id: MediaId) {
    let state = app.state::<AppState>();
    let Ok(m) = project.media(media_id) else { return };
    let spec = JobSpec {
        kind: JobKind::Analyze,
        class: Class::Analysis,
        title: format!("Analysing {}", media_title(&m)),
        subtitle: project.title().ok(),
        project_id: Some(project.id),
        media_id: Some(media_id),
        export_id: None,
    };
    let (app2, project) = (app.clone(), project.clone());
    state.jobs.submit(spec, move |ctx| {
        let state = app2.state::<AppState>();
        state.analysis.lock().running.insert(media_id);
        let running = Running { state: &state, media: media_id };
        let _awake = crate::keep_awake::hold();
        let notify = || {
            let _ = AnalysisChanged { project_id: project.id, media_id }.emit(&app2);
            media_changed(&app2, &project, Some(media_id));
        };
        let shots = run_shots(&app2, &state, &project, media_id, ctx);
        record_stage(&state, &shots, |s, e| project.set_shots_state(media_id, s, e));
        notify();
        let result = shots.and_then(|()| {
            if !finds_faces(&project) {
                return Ok(());
            }
            let faces = run_faces(&app2, &state, &project, media_id, ctx);
            record_stage(&state, &faces, |s, e| project.set_faces_state(media_id, s, e));
            notify();
            faces
        });
        // Asked to start over while this ran: now nothing writes any more.
        if let Some(stage) = running.finish() {
            restart_analysis(&app2, &project, media_id, stage);
            return Ok(());
        }
        result.map_err(JobError::from)
    });
}

/// Clear a media's results from `stage` on and analyse it again. Only call
/// it when no analysis of the media runs (see [`AnalysisRuns`]).
///
/// [`AnalysisRuns`]: crate::state::AnalysisRuns
pub fn restart_analysis(app: &AppHandle, project: &Arc<Project>, media_id: MediaId, stage: AnalysisStage) {
    let reset = match stage {
        AnalysisStage::Shots => project.reset_shots(media_id),
        AnalysisStage::Faces => project.reset_faces(media_id),
    };
    // The cast forgets this media's faces; the user's decisions on them wait
    // for the new tracks.
    if let Err(e) = reset.and_then(|()| project.recluster().map(|_| ())) {
        log::warn!("resetting the analysis: {}", e.detail());
    }
    let _ = CastChanged { project_id: project.id }.emit(app);
    let _ = AnalysisChanged { project_id: project.id, media_id }.emit(app);
    media_changed(app, project, Some(media_id));
    submit_analysis(app, project, media_id);
}

fn run_shots(
    app: &AppHandle,
    state: &AppState,
    project: &Project,
    media_id: MediaId,
    ctx: &JobCtx,
) -> Result<(), StageError> {
    if project.media(media_id).map_err(stage_error)?.shots == StageState::Ready {
        return Ok(());
    }
    install_models(state, &[as_models::ids::TRANSNET], ctx)?;
    project.set_shots_state(media_id, StageState::Running, None).map_err(stage_error)?;
    media_changed(app, project, Some(media_id));
    let plan = project.shots_plan(media_id).map_err(stage_error)?;
    const STAGE: &str = "Finding shots";
    if plan.resume < plan.frames {
        ctx.progress(STAGE, plan.resume as f64 / plan.frames as f64);
        state
            .engine
            .shots(plan.path, plan.info, plan.index, plan.resume, plan.chunk, &ctx.cancel, |e| match e {
                ShotsEvent::Progress(f) => {
                    ctx.progress(STAGE, f);
                    Ok(())
                }
                ShotsEvent::Chunk { start, signals } => {
                    project.store_shot_chunk(media_id, start, &signals).map_err(|e| e.detail())
                }
            })
            .map_err(engine_error)?;
    }
    ctx.stage(STAGE);
    let shots = project.finish_shots(media_id).map_err(stage_error)?;
    log::info!("{shots} shots found (engine prefers {})", state.engine.device().unwrap_or_default());
    Ok(())
}

fn run_faces(
    app: &AppHandle,
    state: &AppState,
    project: &Arc<Project>,
    media_id: MediaId,
    ctx: &JobCtx,
) -> Result<(), StageError> {
    // Faces an older embedder found (anywhere in the project) are found
    // again before this pass adds new ones.
    let outdated = project.faces_by_other_model(as_models::ids::ARCFACE).map_err(stage_error)?;
    let ready = project.media(media_id).map_err(stage_error)?.faces == StageState::Ready;
    if ready && !outdated.contains(&media_id) {
        return Ok(());
    }
    if ready {
        // Its old faces stay until the new model is here: a failed or
        // cancelled download leaves the cast as it was (and tries again on
        // the next launch).
        if let Err(e) = install_models(state, &as_models::ids::FACES, ctx) {
            if !matches!(e, StageError::Cancelled) {
                log::warn!("finding faces again with the new model has to wait");
            }
            return Ok(());
        }
    } else {
        install_models(state, &as_models::ids::FACES, ctx)?;
    }
    if !outdated.is_empty() {
        for id in refresh_outdated_faces(app, project).map_err(stage_error)? {
            if id != media_id {
                queue_analysis(app, project, id);
            }
        }
    }
    project.set_faces_state(media_id, StageState::Running, None).map_err(stage_error)?;
    media_changed(app, project, Some(media_id));
    let plan = project.faces_plan(media_id, as_models::ids::ARCFACE).map_err(stage_error)?;
    const STAGE: &str = "Finding characters";
    if plan.resume < plan.frames {
        ctx.progress(STAGE, plan.resume as f64 / plan.frames as f64);
        state
            .engine
            .faces(plan.path, plan.info, plan.index, plan.shots, plan.resume, plan.chunk, &ctx.cancel, |e| match e {
                FacesEvent::Progress(f) => {
                    ctx.progress(STAGE, f);
                    Ok(())
                }
                FacesEvent::Chunk { start, end, tracks, sprites } => project
                    .store_face_chunk(media_id, start, end, &tracks, &sprites)
                    .map(|_| ())
                    .map_err(|e| e.detail()),
            })
            .map_err(engine_error)?;
    }
    ctx.stage("Grouping characters");
    project.finish_faces(media_id).map_err(stage_error)?;
    let characters = project.recluster().map_err(stage_error)?;
    log::info!("faces found; the cast has {characters} characters");
    let _ = CastChanged { project_id: project.id }.emit(app);
    Ok(())
}

/// Download AI models on first use (verified; resumes an interrupted download).
fn install_models(state: &AppState, ids: &[&str], ctx: &JobCtx) -> Result<(), StageError> {
    for &id in ids {
        let installed = state.models.installed(id).map_err(|e| StageError::Failed(e.to_string()))?;
        if installed.is_some() {
            continue;
        }
        const STAGE: &str = "Downloading AI model";
        ctx.progress(STAGE, 0.0);
        state
            .models
            .install(id, &ctx.cancel, |done, total| ctx.progress(STAGE, done as f64 / total.max(1) as f64))
            .map_err(|e| match e {
                ModelError::Cancelled => StageError::Cancelled,
                e => {
                    log::warn!("{}", e.detail());
                    StageError::Later(e.to_string())
                }
            })?;
    }
    Ok(())
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

/// Make posters from before the current sizes again (sharp full-bleed
/// artwork, small files for small places), one media at a time on the upkeep
/// worker: it never holds up new work. Each media and the Library card show
/// the new pictures as soon as they exist; until then the old ones stay.
pub fn queue_posters(app: &AppHandle, project: &Arc<Project>, ids: Vec<MediaId>) {
    let state = app.state::<AppState>();
    if ids.is_empty() || state.jobs.is_active(|j| j.kind == JobKind::Posters && j.project_id == Some(project.id)) {
        return;
    }
    let spec = JobSpec {
        kind: JobKind::Posters,
        class: Class::Upkeep,
        title: "Sharpening posters".into(),
        subtitle: project.title().ok(),
        project_id: Some(project.id),
        media_id: None,
        export_id: None,
    };
    let (app2, project) = (app.clone(), project.clone());
    state.jobs.submit(spec, move |ctx| {
        let state = app2.state::<AppState>();
        let tools = state.tools().map_err(|e| JobError::Failed(e.message))?;
        for (i, &id) in ids.iter().enumerate() {
            ctx.progress(&format!("Poster {} of {}", i + 1, ids.len()), i as f64 / ids.len() as f64);
            match project.refresh_posters(&tools, id, &ctx.cancel) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(ProjectError::Cancelled) => return Err(JobError::Cancelled),
                // The old poster stays; the next launch tries again.
                Err(e) => {
                    log::warn!("poster of {id}: {}", e.detail());
                    continue;
                }
            }
            // The Library card first: the project refetched on `media:changed`
            // must not show a picture that was just removed.
            sync_library_posters(&app2, &project);
            media_changed(&app2, &project, Some(id));
        }
        Ok(())
    });
}

/// Show a project's current posters on its Library card, without moving it
/// to the front.
fn sync_library_posters(app: &AppHandle, project: &Project) {
    match app.state::<AppState>().library.refresh_posters(project) {
        Ok(true) => {
            let _ = LibraryChanged { project_id: Some(project.id) }.emit(app);
        }
        Ok(false) => {}
        Err(e) => log::warn!("refreshing project posters: {e}"),
    }
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
        let result = project.run_export(&tools, export_id, &ctx.cancel, &|stage, p| ctx.progress(stage, p), &notify);
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

/// On launch: reopen every project, resume interrupted indexing, pending
/// previews and analyses, and make outdated posters again.
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
        // A quit between a poster refresh and its Library card leaves the card
        // on a removed picture; the project's own database knows better.
        sync_library_posters(app, &project);
        match project.recover() {
            Ok(r) => {
                if !r.ingest.is_empty() || !r.proxies.is_empty() || !r.posters.is_empty() {
                    log::info!(
                        "resuming '{}': {} to index, {} previews, {} posters",
                        summary.title,
                        r.ingest.len(),
                        r.proxies.len(),
                        r.posters.len()
                    );
                }
                for id in r.ingest {
                    queue_ingest(app, &project, id);
                }
                for id in r.proxies {
                    queue_proxy(app, &project, id);
                }
                queue_posters(app, &project, r.posters);
            }
            Err(e) => log::warn!("recovering '{}': {e}", summary.title),
        }
        let pending = project.pending_shots().and_then(|mut ids| {
            if finds_faces(&project) {
                // Faces of an older embedder: their analysis installs the new
                // one first, then finds them again (see `run_faces`).
                for id in project.faces_by_other_model(as_models::ids::ARCFACE)? {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                for id in project.pending_faces()? {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            Ok(ids)
        });
        match pending {
            Ok(ids) => {
                if !ids.is_empty() {
                    log::info!("resuming '{}': {} to analyse", summary.title, ids.len());
                }
                for id in ids {
                    queue_analysis(app, &project, id);
                }
            }
            Err(e) => log::warn!("recovering analysis of '{}': {e}", summary.title),
        }
    }
}
