//! The identity bench's labels (`bench/identity`) and what the bench
//! examples share: tracks in, clustering input, which label each track is.

#![allow(dead_code)]

use std::collections::HashMap;

use as_analysis::tracks::{self, FaceTrack};
use as_identity::TrackIn;

pub const STRIDE: u32 = 3;
pub const MAX_GAP: u32 = 8;
pub const FPS: f64 = 24000.0 / 1001.0;

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

fn input_raw(tracks: &[FaceTrack]) -> Vec<TrackIn> {
    tracks
        .iter()
        .map(|t| TrackIn {
            media: 0,
            start: t.start,
            end: t.end,
            spans: tracks::presence_spans(&t.dets, t.end, STRIDE, MAX_GAP),
            seen: t.dets.iter().map(|d| d.frame).collect(),
            embedding: t.embedding.clone(),
            quality: t.quality,
            dets: t.dets.len() as u32,
            pin: None,
            forbid: vec![],
            previous: None,
        })
        .collect()
}

pub fn box_iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
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

pub struct Label {
    pub span: (u32, u32),
    /// `(frame, box)` of every detection.
    pub boxes: Vec<(u32, [f32; 4])>,
    pub name: String,
}

/// The mean box of `boxes` in `from..to`, or the one nearest to it.
pub fn box_in(boxes: &[(u32, [f32; 4])], from: u32, to: u32) -> Option<[f32; 4]> {
    let inside: Vec<[f32; 4]> = boxes.iter().filter(|(f, _)| (from..to).contains(f)).map(|b| b.1).collect();
    if inside.is_empty() {
        let mid = from + (to - from) / 2;
        return boxes.iter().min_by_key(|(f, _)| f.abs_diff(mid)).map(|b| b.1);
    }
    let n = inside.len() as f32;
    Some(std::array::from_fn(|i| inside.iter().map(|b| b[i]).sum::<f32>() / n))
}

pub fn read_labels(path: &str) -> Vec<Label> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .skip(1)
        .map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            let boxes = v[3]
                .split(' ')
                .map(|b| {
                    let (frame, xy) = b.split_once(':').unwrap();
                    let xy: Vec<f32> = xy.split(',').map(|x| x.parse().unwrap()).collect();
                    (frame.parse().unwrap(), [xy[0], xy[1], xy[2], xy[3]])
                })
                .collect();
            Label { span: (v[0].parse().unwrap(), v[1].parse().unwrap()), boxes, name: v[2].to_string() }
        })
        .collect()
}

/// The name each track takes from the labels (`None`: no label fits).
pub fn names_of<'a>(tracks: &[FaceTrack], labels: &'a [Label]) -> Vec<Option<&'a str>> {
    // Each track takes the name of the labels it shares most of the shorter
    // of the two with: the one most of its frames are, among labels whose
    // face is where the track's face is during the frames they share. Tracks
    // a tracker change split or joined keep their names, moving or not.
    let mut truth_name: Vec<Option<&str>> = vec![None; tracks.len()];
    for (i, t) in tracks.iter().enumerate() {
        let mut votes: HashMap<&str, u32> = HashMap::new();
        for l in labels.iter().filter(|l| l.span.0 < t.end && t.start < l.span.1) {
            let overlap = l.span.1.min(t.end) - l.span.0.max(t.start);
            let shorter = (l.span.1 - l.span.0).min(t.end - t.start).max(1);
            if (overlap as f32) < 0.5 * shorter as f32 {
                continue;
            }
            let (from, to) = (l.span.0.max(t.start), l.span.1.min(t.end));
            let mine: Vec<(u32, [f32; 4])> = t.dets.iter().map(|d| (d.frame, d.bbox)).collect();
            let (Some(a), Some(b)) = (box_in(&mine, from, to), box_in(&l.boxes, from, to)) else { continue };
            if box_iou(&a, &b) >= 0.5 {
                *votes.entry(l.name.as_str()).or_default() += overlap;
            }
        }
        truth_name[i] = votes.into_iter().max_by_key(|&(n, v)| (v, std::cmp::Reverse(n))).map(|(n, _)| n);
    }
    truth_name
}

/// The name of each character: the one most of its tracks' frames have
/// (`?` and `x` don't count).
pub fn character_names<'a>(
    tracks: &[FaceTrack],
    truth_name: &[Option<&'a str>],
    got: &[Option<usize>],
) -> HashMap<usize, &'a str> {
    let mut frames_of: HashMap<(usize, &str), u32> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        if let (Some(c), Some(n)) = (got[i], truth_name[i])
            && n != "?"
            && n != "x"
        {
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

/// Seconds on screen per label name (from the labels' spans).
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
    let char_name = character_names(tracks, truth_name, got);
    let screen = label_screen(labels);
    let main = |n: &str| n != "?" && n != "x" && screen[n] as f64 / FPS >= 30.0;
    let mut at: HashMap<u32, Vec<(usize, [f32; 4])>> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        for d in &t.dets {
            at.entry(d.frame).or_default().push((i, d.bbox));
        }
    }
    let (mut all, mut held, mut theirs) = (0u32, 0u32, 0u32);
    for l in labels.iter().filter(|l| main(&l.name)) {
        for (frame, b) in &l.boxes {
            all += 1;
            let best = at
                .get(frame)
                .into_iter()
                .flatten()
                .map(|&(i, bb)| (i, box_iou(b, &bb)))
                .filter(|&(_, iou)| iou >= 0.5)
                .max_by(|x, y| x.1.total_cmp(&y.1));
            if let Some((i, _)) = best {
                held += 1;
                theirs += u32::from(got[i].and_then(|c| char_name.get(&c)).is_some_and(|&n| n == l.name));
            }
        }
    }
    let (held, theirs) = (held as f64 / all as f64, theirs as f64 / all as f64);
    println!("main cast faces (labels)     in a track {held:.3} | on their character {theirs:.3}");
    (held, theirs)
}
