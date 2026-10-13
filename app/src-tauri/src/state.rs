//! App-wide state shared by commands and background work.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use as_api::MediaToolsStatus;
use as_jobs::{Class, JobEvent, JobRunner};
use as_media::Tools;
use as_project::Library;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::{JobChanged, JobProgressEvent};

/// Concurrent jobs per resource class. Two decode-heavy jobs keep a GPU busy
/// without starving the UI's preview; exports run one at a time so encoder
/// sessions are never oversubscribed.
const LIMITS: &[(Class, usize)] = &[(Class::Media, 2), (Class::Export, 1)];

#[derive(Clone)]
pub struct AppState {
    pub library: Arc<Library>,
    pub jobs: JobRunner,
    tools: Arc<OnceLock<Result<Arc<Tools>, String>>>,
    bundled_ffmpeg: Option<PathBuf>,
}

impl AppState {
    pub fn new(app: &AppHandle, library: Library) -> Self {
        let handle = app.clone();
        let jobs = JobRunner::new(LIMITS, move |e| {
            let _ = match e {
                JobEvent::Changed(info) => JobChanged(info).emit(&handle),
                JobEvent::Progress(p) => JobProgressEvent(p).emit(&handle),
            };
        });
        let bundled_ffmpeg = app.path().resource_dir().ok().map(|d| d.join("ffmpeg"));
        Self { library: Arc::new(library), jobs, tools: Arc::new(OnceLock::new()), bundled_ffmpeg }
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

/// Run blocking work (database, filesystem scans) off the async runtime.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError::internal(e.to_string()))?
}
