use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One progress sample. `total == 0` means indeterminate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub stage: String,
    pub done: u64,
    pub total: u64,
    pub eta_ms: Option<u64>,
}

type Callback = dyn Fn(&Progress) + Send + Sync;

/// How long a phase is watched, from its first move, before its time left
/// is told: a first burst says little about its pace.
const MEASURE_FOR: Duration = Duration::from_secs(2);

/// Throttled progress reporter. Producers call [`ProgressSink::update`] as
/// often as they like; the callback fires at most once per `interval`, plus
/// always on stage changes and completion, so UIs never drown in events.
///
/// The time left is measured over a *phase*: the samples whose stage reads
/// the same with its numbers left out ("Clip 3 of 129", "Clip 4 of 129"),
/// from where the first of them stood and from when it first moved. So the
/// pace of a stage that counts its items comes from all of them, not from
/// the item just begun; a stage that resumes half done doesn't count that
/// half as done in no time; and starting up before the first move isn't
/// counted. A phase starts again when its progress goes back (the next
/// model of a download, a pass resumed from its last checkpoint) or its
/// length becomes known.
#[derive(Clone)]
pub struct ProgressSink {
    inner: Arc<Inner>,
}

struct Inner {
    cb: Box<Callback>,
    interval: Duration,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    last_emit: Option<Instant>,
    stage: String,
    phase: Option<Phase>,
}

/// Where and when the current phase began, and how far it got.
struct Phase {
    started: Instant,
    from: u64,
    last: u64,
    total: u64,
}

impl Phase {
    /// Milliseconds left at the phase's pace so far.
    fn eta_ms(&self, now: Instant, done: u64) -> Option<u64> {
        let elapsed = now.duration_since(self.started);
        let gained = done.saturating_sub(self.from);
        if self.total == 0 || done >= self.total || gained == 0 || elapsed < MEASURE_FOR {
            return None;
        }
        let per_ms = gained as f64 / elapsed.as_secs_f64() / 1000.0;
        Some(((self.total - done) as f64 / per_ms) as u64)
    }
}

/// Do two stages read the same with their numbers left out?
fn same_phase(a: &str, b: &str) -> bool {
    let words = |s: &str| s.chars().filter(|c| !c.is_ascii_digit()).collect::<String>();
    words(a) == words(b)
}

impl ProgressSink {
    pub fn new(interval: Duration, cb: impl Fn(&Progress) + Send + Sync + 'static) -> Self {
        Self { inner: Arc::new(Inner { cb: Box::new(cb), interval, state: Mutex::new(State::default()) }) }
    }

    /// A sink that discards everything (tests, headless runs).
    pub fn noop() -> Self {
        Self::new(Duration::from_secs(3600), |_| {})
    }

    pub fn update(&self, stage: &str, done: u64, total: u64) {
        self.update_at(stage, done, total, Instant::now());
    }

    fn update_at(&self, stage: &str, done: u64, total: u64, now: Instant) {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let stage_changed = st.stage != stage;
        let goes_on = st.phase.as_ref().is_some_and(|p| p.total == total && done >= p.last)
            && (!stage_changed || same_phase(&st.stage, stage));
        if stage_changed {
            st.stage = stage.to_owned();
        }
        let mut phase =
            st.phase.take().filter(|_| goes_on).unwrap_or(Phase { started: now, from: done, last: done, total });
        // Its clock starts when it first moves: starting a process or loading
        // models before that says nothing about its pace.
        if done == phase.from {
            phase.started = now;
        }
        phase.last = done;
        let eta_ms = phase.eta_ms(now, done);
        st.phase = Some(phase);
        let finished = total > 0 && done >= total;
        let due = st.last_emit.is_none_or(|t| now.duration_since(t) >= self.inner.interval);
        if !(stage_changed || finished || due) {
            return;
        }
        st.last_emit = Some(now);
        drop(st);
        (self.inner.cb)(&Progress { stage: stage.to_owned(), done, total, eta_ms });
    }
}

impl std::fmt::Debug for ProgressSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressSink").field("interval", &self.inner.interval).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn throttles_but_always_emits_stage_change_and_finish() {
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let sink = ProgressSink::new(Duration::from_secs(60), move |_| {
            h.fetch_add(1, Ordering::SeqCst);
        });
        sink.update("a", 0, 10); // stage change → emit
        for i in 1..9 {
            sink.update("a", i, 10); // throttled
        }
        sink.update("a", 10, 10); // finished → emit
        sink.update("b", 0, 5); // stage change → emit
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    /// A sink that keeps every sample, driven on a clock of whole seconds.
    struct Clocked {
        sink: ProgressSink,
        seen: Arc<Mutex<Vec<Progress>>>,
        t0: Instant,
    }

    impl Clocked {
        fn new() -> Self {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let s = seen.clone();
            let sink = ProgressSink::new(Duration::ZERO, move |p| s.lock().unwrap().push(p.clone()));
            Self { sink, seen, t0: Instant::now() }
        }

        /// The time left told for `done` of `total` at second `at`.
        fn eta_s(&self, at: f64, stage: &str, done: u64, total: u64) -> Option<f64> {
            self.sink.update_at(stage, done, total, self.t0 + Duration::from_secs_f64(at));
            let p = self.seen.lock().unwrap().last().cloned().expect("every sample is emitted");
            assert_eq!((p.stage.as_str(), p.done), (stage, done));
            p.eta_ms.map(|ms| ms as f64 / 1000.0)
        }
    }

    fn near(eta: Option<f64>, want: f64) -> bool {
        eta.is_some_and(|e| (e - want).abs() <= want * 0.02 + 0.01)
    }

    #[test]
    fn an_export_counting_its_clips_tells_the_time_left_of_all_of_them() {
        // 10 clips of 10 s each, the job's progress in thousandths: the
        // stage names the clip, so it changes with every one of them.
        let c = Clocked::new();
        for clip in 0..10u64 {
            for s in 0..10u64 {
                let at = (clip * 10 + s) as f64;
                let eta = c.eta_s(at, &format!("Clip {} of 10", clip + 1), clip * 100 + s * 10, 1000);
                let left = 100.0 - at;
                if at < 2.0 {
                    assert_eq!(eta, None, "too soon to tell at {at} s");
                } else {
                    assert!(near(eta, left), "at {at} s: {eta:?}, not {left} s left");
                }
            }
        }
        // Joining them is a phase of its own, with a pace of its own.
        assert_eq!(c.eta_s(100.0, "Joining into one file", 950, 1000), None);
        assert_eq!(c.eta_s(101.0, "Joining into one file", 960, 1000), None);
        assert!(near(c.eta_s(104.0, "Joining into one file", 990, 1000), 1.0));
        assert_eq!(c.eta_s(105.0, "Joining into one file", 1000, 1000), None, "done: nothing left to tell");
    }

    #[test]
    fn a_resumed_stage_counts_only_what_it_does_now() {
        // Picked up at half way, then 1 % a second.
        let c = Clocked::new();
        assert_eq!(c.eta_s(0.0, "Finding shots", 500, 1000), None);
        assert!(near(c.eta_s(10.0, "Finding shots", 600, 1000), 40.0));
    }

    #[test]
    fn starting_up_before_the_first_move_is_not_counted() {
        // The engine loads its models for 10 s, then finds 1 % a second.
        let c = Clocked::new();
        for at in [0.0, 5.0, 10.0] {
            assert_eq!(c.eta_s(at, "Finding characters", 500, 1000), None);
        }
        assert!(near(c.eta_s(20.0, "Finding characters", 600, 1000), 40.0));
        // A download of unknown length that resumes at half way, then gets
        // 1 % a second: the bytes it had aren't counted as fetched.
        let c = Clocked::new();
        assert_eq!(c.eta_s(0.0, "Downloading AI model", 0, 0), None);
        assert_eq!(c.eta_s(0.3, "Downloading AI model", 500, 1000), None);
        assert!(near(c.eta_s(10.3, "Downloading AI model", 600, 1000), 40.0));
    }

    #[test]
    fn a_long_phase_in_fine_steps_is_told_right_from_its_first_move() {
        // An hour's export, sampled every half second in millionths (as jobs
        // report it): right within a percent as soon as it is told. In
        // thousandths the first step would come 1.8 s late and be counted
        // as made in no time.
        let c = Clocked::new();
        for k in 0..=40 {
            let at = f64::from(k) * 0.5;
            let eta = c.eta_s(at, "Clip 1 of 900", (at / 3600.0 * 1e6).round() as u64, 1_000_000);
            let want = 3600.0 - at;
            assert!(at < 2.0 || eta.is_some_and(|e| (e - want).abs() < want * 0.01), "at {at} s: {eta:?}, not {want}");
        }
    }

    #[test]
    fn a_stage_starting_over_is_measured_anew() {
        // Two models download under one name, the second at half the pace.
        let c = Clocked::new();
        assert_eq!(c.eta_s(0.0, "Downloading AI model", 0, 1000), None);
        assert!(near(c.eta_s(5.0, "Downloading AI model", 500, 1000), 5.0));
        assert_eq!(c.eta_s(10.0, "Downloading AI model", 0, 1000), None);
        assert!(near(c.eta_s(20.0, "Downloading AI model", 500, 1000), 10.0));
        // A stage of unknown length tells no time; one after it is new.
        assert_eq!(c.eta_s(30.0, "Grouping characters", 0, 0), None);
        assert_eq!(c.eta_s(31.0, "Finding people", 0, 1000), None);
        assert!(near(c.eta_s(41.0, "Finding people", 100, 1000), 90.0));
    }

    #[test]
    fn stages_are_one_phase_only_with_the_same_words() {
        assert!(same_phase("Clip 9 of 129", "Clip 10 of 129"));
        assert!(same_phase("Portrait 1 of 7", "Portrait 7 of 7"));
        assert!(!same_phase("Clip 9 of 129", "Joining into one file"));
        assert!(!same_phase("Poster 1 of 3", "Portrait 1 of 3"));
        let c = Clocked::new();
        c.eta_s(0.0, "Portrait 1 of 4", 0, 1000);
        assert!(near(c.eta_s(10.0, "Portrait 2 of 4", 250, 1000), 30.0));
        // The job looks again and finds two more: their run starts over.
        assert_eq!(c.eta_s(11.0, "Portrait 1 of 2", 0, 1000), None);
        assert!(near(c.eta_s(15.0, "Portrait 2 of 2", 500, 1000), 4.0));
    }
}
