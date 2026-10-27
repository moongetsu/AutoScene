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

/// Throttled progress reporter. Producers call [`ProgressSink::update`] as
/// often as they like; the callback fires at most once per `interval`, plus
/// always on stage changes and completion, so UIs never drown in events.
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
    stage_started: Option<Instant>,
    last_done: u64,
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
        let now = Instant::now();
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let stage_changed = st.stage != stage;
        if stage_changed {
            st.stage = stage.to_owned();
        }
        // The ETA clock restarts on fresh work — the first sample, or a
        // `done` that went backward — not on a stage-label change. Per-clip
        // stages like "Clip 7 of 48" report one cumulative `done`, so their
        // ETA spans the whole run; resetting on each label gave every clip
        // its own ETA instead.
        if st.stage_started.is_none() || done < st.last_done {
            st.stage_started = Some(now);
        }
        st.last_done = done;
        let finished = total > 0 && done >= total;
        let due = st.last_emit.is_none_or(|t| now.duration_since(t) >= self.inner.interval);
        if !(stage_changed || finished || due) {
            return;
        }
        st.last_emit = Some(now);
        let eta_ms = match (st.stage_started, total) {
            (Some(start), t) if t > 0 && done > 0 && done < t => {
                let elapsed = now.duration_since(start).as_secs_f64();
                let rate = done as f64 / elapsed.max(1e-3);
                Some(((t - done) as f64 / rate * 1000.0) as u64)
            }
            _ => None,
        };
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

    #[test]
    fn eta_clock_spans_label_changes_and_resets_on_rewind() {
        let etas = Arc::new(Mutex::new(Vec::new()));
        let e = etas.clone();
        let sink = ProgressSink::new(Duration::ZERO, move |p| {
            e.lock().unwrap().push(p.eta_ms);
        });
        sink.update("Clip 1 of 2", 400, 1000);
        std::thread::sleep(Duration::from_millis(50));
        // A new label with a still-climbing `done` is the same unit of
        // work: the ETA reflects the whole run, not just this stage.
        sink.update("Clip 2 of 2", 600, 1000);
        // A rewound `done` is fresh work: the clock restarts, so this ETA
        // is computed against ~0 elapsed — deterministically 9ms.
        sink.update("Clip 2 of 2", 100, 1000);
        let etas = etas.lock().unwrap();
        assert!(etas[1].unwrap() > 15, "{etas:?}");
        assert_eq!(etas[2], Some(9), "{etas:?}");
    }
}
