//! Dev tool: measure how well tracks become characters, against labels a
//! person gave (BCubed precision/recall/F, overall and for the main cast).
//!
//! 1. `identity_bench sheets <file.tracks> <out_dir>` clusters the tracks
//!    (default parameters) and writes, per character, its tracks' thumbnails
//!    numbered `<out_dir>/c<rank>/<n>.jpg` (least typical first) — and the
//!    rest under `u/` — plus `sheets.tsv` (`sheet  n  track`). Tile them with
//!    ffmpeg (`-i %04d.jpg -vf tile=8x8`) to look at.
//! 2. Write `answers.txt`, one line per sheet folder: `c03 = jesse` or
//!    `u = ?`, then exceptions by number: `c03 = jesse; 7 hank; 12 ?`. `?`
//!    is anyone unnamed (an extra, or not a face).
//! 3. `identity_bench labels <file.tracks> <out_dir> <answers.txt> <labels.tsv>`
//!    turns them into labels anchored by time and place on screen
//!    (`start end x0 y0 x1 y1 name`), so they still fit tracks of a later
//!    face pass.
//! 4. `identity_bench score <file.tracks> <labels.tsv> [link] [assign]`
//!    clusters again and scores.
//!
//! `identity_bench separation <file.tracks> <labels.tsv>` measures the
//! embedder alone, without clustering: how well track similarity tells the
//! same named person from two different ones (AUC, and how many same-person
//! pairs pass at the similarity where 1 % / 0.1 % of different-person pairs
//! do), overall and by the quality of the blurrier track of a pair.
//!
//! `identity_bench norms <reference.tracks> <other.tracks> <floor> <full>`
//! carries the embedding lengths quality is read from (`FaceParams::
//! norm_floor`, `norm_full`) over to another embedder: the lengths at the
//! same share of all faces.

use std::collections::HashMap;
use std::path::Path;

use as_analysis::tracks::{self, FaceTrack};
use as_identity::metrics::bcubed;
use as_identity::{Assign, Params, TrackIn, identify};

const STRIDE: u32 = 3;
const MAX_GAP: u32 = 8;
const FPS: f64 = 24000.0 / 1001.0;

fn input(tracks: &[FaceTrack]) -> Vec<TrackIn> {
    tracks
        .iter()
        .map(|t| TrackIn {
            media: 0,
            start: t.start,
            end: t.end,
            spans: tracks::presence_spans(&t.dets, t.end, STRIDE, MAX_GAP),
            embedding: t.embedding.clone(),
            quality: t.quality,
            dets: t.dets.len() as u32,
            pin: None,
            forbid: vec![],
            previous: None,
        })
        .collect()
}

fn box_iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// Characters as per-track groups (index into the outcome's characters).
fn members(o: &as_identity::Outcome) -> Vec<Option<usize>> {
    o.tracks.iter().map(|(a, _)| a.character()).collect()
}

fn read_tracks(path: &str) -> Vec<FaceTrack> {
    tracks::decode(&std::fs::read(path).expect("tracks file")).expect("readable tracks")
}

fn sheets(tracks: &[FaceTrack], out: &Path) {
    let input = input(tracks);
    let o = identify(&input, &Params::default());
    let chars = members(&o);
    let screen = |c: usize| -> u32 {
        (0..tracks.len()).filter(|&i| chars[i] == Some(c)).map(|i| tracks[i].end - tracks[i].start).sum()
    };
    let mut order: Vec<usize> = (0..o.characters.len()).collect();
    order.sort_by_key(|&c| std::cmp::Reverse(screen(c)));
    let _ = std::fs::remove_dir_all(out);
    let mut tsv = String::from("sheet\tn\ttrack\n");
    let mut write = |name: &str, mut ids: Vec<usize>, centroid: Option<&[f32]>| {
        // Least typical first: that's where mistakes are.
        if let Some(c) = centroid {
            ids.sort_by(|&a, &b| {
                as_vision::arcface::cosine(&tracks[a].embedding, c)
                    .total_cmp(&as_vision::arcface::cosine(&tracks[b].embedding, c))
            });
        }
        let dir = out.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for (n, &i) in ids.iter().enumerate() {
            std::fs::write(dir.join(format!("{n:04}.jpg")), &tracks[i].thumb).unwrap();
            tsv += &format!("{name}\t{n}\t{i}\n");
        }
    };
    for (rank, &c) in order.iter().enumerate() {
        let ids: Vec<usize> = (0..tracks.len()).filter(|&i| chars[i] == Some(c)).collect();
        println!("c{rank:02}: {} tracks, {:.0} s", ids.len(), screen(c) as f64 / FPS);
        write(&format!("c{rank:02}"), ids, Some(&o.characters[c].centroid));
    }
    let rest: Vec<usize> = (0..tracks.len()).filter(|&i| chars[i].is_none()).collect();
    println!("u: {} tracks", rest.len());
    write("u", rest, None);
    std::fs::write(out.join("sheets.tsv"), tsv).unwrap();
}

/// `answers.txt` → a name per track.
fn answers(out: &Path, text: &str, n: usize) -> Vec<String> {
    let mut cell: HashMap<(String, usize), usize> = HashMap::new();
    for l in std::fs::read_to_string(out.join("sheets.tsv")).unwrap().lines().skip(1) {
        let v: Vec<&str> = l.split('\t').collect();
        cell.insert((v[0].to_string(), v[1].parse().unwrap()), v[2].parse().unwrap());
    }
    let mut names = vec![String::new(); n];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let mut parts = line.split(';').map(str::trim);
        let (sheet, all) = parts.next().unwrap().split_once('=').expect("`sheet = name`");
        let sheet = sheet.trim();
        let all = all.trim();
        let mut by_n: HashMap<usize, String> = HashMap::new();
        for ex in parts.filter(|p| !p.is_empty()) {
            let (k, name) = ex.split_once(' ').expect("`n name`");
            for k in k.split(',') {
                // `12-15` ranges.
                let (a, b) = k.split_once('-').unwrap_or((k, k));
                for x in a.parse::<usize>().unwrap()..=b.parse::<usize>().unwrap() {
                    by_n.insert(x, name.trim().to_string());
                }
            }
        }
        let mut seen = 0;
        for ((s, k), &i) in &cell {
            if s == sheet {
                names[i] = by_n.get(k).cloned().unwrap_or_else(|| all.to_string());
                seen += 1;
            }
        }
        assert!(seen > 0, "no sheet {sheet}");
    }
    let missing = names.iter().filter(|s| s.is_empty()).count();
    assert_eq!(missing, 0, "{missing} tracks without an answer");
    names
}

fn labels(tracks: &[FaceTrack], out: &Path, answers_path: &str, labels_path: &str) {
    let names = answers(out, &std::fs::read_to_string(answers_path).unwrap(), tracks.len());
    // Every detection's box: `frame:x0,y0,x1,y1` separated by spaces.
    let mut tsv = String::from("start\tend\tname\tboxes\n");
    for (t, name) in tracks.iter().zip(&names) {
        let boxes: Vec<String> =
            t.dets.iter().map(|d| format!("{}:{}", d.frame, d.bbox.map(|x| format!("{x:.3}")).join(","))).collect();
        tsv += &format!("{}\t{}\t{name}\t{}\n", t.start, t.end, boxes.join(" "));
    }
    std::fs::write(labels_path, tsv).unwrap();
    let mut count: HashMap<&str, (usize, u32)> = HashMap::new();
    for (t, n) in tracks.iter().zip(&names) {
        let e = count.entry(n.as_str()).or_default();
        e.0 += 1;
        e.1 += t.end - t.start;
    }
    let mut v: Vec<_> = count.into_iter().collect();
    v.sort_by_key(|(_, (_, f))| std::cmp::Reverse(*f));
    for (n, (k, f)) in v {
        println!("{n:12} {k:5} tracks {:7.1} s", f as f64 / FPS);
    }
}

struct Label {
    span: (u32, u32),
    /// `(frame, box)` of every detection.
    boxes: Vec<(u32, [f32; 4])>,
    name: String,
}

/// The mean box of `boxes` in `from..to`, or the one nearest to it.
fn box_in(boxes: &[(u32, [f32; 4])], from: u32, to: u32) -> Option<[f32; 4]> {
    let inside: Vec<[f32; 4]> = boxes.iter().filter(|(f, _)| (from..to).contains(f)).map(|b| b.1).collect();
    if inside.is_empty() {
        let mid = from + (to - from) / 2;
        return boxes.iter().min_by_key(|(f, _)| f.abs_diff(mid)).map(|b| b.1);
    }
    let n = inside.len() as f32;
    Some(std::array::from_fn(|i| inside.iter().map(|b| b[i]).sum::<f32>() / n))
}

fn read_labels(path: &str) -> Vec<Label> {
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
fn names_of<'a>(tracks: &[FaceTrack], labels: &'a [Label]) -> Vec<Option<&'a str>> {
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

/// Embedder alone: same-person vs different-person track similarity.
fn separation(tracks: &[FaceTrack], labels_path: &str) {
    let labels = read_labels(labels_path);
    let names = names_of(tracks, &labels);
    let named: Vec<usize> = (0..tracks.len()).filter(|&i| names[i].is_some_and(|n| n != "?" && n != "x")).collect();
    let mut same: Vec<(f32, f32)> = Vec::new();
    let mut diff: Vec<(f32, f32)> = Vec::new();
    for (k, &i) in named.iter().enumerate() {
        for &j in &named[k + 1..] {
            let cos = as_vision::arcface::cosine(&tracks[i].embedding, &tracks[j].embedding);
            let q = tracks[i].quality.min(tracks[j].quality);
            if names[i] == names[j] { same.push((cos, q)) } else { diff.push((cos, q)) }
        }
    }
    println!("{} named tracks: {} same-person pairs, {} different-person pairs", named.len(), same.len(), diff.len());
    let report = |what: &str, lo: f32, hi: f32| {
        let pick = |v: &[(f32, f32)]| -> Vec<f32> {
            let mut x: Vec<f32> = v.iter().filter(|p| (lo..hi).contains(&p.1)).map(|p| p.0).collect();
            x.sort_by(f32::total_cmp);
            x
        };
        let (s, d) = (pick(&same), pick(&diff));
        if s.is_empty() || d.is_empty() {
            return;
        }
        // AUC: the chance a same-person pair is more alike than a different-person pair.
        let mut below = 0u64;
        for &x in &s {
            below += d.partition_point(|&y| y < x) as u64;
        }
        let auc = below as f64 / (s.len() as f64 * d.len() as f64);
        let tar = |far: f64| {
            let t = d[((1.0 - far) * d.len() as f64) as usize % d.len()];
            (s.len() - s.partition_point(|&x| x < t)) as f64 / s.len() as f64
        };
        println!(
            "  {what:22} {:6} same / {:7} diff: AUC {auc:.4}, pass at FAR 1% {:.3}, at 0.1% {:.3}",
            s.len(),
            d.len(),
            tar(0.01),
            tar(0.001)
        );
    };
    report("all", f32::NEG_INFINITY, f32::INFINITY);
    report("blurrier quality < 0.3", f32::NEG_INFINITY, 0.3);
    report("blurrier 0.3 .. 0.6", 0.3, 0.6);
    report("blurrier >= 0.6", 0.6, f32::INFINITY);
}

fn score(tracks: &[FaceTrack], labels_path: &str, p: &Params, detail: bool) {
    let labels = read_labels(labels_path);
    let truth_name = names_of(tracks, &labels);
    let matched = truth_name.iter().filter(|n| n.is_some()).count();
    // Scored: tracks with a label that isn't "can't tell".
    let keep: Vec<usize> = (0..tracks.len()).filter(|&i| truth_name[i].is_some_and(|n| n != "x")).collect();
    // Unnamed faces are each someone of their own.
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let truth: Vec<usize> = truth_name
        .iter()
        .enumerate()
        .map(|(i, n)| match n {
            Some(n) if *n != "?" && *n != "x" => {
                let k = ids.len();
                *ids.entry(n).or_insert(k)
            }
            _ => 1_000_000 + i,
        })
        .collect();
    let named: HashMap<usize, &str> = ids.iter().map(|(n, &k)| (k, *n)).collect();
    // The main cast: named people on screen 30 s or more.
    let mut screen: HashMap<usize, u32> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        *screen.entry(truth[i]).or_default() += t.end - t.start;
    }
    let main = |k: usize| named.contains_key(&k) && screen[&k] as f64 / FPS >= 30.0;

    let t0 = std::time::Instant::now();
    let o = identify(&input(tracks), p);
    let ms = t0.elapsed().as_millis();
    let got = members(&o);
    let sub_got: Vec<Option<usize>> = keep.iter().map(|&i| got[i]).collect();
    let sub_truth: Vec<usize> = keep.iter().map(|&i| truth[i]).collect();
    let frames: Vec<f32> = keep.iter().map(|&i| (tracks[i].end - tracks[i].start) as f32).collect();
    let ones = vec![1f32; keep.len()];
    let show = |what: &str, counted: &dyn Fn(usize) -> bool| {
        let counted = |k: usize| counted(keep[k]);
        let by_track = bcubed(&sub_got, &sub_truth, &ones, counted);
        let by_time = bcubed(&sub_got, &sub_truth, &frames, counted);
        println!(
            "{what:28} tracks P {:.3} R {:.3} F {:.3} | screen time P {:.3} R {:.3} F {:.3}",
            by_track.precision, by_track.recall, by_track.f, by_time.precision, by_time.recall, by_time.f
        );
    };
    println!(
        "{} tracks ({matched} labelled, {} scored) → {} characters in {ms} ms; link {} assign {} margin {} unsure {}",
        tracks.len(),
        keep.len(),
        o.characters.len(),
        p.link,
        p.assign,
        p.margin,
        p.unsure
    );
    show("main cast (>= 30 s)", &|i| main(truth[i]));
    show("everyone named", &|i| named.contains_key(&truth[i]));
    found_faces(tracks, &labels, &truth_name, &got);
    // Per main character: where its tracks went.
    let mut mains: Vec<usize> = named.keys().copied().filter(|&k| main(k)).collect();
    mains.sort_by_key(|k| std::cmp::Reverse(screen[k]));
    for k in mains {
        let mine: Vec<usize> = keep.iter().copied().filter(|&i| truth[i] == k).collect();
        let mut into: HashMap<Option<usize>, usize> = HashMap::new();
        for &i in &mine {
            *into.entry(got[i]).or_default() += 1;
        }
        let best = into.iter().filter(|(c, _)| c.is_some()).max_by_key(|(_, n)| **n).map(|(c, n)| (c.unwrap(), *n));
        let (pure, size) = best.map_or((0.0, 0), |(c, n)| {
            let size = keep.iter().filter(|&&i| got[i] == Some(c)).count();
            (n as f64 / size as f64, n)
        });
        let unsure = mine.iter().filter(|&&i| matches!(o.tracks[i].0, Assign::Unsure(_))).count();
        let none = mine.iter().filter(|&&i| matches!(o.tracks[i].0, Assign::None)).count();
        println!(
            "  {:12} {:4} tracks {:6.1} s: biggest character has {size} ({:.0}% of it them), {} other characters, {unsure} unsure, {none} unassigned",
            named[&k],
            mine.len(),
            screen[&k] as f64 / FPS,
            100.0 * pure,
            into.keys().filter(|c| c.is_some()).count().saturating_sub(1),
        );
        if detail {
            let name = |i: usize| truth_name[i].unwrap_or("-");
            if let Some((c, _)) = best {
                let foreign: Vec<String> = keep
                    .iter()
                    .filter(|&&i| got[i] == Some(c) && truth[i] != k)
                    .map(|&i| format!("{}@{}-{}", name(i), tracks[i].start, tracks[i].end))
                    .collect();
                if !foreign.is_empty() {
                    println!("      in their character too: {}", foreign.join(", "));
                }
                let elsewhere: Vec<String> = mine
                    .iter()
                    .filter(|&&i| got[i].is_some_and(|g| g != c))
                    .map(|&i| format!("{}-{} (q {:.2})", tracks[i].start, tracks[i].end, tracks[i].quality))
                    .collect();
                if !elsewhere.is_empty() {
                    println!("      in other characters: {}", elsewhere.join(", "));
                }
            }
        }
    }
}

/// End to end, over the labels rather than the tracks (BCubed only sees
/// faces that became a track): the share of the main cast's labelled faces
/// a track still holds, and the share held by a track of the character
/// named like them (the name most of a character's frames have).
fn found_faces(tracks: &[FaceTrack], labels: &[Label], truth_name: &[Option<&str>], got: &[Option<usize>]) {
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
    let mut screen: HashMap<&str, u32> = HashMap::new();
    for l in labels {
        *screen.entry(l.name.as_str()).or_default() += l.span.1 - l.span.0;
    }
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
                theirs += u32::from(got[i].and_then(|c| char_name.get(&c)).is_some_and(|&(n, _)| n == l.name));
            }
        }
    }
    println!(
        "main cast faces (labels)     in a track {:.3} | on their character {:.3}",
        held as f64 / all as f64,
        theirs as f64 / all as f64
    );
}

/// The share of `sorted` below `v`, and the value at share `q`.
fn share_below(sorted: &[f32], v: f32) -> f32 {
    sorted.partition_point(|&x| x < v) as f32 / sorted.len() as f32
}

fn at_share(sorted: &[f32], q: f32) -> f32 {
    sorted[((q * sorted.len() as f32) as usize).min(sorted.len() - 1)]
}

fn norms(reference: &[FaceTrack], other: &[FaceTrack], floor: f32, full: f32) {
    let sorted = |t: &[FaceTrack]| {
        let mut v: Vec<f32> = t.iter().flat_map(|t| t.dets.iter().map(|d| d.norm)).collect();
        v.sort_by(f32::total_cmp);
        v
    };
    let (r, o) = (sorted(reference), sorted(other));
    for q in [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95] {
        println!("{:3.0}%: {:6.2} | {:6.2}", q * 100.0, at_share(&r, q), at_share(&o, q));
    }
    let (qf, qu) = (share_below(&r, floor), share_below(&r, full));
    println!(
        "floor {floor} is at {:.1}% → {:.2}; full {full} at {:.1}% → {:.2}",
        qf * 100.0,
        at_share(&o, qf),
        qu * 100.0,
        at_share(&o, qu)
    );
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tracks = read_tracks(&a[1]);
    match a[0].as_str() {
        "separation" => separation(&tracks, &a[2]),
        "norms" => norms(&tracks, &read_tracks(&a[2]), a[3].parse().unwrap(), a[4].parse().unwrap()),
        "sheets" => sheets(&tracks, Path::new(&a[2])),
        "labels" => labels(&tracks, Path::new(&a[2]), &a[3], &a[4]),
        "score" => {
            let mut p = Params::default();
            let num = |i: usize| a.get(i).map(|s| s.parse::<f32>().unwrap());
            if let Some(v) = num(3) {
                p.link = v;
            }
            if let Some(v) = num(4) {
                p.assign = v;
            }
            if let Some(v) = num(5) {
                p.margin = v;
            }
            if let Some(v) = num(6) {
                p.unsure = v;
            }
            score(&tracks, &a[2], &p, std::env::var_os("DETAIL").is_some())
        }
        other => panic!("unknown command {other}"),
    }
}
