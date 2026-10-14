//! The app's side: starts the engine process on demand, hands it one job at
//! a time and relays its replies. A crashed engine (GPU driver reset, out of
//! memory, killed) fails only the job it was running; the next job starts a
//! fresh process, and the job itself resumes from its checkpoints.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use as_core::CancelToken;
use as_media::MediaInfo;
use parking_lot::Mutex;
use thiserror::Error;

use crate::protocol::{Blob, Reply, Request, ShotsJob};

#[derive(Debug, Error)]
pub enum EngineError {
    /// The engine didn't start or reported it can't work (`detail` for the log).
    #[error("{message}")]
    Start { message: String, detail: String },
    #[error(
        "The analysis engine stopped unexpectedly. It restarts by itself; the analysis continues where it left off."
    )]
    Crashed(String),
    #[error("{message}")]
    Failed { message: String, detail: String },
    #[error("Cancelled.")]
    Cancelled,
}

impl EngineError {
    pub fn detail(&self) -> String {
        match self {
            Self::Start { detail, .. } => format!("engine start: {detail}"),
            Self::Crashed(why) => format!("engine crashed: {why}"),
            Self::Failed { detail, .. } => detail.clone(),
            Self::Cancelled => "cancelled".into(),
        }
    }
}

/// How to start the engine process.
#[derive(Clone, Debug)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// What a shots job reports while it runs.
pub enum ShotsEvent {
    Progress(f64),
    /// Signals of frames `start..` (`as_analysis::signals` format).
    Chunk {
        start: u32,
        signals: Vec<u8>,
    },
}

enum Msg {
    Reply(Reply),
    Exited,
}

struct Proc {
    child: Child,
    stdin: ChildStdin,
    replies: mpsc::Receiver<Msg>,
    device: String,
}

impl Proc {
    fn send(&mut self, r: &Request) -> std::io::Result<()> {
        let mut line = serde_json::to_string(r).expect("requests always serialise");
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How long a cancelled job may take to stop before the engine is killed.
const CANCEL_GRACE: Duration = Duration::from_secs(10);
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// Engine processes a job may go through before it fails.
const ATTEMPTS: usize = 3;

pub struct Engine {
    launch: Launch,
    /// The running process; also serialises jobs.
    proc: Mutex<Option<Proc>>,
    next_id: AtomicU64,
    /// Process id of the running engine (0 = none), readable during a job.
    pid: AtomicU32,
}

impl Engine {
    pub fn new(launch: Launch) -> Self {
        Self { launch, proc: Mutex::new(None), next_id: AtomicU64::new(1), pid: AtomicU32::new(0) }
    }

    /// Process id of the engine while one runs.
    pub fn pid(&self) -> Option<u32> {
        Some(self.pid.load(Ordering::Relaxed)).filter(|&p| p != 0)
    }

    /// Where the running engine computes (`None` before it first started).
    pub fn device(&self) -> Option<String> {
        self.proc.lock().as_ref().map(|p| p.device.clone())
    }

    /// Measure the shot signals of a media from `resume` on. Blocks until the
    /// job ends; chunks arrive through `on` in order.
    #[allow(clippy::too_many_arguments)]
    pub fn shots(
        &self,
        path: PathBuf,
        info: MediaInfo,
        index: Vec<u8>,
        resume: u32,
        chunk: u32,
        cancel: &CancelToken,
        mut on: impl FnMut(ShotsEvent) -> Result<(), String>,
    ) -> Result<(), EngineError> {
        let mut resume = resume;
        // A crashed engine (or one that went away just as the job was sent)
        // gets fresh tries, each from the last chunk the app received.
        for attempt in 0..ATTEMPTS {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let job =
                ShotsJob { id, path: path.clone(), info: info.clone(), index: Blob(index.clone()), resume, chunk };
            let mut got_any = false;
            let result = self.run(Request::Shots(Box::new(job)), id, cancel, |reply| match reply {
                Reply::Progress { fraction, .. } => on(ShotsEvent::Progress(fraction)),
                Reply::ShotChunk { start, signals, .. } => {
                    got_any = true;
                    resume = start + chunk;
                    on(ShotsEvent::Chunk { start, signals: signals.0 })
                }
                _ => Ok(()),
            });
            match result {
                Err(EngineError::Crashed(why)) if attempt + 1 < ATTEMPTS && !cancel.is_cancelled() => {
                    tracing::warn!(%why, resume, got_any, "engine went away; resuming in a fresh one");
                }
                r => return r,
            }
        }
        unreachable!("the last attempt always returns")
    }

    /// Stop the engine (app shutdown). A running job fails with `Crashed`.
    pub fn shutdown(&self) {
        // Don't wait for a running job's lock: kill through a fresh handle.
        if let Some(mut guard) = self.proc.try_lock()
            && let Some(mut p) = guard.take()
        {
            let _ = p.send(&Request::Shutdown);
            let deadline = Instant::now() + Duration::from_secs(2);
            while p.alive() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            p.kill();
        }
        // A job holding the lock is cancelled by its own token on shutdown;
        // the process dies with the app either way (kill-on-close job object).
    }

    fn run(
        &self,
        req: Request,
        id: u64,
        cancel: &CancelToken,
        mut on: impl FnMut(Reply) -> Result<(), String>,
    ) -> Result<(), EngineError> {
        let mut guard = self.proc.lock();
        if guard.as_mut().is_some_and(|p| !p.alive()) {
            *guard = None;
        }
        if guard.is_none() {
            *guard = Some(self.start()?);
        }
        let p = guard.as_mut().expect("started");
        // Replies left over from an earlier job (or its exit) are stale.
        while let Ok(m) = p.replies.try_recv() {
            if matches!(m, Msg::Exited) {
                *guard = Some(self.start()?);
                break;
            }
        }
        let p = guard.as_mut().expect("started");
        if let Err(e) = p.send(&req) {
            p.kill();
            *guard = None;
            return Err(EngineError::Crashed(format!("sending the job: {e}")));
        }
        let mut cancel_sent: Option<Instant> = None;
        let mut handler_error: Option<String> = None;
        loop {
            if (cancel.is_cancelled() || handler_error.is_some()) && cancel_sent.is_none() {
                let _ = p.send(&Request::Cancel { id });
                cancel_sent = Some(Instant::now());
            }
            if cancel_sent.is_some_and(|t| t.elapsed() > CANCEL_GRACE) {
                tracing::warn!(id, "engine didn't stop after cancel; killing it");
                p.kill();
                *guard = None;
                return Err(match handler_error {
                    Some(e) => EngineError::Failed { message: "Saving the analysis failed.".into(), detail: e },
                    None => EngineError::Cancelled,
                });
            }
            match p.replies.recv_timeout(Duration::from_millis(100)) {
                Ok(Msg::Reply(r)) => match r {
                    Reply::Done { id: rid } if rid == id => {
                        return match handler_error {
                            Some(e) => {
                                Err(EngineError::Failed { message: "Saving the analysis failed.".into(), detail: e })
                            }
                            None => Ok(()),
                        };
                    }
                    Reply::Failed { id: rid, message, detail, cancelled } if rid == id => {
                        return Err(match (handler_error, cancelled) {
                            (Some(e), _) => {
                                EngineError::Failed { message: "Saving the analysis failed.".into(), detail: e }
                            }
                            (None, true) => EngineError::Cancelled,
                            (None, false) => EngineError::Failed { message, detail },
                        });
                    }
                    Reply::Fatal { message, detail } => {
                        p.kill();
                        *guard = None;
                        return Err(EngineError::Start { message, detail });
                    }
                    r @ (Reply::Progress { id: rid, .. } | Reply::ShotChunk { id: rid, .. }) if rid == id => {
                        if handler_error.is_none()
                            && let Err(e) = on(r)
                        {
                            handler_error = Some(e);
                        }
                    }
                    _ => {}
                },
                Ok(Msg::Exited) => {
                    let status = p.child.wait().map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                    *guard = None;
                    return Err(EngineError::Crashed(format!("exited ({status})")));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    p.kill();
                    *guard = None;
                    return Err(EngineError::Crashed("reply channel closed".into()));
                }
            }
        }
    }

    fn start(&self) -> Result<Proc, EngineError> {
        let mut cmd = Command::new(&self.launch.program);
        cmd.args(&self.launch.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = as_media::proc::spawn(&mut cmd).map_err(|e| EngineError::Start {
            message: "The analysis engine couldn't start. Reinstall AutoScene.".into(),
            detail: format!("{}: {e}", self.launch.program.display()),
        })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        // The engine logs to stderr; its lines go into the app's log.
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                tracing::info!(target: "engine", "{line}");
            }
        });
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                match serde_json::from_str::<Reply>(&line) {
                    Ok(r) => {
                        if tx.send(Msg::Reply(r)).is_err() {
                            return;
                        }
                    }
                    Err(e) => tracing::warn!(target: "engine", error = %e, "unreadable reply: {line}"),
                }
            }
            let _ = tx.send(Msg::Exited);
        });
        let mut p = Proc { child, stdin, replies: rx, device: String::new() };
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            match p.replies.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Msg::Reply(Reply::Ready { device })) => {
                    tracing::info!(%device, pid = p.child.id(), "analysis engine ready");
                    self.pid.store(p.child.id(), Ordering::Relaxed);
                    p.device = device;
                    return Ok(p);
                }
                Ok(Msg::Reply(Reply::Fatal { message, detail })) => {
                    p.kill();
                    return Err(EngineError::Start { message, detail });
                }
                Ok(Msg::Reply(_)) => {}
                Ok(Msg::Exited) | Err(_) => {
                    p.kill();
                    return Err(EngineError::Start {
                        message: "The analysis engine couldn't start. Details are in the log.".into(),
                        detail: "no ready message".into(),
                    });
                }
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}
