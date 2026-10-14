//! App-wide state shared by commands and background work.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use as_api::{AnalysisStage, MediaToolsStatus};
use as_core::{AppPaths, MediaId};
use as_engine::{Engine, Launch};
use as_jobs::{Class, JobEvent, JobRunner};
use as_media::Tools;
use as_models::ModelStore;
use as_project::Library;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::{JobChanged, JobProgressEvent};

/// Concurrent jobs per resource class. Two decode-heavy jobs keep a GPU busy
/// without starving the UI's preview; exports run one at a time so encoder
/// sessions are never oversubscribed; the engine analyses one media at a time.
const LIMITS: &[(Class, usize)] = &[(Class::Media, 2), (Class::Export, 1), (Class::Analysis, 1)];

/// Which media an analysis job is working on, and the restarts asked for
/// meanwhile. Both change under one lock, so a restart is either handed to
/// the running job (which makes it once nothing writes the results any more)
/// or made at once — never lost, never made twice.
#[derive(Debug, Default)]
pub struct AnalysisRuns {
    /// Media whose analysis job runs and will still look at `redo`.
    pub running: HashSet<MediaId>,
    /// Stage to clear and analyse again once that job has stopped.
    pub redo: HashMap<MediaId, AnalysisStage>,
}

#[derive(Clone)]
pub struct AppState {
    pub library: Arc<Library>,
    pub jobs: JobRunner,
    /// The analysis engine process (started on demand).
    pub engine: Arc<Engine>,
    /// AI models, downloaded on first use.
    pub models: ModelStore,
    /// Running analyses and the restarts they owe.
    pub analysis: Arc<parking_lot::Mutex<AnalysisRuns>>,
    tools: Arc<OnceLock<Result<Arc<Tools>, String>>>,
    bundled_ffmpeg: Option<PathBuf>,
}

impl AppState {
    pub fn new(app: &AppHandle, library: Library, paths: &AppPaths) -> Self {
        let handle = app.clone();
        let jobs = JobRunner::new(LIMITS, move |e| {
            let _ = match e {
                JobEvent::Changed(info) => JobChanged(info).emit(&handle),
                JobEvent::Progress(p) => JobProgressEvent(p).emit(&handle),
            };
        });
        // Release builds ship ffmpeg as a resource; dev builds use the same
        // pinned build when `cargo xtask fetch-sidecars` has fetched it.
        let bundled_ffmpeg =
            app.path().resource_dir().ok().map(|d| d.join("ffmpeg")).filter(|d| d.is_dir()).or_else(|| {
                cfg!(debug_assertions)
                    .then(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ffmpeg"))
                    .filter(|d| d.is_dir())
            });
        // The ONNX Runtime the analysis engine loads, found the same way.
        let ort_dir = app
            .path()
            .resource_dir()
            .ok()
            .map(|d| d.join("onnxruntime"))
            .filter(|d| d.is_dir())
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("onnxruntime"));
        let models = ModelStore::new(paths.models());
        let engine_cfg = as_engine::Config {
            ort_dir,
            models_dir: paths.models(),
            ffmpeg_dir: bundled_ffmpeg.clone(),
            gpu: true,
            idle_exit: Some(as_engine::IDLE_EXIT),
        };
        let program = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("autoscene-app"));
        let mut args = vec!["--engine".into()];
        args.extend(as_engine::args(&engine_cfg));
        let engine = Arc::new(Engine::new(Launch { program, args }));
        Self {
            library: Arc::new(library),
            jobs,
            engine,
            models,
            analysis: Arc::default(),
            tools: Arc::new(OnceLock::new()),
            bundled_ffmpeg,
        }
    }

    /// ffmpeg/ffprobe, located once. Hardware capabilities are probed on first
    /// use by a job (never on the UI thread).
    pub fn tools(&self) -> AppResult<Arc<Tools>> {
        self.tools
            .get_or_init(|| {
                Tools::locate(self.bundled_ffmpeg.as_deref().filter(|d| d.is_dir()))
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(|m| AppError::new(ErrorCode::Media, m))
    }

    pub fn tools_status(&self) -> MediaToolsStatus {
        match self.tools() {
            Ok(t) => MediaToolsStatus {
                ready: true,
                version: Some(t.version.clone()),
                path: Some(t.ffmpeg.display().to_string()),
                error: None,
            },
            Err(e) => MediaToolsStatus { ready: false, version: None, path: None, error: Some(e.message) },
        }
    }
}

/// Stop all background work before the process goes away (quit, restart,
/// update install), so no ffmpeg is left running or writing a file.
pub fn shutdown_work(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.jobs.shutdown(std::time::Duration::from_secs(3));
        state.engine.shutdown();
    }
}

/// The process didn't go away after [`shutdown_work`] after all (the update
/// installer failed to start): bring background work back.
pub fn resume_work(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.jobs.resume();
    }
}

/// Let the webview load a file through the asset protocol (players, posters).
/// Scope grows per file at runtime; nothing on disk is reachable by default.
pub fn allow_asset(app: &AppHandle, path: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_file(path) {
        log::warn!("asset scope: {}: {e}", path.display());
    }
}

pub fn allow_asset_dir(app: &AppHandle, dir: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(dir, false) {
        log::warn!("asset scope: {}: {e}", dir.display());
    }
}

/// Like [`allow_asset_dir`], including subfolders (face thumbnails per media).
pub fn allow_asset_tree(app: &AppHandle, dir: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
        log::warn!("asset scope: {}: {e}", dir.display());
    }
}

/// Run blocking work (database, filesystem scans) off the async runtime.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError::internal(e.to_string()))?
}
