//! Background work: ingest, playback proxies, analysis (shots, faces and —
//! when the setting asks for it — people), exports and upkeep,
//! queued on `as-jobs` workers. Each job reports through `job:*` events and announces
//! what it changed (`media:changed`, `analysis:changed`, `library:changed`,
//! `export:changed`, `cast:changed`, `portraits:changed`) so the UI refetches
//! exactly what moved.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use as_api::{
    AnalysisStage, CastEdit, ContentKind, ExportSummary, JobInfo, JobKind, JobState, MediaItem, MediaStatus,
    ProxyState, StageState,
};
use as_core::{MediaId, ProjectId};
use as_engine::{Devices, EngineError, FacesEvent, FacesInput, ShotsEvent};
use as_jobs::{Class, JobCtx, JobError, JobSpec};
use as_models::ModelError;
use as_project::{Pass, Portrait, Project, ProjectError};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::events::{AnalysisChanged, CastChanged, ExportChanged, LibraryChanged, PortraitsChanged};
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

/// The cast of a project changed: the UI refetches it, and covers new since
/// get their portraits (see [`queue_portraits`]) — looked for off the
/// caller's thread: a decision in the review answers at once.
pub fn cast_changed(app: &AppHandle, project: &Arc<Project>) {
    let _ = CastChanged { project_id: project.id }.emit(app);
    let (app, project) = (app.clone(), project.clone());
    tauri::async_runtime::spawn_blocking(move || queue_portraits(&app, &project));
}

/// Decisions come in runs (the review answers a face per key press): the
/// other faces are placed again once they pause this long.
const PLACE_FACES_AFTER: std::time::Duration = std::time::Duration::from_millis(1200);

/// The user decided about the cast. The UI shows the decision at once; when
/// it changed who a face is, the other faces follow a moment after the last
/// such decision ([`Project::reassign`]), and the UI refetches again.
pub fn cast_edited(app: &AppHandle, project: &Arc<Project>, edit: &CastEdit) {
    cast_changed(app, project);
    if !edit.moves_faces() {
        return;
    }
    let state = app.state::<AppState>();
    let decision = {
        let mut decisions = state.decisions.lock();
        let n = decisions.entry(project.id).or_default();
        *n += 1;
        *n
    };
    // Only the id waits: a project closed or deleted meanwhile isn't held open.
    let (app, id) = (app.clone(), project.id);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(PLACE_FACES_AFTER).await;
        let state = app.state::<AppState>();
        if state.decisions.lock().get(&id).copied() != Some(decision) {
            return;
        }
        let lib = state.library.clone();
        let placed = tauri::async_runtime::spawn_blocking(move || {
            let project = lib.project(id)?;
            project.reassign().map(|()| project)
        })
        .await;
        match placed {
            Ok(Ok(project)) => cast_changed(&app, &project),
            Ok(Err(e)) => log::warn!("placing faces after a decision: {}", e.detail()),
            Err(e) => log::warn!("placing faces after a decision: {e}"),
        }
    });
}

/// Refresh the project's Library card after its media changed.
pub fn library_changed(app: &AppHandle, project: &Project) {
    let state = app.state::<AppState>();
    if let Err(e) = state.library.refresh_stats(project) {
        log::warn!("refreshing project stats: {}", e.detail());
    }
    let _ = LibraryChanged { project_id: Some(project.id) }.emit(app);
}

/// Runs `then` when the job it is moved into is dropped without having
/// started (cancelled while queued, refused as a duplicate, dropped by a
/// shutdown); [`Unstarted::started`] disarms it.
struct Unstarted<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Unstarted<F> {
    fn started(mut self) {
        self.0 = None;
    }
}

impl<F: FnOnce()> Drop for Unstarted<F> {
    fn drop(&mut self) {
        if let Some(then) = self.0.take() {
            then();
        }
    }
}

/// Index a media (probe, frame index, poster). Queues its preview proxy if it needs one.
pub fn queue_ingest(app: &AppHandle, project: &Arc<Project>, media_id: MediaId) {
    let state = app.state::<AppState>();
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
    let unstarted = {
        let (app, project) = (app.clone(), project.clone());
        Unstarted(Some(move || ingest_dropped(&app, &project, media_id)))
    };
    let (app2, project) = (app.clone(), project.clone());
    let indexing = move |j: &JobInfo| j.kind == JobKind::Ingest && j.media_id == Some(media_id);
    state.jobs.submit_unless(indexing, spec, move |ctx| {
        unstarted.started();
        let state = app2.state::<AppState>();
        let tools = state.tools().map_err(|e| JobError::Failed(e.message))?;
        let language = state.library.audio_language().unwrap_or_else(|e| {
            log::warn!("preferred audio language: {}", e.detail());
            String::new()
        });
        let result = project.ingest(&tools, media_id, &language, &ctx.cancel, &|stage, p| ctx.report(stage, p));
        if matches!(result, Err(ProjectError::Cancelled))
            && !state.jobs.is_shutting_down()
            && let Err(e) = project.set_media_failed(media_id, "Indexing was cancelled.")
        {
            log::warn!("marking a cancelled ingest: {}", e.detail());
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
        // A file that was missing is back (its faces' covers can get
        // portraits), or its analysis went with a new frame index.
        cast_changed(&app2, &project);
        Ok(())
    });
}

/// An ingest dropped before it started. Cancelled (from Activity), the
/// media says so and offers Retry, as when it is stopped while indexing —
/// not "Queued" with nothing to index it, and not indexed anyway on the
/// next launch. A shutdown leaves it queued for the next launch, and a
/// duplicate refused leaves it to the job that is there.
fn ingest_dropped(app: &AppHandle, project: &Project, media_id: MediaId) {
    let state = app.state::<AppState>();
    if state.jobs.is_shutting_down()
        || state.jobs.is_active(|j| j.kind == JobKind::Ingest && j.media_id == Some(media_id))
        || !project.media(media_id).is_ok_and(|m| m.status == MediaStatus::Queued)
    {
        return;
    }
    match project.set_media_failed(media_id, "Indexing was cancelled.") {
        Ok(()) => media_changed(app, project, Some(media_id)),
        Err(e) => log::warn!("marking a cancelled ingest: {}", e.detail()),
    }
}

/// Why an analysis stage stopped.
enum StageError {
    Cancelled,
    /// Something about this media or the analysis itself.
    Failed(String),
    /// The environment (offline model download, an engine that can't start):
    /// the stage waits as pending and runs again on the next launch.
    Later(String),
    /// A panic while the stage wasn't recorded running (before it started,
    /// or once it was done, grouping the cast): the stage stays as it is
    /// recorded, the job fails.
    Bug(String),
}

impl From<StageError> for JobError {
    fn from(e: StageError) -> Self {
        match e {
            StageError::Cancelled => JobError::Cancelled,
            StageError::Failed(m) | StageError::Later(m) | StageError::Bug(m) => JobError::Failed(m),
        }
    }
}

/// Run an analysis stage; a panic in it (a bug in the app's own code: the
/// engine runs in a process of its own) ends it like a failure. A stage
/// recorded running (`recorded`) is recorded failed with what the panic
/// said: the job's own catch would leave it "running" until the next launch,
/// re-run it on every launch, and drop a restart asked for meanwhile.
fn guarded(
    recorded: impl FnOnce() -> Option<StageState>,
    run: impl FnOnce() -> Result<(), StageError>,
) -> Result<(), StageError> {
    catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|panic| {
        let msg = as_jobs::panic_message(&*panic);
        log::error!("analysis panicked: {msg}");
        match recorded() {
            Some(StageState::Running) => Err(StageError::Failed(msg)),
            _ => Err(StageError::Bug(msg)),
        }
    })
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
        Ok(()) | Err(StageError::Bug(_)) => return,
        Err(StageError::Cancelled) if state.jobs.is_shutting_down() => return,
        // Stopped by the user: stays stopped (not resumed on the next launch).
        Err(StageError::Cancelled) => set(StageState::Stopped, None),
        Err(StageError::Later(msg)) => set(StageState::Pending, Some(msg)),
        Err(StageError::Failed(msg)) => set(StageState::Failed, Some(msg)),
    };
    if let Err(e) = outcome {
        log::warn!("recording the analysis state: {}", e.detail());
    }
}

/// Characters are looked for in live action and 3D animation. Anime uses
/// its own detectors and embedder (as_analysis::anime): enabled by the
/// `AUTOSCENE_ANIME_FACES` environment switch until the anime benchmark's
/// sealed test releases the feature (see bench/anime).
fn finds_characters(project: &Project) -> bool {
    match project.content() {
        Ok(ContentKind::Anime) => as_project::anime_faces_enabled(),
        _ => true,
    }
}

/// Media whose faces another embedder found (another model, or the other of
/// Fast Scan and the full pass) are found again: embeddings of two models
/// can't be grouped together. Their decisions wait in
/// `track_edits`. Returns them (pending now, not queued). Only call it once
/// the current embedder is installed, so the old faces stay until then.
fn refresh_outdated_faces(
    app: &AppHandle,
    project: &Arc<Project>,
    model: &str,
) -> as_project::ProjectResult<Vec<MediaId>> {
    let ids = project.refresh_outdated_faces(model)?;
    if !ids.is_empty() {
        let title = project.title().unwrap_or_default();
        log::info!("'{title}': {} media get their faces found with the new model", ids.len());
        media_changed(app, project, None);
        library_changed(app, project);
        cast_changed(app, project);
    }
    Ok(ids)
}

/// Analyse a media in the engine: shots, then faces, then the project's
/// cast is clustered again. Resumes from checkpoints; downloads the models
/// it needs first. Nothing happens when its analysis is already queued or
/// running.
/// Analyse again the media whose faces don't fit the project any more: an
/// older embedder found them, or another search than its footage's (3D
/// animation is searched harder). Their old faces stay until then.
pub fn queue_outdated_faces(app: &AppHandle, project: &Arc<Project>) {
    if !finds_characters(project) {
        return;
    }
    match project.faces_by_other_model(project.face_embedder()) {
        Ok(ids) => {
            for id in ids {
                if project.media(id).is_ok_and(|m| m.status != MediaStatus::Missing) {
                    queue_analysis(app, project, id);
                }
            }
        }
        Err(e) => log::warn!("checking for outdated faces: {}", e.detail()),
    }
}

pub fn queue_analysis(app: &AppHandle, project: &Arc<Project>, media_id: MediaId) {
    submit_analysis(app, project, media_id, true);
}

/// Unregisters a running analysis however its job ends (a panic in a stage
/// ends that stage as failed, and the restart owed is made; one anywhere
/// else drops it: the stages stay as they were and can be retried).
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

/// Queue an analysis job for a media; `only_one`: unless one is queued or
/// running already. (A duplicate finds its stages done and ends at once:
/// one analysis runs at a time.)
fn submit_analysis(app: &AppHandle, project: &Arc<Project>, media_id: MediaId, only_one: bool) {
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
    let analysing = move |j: &JobInfo| only_one && j.kind == JobKind::Analyze && j.media_id == Some(media_id);
    state.jobs.submit_unless(analysing, spec, move |ctx| {
        let state = app2.state::<AppState>();
        state.analysis.lock().running.insert(media_id);
        let running = Running { state: &state, media: media_id };
        let _awake = crate::keep_awake::hold();
        let notify = || {
            let _ = AnalysisChanged { project_id: project.id, media_id }.emit(&app2);
            media_changed(&app2, &project, Some(media_id));
        };
        let recorded = |stage: fn(&MediaItem) -> StageState| project.media(media_id).ok().map(|m| stage(&m));
        let shots = guarded(|| recorded(|m| m.shots), || run_shots(&app2, &state, &project, media_id, ctx));
        record_stage(&state, &shots, |s, e| project.set_shots_state(media_id, s, e));
        notify();
        let result = shots.and_then(|()| {
            if !finds_characters(&project) {
                return Ok(());
            }
            let faces = guarded(|| recorded(|m| m.faces), || run_faces(&app2, &state, &project, media_id, ctx));
            record_stage(&state, &faces, |s, e| project.set_faces_state(media_id, s, e));
            notify();
            faces?;
            // Anime finds its characters whole: no one is looked for by
            // their body, however the setting reads.
            let anime = project.content().map(|c| c == ContentKind::Anime).unwrap_or(false);
            if !project.finds_bodies() || anime {
                return Ok(());
            }
            let bodies = guarded(|| recorded(|m| m.bodies), || run_bodies(&app2, &state, &project, media_id, ctx));
            record_stage(&state, &bodies, |s, e| project.set_bodies_state(media_id, s, e));
            notify();
            bodies
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
    cast_changed(app, project);
    let _ = AnalysisChanged { project_id: project.id, media_id }.emit(app);
    media_changed(app, project, Some(media_id));
    submit_analysis(app, project, media_id, false);
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
        let mut stage = STAGE.to_owned();
        state
            .engine
            .shots(plan.path, plan.info, plan.index, plan.resume, plan.chunk, &ctx.cancel, |e| match e {
                ShotsEvent::Devices { devices, resume } => {
                    stage = devices_reported(project, media_id, Pass::Shots, STAGE, &devices, resume);
                    Ok(())
                }
                ShotsEvent::Progress(f) => {
                    ctx.progress(&stage, f);
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
    let model = project.face_embedder();
    let models = as_project::face_models(model);
    let outdated = project.faces_by_other_model(model).map_err(stage_error)?;
    let ready = project.media(media_id).map_err(stage_error)?.faces == StageState::Ready;
    if ready && !outdated.contains(&media_id) {
        return Ok(());
    }
    if ready {
        // Its old faces stay until the new model is here: a failed or
        // cancelled download leaves the cast as it was (and tries again on
        // the next launch).
        if let Err(e) = install_models(state, models, ctx) {
            if !matches!(e, StageError::Cancelled) {
                log::warn!("finding faces again with the new model has to wait");
            }
            return Ok(());
        }
    } else {
        install_models(state, models, ctx)?;
    }
    if !outdated.is_empty() {
        for id in refresh_outdated_faces(app, project, model).map_err(stage_error)? {
            if id != media_id {
                queue_analysis(app, project, id);
            }
        }
    }
    // People are found in the same pass when their models are here; when
    // they can't be downloaded now, the people pass finds them later.
    // Neither the fast pass nor the anime pass look for anyone by their body.
    let mut people = model != as_models::ids::SFACE
        && model != as_models::ids::ANIME_CCIP
        && project.finds_bodies()
        && match install_models(state, &as_models::ids::BODIES, ctx) {
            Ok(()) => true,
            Err(StageError::Cancelled) => return Err(StageError::Cancelled),
            Err(_) => false,
        };
    project.set_faces_state(media_id, StageState::Running, None).map_err(stage_error)?;
    media_changed(app, project, Some(media_id));
    const STAGE: &str = "Finding characters";
    loop {
        let plan = project.faces_plan(media_id, model).map_err(stage_error)?;
        if plan.resume >= plan.frames {
            break;
        }
        ctx.progress(STAGE, plan.resume as f64 / plan.frames as f64);
        let job = FacesInput {
            path: plan.path,
            info: plan.info,
            index: plan.index,
            shots: plan.shots,
            resume: plan.resume,
            chunk: plan.chunk,
            bodies: people,
            known: None,
            animated: plan.animated,
            fast: plan.fast,
            anime: plan.anime,
        };
        let mut stage = STAGE.to_owned();
        let found = state.engine.faces(job, &ctx.cancel, |e| match e {
            FacesEvent::Devices { devices, resume } => {
                stage = devices_reported(project, media_id, Pass::Faces, STAGE, &devices, resume);
                Ok(())
            }
            FacesEvent::Progress(f) => {
                ctx.progress(&stage, f);
                Ok(())
            }
            FacesEvent::Chunk { start, end, tracks, sprites, bodies } => project
                .store_face_chunk(media_id, start, end, &tracks, &sprites, &bodies)
                .map(|_| ())
                .map_err(|e| e.detail()),
        });
        match found {
            Ok(()) => break,
            // The optional people never cost the faces: the faces go on
            // alone from the last checkpoint, the people pass finds the rest
            // of the people after (or fails on its own).
            Err(e @ (EngineError::Failed { .. } | EngineError::Crashed(_))) if people => {
                log::warn!("finding people with the faces: {}; the faces go on alone", e.detail());
                people = false;
            }
            Err(e) => return Err(engine_error(e)),
        }
    }
    // The project moved to the other pipeline (Fast Scan, 3D animation)
    // while this pass ran: its faces are found again the new way.
    if project.faces_outdated(media_id).map_err(stage_error)? {
        log::info!("faces are now found another way; finding them again");
        return run_faces(app, state, project, media_id, ctx);
    }
    ctx.stage("Grouping characters");
    project.finish_faces(media_id).map_err(stage_error)?;
    let characters = project.recluster().map_err(stage_error)?;
    log::info!("faces found; the cast has {characters} characters");
    cast_changed(app, project);
    Ok(())
}

/// Find a media's people for its faces (when the setting asks for it):
/// whatever the face pass didn't find with them — all of them, for a media
/// whose faces were found before — then the user's word on them is put
/// back and every face placed again with them.
fn run_bodies(
    app: &AppHandle,
    state: &AppState,
    project: &Arc<Project>,
    media_id: MediaId,
    ctx: &JobCtx,
) -> Result<(), StageError> {
    let m = project.media(media_id).map_err(stage_error)?;
    if m.bodies == StageState::Ready || m.faces != StageState::Ready {
        return Ok(());
    }
    install_models(state, &as_models::ids::BODIES, ctx)?;
    project.set_bodies_state(media_id, StageState::Running, None).map_err(stage_error)?;
    media_changed(app, project, Some(media_id));
    let plan = project.bodies_plan(media_id).map_err(stage_error)?;
    const STAGE: &str = "Finding people";
    if plan.resume < plan.frames {
        ctx.progress(STAGE, plan.resume as f64 / plan.frames as f64);
        let job = FacesInput {
            path: plan.path,
            info: plan.info,
            index: plan.index,
            shots: plan.shots,
            resume: plan.resume,
            chunk: plan.chunk,
            bodies: true,
            known: Some(plan.known),
            animated: false,
            fast: false,
            anime: false,
        };
        let mut stage = STAGE.to_owned();
        state
            .engine
            .faces(job, &ctx.cancel, |e| match e {
                FacesEvent::Devices { devices, resume } => {
                    stage = devices_reported(project, media_id, Pass::People, STAGE, &devices, resume);
                    Ok(())
                }
                FacesEvent::Progress(f) => {
                    ctx.progress(&stage, f);
                    Ok(())
                }
                FacesEvent::Chunk { start, end, bodies, .. } => {
                    project.store_body_chunk(media_id, start, end, &bodies).map(|_| ()).map_err(|e| e.detail())
                }
            })
            .map_err(engine_error)?;
    }
    ctx.stage("Grouping characters");
    project.finish_bodies(media_id).map_err(stage_error)?;
    project.reassign().map_err(stage_error)?;
    log::info!("people found");
    cast_changed(app, project);
    Ok(())
}

/// The setting to find people by their bodies changed: every project
/// places its faces again with (or without) them — and when it is on, the
/// media whose people aren't found yet are analysed for them.
pub fn bodies_setting_changed(app: &AppHandle, on: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let projects = match state.library.projects() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("listing projects: {}", e.detail());
                return;
            }
        };
        for summary in projects {
            let project = match state.library.project(summary.id) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("project '{}' unavailable: {}", summary.title, e.detail());
                    continue;
                }
            };
            if !finds_characters(&project) {
                continue;
            }
            if let Err(e) = project.reassign() {
                log::warn!("placing the faces of '{}' again: {}", summary.title, e.detail());
            }
            cast_changed(&app, &project);
            if on {
                // A people pass left "running" with nothing running it (a
                // quit or failed update install mid-way) waits too; one
                // under way isn't queued again (see `queue_analysis`).
                let waiting = project.waiting_bodies().and_then(|mut ids| {
                    for m in project.media_list_uncounted()? {
                        let stuck = m.bodies == StageState::Running && m.faces == StageState::Ready;
                        if stuck && m.status == MediaStatus::Ready && !ids.contains(&m.id) {
                            ids.push(m.id);
                        }
                    }
                    Ok(ids)
                });
                match waiting {
                    Ok(ids) => {
                        let ids: Vec<MediaId> = ids
                            .into_iter()
                            .filter(|&id| project.media(id).is_ok_and(|m| m.status != MediaStatus::Missing))
                            .collect();
                        if !ids.is_empty() {
                            log::info!("'{}': {} media get their people found", summary.title, ids.len());
                        }
                        for id in ids {
                            queue_analysis(&app, &project, id);
                        }
                    }
                    Err(e) => log::warn!("media of '{}' without their people: {}", summary.title, e.detail()),
                }
            }
        }
    });
}

/// Where a pass's models run, as the engine says when it starts on it: the
/// project records the devices with the pass's checkpoints (noting a pass
/// resumed on others), and a pass the GPU couldn't take says in its stage
/// that it runs on the CPU, many times slower. Returns the stage to show.
fn devices_reported(
    project: &Project,
    media_id: MediaId,
    pass: Pass,
    stage: &str,
    devices: &Devices,
    resume: u32,
) -> String {
    match project.record_devices(media_id, pass, &devices.summary(), resume) {
        Ok(Some(before)) => log::warn!(
            "{stage}: resumed at frame {resume} with {} after checkpoints made with {before}; \
             its numbers may differ from an uninterrupted run's in the last digits",
            devices.summary()
        ),
        Ok(None) => {}
        Err(e) => log::warn!("recording the devices of '{stage}': {}", e.detail()),
    }
    let on_cpu = devices.on_cpu();
    if on_cpu.is_empty() {
        return stage.to_owned();
    }
    log::warn!("{stage}: the GPU couldn't take {}; on the CPU (slower)", on_cpu.join(", "));
    format!("{stage} · on the CPU (slower)")
}

/// Download AI models on first use (verified; resumes an interrupted download).
fn install_models(state: &AppState, ids: &[&str], ctx: &JobCtx) -> Result<(), StageError> {
    for &id in ids {
        let installed = state.models.installed(id).map_err(|e| StageError::Failed(e.to_string()))?;
        if installed.is_some() {
            continue;
        }
        const STAGE: &str = "Downloading AI model";
        // Unknown until the first count: a resumed download starts at the
        // bytes it has, which mustn't count as fetched in no time.
        ctx.stage(STAGE);
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
        let result = project.build_proxy(&tools, media_id, &ctx.cancel, &|stage, p| ctx.report(stage, p));
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
    if ids.is_empty() {
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
    let pid = project.id;
    let sharpening = move |j: &JobInfo| j.kind == JobKind::Posters && j.project_id == Some(pid);
    state.jobs.submit_unless(sharpening, spec, move |ctx| {
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

/// Bring a project's cast up to date on the upkeep worker (see
/// [`Project::cast_upkeep`]): grouped by an older version of the grouping,
/// it is grouped again — the user's decisions stay, characters that turn
/// out to be one person become one — and covers made before show the
/// character, not the face in front of them.
pub fn queue_regroup(app: &AppHandle, project: &Arc<Project>) {
    let state = app.state::<AppState>();
    let spec = JobSpec {
        kind: JobKind::Regroup,
        class: Class::Upkeep,
        title: "Grouping characters".into(),
        subtitle: project.title().ok(),
        project_id: Some(project.id),
        media_id: None,
        export_id: None,
    };
    let (app2, project) = (app.clone(), project.clone());
    // One waiting is enough; one already running may have read the cast
    // (or footage) before the change that queues this.
    let pid = project.id;
    let waiting =
        move |j: &JobInfo| j.kind == JobKind::Regroup && j.project_id == Some(pid) && j.state == JobState::Queued;
    state.jobs.submit_unless(waiting, spec, move |ctx| {
        ctx.stage("Grouping characters");
        if let Some(characters) = project.cast_upkeep(&ctx.cancel).map_err(job_error)? {
            log::info!("characters grouped again; the cast has {characters} characters");
        }
        cast_changed(&app2, &project);
        Ok(())
    });
}

/// How often the UI hears of new portraits while they are being made.
const PORTRAITS_SHOWN_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// Make the missing portraits of character covers — 3:4, from the exact
/// frame of the cover's clearest face at the source's full resolution — on
/// the upkeep worker, in the order characters were found (the main cast
/// first, mostly). A card shows its face thumbnail until its portrait
/// exists, then turns sharp.
pub fn queue_portraits(app: &AppHandle, project: &Arc<Project>) {
    let state = app.state::<AppState>();
    // A job queued or running looks again before it ends.
    if let Some(changed) = state.portraits.lock().get_mut(&project.id) {
        *changed = true;
        return;
    }
    match project.missing_portraits() {
        Ok(missing) if missing.is_empty() => return,
        Ok(_) => {}
        Err(e) => {
            log::warn!("covers without portraits: {}", e.detail());
            return;
        }
    }
    // Another change may have queued one meanwhile.
    let mut job = {
        let mut jobs = state.portraits.lock();
        if let Some(changed) = jobs.get_mut(&project.id) {
            *changed = true;
            return;
        }
        jobs.insert(project.id, false);
        PortraitsJob { jobs: state.portraits.clone(), project: project.id, ended: false }
    };
    let spec = JobSpec {
        kind: JobKind::Portraits,
        class: Class::Upkeep,
        title: "Sharpening portraits".into(),
        subtitle: project.title().ok(),
        project_id: Some(project.id),
        media_id: None,
        export_id: None,
    };
    let (app2, project) = (app.clone(), project.clone());
    state.jobs.submit(spec, move |ctx| {
        let state = app2.state::<AppState>();
        let tools = state.tools().map_err(|e| JobError::Failed(e.message))?;
        // The cast is fetched again when portraits are new, at most every
        // couple of seconds: cards turn sharp in batches, not one by one.
        let changed = || {
            let _ = PortraitsChanged { project_id: project.id }.emit(&app2);
        };
        let (mut unseen, mut told) = (false, std::time::Instant::now());
        // What's missing now, and again while the cast changed meanwhile.
        loop {
            let tracks = project.missing_portraits().map_err(job_error)?;
            for (i, &track) in tracks.iter().enumerate() {
                ctx.progress(&format!("Portrait {} of {}", i + 1, tracks.len()), i as f64 / tracks.len() as f64);
                match project.portrait(&tools, track, &ctx.cancel) {
                    Ok(Portrait::Made(_)) => unseen = true,
                    Ok(Portrait::Ready(_) | Portrait::Unavailable) => {}
                    Err(ProjectError::Cancelled) => {
                        if unseen {
                            changed();
                        }
                        return Err(JobError::Cancelled);
                    }
                    // The thumbnail stays; the next launch tries again.
                    Err(e) => log::warn!("portrait of track {track}: {}", e.detail()),
                }
                if unseen && told.elapsed() >= PORTRAITS_SHOWN_EVERY {
                    changed();
                    (unseen, told) = (false, std::time::Instant::now());
                }
            }
            if !job.look_again() {
                break;
            }
        }
        if unseen {
            changed();
        }
        Ok(())
    });
}

/// A project's portraits job as `AppState::portraits` knows it: unregistered
/// when it ends, however it ends — also when it is dropped unstarted
/// (cancelled while queued).
struct PortraitsJob {
    jobs: Arc<parking_lot::Mutex<HashMap<ProjectId, bool>>>,
    project: ProjectId,
    ended: bool,
}

impl PortraitsJob {
    /// Whether the cast changed since the job last looked; if not, the job
    /// ends — under the same lock, so a change can't slip in between.
    fn look_again(&mut self) -> bool {
        let mut jobs = self.jobs.lock();
        if jobs.get(&self.project) == Some(&true) {
            jobs.insert(self.project, false);
            return true;
        }
        jobs.remove(&self.project);
        self.ended = true;
        false
    }
}

impl Drop for PortraitsJob {
    fn drop(&mut self) {
        if !self.ended {
            self.jobs.lock().remove(&self.project);
        }
    }
}

/// Show a project's current posters on its Library card, without moving it
/// to the front.
fn sync_library_posters(app: &AppHandle, project: &Project) {
    match app.state::<AppState>().library.refresh_posters(project) {
        Ok(true) => {
            let _ = LibraryChanged { project_id: Some(project.id) }.emit(app);
        }
        Ok(false) => {}
        Err(e) => log::warn!("refreshing project posters: {}", e.detail()),
    }
}

/// Render an export created with `Project::create_export`.
pub fn queue_export(app: &AppHandle, project: &Arc<Project>, export: &ExportSummary) {
    let state = app.state::<AppState>();
    let export_id = export.id;
    let clips = export.items.len();
    let title = match (clips, export.merged.is_some()) {
        (1, false) => "Exporting 1 clip".into(),
        (1, true) => "Exporting 1 clip into one file".into(),
        (n, false) => format!("Exporting {n} clips"),
        (n, true) => format!("Exporting {n} clips into one file"),
    };
    let spec = JobSpec {
        kind: JobKind::Export,
        class: Class::Export,
        title,
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
                if let Err(fail) = project.fail_export(export_id, &e.message) {
                    log::warn!("marking export {export_id} failed: {}", fail.detail());
                }
                notify();
                return Err(JobError::Failed(e.message));
            }
        };
        notify();
        let run = || project.run_export(&tools, export_id, &ctx.cancel, &|stage, p| ctx.report(stage, p), &notify);
        // A bug in it fails it like an error of its own: it doesn't show
        // "Exporting" until the next launch.
        let result = catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|panic| {
            let msg = as_jobs::panic_message(&*panic);
            log::error!("export {export_id} panicked: {msg}");
            Err(ProjectError::Invalid(msg))
        });
        if let Err(e) = &result
            && !matches!(e, ProjectError::Cancelled)
            && let Err(fail) = project.abandon_export(export_id, &e.to_string())
        {
            log::warn!("marking export {export_id} failed: {}", fail.detail());
        }
        notify();
        let summary = result.map_err(job_error)?;
        if let Some(error) = summary.merged.as_ref().and_then(|f| f.error.clone()) {
            return Err(JobError::Failed(error));
        }
        let failed = summary.items.iter().filter(|i| i.error.is_some()).count();
        if failed > 0 {
            return Err(JobError::Failed(failed_message(failed, summary.items.len(), summary.merged.is_some())));
        }
        Ok(())
    });
}

/// What an export with `failed` of its `clips` failing says.
fn failed_message(failed: usize, clips: usize, one_file: bool) -> String {
    let what = if failed == 1 { "1 clip".to_string() } else { format!("{failed} clips") };
    match (one_file, clips.saturating_sub(failed)) {
        (false, _) => format!("{what} failed to export."),
        (true, 1) => format!("{what} couldn't be exported; the file has the other one."),
        (true, rest) => format!("{what} couldn't be exported; the file has the other {rest}."),
    }
}

/// On launch (and when a shutdown for an update is undone, see
/// `state::resume_work`): reopen every project, resume interrupted
/// indexing, pending previews and analyses, make outdated posters again and
/// the portraits covers don't have yet.
pub fn recover_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let projects = match state.library.projects() {
        Ok(p) => p,
        Err(e) => {
            log::error!("listing projects: {}", e.detail());
            return;
        }
    };
    for summary in projects {
        let project = match state.library.project(summary.id) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("project '{}' unavailable: {}", summary.title, e.detail());
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
            Err(e) => log::warn!("recovering '{}': {}", summary.title, e.detail()),
        }
        // Before the portraits: covers may change.
        match project.cast_needs_upkeep() {
            Ok(true) => {
                log::info!("'{}': the cast is brought up to date", summary.title);
                queue_regroup(app, &project);
            }
            Ok(false) => {}
            Err(e) => log::warn!("checking the cast of '{}': {}", summary.title, e.detail()),
        }
        queue_portraits(app, &project);
        let pending = project.pending_shots().and_then(|mut ids| {
            if finds_characters(&project) {
                // Faces of an older embedder: their analysis installs the new
                // one first, then finds them again (see `run_faces`).
                for id in project.faces_by_other_model(project.face_embedder())? {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                for id in project.pending_faces()? {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                if project.finds_bodies() {
                    for id in project.pending_bodies()? {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                }
            }
            Ok(ids)
        });
        // Files that aren't there wait (pending) instead of failing.
        let pending = pending.map(|ids| {
            ids.into_iter()
                .filter(|&id| project.media(id).is_ok_and(|m| m.status != MediaStatus::Missing))
                .collect::<Vec<_>>()
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
            Err(e) => log::warn!("recovering analysis of '{}': {}", summary.title, e.detail()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use as_api::StageState;

    use super::{StageError, Unstarted, failed_message, guarded};

    #[test]
    fn a_stage_that_panics_fails_only_while_it_runs() {
        let breaks = || -> Result<(), StageError> { panic!("grouping broke") };
        let r = guarded(|| Some(StageState::Running), breaks);
        assert!(matches!(r, Err(StageError::Failed(m)) if m == "grouping broke"), "recorded failed");
        // Done before it broke (or not begun): it stays as it is recorded.
        let r = guarded(|| Some(StageState::Ready), breaks);
        assert!(matches!(r, Err(StageError::Bug(m)) if m == "grouping broke"));
        assert!(matches!(guarded(|| unreachable!(), || Err(StageError::Cancelled)), Err(StageError::Cancelled)));
    }

    #[test]
    fn work_dropped_unstarted_says_so() {
        let told = Cell::new(0);
        drop(Unstarted(Some(|| told.set(told.get() + 1))));
        Unstarted(Some(|| told.set(told.get() + 10))).started();
        assert_eq!(told.get(), 1, "only the work that never started");
    }

    #[test]
    fn failures_say_what_the_file_still_has() {
        assert_eq!(failed_message(1, 3, false), "1 clip failed to export.");
        assert_eq!(failed_message(2, 3, false), "2 clips failed to export.");
        assert_eq!(failed_message(1, 2, true), "1 clip couldn't be exported; the file has the other one.");
        assert_eq!(failed_message(2, 42, true), "2 clips couldn't be exported; the file has the other 40.");
    }
}
