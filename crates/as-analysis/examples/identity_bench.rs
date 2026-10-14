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

fn score(tracks: &[FaceTrack], labels_path: &str, p: &Params, detail: bool) {
    let labels = read_labels(labels_path);
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

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tracks = read_tracks(&a[1]);
    match a[0].as_str() {
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
