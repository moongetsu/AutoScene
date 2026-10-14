//! The engine process: reads requests from stdin, runs one job at a time on
//! the GPU (models stay loaded between jobs), streams replies to stdout.
//!
//! It owns no state on disk. Everything a job produces goes back to the app,
//! which stores it as it arrives — so the engine can be killed at any moment
//! and the job resumes from the app's last checkpoint.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use as_analysis::{AnalysisError, shot_signals, signals};
use as_core::{CancelToken, FrameIndex};
use as_infer::Runtime;
use as_media::{Source, Tools};
use as_models::ModelStore;
use as_vision::TransNet;
use parking_lot::Mutex;

use crate::protocol::{Blob, Reply, Request, ShotsJob};

/// How the engine finds its runtime, models and tools.
#[derive(Clone, Debug)]
pub struct Config {
    /// Folder with the ONNX Runtime library.
    pub ort_dir: PathBuf,
    /// The model store (`AppPaths::models()`).
    pub models_dir: PathBuf,
    /// Bundled ffmpeg folder, if any (otherwise `AUTOSCENE_FFMPEG_DIR` / PATH).
    pub ffmpeg_dir: Option<PathBuf>,
    /// `false` keeps every model on the CPU.
    pub gpu: bool,
    /// Exit after this long without a job, which frees the GPU memory.
    pub idle_exit: Option<Duration>,
}

/// Shared stdout: one reply per line, flushed at once.
#[derive(Clone)]
struct Out(Arc<Mutex<Box<dyn Write + Send>>>);

impl Out {
    fn send(&self, reply: &Reply) {
        let mut line = serde_json::to_string(reply).expect("replies always serialise");
        line.push('\n');
        let mut w = self.0.lock();
        // A broken pipe means the app is gone; the reader thread ends the loop.
        let _ = w.write_all(line.as_bytes()).and_then(|()| w.flush());
    }
}

/// Run the engine until stdin closes, a `shutdown` arrives, or it has been
/// idle for `idle_exit`. Returns the process exit code.
pub fn serve(cfg: Config, input: impl BufRead + Send + 'static, output: impl Write + Send + 'static) -> i32 {
    let out = Out(Arc::new(Mutex::new(Box::new(output))));
    let tools = match Tools::locate(cfg.ffmpeg_dir.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            out.send(&Reply::Fatal { message: e.to_string(), detail: e.detail() });
            return 1;
        }
    };
    let rt = match Runtime::load(&cfg.ort_dir, cfg.gpu) {
        Ok(rt) => rt,
        Err(e) => {
            out.send(&Reply::Fatal { message: e.to_string(), detail: e.detail() });
            return 1;
        }
    };
    out.send(&Reply::Ready { device: rt.preferred_device().label().to_string() });

    // Jobs queue up here; cancellation needs no queue and is handled at once.
    let (tx, rx) = mpsc::channel::<(ShotsJob, CancelToken)>();
    let cancels: Arc<Mutex<HashMap<u64, CancelToken>>> = Arc::default();
    let reader = {
        let cancels = cancels.clone();
        let out = out.clone();
        std::thread::spawn(move || read_requests(input, tx, cancels, out))
    };

    let mut models = Models { store: ModelStore::new(&cfg.models_dir), transnet: None };
    loop {
        let next = match cfg.idle_exit {
            Some(idle) => rx.recv_timeout(idle).map_err(|e| e == mpsc::RecvTimeoutError::Timeout),
            None => rx.recv().map_err(|_| false),
        };
        let (job, cancel) = match next {
            Ok(j) => j,
            Err(true) => {
                tracing::info!("idle; exiting to free the GPU");
                break;
            }
            Err(false) => break,
        };
        let id = job.id;
        let reply = match run_shots(&tools, &rt, &mut models, job, &cancel, &out) {
            Ok(()) => Reply::Done { id },
            Err(e) => {
                if !e.is_cancelled() {
                    tracing::warn!(id, error = %e.detail(), "job failed");
                }
                Reply::Failed { id, message: e.to_string(), detail: e.detail(), cancelled: e.is_cancelled() }
            }
        };
        cancels.lock().remove(&id);
        out.send(&reply);
    }
    // Pending cancellations of jobs that never ran: nothing to report. The
    // reader thread ends with stdin.
    drop(reader);
    0
}

fn read_requests(
    input: impl BufRead,
    tx: mpsc::Sender<(ShotsJob, CancelToken)>,
    cancels: Arc<Mutex<HashMap<u64, CancelToken>>>,
    out: Out,
) {
    // Cancels that arrive before their job (a race the app can't rule out).
    let mut early: HashSet<u64> = HashSet::new();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Request>(&line) {
            Ok(Request::Shots(job)) => {
                let cancel = CancelToken::new();
                if early.remove(&job.id) {
                    cancel.cancel();
                }
                cancels.lock().insert(job.id, cancel.clone());
                if tx.send((*job, cancel)).is_err() {
                    break;
                }
            }
            Ok(Request::Cancel { id }) => match cancels.lock().get(&id) {
                Some(c) => c.cancel(),
                None => {
                    early.insert(id);
                }
            },
            Ok(Request::Shutdown) => break,
            Err(e) => {
                tracing::error!(error = %e, "bad request");
                out.send(&Reply::Fatal {
                    message: "The analysis engine got a bad request.".into(),
                    detail: e.to_string(),
                });
            }
        }
    }
    // Leaving: whatever runs now should stop, and the loop ends after it.
    for c in cancels.lock().values() {
        c.cancel();
    }
}

struct Models {
    store: ModelStore,
    transnet: Option<TransNet>,
}

impl Models {
    fn transnet(&mut self, rt: &Runtime) -> Result<&mut TransNet, AnalysisError> {
        if self.transnet.is_none() {
            let path = self
                .store
                .installed(as_models::ids::TRANSNET)
                .map_err(|e| AnalysisError::Internal(e.detail()))?
                .ok_or_else(|| {
                    AnalysisError::Internal(format!("TransNetV2 isn't installed in {}", self.store.dir().display()))
                })?;
            self.transnet = Some(TransNet::load(rt, &path)?);
        }
        Ok(self.transnet.as_mut().expect("just loaded"))
    }
}

fn run_shots(
    tools: &Tools,
    rt: &Runtime,
    models: &mut Models,
    job: ShotsJob,
    cancel: &CancelToken,
    out: &Out,
) -> Result<(), AnalysisError> {
    let net = models.transnet(rt)?;
    let index = FrameIndex::from_bytes(&job.index.0).map_err(|e| AnalysisError::Internal(e.to_string()))?;
    let started = Instant::now();
    let id = job.id;
    tracing::info!(id, path = %job.path.display(), resume = job.resume, device = ?net.device(), "finding shots");
    shot_signals(
        tools,
        Source { path: Path::new(&job.path), info: &job.info, index: &index },
        net,
        job.resume,
        job.chunk,
        cancel,
        |fraction| out.send(&Reply::Progress { id, fraction }),
        |start, s| {
            out.send(&Reply::ShotChunk { id, start, signals: Blob(signals::encode(s)) });
            Ok(())
        },
    )?;
    let frames = index.len() - job.resume;
    let secs = started.elapsed().as_secs_f64();
    tracing::info!(id, frames, secs, fps = frames as f64 / secs.max(1e-3), "shots measured");
    Ok(())
}
