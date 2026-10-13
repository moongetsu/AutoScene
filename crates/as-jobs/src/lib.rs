//! Background jobs.
//!
//! Work is grouped into resource classes, each with its own worker threads,
//! so a long export never blocks indexing a new episode (and vice versa).
//! Every state change and (throttled) progress sample is reported through
//! one listener, which the app forwards to the UI as `job:*` events — the UI
//! never polls.
//!
//! M1 keeps the queue in memory; interrupted work is recovered from each
//! project's database on open. The persistent queue and the engine
//! supervisor arrive with the analysis engine in M2.

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
    /// Report `fraction` (0..1) of `stage`. Cheap; throttled to 4 Hz.
    pub fn progress(&self, stage: &str, fraction: f64) {
        let done = (fraction.clamp(0.0, 1.0) * 1000.0).round() as u64;
        self.sink.update(stage, done, 1000);
    }

    /// Report a stage whose length is unknown.
    pub fn stage(&self, stage: &str) {
        self.sink.update(stage, 0, 0);
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
    /// Oldest first.
    jobs: VecDeque<Entry>,
    shutting_down: bool,
    /// Last revision handed out; bumped under the lock on every change.
    rev: u64,
    /// Worker threads alive per class (they exit during a shutdown).
    live: HashMap<Class, usize>,
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
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

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
            listener: Box::new(listener),
            limits: limits.iter().map(|&(c, n)| (c, n.max(1))).collect(),
        });
        spawn_missing_workers(&inner, &mut inner.state.lock());
        Self { inner }
    }

    /// Undo a [`shutdown`](Self::shutdown) whose exit didn't happen (an
    /// update installer that failed to start): restart the workers that
    /// exited and close out queued jobs whose work was dropped.
    pub fn resume(&self) {
        let mut changed = Vec::new();
        {
            let mut st = self.inner.state.lock();
            if !st.shutting_down {
                return;
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
    }

    /// Queue `work`. Returns immediately.
    pub fn submit(&self, spec: JobSpec, work: impl FnOnce(&JobCtx) -> JobResult + Send + 'static) -> JobId {
        let id = JobId::new();
        let info = JobInfo {
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
        let info = {
            let mut st = self.inner.state.lock();
            let mut info = info;
            info.rev = st.next_rev();
            st.jobs.push_back(Entry {
                info: info.clone(),
                class: spec.class,
                cancel: CancelToken::new(),
                work: Some(Box::new(work)),
            });
            info
        };
        (self.inner.listener)(JobEvent::Changed(info));
        self.inner.wake.notify_all();
        id
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
    pub fn cancel_where(&self, pred: impl Fn(&JobInfo) -> bool) {
        let mut changed = Vec::new();
        {
            let mut st = self.inner.state.lock();
            let mut rev = st.rev;
            for e in st.jobs.iter_mut().filter(|e| !e.info.state.is_finished() && pred(&e.info)) {
                e.cancel.cancel();
                if e.info.state == JobState::Queued {
                    e.work = None;
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
        {
            let mut st = self.inner.state.lock();
            st.shutting_down = true;
            for e in st.jobs.iter_mut() {
                e.cancel.cancel();
                e.work = None;
            }
        }
        self.inner.wake.notify_all();
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
/// during a shutdown count as alive, so a resume never exceeds the limit.
fn spawn_missing_workers(inner: &Arc<Inner>, st: &mut State) {
    for &(class, n) in &inner.limits {
        let live = st.live.entry(class).or_insert(0);
        for i in *live..n {
            let inner = inner.clone();
            std::thread::Builder::new()
                .name(format!("jobs-{class:?}-{i}").to_lowercase())
                .spawn(move || worker(&inner, class))
                .expect("spawn job worker");
        }
        *live = (*live).max(n);
    }
}

fn worker(inner: &Arc<Inner>, class: Class) {
    loop {
        // Claim the oldest queued job of our class.
        let (id, work, cancel, info) = {
            let mut st = inner.state.lock();
            loop {
                if st.shutting_down {
                    *st.live.entry(class).or_insert(1) -= 1;
                    return;
                }
                let rev = st.rev + 1;
                let next = st
                    .jobs
                    .iter_mut()
                    .find(|e| e.class == class && e.info.state == JobState::Queued && e.work.is_some());
                if let Some(e) = next {
                    e.info.state = JobState::Running;
                    e.info.rev = rev;
                    let claimed = (e.info.id, e.work.take().expect("checked"), e.cancel.clone(), e.info.clone());
                    st.rev = rev;
                    break claimed;
                }
                inner.wake.wait(&mut st);
            }
        };
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
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unexpected error".into());
            tracing::error!(%id, %msg, "job panicked");
            Err(JobError::Failed(msg))
        });

        let info = {
            let mut st = inner.state.lock();
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
        r.resume();
        let after = r.submit(spec("after", Class::Media), |_| Ok(()));
        assert_eq!(finished(&rx, after).state, JobState::Done, "work runs again after resume");
        let jobs = r.jobs();
        assert!(jobs.iter().all(|j| j.state.is_finished()), "no job left looking queued: {jobs:?}");
        assert!(jobs.iter().any(|j| j.id == blocker && j.state == JobState::Cancelled));
        // Workers came back up to the limit, not beyond it.
        assert_eq!(r.inner.state.lock().live[&Class::Media], 2);
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
