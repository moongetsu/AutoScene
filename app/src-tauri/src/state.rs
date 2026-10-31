//! App-wide state shared by commands and background work.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use as_api::{AnalysisStage, MediaToolsStatus};
use as_core::{AppPaths, MediaId, ProjectId};
use as_engine::{Engine, Launch};
use as_jobs::{Class, JobEvent, JobRunner};
use as_media::Tools;
use as_models::ModelStore;
use as_project::Library;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::{JobChanged, JobProgressEvent};
use crate::paths;

/// Concurrent jobs per resource class. Two decode-heavy jobs keep a GPU busy
/// without starving the UI's preview; exports run one at a time so encoder
/// sessions are never oversubscribed; the engine analyses one media at a time;
/// upkeep (one still at a time) runs beside all of them.
const LIMITS: &[(Class, usize)] = &[(Class::Media, 2), (Class::Export, 1), (Class::Analysis, 1), (Class::Upkeep, 1)];

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
    /// Projects with a portraits job queued or running, and whether their
    /// cast changed since it last looked (see `work::queue_portraits`).
    pub portraits: Arc<parking_lot::Mutex<HashMap<ProjectId, bool>>>,
    /// Per project, the last decision that moved faces (see `work::cast_edited`).
    pub decisions: Arc<parking_lot::Mutex<HashMap<ProjectId, u64>>>,
    /// Projects whose AniList look-up runs or failed this session (see
    /// `commands::anilist::look_up`).
    pub anilist: Arc<parking_lot::Mutex<HashMap<ProjectId, crate::commands::anilist::LookUp>>>,
    /// ffmpeg/ffprobe once found (see [`AppState::tools`]).
    tools: Arc<OnceLock<Arc<Tools>>>,
    /// The ffmpeg folder this build uses (see [`sidecar_dir`]).
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
        // The ffmpeg the app and its engine run, and the ONNX Runtime the
        // engine loads (see `sidecar_dir`).
        let resources = app.path().resource_dir().ok();
        let dev = Path::new(env!("CARGO_MANIFEST_DIR"));
        let named = std::env::var_os("AUTOSCENE_FFMPEG_DIR").is_some_and(|d| !d.is_empty());
        let bundled_ffmpeg = sidecar_dir(resources.as_deref(), "ffmpeg", RELEASE, dev, named);
        let ort_dir = sidecar_dir(resources.as_deref(), "onnxruntime", RELEASE, dev, false)
            .unwrap_or_else(|| dev.join("onnxruntime"));
        let models = ModelStore::new(paths.models());
        let engine_cfg = as_engine::Config {
            ort_dir,
            models_dir: paths.models(),
            ffmpeg_dir: bundled_ffmpeg.clone(),
            gpu: true,
            idle_exit: Some(as_engine::IDLE_EXIT),
        };
        // The engine is this very executable, and nothing found by name stands
        // in for it: without one, starting it fails and analysis says so.
        let program = std::env::current_exe().unwrap_or_else(|e| {
            log::error!("can't locate the app's executable to start the analysis engine: {e}");
            PathBuf::new()
        });
        let mut args = vec!["--engine".into()];
        args.extend(as_engine::args(&engine_cfg));
        let engine = Arc::new(Engine::new(Launch { program, args }));
        Self {
            library: Arc::new(library),
            jobs,
            engine,
            models,
            analysis: Arc::default(),
            portraits: Arc::default(),
            decisions: Arc::default(),
            anilist: Arc::default(),
            tools: Arc::new(OnceLock::new()),
            bundled_ffmpeg,
        }
    }

    /// ffmpeg/ffprobe, kept once found. Not finding them isn't kept: the
    /// next use looks again, so a file an antivirus held for a moment, or a
    /// reinstall, doesn't leave every job failing until a restart.
    /// Hardware capabilities are probed once, on first use: the warm-up
    /// after launch, a job, or a plan sizing an export for its encoders
    /// (never on the UI thread).
    pub fn tools(&self) -> AppResult<Arc<Tools>> {
        found(&self.tools, || Tools::locate(self.bundled_ffmpeg.as_deref()).map(Arc::new)).cloned().map_err(|e| {
            log::warn!("media tools: {}", e.detail());
            AppError::new(ErrorCode::Media, e.to_string())
        })
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

/// Whether this is a release build: it uses only the sidecars it ships with.
const RELEASE: bool = !cfg!(debug_assertions);

/// Where a sidecar shipped as a resource (`ffmpeg`, `onnxruntime`) is:
/// `Some` is the only place looked in, `None` leaves ffmpeg to
/// `AUTOSCENE_FFMPEG_DIR` or PATH.
///
/// An installed app (a release build with its resources) uses its own and
/// nothing else: never a build found on PATH or named by
/// `AUTOSCENE_FFMPEG_DIR`. A release binary run from where it was built
/// (no resources beside it, as the marketing capture runs one) and a dev
/// build take the folder that variable names (`named`), else the resource
/// folder when it exists, else the source tree's copy `cargo xtask
/// fetch-sidecars` made (`dev`), else `None`.
fn sidecar_dir(resources: Option<&Path>, name: &str, release: bool, dev: &Path, named: bool) -> Option<PathBuf> {
    let shipped = resources.map(|d| d.join(name)).filter(|d| d.is_dir());
    if release && let Some(own) = shipped {
        return Some(own);
    }
    if named {
        return None;
    }
    shipped.or_else(|| Some(dev.join(name)).filter(|d| d.is_dir()))
}

/// The value in `cell`, else `find`'s once it succeeds. A failure isn't
/// kept, so the next caller looks again.
fn found<T, E>(cell: &OnceLock<T>, find: impl FnOnce() -> Result<T, E>) -> Result<&T, E> {
    if let Some(v) = cell.get() {
        return Ok(v);
    }
    let v = find()?;
    // A caller that found them meanwhile wins; the same tools either way.
    Ok(cell.get_or_init(|| v))
}

/// Stop all background work before the process goes away (quit, restart,
/// update install), so no ffmpeg is left running or writing a file.
pub fn shutdown_work(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        state.jobs.shutdown(std::time::Duration::from_secs(3));
        state.engine.shutdown();
    }
}

/// How long [`resume_work`] waits for work the shutdown stopped to end.
const RESUME_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// The process didn't go away after [`shutdown_work`] after all (the update
/// installer failed to start): bring background work back, and with it what
/// the shutdown interrupted, which it left as a quit does (an analysis
/// `running`, an ingest `indexing`): the launch's recovery queues it again.
/// Off the caller's thread: it waits for the stopped work and opens every
/// project.
pub fn resume_work(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(state) = app.try_state::<AppState>()
            && state.jobs.resume(RESUME_GRACE)
        {
            crate::work::recover_all(&app);
        }
    });
}

/// Let the webview load a file through the asset protocol (players, posters).
/// Scope grows per file at runtime; nothing on disk is reachable by default.
pub fn allow_asset(app: &AppHandle, path: &Path) {
    if grantable(path)
        && let Err(e) = app.asset_protocol_scope().allow_file(path)
    {
        log::warn!("asset scope: {}: {e}", paths::name(path));
    }
}

pub fn allow_asset_dir(app: &AppHandle, dir: &Path) {
    if grantable(dir)
        && let Err(e) = app.asset_protocol_scope().allow_directory(dir, false)
    {
        log::warn!("asset scope: {}: {e}", paths::name(dir));
    }
}

/// Like [`allow_asset_dir`], including subfolders (face thumbnails per media).
pub fn allow_asset_tree(app: &AppHandle, dir: &Path) {
    if grantable(dir)
        && let Err(e) = app.asset_protocol_scope().allow_directory(dir, true)
    {
        log::warn!("asset scope: {}: {e}", paths::name(dir));
    }
}

/// Only plain paths are opened to the webview (see `paths`). Grants come
/// from the databases, and the scope resolves what it's given: a `..` in a
/// stored path would open wherever it leads.
fn grantable(path: &Path) -> bool {
    let plain = paths::is_plain(path);
    if !plain {
        log::warn!("asset scope: refused {}, which isn't a plain path", paths::name(path));
    }
    plain
}

/// Run blocking work (database, filesystem scans) off the async runtime.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| {
        log::error!("a command failed unexpectedly: {e}");
        AppError::internal("Something went wrong. Details are in the log.")
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_app_uses_only_its_own_sidecars() {
        let tmp = std::env::temp_dir().join(format!("autoscene-sidecars-{}", std::process::id()));
        let (resources, dev) = (tmp.join("resources"), tmp.join("src-tauri"));
        std::fs::create_dir_all(dev.join("ffmpeg")).unwrap();

        // A release binary without resources beside it (run where it was
        // built): the named folder, else the source tree's copy.
        assert_eq!(sidecar_dir(Some(&resources), "ffmpeg", true, &dev, true), None);
        assert_eq!(sidecar_dir(None, "ffmpeg", true, &dev, false), Some(dev.join("ffmpeg")));
        // A dev build: the named folder first, then the source tree's copy, else none.
        assert_eq!(sidecar_dir(Some(&resources), "ffmpeg", false, &dev, true), None);
        assert_eq!(sidecar_dir(Some(&resources), "ffmpeg", false, &dev, false), Some(dev.join("ffmpeg")));
        assert_eq!(sidecar_dir(Some(&resources), "onnxruntime", false, &dev, false), None);
        // Installed: its own, whatever the variable names.
        std::fs::create_dir_all(resources.join("ffmpeg")).unwrap();
        assert_eq!(sidecar_dir(Some(&resources), "ffmpeg", true, &dev, true), Some(resources.join("ffmpeg")));
        assert_eq!(sidecar_dir(Some(&resources), "ffmpeg", false, &dev, false), Some(resources.join("ffmpeg")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tools_not_found_are_looked_for_again() {
        let cell = OnceLock::new();
        let mut looks = 0;
        assert_eq!(
            found(&cell, || {
                looks += 1;
                Err("held by the antivirus")
            }),
            Err("held by the antivirus")
        );
        assert_eq!(
            found(&cell, || {
                looks += 1;
                Ok::<_, &str>(7)
            }),
            Ok(&7)
        );
        // Once found, kept.
        assert_eq!(found(&cell, || Err::<i32, _>("never asked")), Ok(&7));
        assert_eq!(looks, 2);
    }
}
