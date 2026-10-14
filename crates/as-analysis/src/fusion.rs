//! Shot fusion: per-frame signals → a gap-free list of segments.
//!
//! 1. **Fades** to and from black or white: anchored on the extreme frames
//!    and measured by the luma moving steadily towards them. Very reliable,
//!    and TransNet is unsure exactly there.
//! 2. **Hard cuts**: TransNet's `single` decides *where* (runs above the
//!    content's threshold), the pixels decide *how many*: the largest jump
//!    near a run is a cut, and so is another jump nearly as large that also
//!    changes the colours and that TransNet backs — that is how 1- and
//!    2-frame shots survive while motion blur after a cut doesn't split it.
//! 3. **Gradual transitions**: runs of `many` at least 3 frames long that
//!    aren't just the echo of a hard cut, plus wipes, which TransNet hardly
//!    sees: short bursts of motion that blend the colours of the pictures
//!    on both sides. Their frames belong to no shot, so a clip never starts
//!    mid-dissolve.
//! 4. **Flashes**: a "cut" whose next 1–3 frames stand out and then the
//!    picture returns (muzzle flash, lightning, anime impact frame) is no cut.
//! 5. **Black** runs (2+ frames) outside fades are their own segments.
//! 6. **Tiny shots** (under 3 frames) stay when both cuts around them are
//!    strong, otherwise they join the more similar neighbour.
//!
//! Pure and deterministic: the same signals always give the same segments.

// Frame-by-frame rules read best with frame indexes.
#![allow(clippy::needless_range_loop)]

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::signals::{COLORS, FrameSignals, LAGS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// Cross-dissolve, wipe or any other blend between two shots.
    Dissolve,
    /// From a shot to black or white.
    FadeOut,
    /// From black or white into a shot.
    FadeIn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case", tag = "type", content = "transition")]
pub enum SegmentKind {
    Shot,
    Transition(TransitionKind),
    /// Black (or, inside a fade, white) frames.
    Black,
    Flash,
}

/// Frames `[start, end)` of one kind. Segments cover the video without gaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start: u32,
    pub end: u32,
    pub kind: SegmentKind,
    /// A shot under 3 frames kept because both cuts around it are clear.
    pub tiny: bool,
}

impl Segment {
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }

    pub fn is_shot(&self) -> bool {
        self.kind == SegmentKind::Shot
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FusionParams {
    /// TransNet `single` needed for a hard cut.
    pub cut: f32,
    /// TransNet `many` needed for a transition frame.
    pub gradual: f32,
    /// Flash: the picture after it is this close to the one before (MAD, 0..255)…
    pub flash_return: f32,
    /// …and the flash frames are at least this far from both.
    pub flash_contrast: f32,
}

impl FusionParams {
    pub fn live() -> Self {
        Self { cut: 0.5, gradual: 0.5, flash_return: 10.0, flash_contrast: 25.0 }
    }

    /// Anime: flat colours and impact frames make TransNet fire more easily.
    pub fn anime() -> Self {
        Self { cut: 0.6, flash_return: 14.0, ..Self::live() }
    }
}

/// A many-head run this short around a hard cut is the cut's echo, not a transition.
const ECHO_LEN: usize = 8;
const MIN_GRADUAL: usize = 3;
const MIN_BLACK: usize = 2;
/// A picture change (MAD) at least this big makes the edge of a black run abrupt.
const BLACK_EDGE: f32 = 8.0;
const MIN_SHOT: u32 = 3;
/// Share of near-black / near-white pixels that makes an extreme frame
/// (room for a logo, subtitles or a burn-in).
const EXTREME_SHARE: f32 = 0.9;
/// A fade spans at least this much luma between shot and extreme, and its
/// own frames cover at least this share of it.
const MIN_FADE_TRAVEL: f32 = 20.0;
const FADE_SHARE: f32 = 0.25;
/// Luma must keep moving by at least this much per frame to stay in a fade.
const FADE_STEP: f32 = 0.3;
/// A second jump next to a cut is a cut of its own at this share of the main
/// jump, this absolute size and this colour change.
const SECOND_CUT: f32 = 0.5;
const MIN_CUT_DIFF: f32 = 30.0;
const MIN_CUT_HIST: f32 = 0.25;
/// Wipes (positions on the way from palette A = 0 to B = 1): this many
/// blended frames, at least `WIPE_SIDE` stable frames on each side, looked
/// for `WIPE_REACH` frames around a centre. Blends start above `WIPE_EDGE`
/// and end below `1 - WIPE_EDGE`; the sides drift at most `WIPE_DRIFT`; the
/// walk never jumps by more than `WIPE_JUMP` (that is a cut) nor turns back
/// by more than `WIPE_BACK`.
const MIN_WIPE: usize = 3;
const MAX_WIPE: usize = 48;
const WIPE_SIDE: usize = 6;
const WIPE_REACH: usize = 36;
const WIPE_EDGE: f32 = 0.05;
const WIPE_DRIFT: f32 = 0.25;
const WIPE_JUMP: f32 = 0.5;
const WIPE_BACK: f32 = 0.15;
/// The pictures before and after a flash are this close (colour distance).
const FLASH_SAME: f32 = 0.1;
/// A cut counts as strong for tiny-shot purposes above these.
const STRONG_SINGLE: f32 = 0.8;
const STRONG_DIFF: f32 = 40.0;

#[derive(Clone, Copy, PartialEq)]
enum Cat {
    Shot,
    Transition(TransitionKind),
    Black,
    Flash,
}

pub fn fuse(sig: &[FrameSignals], p: &FusionParams) -> Vec<Segment> {
    let n = sig.len();
    if n == 0 {
        return Vec::new();
    }
    let d1 = |f: usize| sig[f].diff[0];
    // Signals without pixel shares (older data) fall back to mean and spread.
    let is_black = |f: usize| {
        let s = &sig[f];
        (s.dark >= EXTREME_SHARE && s.mu < 25.0) || (s.mu < 18.0 && s.sd < 4.0)
    };
    let is_white = |f: usize| sig[f].light >= EXTREME_SHARE && sig[f].mu > 200.0;
    let mut cat = vec![Cat::Shot; n];

    // 1. Fades around runs of black or white frames.
    let mut f = 0;
    while f < n {
        let white = is_white(f);
        if !(is_black(f) || white) {
            f += 1;
            continue;
        }
        let extreme = |x: usize| if white { is_white(x) } else { is_black(x) };
        let b0 = f;
        while f < n && extreme(f) {
            f += 1;
        }
        let b1 = f;
        // Luma heading towards the extreme (down for black, up for white).
        let towards = |from: usize, to: usize| {
            let d = sig[to].mu - sig[from].mu;
            if white { d > FADE_STEP } else { d < -FADE_STEP }
        };
        // A frame belongs to the fade-out when it has itself moved towards the
        // extreme since the frame before it.
        let mut a = b0;
        while a >= 2 && cat[a - 1] == Cat::Shot && towards(a - 2, a - 1) {
            a -= 1;
        }
        let mut e = b1;
        while e < n && e + 1 < n && towards(e + 1, e) {
            e += 1;
        }
        // e stops on the first frame of the next shot when the fade runs into it.
        // The fade frames must carry a fair share of the way between the shot
        // and the extreme, not just the step into (or out of) it: a hard cut
        // to black plus a little noise in the frame before it is no fade. A
        // single in-between frame is enough.
        let travel = |x: usize, y: usize| (sig[x].mu - sig[y].mu).abs();
        let gradual = |inside: f32, total: f32| total >= MIN_FADE_TRAVEL && inside >= FADE_SHARE * total;
        let fade_out = b0 > a && gradual(travel(a - 1, b0 - 1), travel(a - 1, b0));
        let fade_in = e > b1 && gradual(travel(b1, e), travel(b1 - 1, e));
        // A burst of a few extreme frames after which the same picture comes
        // back is a flash (step 4), however it ramps up.
        let flash = a >= 1 && b1 < n && b1 - a < LAGS && sig[a - 1].color_distance(&sig[b1]) < FLASH_SAME;
        if (fade_out || fade_in) && !flash {
            if fade_out {
                (a..b0).for_each(|x| cat[x] = Cat::Transition(TransitionKind::FadeOut));
            }
            (b0..b1).for_each(|x| cat[x] = Cat::Black);
            if fade_in {
                (b1..e).for_each(|x| cat[x] = Cat::Transition(TransitionKind::FadeIn));
                f = e;
            }
        }
    }

    // 2. Hard cut boundaries (a boundary at b: frames b-1 and b differ in shot).
    let mut cuts: Vec<usize> = Vec::new();
    let mut f = 0;
    while f < n {
        if sig[f].single <= p.cut {
            f += 1;
            continue;
        }
        let r0 = f;
        // Merge runs separated by a single quiet frame.
        while f < n && (sig[f].single > p.cut || (f + 1 < n && sig[f + 1].single > p.cut)) {
            f += 1;
        }
        let r1 = f - 1;
        let (lo, hi) = (r0.saturating_sub(1).max(1), (r1 + 3).min(n - 1));
        if lo > hi {
            continue;
        }
        let main = (lo..=hi).max_by(|&a, &b| d1(a).total_cmp(&d1(b)).then(b.cmp(&a))).unwrap();
        cuts.push(main);
        let backed = |u: usize| sig[u].single > p.cut / 2.0 || sig[u - 1].single > p.cut / 2.0;
        for u in lo..=hi {
            if u != main
                && d1(u) >= SECOND_CUT * d1(main)
                && d1(u) >= MIN_CUT_DIFF
                && sig[u].hist >= MIN_CUT_HIST
                && backed(u)
            {
                cuts.push(u);
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();

    // 3. Gradual transitions from TransNet.
    let mut f = 0;
    while f < n {
        if sig[f].many <= p.gradual || cat[f] != Cat::Shot {
            f += 1;
            continue;
        }
        let a = f;
        while f < n && sig[f].many > p.gradual && cat[f] == Cat::Shot {
            f += 1;
        }
        let e = f;
        let len = e - a;
        if len < MIN_GRADUAL {
            continue;
        }
        let near_cut = cuts.iter().any(|&b| b + 1 >= a && b <= e + 1);
        if near_cut && len <= ECHO_LEN {
            continue;
        }
        // A dissolve isn't a cut: drop cuts strictly inside it.
        cuts.retain(|&b| b <= a || b >= e);
        let kind = if a > 0 && cat[a - 1] == Cat::Black {
            TransitionKind::FadeIn
        } else if e < n && is_black(e) {
            TransitionKind::FadeOut
        } else {
            TransitionKind::Dissolve
        };
        (a..e).for_each(|x| cat[x] = Cat::Transition(kind));
    }

    // 3b. Wipes and other shaped transitions, which TransNet hardly sees.
    //     The colour signature of a blend is a mix of the two pictures', so a
    //     wipe is a steady walk from one stable palette to another, however
    //     much the pictures themselves move. Searched for in every stretch of
    //     shot frames between cuts; a find splits the stretch.
    let mut stretches: Vec<(usize, usize)> = Vec::new();
    let mut f = 0;
    while f < n {
        if cat[f] != Cat::Shot {
            f += 1;
            continue;
        }
        let r0 = f;
        f += 1;
        while f < n && cat[f] == Cat::Shot && cuts.binary_search(&f).is_err() {
            f += 1;
        }
        stretches.push((r0, f));
    }
    while let Some((r0, r1)) = stretches.pop() {
        if let Some((a, e)) = find_wipe(sig, r0, r1) {
            (a..e).for_each(|x| cat[x] = Cat::Transition(TransitionKind::Dissolve));
            stretches.push((r0, a));
            stretches.push((e, r1));
        }
    }

    // 4. Flashes: frames b..b+k that stand out, then the picture comes back.
    let mut flash_cuts = Vec::new();
    for &b in &cuts {
        for k in 1..LAGS {
            let back = b + k; // first frame after the flash
            if back >= n || b == 0 {
                break;
            }
            // MAD(back, b-1): `back` looks k+1 frames back.
            let returns = sig[back].diff_to(k + 1) < p.flash_return;
            // Every flash frame j differs strongly from b-1 (lag j-b+1) and from `back`.
            let stands_out =
                (b..back).all(|j| sig[j].diff_to(j - b + 1) > p.flash_contrast) && sig[back].diff[0] > p.flash_contrast;
            if returns && stands_out {
                for c in &mut cat[b..back] {
                    if *c == Cat::Shot {
                        *c = Cat::Flash;
                    }
                }
                flash_cuts.push(b);
                flash_cuts.push(back);
                break;
            }
        }
    }
    cuts.retain(|b| !flash_cuts.contains(b));

    // 5. Black runs outside fades, entered and left abruptly (a cut, or the
    //    picture jumping): a dark scene that merely drifts below the black
    //    threshold and back is still one shot.
    let abrupt = |cat: &[Cat], x: usize| {
        x == 0
            || x == n
            || cat[x - 1] != Cat::Shot
            || cat[x] != Cat::Shot
            || d1(x) >= BLACK_EDGE
            || cuts.binary_search(&x).is_ok()
    };
    let mut f = 0;
    while f < n {
        if !(is_black(f) && cat[f] == Cat::Shot) {
            f += 1;
            continue;
        }
        let a = f;
        while f < n && is_black(f) && cat[f] == Cat::Shot {
            f += 1;
        }
        if f - a >= MIN_BLACK && abrupt(&cat, a) && abrupt(&cat, f) {
            (a..f).for_each(|x| cat[x] = Cat::Black);
        }
    }

    // 6. Segments: a new one where the category changes or a cut splits shots.
    let mut segs: Vec<Segment> = Vec::new();
    let mut start = 0;
    for f in 1..=n {
        let split = f == n || cat[f] != cat[f - 1] || (cat[f] == Cat::Shot && cuts.binary_search(&f).is_ok());
        if split {
            let kind = match cat[start] {
                Cat::Shot => SegmentKind::Shot,
                Cat::Transition(k) => SegmentKind::Transition(k),
                Cat::Black => SegmentKind::Black,
                Cat::Flash => SegmentKind::Flash,
            };
            segs.push(Segment { start: start as u32, end: f as u32, kind, tiny: false });
            start = f;
        }
    }

    // 7. Tiny shots: keep between two clear cuts, else merge into the closer neighbour.
    let strong = |b: u32| {
        let b = b as usize;
        if b == 0 || b >= n {
            return true;
        }
        let peak = (b.saturating_sub(2)..=(b + 1).min(n - 1)).map(|x| sig[x].single).fold(0f32, f32::max);
        peak >= STRONG_SINGLE || d1(b) >= STRONG_DIFF
    };
    let mut i = 0;
    while i < segs.len() {
        let s = segs[i];
        if !s.is_shot() || s.len() >= MIN_SHOT {
            i += 1;
            continue;
        }
        if strong(s.start) && strong(s.end) {
            segs[i].tiny = true;
            i += 1;
            continue;
        }
        let prev_shot = i > 0 && segs[i - 1].is_shot();
        let next_shot = i + 1 < segs.len() && segs[i + 1].is_shot();
        // Merge across the weaker of the two cuts.
        let into_prev = match (prev_shot, next_shot) {
            (true, true) => d1(s.start as usize) <= d1((s.end as usize).min(n - 1)),
            (p, _) => p,
        };
        if into_prev {
            segs[i - 1].end = s.end;
            segs.remove(i);
        } else if next_shot {
            segs[i + 1].start = s.start;
            segs.remove(i);
        } else {
            segs[i].tiny = true;
            i += 1;
        }
    }
    segs
}

/// The blended frames `a..e` of a wipe inside the shot frames `r0..r1`, if
/// there is one: stable palette A, then frames that walk steadily from A to
/// B, then stable palette B, with A and B as different as across a cut.
fn find_wipe(sig: &[FrameSignals], r0: usize, r1: usize) -> Option<(usize, usize)> {
    if r1 - r0 < 2 * WIPE_SIDE + MIN_WIPE {
        return None;
    }
    let reach = |x: usize| (x.saturating_sub(WIPE_REACH).max(r0), (x + WIPE_REACH).min(r1 - 1));
    // Candidate centres: where the palettes a reach before and after differ
    // the most, strongest first; centres within a reach of one already tried
    // see the same walk.
    let mut centres: Vec<(usize, f32)> = (r0 + WIPE_SIDE..r1 - WIPE_SIDE)
        .map(|x| {
            let (p, q) = reach(x);
            (x, sig[p].color_distance(&sig[q]))
        })
        .filter(|&(_, d)| d >= MIN_CUT_HIST)
        .collect();
    centres.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut tried: Vec<usize> = Vec::new();
    for (mid, _) in centres {
        if tried.iter().any(|&t| t.abs_diff(mid) < WIPE_REACH) {
            continue;
        }
        tried.push(mid);
        let (ia, ib) = reach(mid);
        match palette_walk(sig, ia, ib) {
            Some((a, e, true)) => return Some((a, e)),
            // A walk too close to the window's edge: look again around it.
            Some((a, e, false)) if reach((a + e) / 2) != (ia, ib) => {
                let (ia, ib) = reach((a + e) / 2);
                if let Some((a, e, true)) = palette_walk(sig, ia, ib) {
                    return Some((a, e));
                }
            }
            _ => {}
        }
    }
    None
}

/// Looks for a walk between the palettes at both ends of the frames
/// `ia..=ib`; returns its blended frames and whether it is a clean single
/// steady walk with stable sides.
fn palette_walk(sig: &[FrameSignals], ia: usize, ib: usize) -> Option<(usize, usize, bool)> {
    // End palettes from three frames each, against noise.
    let mean = |r: std::ops::RangeInclusive<usize>| {
        let k = (r.end() - r.start() + 1) as f32;
        let mut m = [0f32; COLORS];
        for x in r {
            m.iter_mut().zip(&sig[x].colors).for_each(|(m, &c)| *m += c as f32 / k);
        }
        m
    };
    let a_pal = mean(ia..=ia + 2);
    let b_pal = mean(ib - 2..=ib);
    let dir: Vec<f32> = b_pal.iter().zip(&a_pal).map(|(b, a)| b - a).collect();
    let norm: f32 = dir.iter().map(|d| d * d).sum();
    if norm == 0.0 {
        return None;
    }
    // Each frame's place on the way from A (0) to B (1). Mixing colour
    // signatures is linear, so a blend sits exactly at its share of B.
    let pos: Vec<f32> = (ia..=ib)
        .map(|x| sig[x].colors.iter().zip(&a_pal).zip(&dir).map(|((&c, a), d)| (c as f32 - a) * d).sum::<f32>() / norm)
        .collect();
    let ke = pos.iter().position(|&p| p >= 1.0 - WIPE_EDGE)?;
    let ka = pos[..ke].iter().rposition(|&p| p <= WIPE_EDGE)? + 1;
    let (a, e) = (ia + ka, ia + ke);
    let steady = pos[ka..=ke].windows(2).all(|w| w[1] - w[0] <= WIPE_JUMP && w[1] - w[0] >= -WIPE_BACK);
    let sides_hold = pos[..ka].iter().all(|&p| p <= WIPE_DRIFT) && pos[ke..].iter().all(|&p| p >= 1.0 - WIPE_DRIFT);
    let ok = (MIN_WIPE..=MAX_WIPE).contains(&(e - a))
        && ka >= WIPE_SIDE
        && pos.len() - ke >= WIPE_SIDE
        && steady
        && sides_hold
        && pos[ka..ke].iter().all(|&p| p < 1.0 + WIPE_DRIFT);
    Some((a, e, ok))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::BINS;

    /// A synthetic video: `looks[f]` is a picture id; frames with equal ids are
    /// identical. Id 0 is black, 1000 white, 2000+x a frame of luma x.
    fn signals(looks: &[u32], single: &[(usize, f32)], many: &[(usize, usize)]) -> Vec<FrameSignals> {
        let luma = |id: u32| match id {
            0 => 0.0,
            1000 => 255.0,
            2000.. => (id - 2000) as f32,
            _ => 40.0 + (id * 37 % 150) as f32,
        };
        let mut out: Vec<FrameSignals> = looks
            .iter()
            .enumerate()
            .map(|(f, &id)| {
                let mut diff = [0f32; LAGS];
                for (lag, d) in diff.iter_mut().enumerate() {
                    if let Some(&prev) = f.checked_sub(lag + 1).map(|i| &looks[i]) {
                        *d = if prev == id { 1.0 } else { (luma(prev) - luma(id)).abs().max(80.0) };
                    }
                }
                let hist = match f.checked_sub(1).map(|i| looks[i]) {
                    Some(prev) if prev != id => 0.6,
                    _ => 0.0,
                };
                let l = luma(id);
                // One colour bin per channel, derived from the picture id.
                let mut colors = [0u8; COLORS];
                let bins = match id {
                    0 => [0; 3],
                    1000 => [BINS - 1; 3],
                    2000.. => [l as usize >> 4; 3],
                    _ => [id as usize * 5 % BINS, id as usize * 7 % BINS, id as usize * 11 % BINS],
                };
                for (ch, b) in bins.into_iter().enumerate() {
                    colors[ch * BINS + b] = 255;
                }
                FrameSignals {
                    single: 0.01,
                    many: 0.01,
                    mu: l,
                    sd: if id == 0 || id == 1000 { 0.5 } else { 30.0 },
                    diff,
                    hist,
                    dark: if l < 10.0 { 1.0 } else { 0.0 },
                    light: if l > 245.0 { 1.0 } else { 0.0 },
                    colors,
                }
            })
            .collect();
        for &(f, s) in single {
            out[f].single = s;
        }
        for &(a, b) in many {
            (a..b).for_each(|f| out[f].many = 0.9);
        }
        out
    }

    fn kinds(segs: &[Segment]) -> Vec<(u32, u32, SegmentKind)> {
        segs.iter().map(|s| (s.start, s.end, s.kind)).collect()
    }

    use SegmentKind::*;
    use TransitionKind::*;

    #[test]
    fn hard_cut_lands_on_the_pixel_jump() {
        // Shot A 0..10, B 10..20; TransNet peaks one frame early.
        let looks: Vec<u32> = (0..20).map(|f| if f < 10 { 1 } else { 2 }).collect();
        let s = signals(&looks, &[(9, 0.95)], &[]);
        assert_eq!(kinds(&fuse(&s, &FusionParams::live())), [(0, 10, Shot), (10, 20, Shot)]);
    }

    #[test]
    fn dissolve_frames_belong_to_no_shot() {
        let looks: Vec<u32> = (0..30)
            .map(|f| {
                if f < 10 {
                    1
                } else if f < 18 {
                    100 + f
                } else {
                    2
                }
            })
            .collect();
        let s = signals(&looks, &[], &[(10, 18)]);
        assert_eq!(
            kinds(&fuse(&s, &FusionParams::live())),
            [(0, 10, Shot), (10, 18, Transition(Dissolve)), (18, 30, Shot)]
        );
    }

    #[test]
    fn cut_echo_in_many_head_is_not_a_transition() {
        let looks: Vec<u32> = (0..20).map(|f| if f < 10 { 1 } else { 2 }).collect();
        let s = signals(&looks, &[(10, 0.9)], &[(8, 12)]);
        assert_eq!(kinds(&fuse(&s, &FusionParams::live())), [(0, 10, Shot), (10, 20, Shot)]);
    }

    #[test]
    fn flash_frames_are_not_cuts() {
        // Two white frames inside one shot; TransNet fires on both edges.
        let looks: Vec<u32> = (0..20).map(|f| if f == 8 || f == 9 { 9 } else { 1 }).collect();
        let s = signals(&looks, &[(8, 0.9), (10, 0.7)], &[]);
        assert_eq!(kinds(&fuse(&s, &FusionParams::live())), [(0, 8, Shot), (8, 10, Flash), (10, 20, Shot)]);
    }

    /// Luma steps from `from` to `to` over `frames` frames (exclusive of both ends).
    fn ramp(from: u32, to: u32, frames: u32) -> Vec<u32> {
        (1..=frames)
            .map(|i| 2000 + (from as i64 + (to as i64 - from as i64) * i as i64 / (frames as i64 + 1)) as u32)
            .collect()
    }

    #[test]
    fn fade_through_black_is_found_from_the_luma_alone() {
        // TransNet sees nothing; luma 120 → 0 → 120.
        let mut looks: Vec<u32> = vec![2120; 10];
        looks.extend(ramp(120, 0, 4));
        looks.extend([0, 0, 0]);
        looks.extend(ramp(0, 130, 5));
        looks.extend(vec![2130; 10]);
        let s = signals(&looks, &[], &[]);
        assert_eq!(
            kinds(&fuse(&s, &FusionParams::live())),
            [
                (0, 10, Shot),
                (10, 14, Transition(FadeOut)),
                (14, 17, Black),
                (17, 22, Transition(FadeIn)),
                (22, 32, Shot)
            ]
        );
    }

    #[test]
    fn fade_through_white_too() {
        let mut looks: Vec<u32> = vec![2100; 8];
        looks.extend(ramp(100, 255, 3));
        looks.extend([1000, 1000]);
        looks.extend(ramp(255, 90, 6));
        looks.extend(vec![2090; 8]);
        let s = signals(&looks, &[(9, 0.4)], &[(9, 11)]);
        assert_eq!(
            kinds(&fuse(&s, &FusionParams::live())),
            [(0, 8, Shot), (8, 11, Transition(FadeOut)), (11, 13, Black), (13, 19, Transition(FadeIn)), (19, 27, Shot)]
        );
    }

    #[test]
    fn one_in_between_frame_is_a_fade_but_noise_before_a_cut_to_black_is_not() {
        // 120 → 60 → black: a very fast fade.
        let mut looks: Vec<u32> = vec![2120; 10];
        looks.push(2060);
        looks.extend([0; 4]);
        looks.extend(vec![2120; 10]);
        let s = signals(&looks, &[], &[]);
        assert_eq!(
            kinds(&fuse(&s, &FusionParams::live())),
            [(0, 10, Shot), (10, 11, Transition(FadeOut)), (11, 15, Black), (15, 25, Shot)]
        );
        // A hard cut to black; the frame before it happens to be a bit darker.
        let mut looks: Vec<u32> = vec![2120; 9];
        looks.push(2118);
        looks.extend([0; 4]);
        looks.extend(vec![2120; 10]);
        let s = signals(&looks, &[(10, 0.9), (14, 0.9)], &[]);
        assert_eq!(kinds(&fuse(&s, &FusionParams::live())), [(0, 10, Shot), (10, 14, Black), (14, 24, Shot)]);
    }

    #[test]
    fn a_wipe_is_a_transition_but_a_pan_is_not() {
        // A 0..10, a wipe uncovering B over frames 10..20, B from 20 on.
        let looks: Vec<u32> = (0..30)
            .map(|f| {
                if f < 10 {
                    1
                } else if f < 20 {
                    100 + f
                } else {
                    2
                }
            })
            .collect();
        let mut s = signals(&looks, &[], &[]);
        let (a, b) = (s[0].colors, s[29].colors);
        for f in 10..=20 {
            let p = (f - 9) as f32 / 11.0;
            for i in 0..COLORS {
                s[f].colors[i] = (a[i] as f32 * (1.0 - p) + b[i] as f32 * p).round() as u8;
            }
            s[f].diff[0] = 12.0;
            s[f].hist = 0.03;
        }
        assert_eq!(
            kinds(&fuse(&s, &FusionParams::live())),
            [(0, 10, Shot), (10, 20, Transition(Dissolve)), (20, 30, Shot)]
        );
        // The same burst of motion over the same colours: a pan, one shot.
        for x in &mut s {
            x.colors = a;
        }
        assert_eq!(kinds(&fuse(&s, &FusionParams::live())), [(0, 30, Shot)]);
    }

    #[test]
    fn black_with_a_logo_is_still_black() {
        let looks: Vec<u32> = (0..30)
            .map(|f| {
                if f < 10 {
                    1
                } else if f < 14 {
                    0
                } else {
                    2
                }
            })
            .collect();
        let mut s = signals(&looks, &[(10, 0.9), (14, 0.9)], &[]);
        for x in &mut s[10..14] {
            // A bright logo: spread far above 4, but 95 % of pixels black.
            x.sd = 12.0;
            x.mu = 6.0;
            x.dark = 0.95;
        }
        assert_eq!(kinds(&fuse(&s, &FusionParams::live())), [(0, 10, Shot), (10, 14, Black), (14, 30, Shot)]);
    }

    #[test]
    fn tiny_shots_stay_between_clear_cuts_or_merge() {
        // A, a 1-frame shot X with two strong cuts, then B.
        let looks: Vec<u32> = (0..20)
            .map(|f| {
                if f < 10 {
                    1
                } else if f == 10 {
                    7
                } else {
                    2
                }
            })
            .collect();
        let s = signals(&looks, &[(10, 0.95), (11, 0.9)], &[]);
        let segs = fuse(&s, &FusionParams::live());
        assert_eq!(kinds(&segs), [(0, 10, Shot), (10, 11, Shot), (11, 20, Shot)]);
        assert!(segs[1].tiny);
        // Same, but the cut into X is weak (low TransNet, small jump): X joins A.
        let mut s = signals(&looks, &[(10, 0.55), (11, 0.9)], &[]);
        s[10].diff[0] = 12.0;
        let segs = fuse(&s, &FusionParams::live());
        assert_eq!(kinds(&segs), [(0, 11, Shot), (11, 20, Shot)]);
    }

    #[test]
    fn motion_blur_after_a_cut_is_no_tiny_shot() {
        // Cut at 10; frame 11 still moves a lot but keeps the colours.
        let looks: Vec<u32> = (0..20).map(|f| if f < 10 { 1 } else { 2 }).collect();
        let mut s = signals(&looks, &[(10, 0.95), (11, 0.6)], &[]);
        s[11].diff[0] = 60.0;
        s[11].hist = 0.05;
        assert_eq!(kinds(&fuse(&s, &FusionParams::live())), [(0, 10, Shot), (10, 20, Shot)]);
    }

    #[test]
    fn black_gap_is_its_own_segment() {
        let looks: Vec<u32> = (0..30)
            .map(|f| {
                if f < 10 {
                    1
                } else if f < 14 {
                    0
                } else {
                    2
                }
            })
            .collect();
        let s = signals(&looks, &[(10, 0.9), (14, 0.9)], &[]);
        assert_eq!(kinds(&fuse(&s, &FusionParams::live())), [(0, 10, Shot), (10, 14, Black), (14, 30, Shot)]);
    }
}
