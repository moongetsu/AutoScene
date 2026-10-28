//! Background jobs.
//!
//! Work is grouped into resource classes, each with its own worker threads,
//! so a long export never blocks indexing a new episode (and vice versa).
//! Within a class, projects take turns: a big import in one project never
//! holds up another project's work by more than the jobs already running,
//! while each project's own jobs keep the order they were queued in.
//! Every state change and (throttled) progress sample is reported through
//! one listener, which the app forwards to the UI as `job:*` events — the UI
//! never polls.
//!
//! The queue itself lives in memory. What needs doing is persisted where it
//! belongs instead — each project's database records which media still need
//! indexing, a preview or analysis (with checkpoints) — and the app re-queues
//! it on launch, so nothing is lost when the app stops or crashes.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use as_api::{JobInfo, JobKind, JobProgress, JobState, now_ms};
use as_core::{CancelToken, ExportId, JobId, MediaId, ProgressSink, ProjectId};
use parking_lot::{Condvar, Mutex};

/// Resource classes. Each runs its jobs on its own fixed set of workers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    /// Probing, indexing and proxies (decode-heavy).
    Media,
    /// Clip rendering (encode-heavy; hardware encoders have session limits).
    Export,
    /// AI analysis in the engine process (one GPU: one job at a time).
    Analysis,
    /// Quiet upkeep of existing projects (posters made again at new sizes):
    /// its own worker, so it never holds up new work.
    Upkeep,
}

/// What to show for a job and what it belongs to.
#[derive(Clone, Debug)]
pub struct JobSpec {
    pub kind: JobKind,
    pub class: Class,
    pub title: String,
    pub subtitle: Option<String>,
    pub project_id: Option<ProjectId>,
    pub media_id: Option<MediaId>,
    pub export_id: Option<ExportId>,
}

/// Why a job stopped early.
#[derive(Debug)]
pub enum JobError {
    Cancelled,
    Failed(String),
}

impl<E: std::error::Error> From<E> for JobError {
    fn from(e: E) -> Self {
        Self::Failed(e.to_string())
    }
}

pub type JobResult = Result<(), JobError>;

/// Handed to running work: cancellation and progress reporting.
pub struct JobCtx {
    pub id: JobId,
    pub cancel: CancelToken,
    sink: ProgressSink,
}

impl JobCtx {
    /// Report `fraction` (0..1) of `stage`. Cheap; throttled to 4 Hz. In
    /// millionths: the time left is measured from the first move, which
    /// coarser steps would put late (a step of an hour's job is 3.6 s).
    pub fn progress(&self, stage: &str, fraction: f64) {
        let done = (fraction.clamp(0.0, 1.0) * PARTS as f64).round() as u64;
        self.sink.update(stage, done, PARTS);
    }

    /// Report a stage whose length is unknown.
    pub fn stage(&self, stage: &str) {
        self.sink.update(stage, 0, 0);
    }

    /// Report `fraction` of `stage`, or that how far it is isn't known for
    /// now (`None`: its pace is measured anew from the next fraction).
    pub fn report(&self, stage: &str, fraction: Option<f64>) {
        match fraction {
            Some(f) => self.progress(stage, f),
            None => self.stage(stage),
        }
    }
}

#[derive(Clone, Debug)]
pub enum JobEvent {
    Changed(JobInfo),
    Progress(JobProgress),
}

type Work = Box<dyn FnOnce(&JobCtx) -> JobResult + Send>;
type Listener = dyn Fn(JobEvent) + Send + Sync;

struct Entry {
    info: JobInfo,
    class: Class,
    cancel: CancelToken,
    work: Option<Work>,
}

#[derive(Default)]
struct State {
    /// Oldest first (each project's jobs run in this order; projects take
    /// turns, see [`next_job`]).
    jobs: VecDeque<Entry>,
    shutting_down: bool,
    /// Shutdowns begun: a resume undoes only the one it waited for.
    shutdowns: u64,
    /// Last revision handed out; bumped under the lock on every change.
    rev: u64,
    /// Worker threads alive per class (they exit during a shutdown).
    live: HashMap<Class, usize>,
    /// When each project last had a job of a class started (the revision
    /// of that claim): the one served longest ago goes next.
    served: HashMap<(Class, Option<ProjectId>), u64>,
}

impl State {
    fn next_rev(&mut self) -> u64 {
        self.rev += 1;
        self.rev
    }
}

struct Inner {
    state: Mutex<State>,
    wake: Condvar,
    listener: Box<Listener>,
    limits: Vec<(Class, usize)>,
}

/// Finished jobs kept for the Activity drawer.
const KEEP_FINISHED: usize = 100;
/// Parts of a whole progress is reported in.
const PARTS: u64 = 1_000_000;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// What a panic said (its `&str` or `String` message), for the log and the
/// error a job shows.
pub fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unexpected error".into())
}

#[derive(Clone)]
pub struct JobRunner {
    inner: Arc<Inner>,
}

impl JobRunner {
    /// Start workers: `limits` gives the number of concurrent jobs per class.
    pub fn new(limits: &[(Class, usize)], listener: impl Fn(JobEvent) + Send + Sync + 'static) -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            // It only tells the UI: a panic in it must never take a worker
            // (and every job of its class after it) along.
            listener: Box::new(move |e| {
                if let Err(panic) = catch_unwind(AssertUnwindSafe(|| listener(e))) {
                    tracing::error!(msg = %panic_message(&*panic), "job listener panicked");
                }
            }),
            limits: limits.iter().map(|&(c, n)| (c, n.max(1))).collect(),
        });
        spawn_missing_workers(&inner, &mut inner.state.lock());
        Self { inner }
    }

    /// Undo a [`shutdown`](Self::shutdown) whose exit didn't happen (an
    /// update installer that failed to start): once the work it stopped has
    /// ended (up to `grace` more), restart the workers that exited and close
    /// out queued jobs whose work was dropped. Work ending meanwhile still
    /// sees the shutdown, so it leaves its state as a quit would. Returns
    /// whether a shutdown was undone: what it stopped then waits for the
    /// app to queue it again, as on a launch. One begun while it waited
    /// (the app quits after all) is never undone.
    pub fn resume(&self, grace: Duration) -> bool {
        let waited_for = {
            let st = self.inner.state.lock();
            if !st.shutting_down {
                return false;
            }
            st.shutdowns
        };
        self.settle(grace);
        let mut changed = Vec::new();
        {
            let mut st = self.inner.state.lock();
            if !st.shutting_down || st.shutdowns != waited_for {
                return false;
            }
            st.shutting_down = false;
            let mut rev = st.rev;
            for e in st.jobs.iter_mut().filter(|e| e.info.state == JobState::Queued && e.work.is_none()) {
                rev += 1;
                e.info.state = JobState::Cancelled;
                e.info.finished_at = Some(now_ms());
                e.info.rev = rev;
                changed.push(e.info.clone());
            }
            st.rev = rev;
            spawn_missing_workers(&self.inner, &mut st);
        }
        for info in changed {
            (self.inner.listener)(JobEvent::Changed(info));
        }
        true
    }

    /// Queue `work`. Returns immediately.
    ///
    /// During a [`shutdown`](Self::shutdown) nothing is queued any more: the
    /// job is recorded as cancelled at once and `work` dropped, like the work
    /// the shutdown dropped. What it stood for is in the projects' databases
    /// and queued again on the next launch.
    pub fn submit(&self, spec: JobSpec, work: impl FnOnce(&JobCtx) -> JobResult + Send + 'static) -> JobId {
        self.enqueue(spec, Box::new(work), |_| false).expect("nothing is a duplicate")
    }

    /// Queue `work` unless a queued or running job matching `dup` is already
    /// there (`None`, and `work` is dropped). Checked and queued under one
    /// lock, so two callers can't both find none and queue it twice.
    pub fn submit_unless(
        &self,
        dup: impl Fn(&JobInfo) -> bool,
        spec: JobSpec,
        work: impl FnOnce(&JobCtx) -> JobResult + Send + 'static,
    ) -> Option<JobId> {
        self.enqueue(spec, Box::new(work), dup)
    }

    fn enqueue(&self, spec: JobSpec, work: Work, dup: impl Fn(&JobInfo) -> bool) -> Option<JobId> {
        let id = JobId::new();
        let mut info = JobInfo {
            id,
            kind: spec.kind,
            title: spec.title,
            subtitle: spec.subtitle,
            project_id: spec.project_id,
            media_id: spec.media_id,
            export_id: spec.export_id,
            state: JobState::Queued,
            progress: None,
            stage: None,
            eta_ms: None,
            error: None,
            created_at: now_ms(),
            finished_at: None,
            rev: 0,
        };
        // Work that doesn't run is dropped after the lock is released: what
        // it owns may look at the jobs as it goes (see `cancel_where`).
        let (info, refused) = {
            let mut st = self.inner.state.lock();
            if st.jobs.iter().any(|e| !e.info.state.is_finished() && dup(&e.info)) {
                (None, Some(work))
            } else {
                info.rev = st.next_rev();
                let (work, refused) = if st.shutting_down {
                    info.state = JobState::Cancelled;
                    info.finished_at = Some(now_ms());
                    (None, Some(work))
                } else {
                    (Some(work), None)
                };
                st.jobs.push_back(Entry { info: info.clone(), class: spec.class, cancel: CancelToken::new(), work });
                (Some(info), refused)
            }
        };
        drop(refused);
        let info = info?;
        (self.inner.listener)(JobEvent::Changed(info));
        self.inner.wake.notify_all();
        Some(id)
    }

    /// Newest first.
    pub fn jobs(&self) -> Vec<JobInfo> {
        self.inner.state.lock().jobs.iter().rev().map(|e| e.info.clone()).collect()
    }

    /// Is a queued or running job matching `pred` already there?
    pub fn is_active(&self, pred: impl Fn(&JobInfo) -> bool) -> bool {
        self.inner.state.lock().jobs.iter().any(|e| !e.info.state.is_finished() && pred(&e.info))
    }

    /// Cancel one job. Queued jobs end immediately; running ones are signalled
    /// and stop at their next cancellation check (ffmpeg is killed within ~40 ms).
    pub fn cancel(&self, id: JobId) {
        self.cancel_where(|i| i.id == id);
    }

    /// Cancel every unfinished job matching `pred` (e.g. all jobs of a project being deleted).
    ///
    /// The work of queued jobs is dropped once the lock is released, so what
    /// it owns can act on being dropped unstarted (and look at the jobs while
    /// it does).
    pub fn cancel_where(&self, pred: impl Fn(&JobInfo) -> bool) {
        let mut changed = Vec::new();
        let mut dropped = Vec::new();
        {
            let mut st = self.inner.state.lock();
            let mut rev = st.rev;
            for e in st.jobs.iter_mut().filter(|e| !e.info.state.is_finished() && pred(&e.info)) {
                e.cancel.cancel();
                if e.info.state == JobState::Queued {
                    dropped.extend(e.work.take());
                    e.info.state = JobState::Cancelled;
                    e.info.finished_at = Some(now_ms());
                    rev += 1;
                    e.info.rev = rev;
                    changed.push(e.info.clone());
                }
            }
            st.rev = rev;
        }
        for info in changed {
            (self.inner.listener)(JobEvent::Changed(info));
        }
        drop(dropped);
    }

    /// The app is quitting: running work is being cancelled to exit, not
    /// because someone asked for it.
    pub fn is_shutting_down(&self) -> bool {
        self.inner.state.lock().shutting_down
    }

    /// Forget finished jobs (Activity → Clear).
    pub fn clear_finished(&self) {
        self.inner.state.lock().jobs.retain(|e| !e.info.state.is_finished());
    }

    /// Cancel everything and wait up to `grace` for running work to stop, so
    /// no ffmpeg process outlives the app.
    pub fn shutdown(&self, grace: Duration) {
        let dropped: Vec<Work> = {
            let mut st = self.inner.state.lock();
            st.shutting_down = true;
            st.shutdowns += 1;
            st.jobs
                .iter_mut()
                .filter_map(|e| {
                    e.cancel.cancel();
                    e.work.take()
                })
                .collect()
        };
        drop(dropped);
        self.inner.wake.notify_all();
        self.settle(grace);
    }

    /// Wait up to `grace` until no job is running.
    fn settle(&self, grace: Duration) {
        let deadline = std::time::Instant::now() + grace;
        while std::time::Instant::now() < deadline {
            if !self.inner.state.lock().jobs.iter().any(|e| e.info.state == JobState::Running) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Bring every class back to its worker limit. Workers still finishing a job
/// during a shutdown count as alive, so a resume never exceeds the limit;
/// one that saw the shutdown and is leaving doesn't, so it is replaced.
fn spawn_missing_workers(inner: &Arc<Inner>, st: &mut State) {
    for &(class, n) in &inner.limits {
        let live = st.live.entry(class).or_insert(0);
        while *live < n {
            let worker = inner.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("jobs-{class:?}-{}", *live).to_lowercase())
                .spawn(move || run_worker(&worker, class));
            if let Err(e) = spawned {
                tracing::error!(?class, error = %e, "couldn't start a job worker");
                break;
            }
            *live += 1;
        }
    }
}

/// The next job a worker of `class` takes: projects take turns — the one
/// with the fewest jobs of the class running, then the one served longest
/// ago — and each project's jobs go oldest first. (A plain "not the project
/// served last" would let two projects take turns and starve a third.)
fn next_job(st: &State, class: Class) -> Option<usize> {
    let mut running: HashMap<Option<ProjectId>, usize> = HashMap::new();
    for e in st.jobs.iter().filter(|e| e.class == class && e.info.state == JobState::Running) {
        *running.entry(e.info.project_id).or_default() += 1;
    }
    st.jobs
        .iter()
        .enumerate()
        .filter(|(_, e)| e.class == class && e.info.state == JobState::Queued && e.work.is_some())
        .min_by_key(|&(i, e)| {
            let project = e.info.project_id;
            (running.get(&project).copied().unwrap_or(0), st.served.get(&(class, project)).copied().unwrap_or(0), i)
        })
        .map(|(i, _)| i)
}

/// A worker thread as `live` counts it: however the thread ends (a shutdown
/// asks it to; a panic outside its jobs' own catch), it stops counting. A
/// panic brings its class back to its limit and fails the job it held, so
/// nothing waits for a thread that is gone. (No event is sent from there: a
/// panic while unwinding would abort the app.)
struct Alive<'a> {
    inner: &'a Arc<Inner>,
    class: Class,
    holding: Option<JobId>,
    /// Still in `live` (see [`Alive::leave`]).
    counted: bool,
}

impl Alive<'_> {
    /// Stop counting, under the lock that saw the shutdown: a resume right
    /// after then starts another worker instead of counting this one, which
    /// is on its way out.
    fn leave(&mut self, st: &mut State) {
        if let Some(live) = st.live.get_mut(&self.class) {
            *live = live.saturating_sub(1);
        }
        self.counted = false;
    }
}

impl Drop for Alive<'_> {
    fn drop(&mut self) {
        let mut st = self.inner.state.lock();
        if self.counted {
            self.leave(&mut st);
        }
        if !std::thread::panicking() {
            return;
        }
        tracing::error!(class = ?self.class, job = ?self.holding, "a job worker panicked");
        let rev = st.next_rev();
        if let Some(e) = st.jobs.iter_mut().find(|e| Some(e.info.id) == self.holding && !e.info.state.is_finished()) {
            e.info.state = JobState::Failed;
            e.info.error = Some("unexpected error".into());
            e.info.eta_ms = None;
            e.info.finished_at = Some(now_ms());
            e.info.rev = rev;
        }
        if !st.shutting_down {
            spawn_missing_workers(self.inner, &mut st);
        }
    }
}

fn run_worker(inner: &Arc<Inner>, class: Class) {
    let mut alive = Alive { inner, class, holding: None, counted: true };
    loop {
        // Claim the next queued job of our class.
        let (id, work, cancel, info) = {
            let mut st = inner.state.lock();
            loop {
                if st.shutting_down {
                    alive.leave(&mut st);
                    drop(st);
                    return;
                }
                let rev = st.rev + 1;
                if let Some(i) = next_job(&st, class) {
                    let e = &mut st.jobs[i];
                    e.info.state = JobState::Running;
                    e.info.rev = rev;
                    let claimed = (e.info.id, e.work.take().expect("checked"), e.cancel.clone(), e.info.clone());
                    st.rev = rev;
                    st.served.insert((class, claimed.3.project_id), rev);
                    break claimed;
                }
                inner.wake.wait(&mut st);
            }
        };
        alive.holding = Some(id);
        (inner.listener)(JobEvent::Changed(info));

        let sink = {
            let inner = inner.clone();
            ProgressSink::new(PROGRESS_INTERVAL, move |p| {
                let progress = (p.total > 0).then(|| p.done as f64 / p.total as f64);
                let stage = (!p.stage.is_empty()).then(|| p.stage.clone());
                let eta_ms = p.eta_ms.map(|e| e as i64);
                let rev = {
                    let mut st = inner.state.lock();
                    let rev = st.next_rev();
                    match st.jobs.iter_mut().find(|e| e.info.id == id) {
                        Some(e) => {
                            e.info.progress = progress;
                            e.info.stage = stage.clone();
                            e.info.eta_ms = eta_ms;
                            e.info.rev = rev;
                            rev
                        }
                        None => return,
                    }
                };
                (inner.listener)(JobEvent::Progress(JobProgress { id, rev, progress, stage, eta_ms }));
            })
        };
        let ctx = JobCtx { id, cancel: cancel.clone(), sink };
        let result = catch_unwind(AssertUnwindSafe(|| work(&ctx))).unwrap_or_else(|panic| {
            let msg = panic_message(&*panic);
            tracing::error!(%id, %msg, "job panicked");
            Err(JobError::Failed(msg))
        });

        let info = {
            let mut st = inner.state.lock();
            alive.holding = None;
            let rev = st.next_rev();
            let Some(e) = st.jobs.iter_mut().find(|e| e.info.id == id) else { continue };
            e.info.rev = rev;
            match result {
                Ok(()) => {
                    e.info.state = JobState::Done;
                    e.info.progress = Some(1.0);
                }
                // A cancelled job's work often fails with an I/O or ffmpeg error
                // on the way out; the cancel flag decides what it was.
                Err(_) if cancel.is_cancelled() => e.info.state = JobState::Cancelled,
                Err(JobError::Cancelled) => e.info.state = JobState::Cancelled,
                Err(JobError::Failed(msg)) => {
                    tracing::warn!(%id, title = %e.info.title, error = %msg, "job failed");
                    e.info.state = JobState::Failed;
                    e.info.error = Some(msg);
                }
            }
            e.info.eta_ms = None;
            e.info.finished_at = Some(now_ms());
            let info = e.info.clone();
            trim_finished(&mut st);
            info
        };
        (inner.listener)(JobEvent::Changed(info));
    }
}

fn trim_finished(st: &mut State) {
    let finished = st.jobs.iter().filter(|e| e.info.state.is_finished()).count();
    let mut excess = finished.saturating_sub(KEEP_FINISHED);
    st.jobs.retain(|e| {
        if excess > 0 && e.info.state.is_finished() {
            excess -= 1;
            false
        } else {
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    fn spec(title: &str, class: Class) -> JobSpec {
        JobSpec {
            kind: JobKind::Ingest,
            class,
            title: title.into(),
            subtitle: None,
            project_id: None,
            media_id: None,
            export_id: None,
        }
    }

    fn runner(limits: &[(Class, usize)]) -> (JobRunner, mpsc::Receiver<JobEvent>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        (JobRunner::new(limits, move |e| drop(tx.lock().send(e))), rx)
    }

    /// Wait until job `id` reaches a finished state.
    fn finished(rx: &mpsc::Receiver<JobEvent>, id: JobId) -> JobInfo {
        loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("job event") {
                JobEvent::Changed(i) if i.id == id && i.state.is_finished() => return i,
                _ => {}
            }
        }
    }

    #[test]
    fn runs_reports_and_finishes() {
        let (r, rx) = runner(&[(Class::Media, 1)]);
        let id = r.submit(spec("index", Class::Media), |ctx| {
            ctx.progress("Indexing", 0.5);
            Ok(())
        });
        let info = finished(&rx, id);
        assert_eq!(info.state, JobState::Done);
        assert_eq!(info.progress, Some(1.0));
        let failed = r.submit(spec("broken", Class::Media), |_| Err(JobError::Failed("no video".into())));
        let info = finished(&rx, failed);
        assert_eq!((info.state, info.error.as_deref()), (JobState::Failed, Some("no video")));
        let panicked = r.submit(spec("panic", Class::Media), |_| panic!("boom"));
        assert_eq!(finished(&rx, panicked).state, JobState::Failed);
        assert_eq!(r.jobs().len(), 3);
        r.clear_finished();
        assert!(r.jobs().is_empty());
    }

    #[test]
    fn progress_is_told_in_fine_steps_and_unknown_when_it_is() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let ctx = JobCtx {
            id: JobId::new(),
            cancel: CancelToken::new(),
            sink: ProgressSink::new(Duration::ZERO, move |p| s.lock().push((p.done, p.total))),
        };
        ctx.progress("Clip 1 of 2", 0.000_4);
        ctx.report("Clip 1 of 2", None);
        ctx.report("Clip 2 of 2", Some(0.5));
        assert_eq!(*seen.lock(), [(400, PARTS), (0, 0), (PARTS / 2, PARTS)]);
    }

    #[test]
    fn cancels_queued_and_running_jobs() {
        let (r, rx) = runner(&[(Class::Export, 1)]);
        let (started_tx, started_rx) = mpsc::channel();
        let running = r.submit(spec("long", Class::Export), move |ctx| {
            started_tx.send(()).unwrap();
            while !ctx.cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(JobError::Failed("ffmpeg killed".into()))
        });
        let queued = r.submit(spec("next", Class::Export), |_| Ok(()));
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(r.is_active(|i| i.id == queued));
        r.cancel(queued);
        assert_eq!(finished(&rx, queued).state, JobState::Cancelled);
        r.cancel(running);
        assert_eq!(finished(&rx, running).state, JobState::Cancelled, "cancel wins over the error it causes");
    }

    #[test]
    fn resume_after_shutdown_runs_new_work() {
        let (r, rx) = runner(&[(Class::Media, 2)]);
        let blocker = r.submit(spec("long", Class::Media), |ctx| {
            while !ctx.cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(JobError::Cancelled)
        });
        while r.jobs().iter().all(|j| j.state != JobState::Running) {
            std::thread::sleep(Duration::from_millis(5));
        }
        r.submit(spec("never ran", Class::Media), |_| Ok(()));
        r.shutdown(Duration::from_secs(2));
        // Nothing is queued while the app shuts down: it ends at once.
        let ran = Arc::new(AtomicBool::new(false));
        let r2 = ran.clone();
        let refused = r.submit(spec("too late", Class::Media), move |_| {
            r2.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(finished(&rx, refused).state, JobState::Cancelled);
        assert!(!r.is_active(|j| j.id == refused));
        assert!(r.resume(Duration::from_secs(5)));
        assert!(!r.resume(Duration::from_secs(5)), "nothing more to undo");
        let after = r.submit(spec("after", Class::Media), |_| Ok(()));
        assert_eq!(finished(&rx, after).state, JobState::Done, "work runs again after resume");
        let jobs = r.jobs();
        assert!(jobs.iter().all(|j| j.state.is_finished()), "no job left looking queued: {jobs:?}");
        assert!(jobs.iter().any(|j| j.id == blocker && j.state == JobState::Cancelled));
        // Workers came back up to the limit, not beyond it.
        assert_eq!(r.inner.state.lock().live[&Class::Media], 2);
        assert!(!ran.load(Ordering::SeqCst), "work refused during the shutdown never runs");
    }

    /// Work that, once cancelled, stops only when the returned sender says
    /// so (or is dropped), then records whether the runner still said it was
    /// shutting down.
    fn slow_to_stop(r: &JobRunner) -> (JobId, mpsc::Sender<()>, Arc<AtomicBool>) {
        let (release, released) = mpsc::channel::<()>();
        let saw = Arc::new(AtomicBool::new(false));
        let (seen, jobs) = (saw.clone(), r.clone());
        let id = r.submit(spec("slow to stop", Class::Analysis), move |ctx| {
            while !ctx.cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = released.recv();
            seen.store(jobs.is_shutting_down(), Ordering::SeqCst);
            Err(JobError::Cancelled)
        });
        while !r.is_active(|j| j.id == id && j.state == JobState::Running) {
            std::thread::sleep(Duration::from_millis(5));
        }
        (id, release, saw)
    }

    #[test]
    fn a_resume_waits_for_the_work_the_shutdown_stopped() {
        let (r, rx) = runner(&[(Class::Analysis, 1)]);
        assert!(!r.resume(Duration::from_secs(5)), "no shutdown to undo");
        // Work that takes a moment to stop sees the shutdown when it does,
        // so it leaves its state as a quit would (for the app to resume).
        let (slow, release, saw) = slow_to_stop(&r);
        r.shutdown(Duration::from_millis(20));
        assert!(r.is_active(|j| j.id == slow), "still stopping after the shutdown's grace");
        let later = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(release);
        });
        assert!(r.resume(Duration::from_secs(5)));
        later.join().unwrap();
        assert!(saw.load(Ordering::SeqCst), "it ended while the shutdown held");
        assert_eq!(finished(&rx, slow).state, JobState::Cancelled);
        let after = r.submit(spec("after", Class::Analysis), |_| Ok(()));
        assert_eq!(finished(&rx, after).state, JobState::Done);
        assert_eq!(r.inner.state.lock().live[&Class::Analysis], 1, "the worker that left was replaced");
        assert!(!r.resume(Duration::from_secs(5)), "nothing more to undo");
    }

    #[test]
    fn a_shutdown_begun_while_a_resume_waits_is_never_undone() {
        let (r, _rx) = runner(&[(Class::Analysis, 1)]);
        let (_, release, _) = slow_to_stop(&r);
        r.shutdown(Duration::from_millis(20));
        // The app quits after all while the resume waits for the slow job.
        let quitting = r.clone();
        let quit = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            quitting.shutdown(Duration::from_millis(20));
            drop(release);
        });
        assert!(!r.resume(Duration::from_secs(5)));
        quit.join().unwrap();
        assert!(r.is_shutting_down());
        let refused = r.submit(spec("too late", Class::Analysis), |_| Ok(()));
        assert!(!r.is_active(|j| j.id == refused), "nothing runs while the app quits");
    }

    /// Work that records `title` in `ran` when it runs.
    fn recorder(ran: &Arc<Mutex<Vec<String>>>, title: &str) -> impl FnOnce(&JobCtx) -> JobResult + Send + 'static {
        let (ran, title) = (ran.clone(), title.to_string());
        move |_| {
            ran.lock().push(title);
            Ok(())
        }
    }

    fn of(title: &str, class: Class, project: Option<ProjectId>) -> JobSpec {
        JobSpec { project_id: project, ..spec(title, class) }
    }

    /// Hold the only worker of `class` until the returned sender is used.
    fn hold(
        r: &JobRunner,
        rx: &mpsc::Receiver<JobEvent>,
        class: Class,
        project: Option<ProjectId>,
    ) -> (JobId, mpsc::Sender<()>) {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let id = r.submit(of("blocker", class, project), move |_| {
            release_rx.recv().ok();
            Ok(())
        });
        loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("job event") {
                JobEvent::Changed(i) if i.id == id && i.state == JobState::Running => return (id, release_tx),
                _ => {}
            }
        }
    }

    #[test]
    fn projects_take_turns() {
        let (r, rx) = runner(&[(Class::Media, 1)]);
        let (a, b, c) = (Some(ProjectId::new()), Some(ProjectId::new()), Some(ProjectId::new()));
        let ran = Arc::new(Mutex::new(Vec::new()));
        // A big import in A, then two files in B and one in C.
        let (_, release) = hold(&r, &rx, Class::Media, a);
        let mut last = None;
        for i in 1..=4 {
            last = Some(r.submit(of("a", Class::Media, a), recorder(&ran, &format!("a{i}"))));
        }
        r.submit(of("b", Class::Media, b), recorder(&ran, "b1"));
        r.submit(of("b", Class::Media, b), recorder(&ran, "b2"));
        r.submit(of("c", Class::Media, c), recorder(&ran, "c1"));
        release.send(()).unwrap();
        finished(&rx, last.unwrap());
        // B and C wait for no more than the job of A that ran; nobody is
        // starved, and A's jobs keep their order.
        assert_eq!(*ran.lock(), ["b1", "c1", "a1", "b2", "a2", "a3", "a4"]);
    }

    #[test]
    fn a_project_keeps_its_order() {
        let (r, rx) = runner(&[(Class::Analysis, 1)]);
        let p = Some(ProjectId::new());
        let ran = Arc::new(Mutex::new(Vec::new()));
        let (_, release) = hold(&r, &rx, Class::Analysis, p);
        let mut last = None;
        for i in 1..=5 {
            last = Some(r.submit(of("x", Class::Analysis, p), recorder(&ran, &i.to_string())));
        }
        release.send(()).unwrap();
        finished(&rx, last.unwrap());
        assert_eq!(*ran.lock(), ["1", "2", "3", "4", "5"]);
    }

    #[test]
    fn a_duplicate_is_never_queued_twice() {
        let (r, rx) = runner(&[(Class::Media, 1)]);
        let (_, release) = hold(&r, &rx, Class::Media, None);
        let start = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (r, start) = (r.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    r.submit_unless(|j| j.title == "index e1", spec("index e1", Class::Media), |_| Ok(()))
                })
            })
            .collect();
        let queued: Vec<JobId> = threads.into_iter().filter_map(|t| t.join().unwrap()).collect();
        assert_eq!(queued.len(), 1, "exactly one of the racing submits queues it");
        assert_eq!(r.jobs().iter().filter(|j| j.title == "index e1").count(), 1);
        release.send(()).unwrap();
        assert_eq!(finished(&rx, queued[0]).state, JobState::Done);
        // Once it has finished, it can be queued again.
        assert!(r.submit_unless(|j| j.title == "index e1", spec("index e1", Class::Media), |_| Ok(())).is_some());
    }

    #[test]
    fn a_panicking_listener_stops_nothing() {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        // Panics whenever a job starts or finishes: the worker carries on.
        let r = JobRunner::new(&[(Class::Export, 1)], move |e| {
            if let JobEvent::Changed(i) = &e
                && matches!(i.state, JobState::Running | JobState::Done)
            {
                drop(tx.lock().send(i.clone()));
                panic!("the UI is gone");
            }
        });
        let first = r.submit(spec("first", Class::Export), |_| Ok(()));
        let second = r.submit(spec("second", Class::Export), |_| Ok(()));
        let done = |id: JobId| loop {
            let i = rx.recv_timeout(Duration::from_secs(10)).expect("job event");
            if i.id == id && i.state == JobState::Done {
                return;
            }
        };
        done(first);
        done(second);
        assert_eq!(r.inner.state.lock().live[&Class::Export], 1);
    }

    #[test]
    fn a_worker_that_panics_is_replaced() {
        let (r, rx) = runner(&[(Class::Upkeep, 1)]);
        // Stand in for the one worker: it panics holding a job.
        let held = {
            let mut st = r.inner.state.lock();
            let id = JobId::new();
            let rev = st.next_rev();
            let info = JobInfo {
                id,
                kind: JobKind::Regroup,
                title: "held".into(),
                subtitle: None,
                project_id: None,
                media_id: None,
                export_id: None,
                state: JobState::Running,
                progress: None,
                stage: None,
                eta_ms: None,
                error: None,
                created_at: now_ms(),
                finished_at: None,
                rev,
            };
            st.jobs.push_back(Entry { info, class: Class::Upkeep, cancel: CancelToken::new(), work: None });
            id
        };
        let inner = r.inner.clone();
        let worker = std::thread::spawn(move || {
            let _alive = Alive { inner: &inner, class: Class::Upkeep, holding: Some(held), counted: true };
            panic!("a bug outside the job");
        });
        assert!(worker.join().is_err());
        let st = r.inner.state.lock();
        assert_eq!(st.live[&Class::Upkeep], 1, "back to its limit, not beyond");
        let e = st.jobs.iter().find(|e| e.info.id == held).unwrap();
        assert_eq!((e.info.state, e.info.error.as_deref()), (JobState::Failed, Some("unexpected error")));
        drop(st);
        let after = r.submit(spec("after", Class::Upkeep), |_| Ok(()));
        assert_eq!(finished(&rx, after).state, JobState::Done);
    }

    #[test]
    fn dropped_work_can_look_at_the_jobs() {
        /// Asks the runner about itself when dropped unstarted.
        struct Unstarted(JobRunner, Arc<AtomicBool>);
        impl Drop for Unstarted {
            fn drop(&mut self) {
                self.1.store(!self.0.is_shutting_down(), Ordering::SeqCst);
            }
        }
        let (r, rx) = runner(&[(Class::Media, 1)]);
        let (_, release) = hold(&r, &rx, Class::Media, None);
        let told = Arc::new(AtomicBool::new(false));
        let guard = Unstarted(r.clone(), told.clone());
        let queued = r.submit(spec("queued", Class::Media), move |_| {
            drop(guard);
            Ok(())
        });
        // Under the runner's lock this would deadlock.
        r.cancel(queued);
        assert!(told.load(Ordering::SeqCst), "dropped as a cancel, not as a shutdown");
        release.send(()).unwrap();
    }

    #[test]
    fn revisions_order_every_change() {
        let (r, rx) = runner(&[(Class::Media, 1)]);
        let id = r.submit(spec("index", Class::Media), |ctx| {
            ctx.progress("Indexing", 0.5);
            Ok(())
        });
        let mut revs = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
                JobEvent::Changed(i) => {
                    revs.push(i.rev);
                    if i.id == id && i.state.is_finished() {
                        break;
                    }
                }
                JobEvent::Progress(p) => revs.push(p.rev),
            }
        }
        let mut sorted = revs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), revs.len(), "every change has its own revision: {revs:?}");
        assert_eq!(r.jobs()[0].rev, *sorted.last().unwrap(), "the snapshot carries the latest revision");
    }

    #[test]
    fn classes_do_not_block_each_other() {
        let (r, rx) = runner(&[(Class::Media, 1), (Class::Export, 1)]);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let blocker = r.submit(spec("long export", Class::Export), move |_| {
            release_rx.recv().ok();
            Ok(())
        });
        let quick = r.submit(spec("index", Class::Media), |_| Ok(()));
        assert_eq!(finished(&rx, quick).state, JobState::Done);
        release_tx.send(()).unwrap();
        assert_eq!(finished(&rx, blocker).state, JobState::Done);
    }
}
