//! The app's side: starts the engine process on demand, hands it one job at
//! a time and relays its replies. A crashed engine (GPU driver reset, out of
//! memory, killed) fails only the job it was running; the job gets a fresh
//! process and resumes from the last chunk the app received.

use std::cell::Cell;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use as_core::CancelToken;
use as_media::MediaInfo;
use parking_lot::Mutex;
use thiserror::Error;

use crate::protocol::{Blob, FacesJob, Reply, Request, ShotsJob};

#[derive(Debug, Error)]
pub enum EngineError {
    /// The engine didn't start or reported it can't work (`detail` for the log).
    #[error("{message}")]
    Start { message: String, detail: String },
    #[error("The analysis engine stopped unexpectedly. Details are in the log.")]
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

    fn saving(detail: String) -> Self {
        Self::Failed { message: "Saving the analysis failed.".into(), detail }
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

/// What a faces job reports while it runs.
pub enum FacesEvent {
    Progress(f64),
    /// Face tracks of frames `start..end` (`as_analysis::tracks` format),
    /// the sprites of its shots and the people found there
    /// (`as_analysis::bodies` format; empty when none were looked for).
    Chunk {
        start: u32,
        end: u32,
        tracks: Vec<u8>,
        sprites: Vec<as_analysis::ShotSprite>,
        bodies: Vec<u8>,
    },
}

/// What a faces job looks at.
#[derive(Clone, Debug)]
pub struct FacesInput {
    pub path: PathBuf,
    pub info: MediaInfo,
    /// `FrameIndex::to_bytes`.
    pub index: Vec<u8>,
    /// Shot ranges, sorted.
    pub shots: Vec<(u32, u32)>,
    /// First frame to look at: 0 or the end of a chunk stored before.
    pub resume: u32,
    pub chunk: u32,
    /// Look for people too, tied to the faces.
    pub bodies: bool,
    /// Faces found before (`as_analysis::tracks::encode_known`): only
    /// people are looked for.
    pub known: Option<Vec<u8>>,
    /// 3D animation: faces are searched at more sizes
    /// (`as_analysis::FaceParams::animated`).
    pub animated: bool,
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

    /// Kill (if still running) and reap.
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Lines of a child's pipe, whatever their encoding (a stray non-UTF-8 line
/// must not end the reader: the child would block on a full pipe).
fn pipe_lines(r: impl Read) -> impl Iterator<Item = String> {
    let mut r = BufReader::new(r);
    std::iter::from_fn(move || {
        let mut buf = Vec::new();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                while buf.last().is_some_and(|&b| b == b'\n' || b == b'\r') {
                    buf.pop();
                }
                Some(String::from_utf8_lossy(&buf).into_owned())
            }
        }
    })
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

    /// Where the running engine prefers to compute (`None` before it first
    /// started; a model the GPU can't take runs on the CPU).
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
        let resume = Cell::new(resume);
        let index = Blob(index);
        self.with_retries(
            cancel,
            |id| {
                let (path, info, index, resume) = (path.clone(), info.clone(), index.clone(), resume.get());
                Request::Shots(Box::new(ShotsJob { id, path, info, index, resume, chunk }))
            },
            |reply| match reply {
                Reply::Progress { fraction, .. } => on(ShotsEvent::Progress(fraction)),
                Reply::ShotChunk { start, len, signals, .. } => {
                    on(ShotsEvent::Chunk { start, signals: signals.0 })?;
                    // Only what was stored counts as done.
                    resume.set(start + len);
                    Ok(())
                }
                _ => Ok(()),
            },
        )
    }

    /// Find the face tracks of a media's shots (and the people tied to
    /// them, or the people alone for faces found before) from `job.resume`
    /// on. Blocks until the job ends; chunks arrive through `on` in order.
    pub fn faces(
        &self,
        job: FacesInput,
        cancel: &CancelToken,
        mut on: impl FnMut(FacesEvent) -> Result<(), String>,
    ) -> Result<(), EngineError> {
        let resume = Cell::new(job.resume);
        let (index, known) = (Blob(job.index), job.known.map(Blob));
        self.with_retries(
            cancel,
            |id| {
                Request::Faces(Box::new(FacesJob {
                    id,
                    path: job.path.clone(),
                    info: job.info.clone(),
                    index: index.clone(),
                    shots: job.shots.clone(),
                    resume: resume.get(),
                    chunk: job.chunk,
                    bodies: job.bodies,
                    known: known.clone(),
                    animated: job.animated,
                }))
            },
            |reply| match reply {
                Reply::Progress { fraction, .. } => on(FacesEvent::Progress(fraction)),
                Reply::FaceChunk { start, end, tracks, sprites, bodies, .. } => {
                    let sprites = sprites
                        .into_iter()
                        .map(|s| as_analysis::ShotSprite { shot: s.shot, end: s.end, cells: s.cells, jpeg: s.jpeg.0 })
                        .collect();
                    on(FacesEvent::Chunk { start, end, tracks: tracks.0, sprites, bodies: bodies.0 })?;
                    resume.set(end);
                    Ok(())
                }
                _ => Ok(()),
            },
        )
    }

    /// Run a job, giving it a fresh engine (and a fresh request, built from
    /// what has been stored so far) when the engine goes away under it.
    fn with_retries(
        &self,
        cancel: &CancelToken,
        make: impl Fn(u64) -> Request,
        mut on: impl FnMut(Reply) -> Result<(), String>,
    ) -> Result<(), EngineError> {
        for attempt in 0..ATTEMPTS {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            match self.run(make(id), id, cancel, &mut on) {
                Err(EngineError::Crashed(why)) if attempt + 1 < ATTEMPTS && !cancel.is_cancelled() => {
                    tracing::warn!(%why, "engine went away; resuming in a fresh one");
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
            self.pid.store(0, Ordering::Relaxed);
        }
        // A job holding the lock is cancelled by its own token on shutdown;
        // the process dies with the app either way (kill-on-close job object).
    }

    /// Kill and forget the engine in `slot`.
    fn discard(&self, slot: &mut Option<Proc>) {
        if let Some(mut p) = slot.take() {
            p.kill();
        }
        self.pid.store(0, Ordering::Relaxed);
    }

    fn run(
        &self,
        req: Request,
        id: u64,
        cancel: &CancelToken,
        on: &mut impl FnMut(Reply) -> Result<(), String>,
    ) -> Result<(), EngineError> {
        let mut guard = self.proc.lock();
        if guard.as_mut().is_some_and(|p| !p.alive()) {
            self.discard(&mut guard);
        }
        // Replies left over from an earlier job are stale; an engine that
        // exited since (idle) is replaced.
        if let Some(p) = guard.as_mut() {
            let mut exited = false;
            while let Ok(m) = p.replies.try_recv() {
                exited |= matches!(m, Msg::Exited);
            }
            if exited {
                self.discard(&mut guard);
            }
        }
        if guard.is_none() {
            *guard = Some(self.start(cancel)?);
        }
        let p = guard.as_mut().expect("started");
        if let Err(e) = p.send(&req) {
            self.discard(&mut guard);
            return Err(EngineError::Crashed(format!("sending the job: {e}")));
        }
        let mut cancel_sent: Option<Instant> = None;
        let mut handler_error: Option<String> = None;
        loop {
            let p = guard.as_mut().expect("running");
            if (cancel.is_cancelled() || handler_error.is_some()) && cancel_sent.is_none() {
                let _ = p.send(&Request::Cancel { id });
                cancel_sent = Some(Instant::now());
            }
            if cancel_sent.is_some_and(|t| t.elapsed() > CANCEL_GRACE) {
                tracing::warn!(id, "engine didn't stop after cancel; killing it");
                self.discard(&mut guard);
                return Err(handler_error.map_or(EngineError::Cancelled, EngineError::saving));
            }
            match p.replies.recv_timeout(Duration::from_millis(100)) {
                Ok(Msg::Reply(r)) => match r {
                    Reply::Done { id: rid } if rid == id => {
                        return handler_error.map_or(Ok(()), |e| Err(EngineError::saving(e)));
                    }
                    Reply::Failed { id: rid, message, detail, cancelled } if rid == id => {
                        return Err(match (handler_error, cancelled) {
                            (Some(e), _) => EngineError::saving(e),
                            (None, true) => EngineError::Cancelled,
                            (None, false) => EngineError::Failed { message, detail },
                        });
                    }
                    Reply::Fatal { message, detail } => {
                        self.discard(&mut guard);
                        return Err(EngineError::Start { message, detail });
                    }
                    r @ (Reply::Progress { id: rid, .. }
                    | Reply::ShotChunk { id: rid, .. }
                    | Reply::FaceChunk { id: rid, .. })
                        if rid == id =>
                    {
                        if handler_error.is_none()
                            && let Err(e) = on(r)
                        {
                            handler_error = Some(e);
                        }
                    }
                    _ => {}
                },
                Ok(Msg::Exited) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let mut p = guard.take().expect("running");
                    p.kill();
                    let status = p.child.wait().map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
                    self.pid.store(0, Ordering::Relaxed);
                    // A failed save outranks the crash: retrying past an
                    // unsaved chunk would leave a hole.
                    return Err(match handler_error {
                        Some(e) => EngineError::saving(e),
                        None if cancel.is_cancelled() => EngineError::Cancelled,
                        None => EngineError::Crashed(format!("exited ({status})")),
                    });
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn start(&self, cancel: &CancelToken) -> Result<Proc, EngineError> {
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
            for line in pipe_lines(stderr) {
                tracing::info!(target: "engine", "{line}");
            }
        });
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in pipe_lines(stdout) {
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
            if cancel.is_cancelled() {
                p.kill();
                return Err(EngineError::Cancelled);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match p.replies.recv_timeout(left.min(Duration::from_millis(100))) {
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
                Err(mpsc::RecvTimeoutError::Timeout) if !left.is_zero() => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_lines_survive_odd_bytes() {
        let raw: &[u8] = b"one\r\n\xff\xfe bad\ntwo";
        let lines: Vec<String> = pipe_lines(raw).collect();
        assert_eq!(lines, ["one", "\u{fffd}\u{fffd} bad", "two"]);
    }
}
