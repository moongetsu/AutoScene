use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// One progress sample. `total == 0` means indeterminate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub stage: String,
    pub done: u64,
    pub total: u64,
    pub eta_ms: Option<u64>,
    /// While the stage reads a media's frames one by one (indexing).
    pub frames: Option<FrameCount>,
}

/// The frames of a media a stage has read, while it reads them one by one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameCount {
    /// Frames read so far: counted, not estimated.
    pub done: u64,
    /// About how many the media has, from its length and frame rate as its
    /// file states them (only reading every one counts them exactly);
    /// `None` when the file doesn't say.
    pub total: Option<u64>,
    /// The frames a second the media plays at, as its file states it: the
    /// frames read a second over it is how many times real time they go.
    pub rate: Option<f64>,
    /// Frames read a second over the last few seconds, measured by the
    /// [`ProgressSink`] (what the stage gives is replaced); `None` until
    /// they have gone for long enough to tell.
    pub per_sec: Option<f64>,
}

type Callback = dyn Fn(&Progress) + Send + Sync;

/// How long a phase is watched, from its first move, before its time left
/// is told: a first burst says little about its pace.
const MEASURE_FOR: Duration = Duration::from_secs(2);

/// The span frames read a second are measured over: a few of ffmpeg's
/// samples (two a second), so the pace told is the one now, and steady.
const PACE_OVER: Duration = Duration::from_secs(4);

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
    pace: Pace,
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

/// Frames read over the last [`PACE_OVER`]: how many at which moment.
#[derive(Default)]
struct Pace {
    samples: VecDeque<(Instant, u64)>,
    moved: bool,
}

impl Pace {
    /// Add `done` frames read by `now`. Returns the frames read a second
    /// since [`PACE_OVER`] ago, or since they first moved if that is later,
    /// once that is [`MEASURE_FOR`] long. A stall reads as 0.
    fn add(&mut self, now: Instant, done: u64) -> Option<f64> {
        match self.samples.back() {
            // Read from the start again (a pass started over): measured anew.
            Some(&(_, last)) if done < last => {
                self.samples.clear();
                self.moved = false;
            }
            // Not moved yet: starting up says nothing about the pace.
            Some(&(_, last)) if done == last && !self.moved => self.samples.clear(),
            Some(_) => self.moved = true,
            None => {}
        }
        self.samples.push_back((now, done));
        // The oldest sample kept is the newest one at least PACE_OVER old.
        while self.samples.len() > 2 && now.duration_since(self.samples[1].0) >= PACE_OVER {
            self.samples.pop_front();
        }
        let (&(t0, d0), &(t1, d1)) = (self.samples.front()?, self.samples.back()?);
        let span = t1.duration_since(t0);
        (span >= MEASURE_FOR).then(|| (d1 - d0) as f64 / span.as_secs_f64())
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
        self.update_at(stage, done, total, None, Instant::now());
    }

    /// [`update`](Self::update) of a stage that reads a media's frames one
    /// by one: `frames` says how many so far, and the sink measures how
    /// fast they go (its `per_sec`).
    pub fn update_frames(&self, stage: &str, done: u64, total: u64, frames: FrameCount) {
        self.update_at(stage, done, total, Some(frames), Instant::now());
    }

    fn update_at(&self, stage: &str, done: u64, total: u64, frames: Option<FrameCount>, now: Instant) {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let frames = match frames {
            Some(f) => Some(FrameCount { per_sec: st.pace.add(now, f.done), ..f }),
            None => {
                st.pace = Pace::default();
                None
            }
        };
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
        (self.inner.cb)(&Progress { stage: stage.to_owned(), done, total, eta_ms, frames });
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
            self.sink.update_at(stage, done, total, None, self.t0 + Duration::from_secs_f64(at));
            let p = self.seen.lock().unwrap().last().cloned().expect("every sample is emitted");
            assert_eq!((p.stage.as_str(), p.done), (stage, done));
            p.eta_ms.map(|ms| ms as f64 / 1000.0)
        }

        /// The frames a second told for `done` frames read by second `at`.
        fn per_sec(&self, at: f64, done: u64) -> Option<f64> {
            let frames = FrameCount { done, total: Some(10_000), rate: Some(25.0), per_sec: None };
            self.sink.update_at("Indexing frames", done, 10_000, Some(frames), self.t0 + Duration::from_secs_f64(at));
            let p = self.seen.lock().unwrap().last().cloned().expect("every sample is emitted");
            let f = p.frames.expect("frames are told");
            assert_eq!((f.done, f.total, f.rate), (done, Some(10_000), Some(25.0)), "told as given");
            f.per_sec
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
    fn frames_a_second_are_told_once_they_have_gone_for_a_while() {
        // ffmpeg's samples, two a second: starting up for 3 s, then 100
        // frames a second.
        let c = Clocked::new();
        for k in 0..=6 {
            assert_eq!(c.per_sec(f64::from(k) * 0.5, 0), None, "starting up");
        }
        for k in 1..=3 {
            assert_eq!(c.per_sec(3.0 + f64::from(k) * 0.5, k as u64 * 50), None, "too soon to tell");
        }
        assert!(near(c.per_sec(5.0, 200), 100.0), "the start-up isn't counted");
        // It slows to 50 a second: told within the span the pace is measured over.
        let mut done = 200;
        for k in 1..=8 {
            done += 25;
            let pace = c.per_sec(5.0 + f64::from(k) * 0.5, done);
            assert!(pace.is_some_and(|p| (50.0..=100.0).contains(&p)), "between the two at {k}: {pace:?}");
        }
        assert!(near(c.per_sec(9.5, done + 25), 50.0));
        // A stall reads as none read, not as the pace before it.
        for k in 1..=9 {
            c.per_sec(9.5 + f64::from(k) * 0.5, done + 25);
        }
        assert_eq!(c.per_sec(14.5, done + 25), Some(0.0));
    }

    #[test]
    fn frames_read_again_are_measured_anew() {
        // A pass started over (in software, the GPU's had failed) counts
        // from 0 again, at its own pace.
        let c = Clocked::new();
        for k in 0..=10 {
            c.per_sec(f64::from(k) * 0.5, k as u64 * 100);
        }
        assert!(near(c.per_sec(5.5, 1100), 200.0));
        assert_eq!(c.per_sec(6.0, 0), None);
        assert_eq!(c.per_sec(7.0, 20), None);
        assert!(near(c.per_sec(8.5, 50), 20.0), "from where it started over");
        // A stage between them that reads no frames tells none, and the
        // frames after it are measured anew.
        c.sink.update_at("Creating poster", 960, 1000, None, c.t0 + Duration::from_secs(9));
        assert_eq!(c.seen.lock().unwrap().last().and_then(|p| p.frames), None);
        assert_eq!(c.per_sec(10.0, 60), None);
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
