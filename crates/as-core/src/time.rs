//! Frame-exact time model.
//!
//! Every media gets a [`FrameIndex`]: the presentation timestamp of each frame
//! the decoder actually outputs (built from ffmpeg `-stats_enc_pre`, never from
//! container packets, which disagree around edit lists, open-GOP leading
//! frames and corrupt frames). Shots, tracks, clips and overrides are all
//! [`FrameRange`]s on that grid. Seconds appear only when talking to ffmpeg or
//! the `<video>` element, and cut points sit at the **midpoint between two
//! frames** so float rounding can never move a cut by one frame.

use serde::{Deserialize, Serialize};

use crate::{CoreError, CoreResult};

/// Index of a decoded frame in presentation order.
pub type FrameIdx = u32;

// ---------------------------------------------------------------------------
// Rational
// ---------------------------------------------------------------------------

/// Exact rational number (time bases, frame rates). Always reduced, `den > 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "rational with zero denominator");
        let sign = if den < 0 { -1 } else { 1 };
        let g = gcd(num.unsigned_abs(), den.unsigned_abs()).max(1) as i64;
        Self { num: sign * num / g, den: sign * den / g }
    }

    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    pub fn recip(self) -> Self {
        Self::new(self.den, self.num)
    }

    /// Parse `"24000/1001"`, `"25"` or `"23.976"` (decimal is converted exactly).
    pub fn parse(s: &str) -> CoreResult<Self> {
        let s = s.trim();
        let bad = || CoreError::BadRational(s.to_owned());
        if let Some((n, d)) = s.split_once(['/', ':']) {
            let n: i64 = n.trim().parse().map_err(|_| bad())?;
            let d: i64 = d.trim().parse().map_err(|_| bad())?;
            if d == 0 {
                return Err(bad());
            }
            return Ok(Self::new(n, d));
        }
        if let Some((int, frac)) = s.split_once('.') {
            let scale = 10i64.checked_pow(frac.len() as u32).ok_or_else(bad)?;
            let digits: i64 = format!("{int}{frac}").parse().map_err(|_| bad())?;
            return Ok(Self::new(digits, scale));
        }
        Ok(Self::new(s.parse().map_err(|_| bad())?, 1))
    }
}

impl std::fmt::Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

// ---------------------------------------------------------------------------
// FrameRange
// ---------------------------------------------------------------------------

/// Half-open range of frames `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, specta::Type)]
pub struct FrameRange {
    pub start: FrameIdx,
    pub end: FrameIdx,
}

impl FrameRange {
    pub fn new(start: FrameIdx, end: FrameIdx) -> Self {
        debug_assert!(start <= end, "FrameRange start {start} > end {end}");
        Self { start, end: end.max(start) }
    }

    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    pub fn contains(&self, f: FrameIdx) -> bool {
        f >= self.start && f < self.end
    }

    pub fn overlaps(&self, o: &FrameRange) -> bool {
        self.start < o.end && o.start < self.end
    }

    pub fn intersect(&self, o: &FrameRange) -> Option<FrameRange> {
        let s = self.start.max(o.start);
        let e = self.end.min(o.end);
        (s < e).then(|| FrameRange::new(s, e))
    }

    /// Overlap length divided by union length.
    pub fn iou(&self, o: &FrameRange) -> f32 {
        let inter = self.intersect(o).map_or(0, |r| r.len());
        let union = self.len() + o.len() - inter;
        if union == 0 { 0.0 } else { inter as f32 / union as f32 }
    }
}

// ---------------------------------------------------------------------------
// FrameIndex
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum PtsStore {
    /// Constant frame rate: `pts(f) = first + f * step`.
    Cfr { first: i64, step: i64, n: u32 },
    /// Arbitrary (VFR, gaps): explicit timestamps.
    Explicit(Vec<i64>),
}

/// Presentation timestamps of every decoded frame of one media.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameIndex {
    /// Time base of `pts` values.
    pub tb: Rational,
    /// Nominal (container/stream) frame rate, used for timecode display only.
    pub nominal_fps: Rational,
    pts: PtsStore,
    /// Duration of the final frame, in `tb` units.
    last_dur: i64,
    /// Timestamps stray from the nominal frame grid (true variable frame rate).
    vfr: bool,
}

impl FrameIndex {
    /// Build from decoder output timestamps (strictly increasing). Detects the
    /// constant-rate fast path automatically.
    pub fn from_pts(tb: Rational, nominal_fps: Rational, pts: Vec<i64>) -> CoreResult<Self> {
        if pts.is_empty() {
            return Err(CoreError::EmptyIndex);
        }
        if let Some(i) = pts.windows(2).position(|w| w[1] <= w[0]) {
            return Err(CoreError::NonMonotonic(i + 1));
        }
        let fallback_dur = {
            // One nominal frame in tb units: tb_den / (fps * tb_num).
            let d = (tb.den as f64) / (nominal_fps.to_f64().max(1e-6) * tb.num as f64);
            (d.round() as i64).max(1)
        };
        let last_dur = if pts.len() >= 2 { pts[pts.len() - 1] - pts[pts.len() - 2] } else { fallback_dur };
        let n = pts.len() as u32;
        let store = if pts.len() >= 2 {
            let step = pts[1] - pts[0];
            if pts.windows(2).all(|w| w[1] - w[0] == step) {
                PtsStore::Cfr { first: pts[0], step, n }
            } else {
                PtsStore::Explicit(pts)
            }
        } else {
            PtsStore::Cfr { first: pts[0], step: fallback_dur, n }
        };
        let vfr = detect_vfr(&store, tb, nominal_fps);
        Ok(Self { tb, nominal_fps, pts: store, last_dur, vfr })
    }

    /// Synthetic constant-rate index (tests, fixtures).
    pub fn cfr(tb: Rational, fps: Rational, first: i64, n: u32) -> Self {
        let step = ((tb.den as i128 * fps.den as i128) / (tb.num as i128 * fps.num as i128)) as i64;
        Self {
            tb,
            nominal_fps: fps,
            pts: PtsStore::Cfr { first, step: step.max(1), n },
            last_dur: step.max(1),
            vfr: false,
        }
    }

    pub fn len(&self) -> u32 {
        match &self.pts {
            PtsStore::Cfr { n, .. } => *n,
            PtsStore::Explicit(v) => v.len() as u32,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True variable frame rate. A constant-rate stream stored in a coarse
    /// time base (24 fps in MKV's 1 ms ticks → 41/42 ms steps) is *not* VFR.
    pub fn is_vfr(&self) -> bool {
        self.vfr
    }

    /// Timestamp of frame `f` in `tb` units. `f == len()` gives the end time.
    pub fn pts(&self, f: FrameIdx) -> i64 {
        let n = self.len();
        assert!(f <= n, "frame {f} out of range (len {n})");
        if f == n {
            return self.pts(n - 1) + self.last_dur;
        }
        match &self.pts {
            PtsStore::Cfr { first, step, .. } => first + f as i64 * step,
            PtsStore::Explicit(v) => v[f as usize],
        }
    }

    /// Absolute presentation time of frame `f` in seconds (includes the
    /// stream start offset — matches ffmpeg `-copyts` timestamps).
    pub fn t(&self, f: FrameIdx) -> f64 {
        self.pts(f) as f64 * self.tb.num as f64 / self.tb.den as f64
    }

    /// Time of the first frame (stream start offset).
    pub fn t0(&self) -> f64 {
        self.t(0)
    }

    /// Display duration of frame `f` in seconds.
    pub fn frame_dur(&self, f: FrameIdx) -> f64 {
        self.t(f + 1) - self.t(f)
    }

    /// Total duration in seconds.
    pub fn duration(&self) -> f64 {
        self.t(self.len()) - self.t0()
    }

    /// Cut point *before* frame `f`: midway between `f-1` and `f`, or half a
    /// frame before the first frame. Safe to feed to `trim=start=`.
    pub fn cut_before(&self, f: FrameIdx) -> f64 {
        if f == 0 { self.t(0) - 0.5 * self.frame_dur(0) } else { 0.5 * (self.t(f - 1) + self.t(f)) }
    }

    /// Cut point *after* the last frame of an exclusive-end range `[.., end)`:
    /// midway between `end-1` and `end`, or half a frame past the final frame.
    pub fn cut_after(&self, end: FrameIdx) -> f64 {
        let n = self.len();
        assert!(end >= 1 && end <= n, "cut_after({end}) out of range (len {n})");
        if end == n { self.t(n - 1) + 0.5 * self.frame_dur(n - 1) } else { 0.5 * (self.t(end - 1) + self.t(end)) }
    }

    /// Frame displayed at absolute time `t` (seconds): the last frame whose
    /// timestamp is `<= t` (plus a 1/4-frame tolerance for float noise).
    pub fn frame_at(&self, t: f64) -> FrameIdx {
        let n = self.len();
        if t <= self.t(0) {
            return 0;
        }
        let tol = 0.25 * self.frame_dur(0);
        let target = t + tol;
        match &self.pts {
            PtsStore::Cfr { first, step, .. } => {
                let ticks = target * self.tb.den as f64 / self.tb.num as f64;
                let f = ((ticks - *first as f64) / *step as f64).floor();
                (f.max(0.0) as u32).min(n - 1)
            }
            PtsStore::Explicit(v) => {
                let ticks = (target * self.tb.den as f64 / self.tb.num as f64) as i64;
                let i = v.partition_point(|&p| p <= ticks);
                (i.saturating_sub(1) as u32).min(n - 1)
            }
        }
    }

    // -- persistence --------------------------------------------------------

    const MAGIC: &'static [u8; 4] = b"ASFI";

    /// Compact binary form (zstd) for the `frame_index` table.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity(64);
        raw.extend_from_slice(Self::MAGIC);
        for v in [self.tb.num, self.tb.den, self.nominal_fps.num, self.nominal_fps.den, self.last_dur] {
            raw.extend_from_slice(&v.to_le_bytes());
        }
        match &self.pts {
            PtsStore::Cfr { first, step, n } => {
                raw.push(0);
                raw.extend_from_slice(&first.to_le_bytes());
                raw.extend_from_slice(&step.to_le_bytes());
                raw.extend_from_slice(&n.to_le_bytes());
            }
            PtsStore::Explicit(v) => {
                raw.push(1);
                raw.extend_from_slice(&(v.len() as u32).to_le_bytes());
                raw.extend_from_slice(&v[0].to_le_bytes());
                // Delta encoding compresses VFR indexes ~50x.
                for w in v.windows(2) {
                    raw.extend_from_slice(&((w[1] - w[0]) as u32).to_le_bytes());
                }
            }
        }
        zstd::encode_all(raw.as_slice(), 9).expect("zstd encode to Vec cannot fail")
    }

    pub fn from_bytes(blob: &[u8]) -> CoreResult<Self> {
        let bad = |m: &str| CoreError::CorruptIndex(m.to_owned());
        let raw = zstd::decode_all(blob).map_err(|e| CoreError::CorruptIndex(e.to_string()))?;
        let mut r = Reader { b: &raw, i: 0 };
        if r.take(4).ok_or_else(|| bad("short"))? != Self::MAGIC {
            return Err(bad("magic"));
        }
        let mut h = [0i64; 5];
        for v in &mut h {
            *v = r.i64().ok_or_else(|| bad("header"))?;
        }
        let tb = Rational::new(h[0], h[1]);
        let nominal_fps = Rational::new(h[2], h[3]);
        let last_dur = h[4];
        let pts = match r.take(1).ok_or_else(|| bad("kind"))?[0] {
            0 => {
                let first = r.i64().ok_or_else(|| bad("cfr"))?;
                let step = r.i64().ok_or_else(|| bad("cfr"))?;
                let n = r.u32().ok_or_else(|| bad("cfr"))?;
                PtsStore::Cfr { first, step, n }
            }
            1 => {
                let n = r.u32().ok_or_else(|| bad("len"))? as usize;
                let mut v = Vec::with_capacity(n);
                v.push(r.i64().ok_or_else(|| bad("first"))?);
                for _ in 1..n {
                    let d = r.u32().ok_or_else(|| bad("delta"))? as i64;
                    v.push(v[v.len() - 1] + d);
                }
                PtsStore::Explicit(v)
            }
            _ => return Err(bad("kind")),
        };
        let vfr = detect_vfr(&pts, tb, nominal_fps);
        Ok(Self { tb, nominal_fps, pts, last_dur, vfr })
    }
}

/// Does any timestamp sit more than one tick off the ideal nominal-rate grid?
fn detect_vfr(store: &PtsStore, tb: Rational, fps: Rational) -> bool {
    let PtsStore::Explicit(v) = store else { return false };
    let step = (tb.den as f64 * fps.den as f64) / (tb.num as f64 * fps.num as f64);
    let first = v[0] as f64;
    v.iter().enumerate().any(|(i, &p)| (p as f64 - (first + i as f64 * step)).abs() > 1.0 + 1e-6)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i + n)?;
        self.i += n;
        Some(s)
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

// ---------------------------------------------------------------------------
// Timecode
// ---------------------------------------------------------------------------

/// SMPTE-style timecode for display and sidecars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timecode {
    pub hours: u32,
    pub minutes: u32,
    pub seconds: u32,
    pub frames: u32,
    pub drop_frame: bool,
}

impl Timecode {
    /// Timecode of frame `f` counted at nominal rate `fps`. 29.97/59.94 use
    /// drop-frame counting when `drop_frame` is requested.
    pub fn from_frame(f: u64, fps: Rational, drop_frame: bool) -> Self {
        let base = fps.to_f64().round().max(1.0) as u64;
        let is_ntsc = fps.den == 1001 && (base == 30 || base == 60);
        let df = drop_frame && is_ntsc;
        let mut n = f;
        if df {
            // SMPTE 12M: drop 2 (or 4 at 59.94) frame numbers each minute except every 10th.
            let drop = base / 15;
            let per_10min = base * 600 - drop * 9;
            let per_min = base * 60 - drop;
            let tens = n / per_10min;
            let rem = n % per_10min;
            n += drop * 9 * tens + if rem > drop { drop * ((rem - drop) / per_min) } else { 0 };
        }
        Self {
            hours: (n / (base * 3600)) as u32,
            minutes: ((n / (base * 60)) % 60) as u32,
            seconds: ((n / base) % 60) as u32,
            frames: (n % base) as u32,
            drop_frame: df,
        }
    }
}

impl std::fmt::Display for Timecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sep = if self.drop_frame { ';' } else { ':' };
        write!(f, "{:02}:{:02}:{:02}{sep}{:02}", self.hours, self.minutes, self.seconds, self.frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB: Rational = Rational { num: 1, den: 24000 };
    const FPS: Rational = Rational { num: 24000, den: 1001 };

    #[test]
    fn rational_parse_and_reduce() {
        assert_eq!(Rational::parse("24000/1001").unwrap(), Rational::new(24000, 1001));
        assert_eq!(Rational::parse("30/1").unwrap(), Rational::new(30, 1));
        assert_eq!(Rational::parse("23.976").unwrap(), Rational::new(2997, 125));
        assert_eq!(Rational::new(2, -4), Rational::new(-1, 2));
        assert!(Rational::parse("1/0").is_err());
    }

    #[test]
    fn cfr_index_with_start_offset() {
        // 7.25 s start offset, 23.976 fps.
        let first = (7.25 * 24000.0) as i64;
        let pts: Vec<i64> = (0..100).map(|i| first + i * 1001).collect();
        let idx = FrameIndex::from_pts(TB, FPS, pts).unwrap();
        assert!(!idx.is_vfr());
        assert_eq!(idx.len(), 100);
        assert!((idx.t0() - 7.25).abs() < 1e-9);
        assert!((idx.duration() - 100.0 * 1001.0 / 24000.0).abs() < 1e-9);
        for f in 0..100 {
            assert_eq!(idx.frame_at(idx.t(f)), f);
            // A cut point strictly separates neighbouring frames.
            if f > 0 {
                let c = idx.cut_before(f);
                assert!(idx.t(f - 1) < c && c < idx.t(f));
                assert_eq!(idx.frame_at(c + 1e-6), f - 1, "cut midpoint maps to previous frame");
            }
        }
    }

    #[test]
    fn vfr_index_round_trips_and_resolves_frames() {
        // 24, then 60, then 30 fps segments in a 1/1000 time base (MKV-style).
        let mut pts = vec![];
        let mut t = 0i64;
        for (n, step) in [(48, 42), (120, 17), (60, 33)] {
            for _ in 0..n {
                pts.push(t);
                t += step;
            }
        }
        let idx = FrameIndex::from_pts(Rational::new(1, 1000), Rational::new(30, 1), pts.clone()).unwrap();
        assert!(idx.is_vfr());
        let back = FrameIndex::from_bytes(&idx.to_bytes()).unwrap();
        assert_eq!(back, idx);
        for (f, &p) in pts.iter().enumerate() {
            assert_eq!(back.pts(f as u32), p);
            assert_eq!(back.frame_at(p as f64 / 1000.0), f as u32);
        }
    }

    #[test]
    fn rounded_cfr_is_not_vfr() {
        // 24 fps stored in 1 ms ticks: steps alternate 41/42.
        let pts: Vec<i64> = (0..240).map(|i| (i as f64 * 1000.0 / 24.0).round() as i64).collect();
        let idx = FrameIndex::from_pts(Rational::new(1, 1000), Rational::new(24, 1), pts).unwrap();
        assert!(!idx.is_vfr());
        assert!(!FrameIndex::from_bytes(&idx.to_bytes()).unwrap().is_vfr());
    }

    #[test]
    fn cfr_round_trip_bytes() {
        let idx = FrameIndex::cfr(TB, FPS, 0, 64_800);
        assert_eq!(FrameIndex::from_bytes(&idx.to_bytes()).unwrap(), idx);
        assert!(idx.to_bytes().len() < 80);
    }

    #[test]
    fn rejects_non_monotonic() {
        assert!(matches!(FrameIndex::from_pts(TB, FPS, vec![0, 1001, 1001]), Err(CoreError::NonMonotonic(2))));
    }

    #[test]
    fn cut_after_end_of_media() {
        let idx = FrameIndex::cfr(TB, FPS, 0, 10);
        let c = idx.cut_after(10);
        assert!(c > idx.t(9) && c < idx.t(10));
    }

    #[test]
    fn frame_range_ops() {
        let a = FrameRange::new(10, 20);
        let b = FrameRange::new(15, 30);
        assert!(a.overlaps(&b));
        assert_eq!(a.intersect(&b), Some(FrameRange::new(15, 20)));
        assert!((a.iou(&b) - 5.0 / 20.0).abs() < 1e-6);
        assert!(!a.contains(20));
    }

    #[test]
    fn timecodes() {
        let tc = Timecode::from_frame(24 * 3600 + 24 * 61 + 5, Rational::new(24, 1), false);
        assert_eq!(tc.to_string(), "01:01:01:05");
        // 29.97 DF: frame 1800 is 00:01:00;02 (frames 00 and 01 are dropped).
        let df = Timecode::from_frame(1800, Rational::new(30000, 1001), true);
        assert_eq!(df.to_string(), "00:01:00;02");
        // Every 10th minute keeps its frames: 17982 frames = 00:10:00;00.
        assert_eq!(Timecode::from_frame(17982, Rational::new(30000, 1001), true).to_string(), "00:10:00;00");
    }
}
