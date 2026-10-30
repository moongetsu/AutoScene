//! The anime bench's numbers beside BCubed (`bench/anime/README.md`): the
//! main cast's coverage over time, over-segmentation, scenepack clips, and
//! bootstrap intervals by figure.

#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use as_analysis::tracks::FaceTrack;

use crate::labels::{Label, box_weights, frame_pairs, named};

/// Per name of the labels `counted` takes: `[labelled, held, theirs]`, in
/// frames — the frames its labels have a box in (each standing for the
/// frames up to the label's next box, [`box_weights`]), those a track holds
/// (one to one per frame, [`frame_pairs`]), and those a track of a
/// character named like them holds (`char_name`). Frames `frame_ok` leaves
/// out don't count, not even as frames a box before them stands for.
pub fn coverage<'a>(
    tracks: &[FaceTrack],
    labels: &'a [Label],
    got: &[Option<usize>],
    char_name: &HashMap<usize, &str>,
    counted: impl Fn(&Label) -> bool,
    frame_ok: impl Fn(u32) -> bool,
) -> HashMap<&'a str, [f64; 3]> {
    let mut out: HashMap<&str, [f64; 3]> = HashMap::new();
    // Per label, the frames each of its boxes stands for.
    let mut weight: HashMap<(usize, u32), f64> = HashMap::new();
    for (l, label) in labels.iter().enumerate().filter(|(_, l)| counted(l)) {
        let e = out.entry(label.name.as_str()).or_default();
        for (f, w) in box_weights(label, &frame_ok) {
            e[0] += f64::from(w);
            weight.insert((l, f), f64::from(w));
        }
    }
    frame_pairs(
        tracks,
        labels,
        |_, _| true,
        |f, l, i| {
            let Some(&w) = weight.get(&(l, f)) else { return };
            let name = labels[l].name.as_str();
            let e = out.get_mut(name).unwrap();
            e[1] += w;
            if got[i].and_then(|c| char_name.get(&c)).is_some_and(|&n| n == name) {
                e[2] += w;
            }
        },
    );
    out
}

/// Per name: the characters needed to hold `share` of its frames in
/// characters (the biggest first), over the tracks `counted` takes.
/// Unassigned tracks are left out: coverage counts them, a merge can't
/// clean them. So a name mostly unassigned may need one character alone;
/// the bench prints its unassigned share beside it.
pub fn groups_for<'a>(
    tracks: &[FaceTrack],
    truth: &[Option<&'a str>],
    got: &[Option<usize>],
    counted: impl Fn(usize) -> bool,
    share: f64,
) -> HashMap<&'a str, usize> {
    let mut by: HashMap<&str, HashMap<usize, u32>> = HashMap::new();
    for (i, t) in tracks.iter().enumerate().filter(|&(i, _)| counted(i)) {
        if let (Some(n), Some(c)) = (truth[i].and_then(named), got[i]) {
            *by.entry(n).or_default().entry(c).or_default() += t.end - t.start;
        }
    }
    by.into_iter()
        .map(|(n, groups)| {
            let mut f: Vec<u32> = groups.into_values().collect();
            f.sort_unstable_by(|a, b| b.cmp(a));
            let total: f64 = f.iter().map(|&x| f64::from(x)).sum();
            let (mut sum, mut k) = (0.0, 0);
            while k < f.len() && sum < share * total - 1e-9 {
                sum += f64::from(f[k]);
                k += 1;
            }
            (n, k)
        })
        .collect()
}

// What makes a good clip, as `as_project::pack` ranks shots for "best first".
const W_QUALITY: f32 = 0.35;
const W_SIZE: f32 = 0.30;
const W_COVERAGE: f32 = 0.20;
const W_LENGTH: f32 = 0.10;
const W_SOLO: f32 = 0.05;
const FULL_SIZE: f32 = 0.45;
const GOOD_LENGTH: (f64, f64) = (1.0, 6.0);

fn length_fit(seconds: f64) -> f32 {
    let (lo, hi) = GOOD_LENGTH;
    (if seconds < lo {
        seconds / lo
    } else if seconds > hi {
        hi / seconds
    } else {
        1.0
    }) as f32
}

/// Per character, its pack: the shots (indexes into `shots`, in order) a
/// track assigned to it overlaps, in the order of the app's "best first"
/// (`as_project::pack::make` with `PackOrder::Best`). Shots `shot_ok`
/// leaves out are in no pack.
///
/// Mirrors `shots_of` and `better` of `crates/as-project/src/pack.rs`,
/// simplified where an example can't follow the app: no bodies (no
/// faceless shots), no hidden characters, no filters, limit or picks by
/// hand; a track is in every shot it overlaps (the app stores the one shot
/// it belongs to); a shot's seconds are its frames at `fps` (the app reads
/// the frame grid's timestamps).
pub fn packs(
    tracks: &[FaceTrack],
    got: &[Option<usize>],
    shots: &[(u32, u32)],
    shot_ok: impl Fn(usize) -> bool,
    fps: f64,
) -> HashMap<usize, Vec<usize>> {
    let in_shot = in_shots(tracks, got, shots);
    struct Cand {
        shot: usize,
        score: f32,
        on_screen: i64,
        face_h: f32,
    }
    let mut by: HashMap<usize, Vec<Cand>> = HashMap::new();
    for (s, ids) in in_shot.iter().enumerate().filter(|&(s, ids)| shot_ok(s) && !ids.is_empty()) {
        let (start, end) = shots[s];
        let mut chars: Vec<usize> = ids.iter().map(|&i| got[i].unwrap()).collect();
        chars.sort_unstable();
        chars.dedup();
        for &c in &chars {
            let mut rows: Vec<usize> = ids.iter().copied().filter(|&i| got[i] == Some(c)).collect();
            // The app's row order: big enough faces first, then the clearest.
            rows.sort_by(|&a, &b| {
                let (x, y) = (&tracks[a], &tracks[b]);
                (y.face_height() >= 0.12)
                    .cmp(&(x.face_height() >= 0.12))
                    .then(y.quality.total_cmp(&x.quality))
                    .then(a.cmp(&b))
            });
            let quality = tracks[rows[0]].quality;
            let face_h = rows.iter().map(|&i| tracks[i].face_height()).fold(0.0, f32::max);
            let mut spans: Vec<(u32, u32)> = rows.iter().map(|&i| (tracks[i].start, tracks[i].end)).collect();
            spans.sort_unstable();
            let (mut covered, mut reach) = (0u32, start);
            for (a, b) in spans {
                let (a, b) = (a.max(reach).max(start), b.min(end));
                if b > a {
                    covered += b - a;
                    reach = b;
                }
            }
            let coverage = if end > start { covered as f32 / (end - start) as f32 } else { 0.0 };
            let seconds = f64::from(end - start) / fps;
            let score = W_QUALITY * quality.clamp(0.0, 1.0)
                + W_SIZE * (face_h / FULL_SIZE).min(1.0)
                + W_COVERAGE * coverage.min(1.0)
                + W_LENGTH * length_fit(seconds)
                + W_SOLO * if chars.len() == 1 { 1.0 } else { 0.0 };
            let on_screen = (f64::from(covered.min(end - start)) * 10.0 / fps).round() as i64;
            by.entry(c).or_default().push(Cand { shot: s, score, on_screen, face_h });
        }
    }
    // The higher score; of equals the longer on screen (to a tenth of a
    // second), the bigger face, the earlier shot.
    let better = |a: &Cand, b: &Cand| -> Ordering {
        b.score
            .total_cmp(&a.score)
            .then(b.on_screen.cmp(&a.on_screen))
            .then(b.face_h.total_cmp(&a.face_h))
            .then(a.shot.cmp(&b.shot))
    };
    by.into_iter()
        .map(|(c, mut v)| {
            v.sort_by(better);
            (c, v.into_iter().map(|x| x.shot).collect())
        })
        .collect()
}

/// Per shot (in order, `shots` sorted), the tracks in a character that
/// overlap it.
pub fn in_shots(tracks: &[FaceTrack], got: &[Option<usize>], shots: &[(u32, u32)]) -> Vec<Vec<usize>> {
    let mut in_shot: Vec<Vec<usize>> = vec![Vec::new(); shots.len()];
    for (i, t) in tracks.iter().enumerate().filter(|&(i, _)| got[i].is_some()) {
        let first = shots.partition_point(|s| s.1 <= t.start);
        for (s, shot) in shots.iter().enumerate().skip(first) {
            if shot.0 >= t.end {
                break;
            }
            in_shot[s].push(i);
        }
    }
    in_shot
}

/// Per name, the shots it is truly in: from shot presence (the presence
/// row holding a shot's middle frame; `rows` are `start end names`).
pub fn true_shots_presence(rows: &[(u32, u32, Vec<String>)], shots: &[(u32, u32)]) -> HashMap<String, HashSet<usize>> {
    let mut out: HashMap<String, HashSet<usize>> = HashMap::new();
    for (s, &(a, b)) in shots.iter().enumerate() {
        let mid = a + (b - a) / 2;
        for (_, _, names) in rows.iter().filter(|r| (r.0..r.1).contains(&mid)) {
            for n in names.iter().filter(|n| named(n).is_some()) {
                out.entry(n.clone()).or_default().insert(s);
            }
        }
    }
    out
}

/// Per name, the shots it is truly in: those a label of it overlaps.
pub fn true_shots_labels(labels: &[Label], shots: &[(u32, u32)]) -> HashMap<String, HashSet<usize>> {
    let mut out: HashMap<String, HashSet<usize>> = HashMap::new();
    for l in labels.iter().filter(|l| named(&l.name).is_some()) {
        let first = shots.partition_point(|s| s.1 <= l.span.0);
        for (s, shot) in shots.iter().enumerate().skip(first) {
            if shot.0 >= l.span.1 {
                break;
            }
            out.entry(l.name.clone()).or_default().insert(s);
        }
    }
    out
}

/// A name's pack against the shots it is truly in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PackScore {
    /// Shots in the pack, the ones of them right, shots the name is in.
    pub shots: usize,
    pub right: usize,
    pub truth: usize,
    /// Right among the first 10 and 25, and how many there are: a pack of
    /// fewer shots is judged on all of them (3 shots, 3 right: P@10 1.0).
    pub at10: (usize, usize),
    pub at25: (usize, usize),
}

impl PackScore {
    pub fn of(pack: &[usize], truth: &HashSet<usize>) -> Self {
        let right = |k: usize| pack.iter().take(k).filter(|s| truth.contains(s)).count();
        Self {
            shots: pack.len(),
            right: right(pack.len()),
            truth: truth.len(),
            at10: (right(10), pack.len().min(10)),
            at25: (right(25), pack.len().min(25)),
        }
    }
}

/// `a / b`, NaN when `b` is 0.
pub fn ratio(a: f64, b: f64) -> f64 {
    if b > 0.0 { a / b } else { f64::NAN }
}

/// Per item, the BCubed precision and recall it adds before averaging
/// (as `as_identity::metrics::bcubed`, which averages them, weighted, over
/// the counted items).
pub fn bcubed_terms(assigned: &[Option<usize>], truth: &[usize], weight: &[f32]) -> Vec<(f64, f64)> {
    let mut by_char: HashMap<usize, f64> = HashMap::new();
    let mut by_person: HashMap<usize, f64> = HashMap::new();
    let mut both: HashMap<(usize, usize), f64> = HashMap::new();
    for i in 0..truth.len() {
        let w = f64::from(weight[i]);
        *by_person.entry(truth[i]).or_default() += w;
        if let Some(c) = assigned[i] {
            *by_char.entry(c).or_default() += w;
            *both.entry((c, truth[i])).or_default() += w;
        }
    }
    (0..truth.len())
        .map(|i| {
            let w = f64::from(weight[i]);
            let (shared, size) = match assigned[i] {
                Some(c) => (both[&(c, truth[i])], by_char[&c]),
                None => (w, w),
            };
            (shared / size.max(1e-12), shared / by_person[&truth[i]].max(1e-12))
        })
        .collect()
}

/// What one figure adds to the bench's numbers: weighted BCubed precision
/// and recall sums and their weight, per track and per screen time, and
/// its pack's right shots and shots.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Item {
    pub track: [f64; 3],
    pub time: [f64; 3],
    pub clip: [f64; 2],
}

impl Item {
    pub fn add(&mut self, other: &Item) {
        (0..3).for_each(|k| {
            self.track[k] += other.track[k];
            self.time[k] += other.time[k];
        });
        (0..2).for_each(|k| self.clip[k] += other.clip[k]);
    }
}

pub const METRICS: [&str; 3] = ["per track F", "screen time F", "clip precision"];

fn f_of(s: [f64; 3]) -> f64 {
    let (p, r) = (ratio(s[0], s[2]), ratio(s[1], s[2]));
    if p.is_nan() || r.is_nan() {
        f64::NAN
    } else if p + r > 0.0 {
        2.0 * p * r / (p + r)
    } else {
        0.0
    }
}

/// Per track F, screen time F and clip precision of these items together.
pub fn metrics<'a>(items: impl IntoIterator<Item = &'a Item>) -> [f64; 3] {
    let mut t = Item::default();
    items.into_iter().for_each(|i| t.add(i));
    [f_of(t.track), f_of(t.time), ratio(t.clip[0], t.clip[1])]
}

/// SplitMix64: deterministic draws for the bootstrap.
struct Draw(u64);

impl Draw {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) % n as u64) as usize
    }
}

/// The 2.5 % and 97.5 % points of `v` (NaNs left out).
fn interval(mut v: Vec<f64>) -> (f64, f64) {
    v.retain(|x| !x.is_nan());
    if v.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((q * (v.len() - 1) as f64).round() as usize).min(v.len() - 1)];
    (at(0.025), at(0.975))
}

/// 95 % intervals of [`metrics`] by resampling the items (figures) with
/// replacement, `n` times from `seed`.
pub fn bootstrap(items: &[Item], n: usize, seed: u64) -> [(f64, f64); 3] {
    let mut draw = Draw(seed);
    let mut runs: [Vec<f64>; 3] = Default::default();
    for _ in 0..n {
        let pick: Vec<&Item> = (0..items.len()).map(|_| &items[draw.below(items.len())]).collect();
        let m = metrics(pick);
        (0..3).for_each(|k| runs[k].push(m[k]));
    }
    runs.map(interval)
}

/// The difference `b − a` of [`metrics`] of two runs over the same figures
/// (`a[k]` and `b[k]` one figure), and its 95 % paired-bootstrap interval:
/// each resample draws figures once for both runs.
pub fn paired(a: &[Item], b: &[Item], n: usize, seed: u64) -> [(f64, f64, f64); 3] {
    assert_eq!(a.len(), b.len(), "the same figures in both runs");
    let (ma, mb) = (metrics(a), metrics(b));
    let mut draw = Draw(seed);
    let mut runs: [Vec<f64>; 3] = Default::default();
    for _ in 0..n {
        let pick: Vec<usize> = (0..a.len()).map(|_| draw.below(a.len())).collect();
        let (x, y) = (metrics(pick.iter().map(|&k| &a[k])), metrics(pick.iter().map(|&k| &b[k])));
        (0..3).for_each(|k| runs[k].push(y[k] - x[k]));
    }
    let iv = runs.map(interval);
    std::array::from_fn(|k| (mb[k] - ma[k], iv[k].0, iv[k].1))
}

const ITEM_HEADER: &str = "figure\ttrack_p\ttrack_r\ttrack_w\ttime_p\ttime_r\ttime_w\tclip_right\tclip_shots";

/// `ITEMS=<file>`: one row per figure, what it adds ([`Item`]).
pub fn write_items(path: &str, items: &[(String, Item)]) {
    let mut tsv = format!("{ITEM_HEADER}\n");
    for (name, i) in items {
        let v: Vec<String> = i.track.iter().chain(&i.time).chain(&i.clip).map(|x| format!("{x}")).collect();
        tsv += &format!("{name}\t{}\n", v.join("\t"));
    }
    std::fs::write(path, tsv).unwrap_or_else(|e| panic!("{path}: {e}"));
}

pub fn read_items(path: &str) -> Vec<(String, Item)> {
    crate::labels::tsv_rows(path)
        .into_iter()
        .map(|v| {
            let x: Vec<f64> = v[1..9].iter().map(|s| s.parse().unwrap()).collect();
            (v[0].clone(), Item { track: [x[0], x[1], x[2]], time: [x[3], x[4], x[5]], clip: [x[6], x[7]] })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labels::tests::track;

    /// Shots of 100 frames (4.2 s at 24 fps: a good length) from frame 0.
    fn shots(n: u32) -> Vec<(u32, u32)> {
        (0..n).map(|k| (100 * k, 100 * (k + 1))).collect()
    }

    /// A box `h` of the frame high.
    fn high(h: f32) -> [f32; 4] {
        [0.10, 0.10, 0.30, 0.10 + h]
    }

    #[test]
    fn packs_rank_by_quality_size_and_coverage() {
        let mut t = vec![
            track(0, 100, high(0.2)),
            track(100, 200, high(0.2)),
            track(200, 300, high(0.45)),
            // Half the shot.
            track(300, 350, high(0.2)),
        ];
        for (t, q) in t.iter_mut().zip([0.5, 0.9, 0.5, 0.9]) {
            t.quality = q;
        }
        // Scores: shot 2 the biggest face (0.675), 1 the clearest (0.648),
        // 3 as clear but half covered (0.548), 0 (0.508); the rest alike.
        let p = packs(&t, &[Some(0); 4], &shots(4), |_| true, 24.0);
        assert_eq!(p[&0], vec![2, 1, 3, 0]);
        // A shot left out is in no pack; a track in no character in none.
        let p = packs(&t, &[Some(0), Some(0), None, Some(0)], &shots(4), |s| s != 1, 24.0);
        assert_eq!(p[&0], vec![3, 0]);
    }

    #[test]
    fn precision_at_10_and_25_in_pack_order() {
        // 30 shots, each clearer than the one before: the pack runs from
        // the last shot back to the first. Every third shot is wrong.
        let mut t: Vec<FaceTrack> = (0..30).map(|k| track(100 * k, 100 * (k + 1), high(0.2))).collect();
        for (k, t) in t.iter_mut().enumerate() {
            t.quality = 0.1 + 0.02 * k as f32;
        }
        let pack = &packs(&t, &[Some(0); 30], &shots(30), |_| true, 24.0)[&0];
        assert_eq!(*pack, (0..30).rev().collect::<Vec<usize>>());
        let truth: HashSet<usize> = (0..30).filter(|s| s % 3 != 0).collect();
        let s = PackScore::of(pack, &truth);
        // First 10: 29..20 hold 27, 24, 21 wrong; first 25: 29..5 hold 8.
        assert_eq!((s.shots, s.right, s.truth, s.at10, s.at25), (30, 20, 20, (7, 10), (17, 25)));
        // In time order the first 10 would hold 0, 3, 6, 9 wrong.
        let by_time: Vec<usize> = (0..30).collect();
        assert_eq!(PackScore::of(&by_time, &truth).at10, (6, 10));
        // A pack shorter than 10 is judged on its shots.
        let short = PackScore::of(&[1, 2, 3], &truth);
        assert_eq!((short.at10, short.at25), ((2, 3), (2, 3)));
    }

    #[test]
    fn true_shots_from_presence_or_labels() {
        let rows = vec![
            (0, 150, vec!["TAN".to_string(), "x".into()]),
            (150, 300, vec!["NEZ".into(), "?e".into()]),
            (300, 400, vec![]),
        ];
        // A shot's middle frame picks the row: shot 1 (middle 150) is NEZ's.
        let p = true_shots_presence(&rows, &shots(4));
        assert_eq!(p.len(), 2, "only known characters: {p:?}");
        assert_eq!(p["TAN"], HashSet::from([0]));
        assert_eq!(p["NEZ"], HashSet::from([1, 2]));
        let t = track(120, 260, high(0.2));
        let labels =
            [crate::labels::tests::label_of(&t, "TAN", high(0.2)), crate::labels::tests::label_of(&t, "?u", high(0.2))];
        let l = true_shots_labels(&labels, &shots(4));
        assert_eq!(l.len(), 1);
        assert_eq!(l["TAN"], HashSet::from([1, 2]));
    }
}
