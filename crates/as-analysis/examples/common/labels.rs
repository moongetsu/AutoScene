//! The identity bench's labels (`bench/identity`, `bench/anime`) and what
//! the bench examples share: tracks in, clustering input, which label each
//! track is.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use as_analysis::tracks::{self, FaceDet, FaceTrack};
use as_identity::TrackIn;

pub const STRIDE: u32 = 3;
pub const MAX_GAP: u32 = 8;
pub const FPS: f64 = 24000.0 / 1001.0;

/// Frames per second of the labelled media: `FPS=<num>/<den>` (or a plain
/// number) first, then `INFO=<bb.info.json>` (the media probe), [`FPS`] by
/// default.
pub fn fps() -> f64 {
    static F: OnceLock<f64> = OnceLock::new();
    *F.get_or_init(|| {
        if let Ok(v) = std::env::var("FPS") {
            return parse_fps(&v);
        }
        if let Some(info) = std::env::var_os("INFO") {
            let info: as_media::MediaInfo =
                serde_json::from_str(&std::fs::read_to_string(&info).unwrap_or_else(|e| panic!("INFO: {e}")))
                    .expect("INFO: media info");
            return info.video.avg_frame_rate.unwrap_or(info.video.r_frame_rate).to_f64();
        }
        FPS
    })
}

/// `<num>/<den>` or a plain number, as frames per second (above 0).
pub fn parse_fps(v: &str) -> f64 {
    let num = |s: &str| s.trim().parse::<f64>().unwrap_or_else(|_| panic!("FPS={v}: <num>/<den>"));
    let fps = match v.split_once('/') {
        Some((n, d)) => {
            let d = num(d);
            assert!(d > 0.0, "FPS={v}: the denominator must be above 0");
            num(n) / d
        }
        None => num(v),
    };
    assert!(fps.is_finite() && fps > 0.0, "FPS={v}: frames per second must be above 0");
    fps
}

/// How a label's name is scored (`bench/anime/FORMATS.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind<'a> {
    /// A known character: a gallery code (a name in older labels).
    Named(&'a str),
    /// Someone of their own: `?e` (a character outside the gallery; `?` in
    /// older labels) or `n` (not a character's head).
    OwnPerson,
    /// Left out of the score: `?u` (unsure between codes) or `x` (a head
    /// nobody can tell).
    Excluded,
}

pub fn kind(name: &str) -> Kind<'_> {
    match name {
        "?e" | "?" | "n" => Kind::OwnPerson,
        "?u" | "x" => Kind::Excluded,
        code => Kind::Named(code),
    }
}

/// The name, when it is a known character's.
pub fn named(name: &str) -> Option<&str> {
    match kind(name) {
        Kind::Named(n) => Some(n),
        _ => None,
    }
}

pub fn input(tracks: &[FaceTrack]) -> Vec<TrackIn> {
    let mut v = input_raw(tracks);
    // Experiment: `CL_CAP=<cos>` drops the same-frame cannot-link of two
    // tracks at least this alike (the frames they share leave the smaller).
    if let Some(cap) = std::env::var("CL_CAP").ok().and_then(|s| s.parse::<f32>().ok()) {
        let mut at: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, t) in v.iter().enumerate() {
            for &f in &t.seen {
                at.entry(f).or_default().push(i);
            }
        }
        let mut drop: HashMap<usize, Vec<u32>> = HashMap::new();
        let mut pairs = std::collections::HashSet::new();
        for (&f, ids) in &at {
            for (k, &a) in ids.iter().enumerate() {
                for &b in &ids[k + 1..] {
                    let s: f32 = v[a].embedding.iter().zip(&v[b].embedding).map(|(x, y)| x * y).sum();
                    if s >= cap {
                        let small = if v[a].dets <= v[b].dets { a } else { b };
                        drop.entry(small).or_default().push(f);
                        pairs.insert((a.min(b), a.max(b)));
                    }
                }
            }
        }
        for (i, fs) in drop {
            // Never all of them: no frames means "unknown", and the spans
            // would stand in (cannot-links with everyone on screen).
            if v[i].seen.iter().any(|f| !fs.contains(f)) {
                v[i].seen.retain(|f| !fs.contains(f));
            }
        }
        eprintln!("CL_CAP {cap}: dropped {} same-frame pairs", pairs.len());
    }
    v
}

/// The frames between samples and the samples a face may be missed, of
/// the face pass the tracks come from: `GRID=<stride>,<gap>` (Fast Scan at
/// 24 fps: `GRID=6,8`), the full pass's by default.
pub fn grid() -> (u32, u32) {
    std::env::var("GRID").ok().map_or((STRIDE, MAX_GAP), |v| {
        let (s, g) = v.split_once(',').expect("GRID=<stride>,<gap>");
        (s.parse().unwrap(), g.parse().unwrap())
    })
}

fn input_raw(tracks: &[FaceTrack]) -> Vec<TrackIn> {
    let (stride, max_gap) = grid();
    tracks
        .iter()
        .map(|t| TrackIn {
            media: 0,
            start: t.start,
            end: t.end,
            spans: tracks::presence_spans(&t.dets, t.end, stride, max_gap),
            seen: t.dets.iter().map(|d| d.frame).collect(),
            embedding: t.embedding.clone(),
            quality: t.quality,
            dets: t.dets.len() as u32,
            weight: 1.0,
            pin: None,
            forbid: vec![],
            previous: None,
        })
        .collect()
}

fn area(a: &[f32; 4]) -> f32 {
    (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0)
}

fn intersection(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    iw * ih
}

fn centre(a: &[f32; 4]) -> (f32, f32) {
    ((a[0] + a[2]) / 2.0, (a[1] + a[3]) / 2.0)
}

pub fn box_iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let inter = intersection(a, b);
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// Whether two boxes are one head: IoU at least 0.5, or the smaller box's
/// centre inside the larger with at least 0.7 of the smaller inside it (a
/// face box inside a head box: labels and detectors box heads differently).
pub fn same_head(a: &[f32; 4], b: &[f32; 4]) -> bool {
    if box_iou(a, b) >= 0.5 {
        return true;
    }
    let (small, big) = if area(a) <= area(b) { (a, b) } else { (b, a) };
    let (x, y) = centre(small);
    let inside = (big[0]..=big[2]).contains(&x) && (big[1]..=big[3]).contains(&y);
    inside && intersection(small, big) >= 0.7 * area(small).max(1e-12)
}

/// Minimum-cost assignment of rows to columns (`cost[r][c]`, no more rows
/// than columns): the column of each row. Kuhn–Munkres, O(n²m).
fn hungarian(cost: &[Vec<f64>]) -> Vec<usize> {
    let n = cost.len();
    let m = cost.first().map_or(0, Vec::len);
    assert!(n <= m, "no more rows than columns");
    let (mut u, mut v) = (vec![0f64; n + 1], vec![0f64; m + 1]);
    let (mut p, mut way) = (vec![0usize; m + 1], vec![0usize; m + 1]);
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0;
        let mut minv = vec![f64::INFINITY; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            used[j0] = true;
            let (i0, mut delta, mut j1) = (p[j0], f64::INFINITY, 0);
            for j in 1..=m {
                if !used[j] {
                    let cur = cost[i0 - 1][j - 1] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=m {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut col = vec![0; n];
    for j in 1..=m {
        if p[j] > 0 {
            col[p[j] - 1] = j - 1;
        }
    }
    col
}

/// The boxes of one frame paired one to one: `(label, found)` indexes. Only
/// boxes that are one head ([`same_head`]) and that `allowed` lets pair;
/// of those as many pairs as can be, then the ones with the most overlap
/// (IoU), then the nearest centres — intersection over the smaller box only
/// opens the gate, so a small head inside a big one in front of it (over
/// the shoulder) pairs with its own box, not the big one.
pub fn pair_boxes(
    labels: &[[f32; 4]],
    found: &[[f32; 4]],
    allowed: impl Fn(usize, usize) -> bool,
) -> Vec<(usize, usize)> {
    let (n, m) = (labels.len(), found.len());
    let ok = |a: usize, b: usize| allowed(a, b) && same_head(&labels[a], &found[b]);
    let gate: Vec<Vec<bool>> = (0..n).map(|a| (0..m).map(|b| ok(a, b)).collect()).collect();
    if !gate.iter().flatten().any(|&g| g) {
        return Vec::new();
    }
    // A pair that may not be costs more than any set of pairs that may: as
    // many pairs as can be come first.
    let never = 10.0 * (n.max(m) as f64 + 1.0);
    let cost = |a: usize, b: usize| -> f64 {
        if !gate[a][b] {
            return never;
        }
        let ((ax, ay), (bx, by)) = (centre(&labels[a]), centre(&found[b]));
        1.0 - box_iou(&labels[a], &found[b]) as f64 + 1e-4 * ((ax - bx).hypot(ay - by)) as f64
    };
    let pairs: Vec<(usize, usize)> = if n <= m {
        let c: Vec<Vec<f64>> = (0..n).map(|a| (0..m).map(|b| cost(a, b)).collect()).collect();
        hungarian(&c).into_iter().enumerate().collect()
    } else {
        let c: Vec<Vec<f64>> = (0..m).map(|b| (0..n).map(|a| cost(a, b)).collect()).collect();
        hungarian(&c).into_iter().enumerate().map(|(b, a)| (a, b)).collect()
    };
    pairs.into_iter().filter(|&(a, b)| gate[a][b]).collect()
}

/// The box of `dets` (in frame order) at `frame`: the detection there, or
/// the nearest one.
pub fn box_near(dets: &[FaceDet], frame: u32) -> Option<[f32; 4]> {
    debug_assert!(dets.is_sorted_by_key(|d| d.frame), "box_near needs frame-sorted dets");
    let k = dets.partition_point(|d| d.frame < frame);
    let after = dets.get(k);
    let before = k.checked_sub(1).map(|i| &dets[i]);
    match (before, after) {
        (Some(b), Some(a)) => Some(if a.frame - frame < frame - b.frame { a.bbox } else { b.bbox }),
        (b, a) => b.or(a).map(|d| d.bbox),
    }
}

/// Per frame a label has a box in, the labels' boxes there paired one to
/// one ([`pair_boxes`]) with the boxes of the tracks on screen then (by
/// their presence spans): `each(frame, label, track)` per pair. `allowed`
/// gates a label–track pair.
pub fn frame_pairs(
    tracks: &[FaceTrack],
    labels: &[Label],
    allowed: impl Fn(usize, usize) -> bool,
    mut each: impl FnMut(u32, usize, usize),
) {
    const BIN: u32 = 256;
    let (stride, max_gap) = grid();
    let mut bins: HashMap<u32, Vec<(usize, u32, u32)>> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        for (a, b) in tracks::presence_spans(&t.dets, t.end, stride, max_gap) {
            for k in a / BIN..=b.saturating_sub(1) / BIN {
                bins.entry(k).or_default().push((i, a, b));
            }
        }
    }
    let mut at: BTreeMap<u32, Vec<(usize, [f32; 4])>> = BTreeMap::new();
    for (l, label) in labels.iter().enumerate() {
        for &(f, b) in &label.boxes {
            at.entry(f).or_default().push((l, b));
        }
    }
    for (f, here) in at {
        let mut on: Vec<usize> = bins
            .get(&(f / BIN))
            .into_iter()
            .flatten()
            .filter(|&&(i, a, b)| (a..b).contains(&f) && here.iter().any(|&(l, _)| allowed(l, i)))
            .map(|x| x.0)
            .collect();
        on.sort_unstable();
        on.dedup();
        if on.is_empty() {
            continue;
        }
        let found: Vec<[f32; 4]> = on.iter().map(|&i| box_near(&tracks[i].dets, f).unwrap()).collect();
        let boxes: Vec<[f32; 4]> = here.iter().map(|x| x.1).collect();
        for (a, b) in pair_boxes(&boxes, &found, |a, b| allowed(here[a].0, on[b])) {
            each(f, here[a].0, on[b]);
        }
    }
}

/// Characters as per-track groups (index into the outcome's characters).
pub fn members(o: &as_identity::Outcome) -> Vec<Option<usize>> {
    o.tracks.iter().map(|(a, _)| a.character()).collect()
}

/// The tracks of a `.tracks` file. `EMB_OVERRIDE=<file>` swaps their
/// embeddings for others (any length, made unit length): `u32 n, u32 dim`,
/// then `n × dim` f32, little-endian, in the tracks' order — to try another
/// embedder (or several fused) on the same tracks.
pub fn read_tracks(path: &str) -> Vec<FaceTrack> {
    let mut out = tracks::decode(&std::fs::read(path).expect("tracks file")).expect("readable tracks");
    if let Some(over) = std::env::var_os("EMB_OVERRIDE") {
        let b = std::fs::read(over).expect("embedding override");
        let word = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
        let (n, dim) = (word(0) as usize, word(1) as usize);
        assert_eq!(n, out.len(), "one embedding per track");
        for (k, t) in out.iter_mut().enumerate() {
            let mut e: Vec<f32> = (0..dim).map(|j| f32::from_bits(word(2 + k * dim + j))).collect();
            let len = e.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
            e.iter_mut().for_each(|x| *x /= len);
            t.embedding = e;
        }
    }
    out
}

#[derive(Clone)]
pub struct Label {
    pub span: (u32, u32),
    /// `(frame, box)` of every detection.
    pub boxes: Vec<(u32, [f32; 4])>,
    pub name: String,
    /// `back`, `chibi`, `mask`, … (the optional fifth column).
    pub flags: Vec<String>,
}

/// The rows of a tab-separated file after its header line (`#` comment
/// lines and empty lines skipped).
pub fn tsv_rows(path: &str) -> Vec<Vec<String>> {
    parse_tsv(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}")))
}

/// [`tsv_rows`] of a file's text.
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .skip(1)
        .map(|l| l.split('\t').map(|s| s.trim_end_matches('\r').to_string()).collect())
        .collect()
}

/// `start end name boxes [flags]`.
pub fn read_labels(path: &str) -> Vec<Label> {
    labels_of(tsv_rows(path))
}

/// [`read_labels`] of a file's rows.
pub fn labels_of(rows: Vec<Vec<String>>) -> Vec<Label> {
    rows.into_iter()
        .map(|v| {
            let boxes = v[3]
                .split(' ')
                .filter(|b| !b.is_empty())
                .map(|b| {
                    let (frame, xy) = b.split_once(':').unwrap();
                    let xy: Vec<f32> = xy.split(',').map(|x| x.parse().unwrap()).collect();
                    (frame.parse().unwrap(), [xy[0], xy[1], xy[2], xy[3]])
                })
                .collect();
            let flags = v.get(4).map_or(Vec::new(), |f| {
                f.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
            });
            Label { span: (v[0].parse().unwrap(), v[1].parse().unwrap()), boxes, name: v[2].clone(), flags }
        })
        .collect()
}

/// The frames each box of `label` stands for, as `(frame, frames)`: from
/// its frame up to the label's next box (the last one up to the label's
/// end), only the frames `frame_ok` takes. A box in a frame it leaves out
/// isn't listed; one before a left-out stretch stands for its own frames
/// only, not for the stretch.
pub fn box_weights(label: &Label, frame_ok: impl Fn(u32) -> bool) -> Vec<(u32, u32)> {
    let mut frames: Vec<u32> = label.boxes.iter().map(|b| b.0).collect();
    frames.sort_unstable();
    frames.dedup();
    (0..frames.len())
        .filter(|&k| frame_ok(frames[k]))
        .map(|k| {
            let f = frames[k];
            let until = frames.get(k + 1).copied().unwrap_or(label.span.1.max(f + 1));
            (f, (f..until).filter(|&g| frame_ok(g)).count() as u32)
        })
        .collect()
}

/// The name each track takes from the labels (`None`: no label fits).
pub fn names_of<'a>(tracks: &[FaceTrack], labels: &'a [Label]) -> Vec<Option<&'a str>> {
    // Each track takes the name most of its frames have, among labels it
    // shares most of the shorter of the two with. In each frame a label has
    // a box in, labels and tracks pair one to one, a pair only where both
    // boxes are one head ([`pair_boxes`]); a pair votes with the frames the
    // label's box stands for ([`box_weights`]), up to the track's next pair
    // (two labels taking turns on one track, one with sparse boxes, split
    // its frames by turn). Of names with as many frames, a known
    // character's wins, then someone of their own, then a name left out
    // (`?u`, `x`), then the first by name. Tracks a tracker change split or
    // joined keep their names, moving or not.
    let shares = |l: usize, i: usize| {
        let (l, t) = (&labels[l], &tracks[i]);
        if !(l.span.0 < t.end && t.start < l.span.1) {
            return false;
        }
        let overlap = l.span.1.min(t.end) - l.span.0.max(t.start);
        let shorter = (l.span.1 - l.span.0).min(t.end - t.start).max(1);
        overlap as f32 >= 0.5 * shorter as f32
    };
    let mut weight: HashMap<(usize, u32), u32> = HashMap::new();
    for (l, label) in labels.iter().enumerate() {
        for (f, w) in box_weights(label, |_| true) {
            weight.insert((l, f), w);
        }
    }
    let rank = |n: &str| match kind(n) {
        Kind::Named(_) => 0,
        Kind::OwnPerson => 1,
        Kind::Excluded => 2,
    };
    let mut pairs: Vec<Vec<(u32, usize)>> = vec![Vec::new(); tracks.len()];
    frame_pairs(tracks, labels, shares, |f, l, i| pairs[i].push((f, l)));
    pairs
        .into_iter()
        .map(|mut p| {
            p.sort_unstable();
            let mut votes: HashMap<&str, u32> = HashMap::new();
            for (k, &(f, l)) in p.iter().enumerate() {
                let until = (f + weight[&(l, f)]).min(p.get(k + 1).map_or(u32::MAX, |x| x.0));
                *votes.entry(labels[l].name.as_str()).or_default() += until - f;
            }
            votes
                .into_iter()
                .max_by_key(|&(n, v)| (v, std::cmp::Reverse(rank(n)), std::cmp::Reverse(n)))
                .map(|(n, _)| n)
        })
        .collect()
}

/// The name of each character: the one most of its tracks' frames have
/// (only known characters count).
pub fn character_names<'a>(
    tracks: &[FaceTrack],
    truth_name: &[Option<&'a str>],
    got: &[Option<usize>],
) -> HashMap<usize, &'a str> {
    let mut frames_of: HashMap<(usize, &str), u32> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        if let (Some(c), Some(n)) = (got[i], truth_name[i].and_then(named)) {
            *frames_of.entry((c, n)).or_default() += t.end - t.start;
        }
    }
    let mut char_name: HashMap<usize, (&str, u32)> = HashMap::new();
    for (&(c, n), &f) in &frames_of {
        let e = char_name.entry(c).or_insert((n, 0));
        if (f, std::cmp::Reverse(n)) > (e.1, std::cmp::Reverse(e.0)) {
            *e = (n, f);
        }
    }
    char_name.into_iter().map(|(c, (n, _))| (c, n)).collect()
}

/// Frames on screen per label name (from the labels' spans).
pub fn label_screen(labels: &[Label]) -> HashMap<&str, u32> {
    let mut screen: HashMap<&str, u32> = HashMap::new();
    for l in labels {
        *screen.entry(l.name.as_str()).or_default() += l.span.1 - l.span.0;
    }
    screen
}

/// End to end, over the labels rather than the tracks (BCubed only sees
/// faces that became a track): the share of the main cast's labelled faces
/// a track still holds, and the share held by a track of the character
/// named like them (the name most of a character's frames have).
pub fn found_faces(
    tracks: &[FaceTrack],
    labels: &[Label],
    truth_name: &[Option<&str>],
    got: &[Option<usize>],
) -> (f64, f64) {
    let screen = label_screen(labels);
    let main = |n: &str| named(n).is_some() && screen[n] as f64 / fps() >= 30.0;
    found_faces_of(tracks, labels, truth_name, got, main, |_| true)
}

/// [`found_faces`] of the labels `main` names, in the frames `counted` takes.
/// Labelled boxes and the tracks' boxes pair as [`frame_pairs`] pairs them
/// (the pairs of the coverage over time, counted per box, not per frame).
pub fn found_faces_of(
    tracks: &[FaceTrack],
    labels: &[Label],
    truth_name: &[Option<&str>],
    got: &[Option<usize>],
    main: impl Fn(&str) -> bool,
    counted: impl Fn(u32) -> bool,
) -> (f64, f64) {
    let char_name = character_names(tracks, truth_name, got);
    let is_main: Vec<bool> = labels.iter().map(|l| main(&l.name)).collect();
    let all: u32 = labels
        .iter()
        .zip(&is_main)
        .filter(|&(_, &m)| m)
        .map(|(l, _)| l.boxes.iter().filter(|(f, _)| counted(*f)).count() as u32)
        .sum();
    let (mut held, mut theirs) = (0u32, 0u32);
    frame_pairs(
        tracks,
        labels,
        |_, _| true,
        |f, l, i| {
            if is_main[l] && counted(f) {
                held += 1;
                theirs += u32::from(got[i].and_then(|c| char_name.get(&c)).is_some_and(|&n| n == labels[l].name));
            }
        },
    );
    let (held, theirs) = (held as f64 / all as f64, theirs as f64 / all as f64);
    println!("main cast faces (labels)     in a track {held:.3} | on their character {theirs:.3}");
    (held, theirs)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A track with one detection every [`STRIDE`] frames in `start..end`, all at `bbox`.
    pub fn track(start: u32, end: u32, bbox: [f32; 4]) -> FaceTrack {
        let dets = (start..end)
            .step_by(STRIDE as usize)
            .map(|frame| FaceDet { frame, bbox, score: 0.9, norm: 20.0, quality: 0.8 })
            .collect();
        FaceTrack { start, end, shot: start, dets, embedding: vec![1.0], quality: 0.8, thumb: Vec::new() }
    }

    /// A label of `t`'s frames, with `bbox` in each.
    pub fn label_of(t: &FaceTrack, name: &str, bbox: [f32; 4]) -> Label {
        Label {
            span: (t.start, t.end),
            boxes: t.dets.iter().map(|d| (d.frame, bbox)).collect(),
            name: name.into(),
            flags: Vec::new(),
        }
    }

    #[test]
    fn kinds_of_names() {
        assert_eq!(kind("TAN"), Kind::Named("TAN"));
        assert_eq!(kind("walter"), Kind::Named("walter"));
        for own in ["?e", "?", "n"] {
            assert_eq!(kind(own), Kind::OwnPerson);
        }
        for out in ["?u", "x"] {
            assert_eq!(kind(out), Kind::Excluded);
        }
    }

    #[test]
    fn a_face_box_inside_a_head_box_is_that_head() {
        let head = [0.30, 0.20, 0.50, 0.50];
        let face = [0.34, 0.28, 0.46, 0.46];
        assert!(box_iou(&head, &face) < 0.5);
        assert!(same_head(&head, &face) && same_head(&face, &head));
        // The same box beside the head is not it.
        assert!(!same_head(&head, &[0.48, 0.28, 0.60, 0.46]));
    }

    #[test]
    fn over_the_shoulder_each_head_keeps_its_own_label() {
        // A big head in front (seen from behind, over the shoulder) and a
        // small one further back, inside the big one's box: the small box
        // is one head with the big one by intersection over the smaller.
        let (big, small) = ([0.00, 0.20, 0.60, 1.00], [0.35, 0.30, 0.45, 0.45]);
        assert!(same_head(&big, &small));
        let front = track(0, 60, [0.01, 0.21, 0.59, 0.99]);
        let back = track(0, 60, [0.355, 0.305, 0.445, 0.445]);
        let labels = [label_of(&front, "FRONT", big), label_of(&back, "BACK", small)];
        // Either order of the tracks: the pick is by IoU, not by who comes first.
        assert_eq!(names_of(&[front.clone(), back.clone()], &labels), vec![Some("FRONT"), Some("BACK")]);
        assert_eq!(names_of(&[back.clone(), front.clone()], &labels), vec![Some("BACK"), Some("FRONT")]);
        // Only the small head found: it is the small label's, not the big one's.
        assert_eq!(names_of(std::slice::from_ref(&back), &labels), vec![Some("BACK")]);
        // Only the big head found: the big label's.
        assert_eq!(names_of(std::slice::from_ref(&front), &labels), vec![Some("FRONT")]);
    }

    #[test]
    fn a_small_box_is_a_big_ones_head_from_0_7_of_it_inside() {
        // Centres inside the big box all three, IoU far below 0.5: only the
        // share of the small box inside decides.
        let big = [0.00, 0.00, 0.50, 0.50];
        let (out, edge, inside) = ([0.38, 0.10, 0.58, 0.20], [0.37, 0.10, 0.57, 0.20], [0.35, 0.10, 0.55, 0.20]);
        for b in [out, edge, inside] {
            assert!(box_iou(&big, &b) < 0.1);
        }
        assert!(!same_head(&big, &out), "0.6 inside");
        assert!(!same_head(&big, &edge), "0.65 inside");
        assert!(same_head(&big, &inside) && same_head(&inside, &big), "0.75 inside");
    }

    /// A label of `name` over `span`, a box at each of `frames`.
    fn label_at(name: &str, span: (u32, u32), frames: &[u32], bbox: [f32; 4]) -> Label {
        Label { span, boxes: frames.iter().map(|&f| (f, bbox)).collect(), name: name.into(), flags: Vec::new() }
    }

    #[test]
    fn a_tie_goes_to_a_known_character_then_to_someone_of_their_own() {
        let b = [0.30, 0.20, 0.50, 0.50];
        let t = track(0, 24, b);
        // Each label holds half the track: 12 frames from two boxes.
        let tie = |first: &str, second: &str| {
            let labels = [label_at(first, (0, 12), &[0, 6], b), label_at(second, (12, 24), &[12, 18], b)];
            names_of(std::slice::from_ref(&t), &labels)[0].map(String::from)
        };
        for (a, z, wins) in
            [("?u", "TAN", "TAN"), ("TAN", "?u", "TAN"), ("?e", "TAN", "TAN"), ("x", "?e", "?e"), ("?", "n", "?")]
        {
            assert_eq!(tie(a, z).as_deref(), Some(wins), "{a} against {z}");
        }
        // Of two codes, the first by name.
        assert_eq!(tie("TAN", "NEZ").as_deref(), Some("NEZ"));
    }

    #[test]
    fn a_vote_weighs_the_frames_its_box_stands_for() {
        // Three boxes over 3 frames against two over 21: the 21 frames win.
        let b = [0.30, 0.20, 0.50, 0.50];
        let t = track(0, 24, b);
        let labels = [label_at("?u", (0, 3), &[0, 1, 2], b), label_at("TAN", (3, 24), &[3, 12], b)];
        assert_eq!(names_of(std::slice::from_ref(&t), &labels), vec![Some("TAN")]);
        // Two labels taking turns on the track, `?` with sparse boxes (24
        // frames by its own boxes, 18 for TAN): each frame is the last
        // box's before it, 9 frames `?` and 15 TAN.
        let labels = [label_at("?", (0, 24), &[0, 6, 21], b), label_at("TAN", (3, 21), &[3, 9, 12, 15, 18], b)];
        assert_eq!(names_of(std::slice::from_ref(&t), &labels), vec![Some("TAN")]);
    }

    #[test]
    fn a_box_before_a_left_out_stretch_stands_for_its_own_frames() {
        let l = label_at("TAN", (0, 40), &[0, 6, 12, 18, 24, 30, 36], [0.1, 0.1, 0.3, 0.3]);
        assert_eq!(box_weights(&l, |_| true), vec![(0, 6), (6, 6), (12, 6), (18, 6), (24, 6), (30, 6), (36, 4)]);
        let w = box_weights(&l, |f| !(10..30).contains(&f));
        assert_eq!(w, vec![(0, 6), (6, 4), (30, 6), (36, 4)]);
        assert_eq!(w.iter().map(|x| x.1).sum::<u32>(), 20, "the 20 frames outside 10..30");
    }

    #[test]
    fn frames_per_second() {
        assert!((parse_fps("24000/1001") - 23.976).abs() < 1e-3);
        assert_eq!(parse_fps(" 25 "), 25.0);
        assert_eq!(parse_fps("748327500/31212437"), 748327500.0 / 31212437.0);
    }

    #[test]
    #[should_panic(expected = "denominator")]
    fn frames_per_second_never_over_0() {
        parse_fps("25/0");
    }

    #[test]
    fn labels_read_flags_and_skip_comments() {
        let text = "# before the header\n\
                    start\tend\tname\tboxes\tflags\n\
                    # between rows\n\
                    0\t30\tTAN\t0:0.1,0.1,0.3,0.4 15:0.1,0.1,0.3,0.4\tback, chibi\r\n\
                    30\t60\t?u\t30:0.1,0.1,0.3,0.4\n\
                    \n\
                    60\t90\tNEZ\t60:0.1,0.1,0.3,0.4\t\n";
        let l = labels_of(parse_tsv(text));
        assert_eq!(l.len(), 3);
        assert_eq!((l[0].span, l[0].name.as_str(), l[0].boxes.len()), ((0, 30), "TAN", 2));
        assert_eq!(l[0].boxes[1], (15, [0.1, 0.1, 0.3, 0.4]));
        assert_eq!(l[0].flags, ["back", "chibi"]);
        assert!(l[1].flags.is_empty() && l[2].flags.is_empty());
        assert_eq!((l[1].name.as_str(), l[2].name.as_str()), ("?u", "NEZ"));
    }

    #[test]
    fn one_to_one_takes_as_many_pairs_as_can_be() {
        // Label 0 fits both found boxes and box 0 best, label 1 only box 0:
        // label 0 taking its best box would leave label 1 without one.
        let labels = [[0.10, 0.10, 0.50, 0.50]; 2];
        let found = [[0.10, 0.10, 0.50, 0.50], [0.12, 0.12, 0.48, 0.48]];
        let mut p = pair_boxes(&labels, &found, |a, b| (a, b) != (1, 1));
        p.sort_unstable();
        assert_eq!(p, vec![(0, 1), (1, 0)]);
        // Without that, each label takes the box it overlaps most.
        let p = pair_boxes(&labels[..1], &found, |_, _| true);
        assert_eq!(p, vec![(0, 0)]);
    }
}
