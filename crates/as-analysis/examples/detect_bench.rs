//! Dev tool: what the face detector finds in chosen frames at each size it
//! can look at, and what each size costs.
//! `cargo run --release -p as-analysis --example detect_bench -- <ort_dir> <models_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path> <frames.txt> <out_dir>`
//! `frames.txt` holds one frame number per line. Each frame is decoded as
//! the face pass decodes its samples and saved as `<out_dir>/frames/<frame>.jpg`;
//! `<out_dir>/px_dets.json` lists, per frame, every face found down to score
//! `LOW` (default 0.2) at the decoded size (`"1"`), half of it (`"2"`) and a
//! quarter (`"4"`), as `[x1, y1, x2, y2, score]` in decoded pixels.
//! Then each size is timed over all frames (`REPEAT` rounds, default 3), and
//! the face pass's own search ([`as_analysis::find_faces`]) for live action
//! and for 3D animation (`FaceParams::animated`), whose faces go to
//! `<out_dir>/search_<Once|Scales>.json`. The decoded frames are kept in
//! `<out_dir>/frames.rgb` and read from there by the next run.
//!
//! Anime bench (`bench/anime/FORMATS.md`), against keyframe ground truth:
//! - `detect_bench score <keyframes.json> <dets.json> [threshold]` matches
//!   each frame's detections to its heads one to one, best score first, at
//!   IoU ≥ 0.5 (a character's head before any other box), and keeps those at
//!   `threshold` (default: every detection). Recall is over character heads
//!   (`H`, `Hm`, `Hk`, `Hb`, `Hn`): overall, per class and per stratum (flags
//!   `back`, `chibi`, `mask`, `graded`, `mono`, `impact`, where class `Hb` is
//!   `back` too, `Hk` `chibi` and `Hn` `creature`; `small` under 0.08 of the
//!   frame height, `close` from 0.33, else `mid`; `crowd`, a frame with 5 or
//!   more character heads). It is pooled over random and targeted keyframes,
//!   not reweighted by stratum. False alarms are detections on no head, on a
//!   head that isn't a character (`O`, `N`, `S:<id>` counted as `S`) and
//!   second ones on a head already found. Shares come with Wilson 95 %
//!   intervals; a sweep over the threshold gives the recall at precision 0.95
//!   and 0.97 and the threshold that reaches it. `SPANS=<spans.tsv>` leaves
//!   out the keyframes in OP/ED, eyecatch, recap and preview. Both files are
//!   checked: known classes and flags only, boxes as shares of the frame.
//! - `detect_bench paired <keyframes.json> <best.dets.json> <other.dets.json>
//!   [<keyframes.json> <best> <other> …] [precision]` compares two variants on
//!   the keyframes of one or more projects (one per series): per variant the
//!   threshold with the highest macro mean recall over the projects while
//!   each keeps precision ≥ `precision` (default 0.95), and the gap of that
//!   recall with a paired cluster bootstrap (`BOOT` resamples, default 2000,
//!   seeded by `SEED`, default 1) that draws each project's shots on their
//!   own, the threshold chosen anew in each resample. The shots come from
//!   `SEGMENTS=<a.segments.tsv>,<b…>` (one `bb.segments.tsv` per project, on
//!   its keyframes' timeline); without them each keyframe is a shot of its
//!   own, which makes the interval too narrow when keyframes share a shot.
//!   `SPANS=<a.spans.tsv>,<b…>` as above, one per project. E1 keeps `other`
//!   when the upper 95 % bound of the gap is below 2 points; there is no
//!   verdict when `best` isn't ahead on the sample itself.

#[path = "common/anime.rs"]
mod anime;
#[path = "common/labels.rs"]
mod labels;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anime::{Spans, read_shots};
use as_analysis::{DETECT_BATCH, FaceParams, detect_looks, frame_sizes};
use as_core::{CancelToken, FrameIndex};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::{Scrfd, image};
use serde::Deserialize;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("score") => return score(&a[1..]),
        Some("paired") => return paired(&a[1..]),
        _ => {}
    }
    let (ort, models, ffmpeg, dir) = (&a[0], Path::new(&a[1]), &a[2], Path::new(&a[3]));
    let (list, out) = (&a[4], Path::new(&a[5]));
    let low: f32 = std::env::var("LOW").map_or(0.2, |v| v.parse().unwrap());
    let repeat: usize = std::env::var("REPEAT").map_or(3, |v| v.parse().unwrap());
    let wanted: Vec<u32> = std::fs::read_to_string(list)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().unwrap())
        .collect();

    let index = FrameIndex::from_bytes(&std::fs::read(dir.join("bb.index")).unwrap()).unwrap();
    let info: MediaInfo = serde_json::from_str(&std::fs::read_to_string(dir.join("bb.info.json")).unwrap()).unwrap();
    let path = PathBuf::from(std::fs::read_to_string(dir.join("bb.path")).unwrap().trim());
    let tools = Tools::locate(Some(Path::new(ffmpeg))).unwrap();
    let rt = Runtime::load(Path::new(ort), true).unwrap();
    let params = FaceParams::default();
    let ((w, h), _) = frame_sizes(&info, &params);
    std::fs::create_dir_all(out.join("frames")).unwrap();

    // Each frame on its own, exactly as the pass decodes a sample there
    // (kept raw in `<out_dir>/frames.rgb` for the next run).
    let cancel = CancelToken::new();
    let t = Instant::now();
    let (raw, raw_list) = (out.join("frames.rgb"), out.join("frames.rgb.list"));
    let list_text: String = wanted.iter().map(|f| format!("{f}\n")).collect();
    let cached = std::fs::read_to_string(&raw_list).is_ok_and(|l| l == list_text);
    let mut frames: Vec<Vec<u8>> = std::fs::read(&raw)
        .ok()
        .filter(|b| cached && b.len() == wanted.len() * w * h * 3)
        .map(|b| b.chunks_exact(w * h * 3).map(<[u8]>::to_vec).collect())
        .unwrap_or_default();
    let fresh = frames.is_empty();
    for &f in wanted.iter().skip(frames.len()) {
        let mut got = None;
        as_media::frames::decode_samples(
            &tools,
            Source { path: &path, info: &info, index: &index },
            f,
            params.stride,
            (w as u32, h as u32),
            &cancel,
            |_, rgb| {
                got = Some(rgb.to_vec());
                Ok(false)
            },
        )
        .unwrap();
        let rgb = got.expect("a frame");
        std::fs::write(out.join("frames").join(format!("{f}.jpg")), image::jpeg(&rgb, w, h, 92)).unwrap();
        frames.push(rgb);
    }
    if fresh {
        std::fs::write(&raw, frames.concat()).unwrap();
        std::fs::write(&raw_list, list_text).unwrap();
    }
    println!("decoded {} frames of {w}x{h} in {:.1}s", frames.len(), t.elapsed().as_secs_f64());

    // The frame at each size: shrunk 1, 2 and 4 times.
    let shrinks = [1usize, 2, 4];
    let sized: Vec<Vec<Vec<u8>>> = shrinks
        .iter()
        .map(|&k| {
            frames
                .iter()
                .map(|rgb| {
                    let mut x = rgb.clone();
                    let (mut sw, mut sh) = (w, h);
                    while sw > w / k {
                        x = image::half(&x, sw, sh);
                        (sw, sh) = (sw / 2, sh / 2);
                    }
                    x
                })
                .collect()
        })
        .collect();
    let model = models.join("det_10g.onnx");
    let mut detectors: Vec<Scrfd> =
        shrinks.iter().map(|&k| Scrfd::load(&rt, &model, (w / k, h / k), DETECT_BATCH).unwrap()).collect();

    let mut json = String::from("[");
    let found: Vec<Vec<Vec<as_vision::FaceBox>>> = detectors
        .iter_mut()
        .zip(&sized)
        .map(|(d, imgs)| {
            let refs: Vec<&[u8]> = imgs.iter().map(Vec::as_slice).collect();
            d.detect_many(&refs, low).unwrap()
        })
        .collect();
    for (i, &f) in wanted.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        write!(json, "{{\"frame\":{f},\"w\":{w},\"h\":{h}").unwrap();
        for (s, &k) in shrinks.iter().enumerate() {
            write!(json, ",\"{k}\":[").unwrap();
            for (j, b) in found[s][i].iter().map(|b| b.scaled(k as f32)).enumerate() {
                if j > 0 {
                    json.push(',');
                }
                let [x1, y1, x2, y2] = b.bbox;
                write!(json, "[{x1:.1},{y1:.1},{x2:.1},{y2:.1},{:.4}]", b.score).unwrap();
            }
            json.push(']');
        }
        json.push('}');
    }
    json.push(']');
    // Pixel-space probe output, not the FORMATS.md `dets.json` the scorer
    // reads — a different name so the two never collide.
    std::fs::write(out.join("px_dets.json"), json).unwrap();

    // Cost of each size alone (detector only), at the live-action threshold.
    let n = frames.len();
    for ((d, imgs), k) in detectors.iter_mut().zip(&sized).zip(shrinks) {
        let refs: Vec<&[u8]> = imgs.iter().map(Vec::as_slice).collect();
        let _ = d.detect_many(&refs[..DETECT_BATCH.min(n)], 0.5).unwrap(); // warm-up
        let mut best = f64::MAX;
        for _ in 0..repeat {
            let t = Instant::now();
            std::hint::black_box(d.detect_many(&refs, 0.5).unwrap());
            best = best.min(t.elapsed().as_secs_f64());
        }
        println!("1/{k} ({}x{}): {:.2} ms/frame on {:?}", w / k, h / k, 1000.0 * best / n as f64, d.device());
    }

    // The pass's own search, shrinking included, per footage.
    let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    drop(detectors);
    for p in [FaceParams::default(), FaceParams::animated()] {
        let looks = detect_looks(&info, &p);
        let mut ds: Vec<Scrfd> =
            looks.iter().map(|l| Scrfd::load(&rt, &model, l.size, DETECT_BATCH).unwrap()).collect();
        let _ = as_analysis::find_faces(&mut ds, &looks, (w, h), &p, &refs[..DETECT_BATCH.min(n)]).unwrap();
        let mut best = f64::MAX;
        let mut got = Vec::new();
        for _ in 0..repeat {
            let t = Instant::now();
            got = as_analysis::find_faces(&mut ds, &looks, (w, h), &p, &refs).unwrap();
            best = best.min(t.elapsed().as_secs_f64());
        }
        let faces: usize = got.iter().map(Vec::len).sum();
        // What it kept, to check against the per-size boxes above.
        let mut kept = String::from("[");
        for (i, (f, found)) in wanted.iter().zip(&got).enumerate() {
            let boxes: Vec<String> = found
                .iter()
                .map(|b| {
                    format!("[{:.1},{:.1},{:.1},{:.1},{:.4}]", b.bbox[0], b.bbox[1], b.bbox[2], b.bbox[3], b.score)
                })
                .collect();
            write!(kept, "{}{{\"frame\":{f},\"faces\":[{}]}}", if i > 0 { "," } else { "" }, boxes.join(",")).unwrap();
        }
        kept.push(']');
        std::fs::write(out.join(format!("search_{:?}.json", p.search)), kept).unwrap();
        println!(
            "{:?} at {} {looks:?}: {:.2} ms/frame, {faces} faces",
            p.search,
            p.det_threshold,
            1000.0 * best / n as f64
        );
    }
}

// Scoring against keyframe ground truth.

/// Classes of a character's head. A detection on any other class (`O`, `N`
/// or `S:<id>`, a second box on a head) is a false alarm.
const HEADS: [&str; 5] = ["H", "Hm", "Hk", "Hb", "Hn"];
const FLAGS: [&str; 8] = ["back", "chibi", "mask", "graded", "mono", "impact", "flashback", "dark"];
/// The strata a head counts in by its flag, or by its class: `Hb` is the back
/// of a head, `Hk` chibi and `Hn` a creature (no flag of its own). `Hm` holds
/// transformations too, so `mask` goes by the flag alone.
const MARKED: [(&str, Option<&str>); 8] = [
    ("back", Some("Hb")),
    ("chibi", Some("Hk")),
    ("mask", None),
    ("graded", None),
    ("mono", None),
    ("impact", None),
    ("creature", Some("Hn")),
    ("dark", None),
];
const STRATA: [&str; 12] =
    ["back", "chibi", "mask", "graded", "mono", "impact", "creature", "dark", "small", "mid", "close", "crowd"];
const IOU: f64 = 0.5;
/// How far a box may reach outside the frame (a detector needn't clip).
const SLACK: f64 = 0.02;

#[derive(Deserialize)]
struct Keyframes {
    frames: BTreeMap<String, KeyFrame>,
}

#[derive(Deserialize)]
struct KeyFrame {
    heads: Vec<Head>,
    /// Added on purpose to fill a rare stratum, not drawn at random: it
    /// counts in its strata but not in the overall numbers.
    #[serde(default)]
    targeted: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct Head {
    #[serde(rename = "box")]
    bbox: [f64; 4],
    class: String,
    #[serde(default)]
    flags: Vec<String>,
}

#[derive(Deserialize)]
struct Dets {
    #[serde(default)]
    model: String,
    #[serde(default)]
    input: String,
    frames: BTreeMap<String, Vec<[f64; 5]>>,
}

/// One keyframe: its frame, its heads and the detections on it
/// (`x0, y0, x1, y1, score`).
struct Frame {
    at: u32,
    targeted: bool,
    heads: Vec<Head>,
    dets: Vec<[f64; 5]>,
}

impl Frame {
    fn characters(&self) -> usize {
        self.heads.iter().filter(|h| character(h).is_some()).count()
    }
}

/// What a detection landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    /// The first detection on head `i` (of any class).
    Head(usize),
    /// A further detection on head `i`, already taken by a better one.
    Duplicate(usize),
    /// No head at IoU ≥ 0.5.
    Stray,
}

fn character(h: &Head) -> Option<&'static str> {
    HEADS.iter().copied().find(|c| *c == h.class)
}

/// A class FORMATS.md knows: a character's head, `O`, `N`, `I` or `S:<id>`.
fn known_class(c: &str) -> bool {
    HEADS.contains(&c) || ["O", "N", IGNORE].contains(&c) || c.strip_prefix("S:").is_some_and(|id| !id.is_empty())
}

/// An ignore region (an unnamed swarm creature): a detection on it is neither
/// found nor a false alarm, and it is no head to find.
const IGNORE: &str = "I";

/// Whether `hit` landed on an ignore region of `f`.
fn ignored(f: &Frame, hit: Hit) -> bool {
    matches!(hit, Hit::Head(i) | Hit::Duplicate(i) if f.heads[i].class == IGNORE)
}

/// Panics unless `b` is `x0, y0, x1, y1` as shares of the frame, within
/// [`SLACK`]; a detection may have no area, a head may not.
fn check_box(b: &[f64], empty_ok: bool, what: impl Fn() -> String) {
    let inside = |v: f64| (-SLACK..=1.0 + SLACK).contains(&v);
    let ordered = |lo: f64, hi: f64| lo < hi || (empty_ok && lo == hi);
    assert!(
        b.iter().all(|&v| inside(v)) && ordered(b[0], b[2]) && ordered(b[1], b[3]),
        "{}: box {b:?} isn't x0,y0,x1,y1 as shares of the frame (pixels?)",
        what()
    );
}

/// The keyframes outside `spans` in frame order with their detections, and
/// the detector's name. Panics on a class or flag FORMATS.md doesn't know, on
/// a box that isn't in shares of the frame, and when no keyframe has an
/// entry but other frames do (the files don't share a frame grid).
fn load(keyframes: &str, dets: &str, spans: &Spans) -> (Vec<Frame>, String) {
    let k: Keyframes = serde_json::from_str(&std::fs::read_to_string(keyframes).unwrap()).unwrap();
    let d: Dets = serde_json::from_str(&std::fs::read_to_string(dets).unwrap()).unwrap();
    let mut found: BTreeMap<u32, Vec<[f64; 5]>> = d.frames.into_iter().map(|(f, v)| (f.parse().unwrap(), v)).collect();
    let targeted: std::collections::BTreeSet<u32> =
        k.frames.iter().filter(|(_, v)| v.targeted).map(|(f, _)| f.parse().unwrap()).collect();
    let mut heads: BTreeMap<u32, Vec<Head>> =
        k.frames.into_iter().map(|(f, v)| (f.parse().unwrap(), v.heads)).collect();
    for (f, hs) in &heads {
        for h in hs {
            assert!(known_class(&h.class), "{keyframes}: frame {f}: unknown class {:?}", h.class);
            if let Some(g) = h.flags.iter().find(|g| !FLAGS.contains(&g.as_str())) {
                panic!("{keyframes}: frame {f}: unknown flag {g:?}");
            }
            check_box(&h.bbox, false, || format!("{keyframes}: frame {f}"));
        }
    }
    for (f, xs) in &found {
        for x in xs {
            check_box(&x[..4], true, || format!("{dets}: frame {f}"));
            assert!(x[4].is_finite(), "{dets}: frame {f}: score {}", x[4]);
        }
    }
    let skipped = heads.keys().filter(|&&f| spans.holds(f)).count();
    heads.retain(|&f, _| !spans.holds(f));
    found.retain(|&f, _| !spans.holds(f));
    let absent = heads.keys().filter(|f| !found.contains_key(f)).count();
    let frames: Vec<Frame> = heads
        .into_iter()
        .map(|(at, heads)| Frame {
            at,
            targeted: targeted.contains(&at),
            heads,
            dets: found.remove(&at).unwrap_or_default(),
        })
        .collect();
    let name = format!("{} ({}) from {dets}", d.model, d.input);
    println!(
        "{name}: {} keyframes ({skipped} in repeated spans left out), {absent} of them without an entry (no \
         detections), {} other frames ignored",
        frames.len(),
        found.len()
    );
    if !frames.is_empty() && absent == frames.len() {
        assert!(
            found.is_empty(),
            "{dets}: no keyframe has an entry but {} other frames do: do both files use the same frame grid \
             (project offsets)?",
            found.len()
        );
        println!("{dets}: no detections on any keyframe");
    }
    (frames, name)
}

fn iou(a: &[f64], b: &[f64]) -> f64 {
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - w * h;
    if union > 0.0 { w * h / union } else { 0.0 }
}

/// Greedy one-to-one matching, best score first: a detection takes the free
/// character head it overlaps most at IoU ≥ 0.5, else the free head of
/// another class it overlaps most (so a poster or a second box never takes a
/// detection from a character, as COCO prefers boxes it doesn't ignore); one
/// that overlaps only taken heads is a duplicate of the one it overlaps most.
/// Returns `(score, hit)` best first. A hit depends only on better
/// detections, so raising the threshold keeps a prefix with the same hits.
fn match_frame(heads: &[Head], dets: &[[f64; 5]]) -> Vec<(f64, Hit)> {
    let mut order: Vec<&[f64; 5]> = dets.iter().collect();
    order.sort_by(|a, b| b[4].total_cmp(&a[4]));
    let mut taken = vec![false; heads.len()];
    let mut hits = Vec::with_capacity(order.len());
    for d in order {
        // The head it overlaps most of those `pick(taken, character)` allows.
        let best = |pick: fn(bool, bool) -> bool| {
            heads
                .iter()
                .enumerate()
                .filter(|&(i, h)| pick(taken[i], character(h).is_some()))
                .map(|(i, h)| (i, iou(&h.bbox, &d[..4])))
                .filter(|&(_, v)| v >= IOU)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        let hit = match (best(|t, c| !t && c), best(|t, _| !t), best(|t, _| t)) {
            (Some(i), _, _) | (None, Some(i), _) => Hit::Head(i),
            (None, None, Some(i)) => Hit::Duplicate(i),
            (None, None, None) => Hit::Stray,
        };
        if let Hit::Head(i) = hit {
            taken[i] = true;
        }
        hits.push((d[4], hit));
    }
    hits
}

/// The strata a character head counts in.
fn strata(h: &Head, crowd: bool) -> Vec<&'static str> {
    let mut s: Vec<&'static str> = MARKED
        .iter()
        .filter(|(name, class)| h.flags.iter().any(|f| f == name) || *class == Some(h.class.as_str()))
        .map(|(name, _)| *name)
        .collect();
    // Heights are differences of decimals: 0.43 − 0.1 falls just under 0.33.
    let height = h.bbox[3] - h.bbox[1] + 1e-9;
    s.push(if height < 0.08 {
        "small"
    } else if height >= 0.33 {
        "close"
    } else {
        "mid"
    });
    if crowd {
        s.push("crowd");
    }
    s
}

#[derive(Debug, Default)]
struct Tally {
    /// Character heads `(found, all)`: overall, per class, per stratum.
    all: (usize, usize),
    class: BTreeMap<&'static str, (usize, usize)>,
    stratum: BTreeMap<&'static str, (usize, usize)>,
    /// False alarms: on no head, second ones on a head, and per class of a
    /// head that isn't a character (`S:<id>` as `S`).
    stray: usize,
    duplicate: usize,
    other: BTreeMap<String, usize>,
}

impl Tally {
    fn false_alarms(&self) -> usize {
        self.stray + self.duplicate + self.other.values().sum::<usize>()
    }

    /// `(right, all)` detections kept.
    fn precision(&self) -> (usize, usize) {
        (self.all.0, self.all.0 + self.false_alarms())
    }
}

/// Counts at `threshold` from each frame's [`match_frame`].
fn tally(frames: &[Frame], hits: &[Vec<(f64, Hit)>], threshold: f64) -> Tally {
    let mut t = Tally::default();
    for (f, hits) in frames.iter().zip(hits) {
        let kept: Vec<Hit> = hits.iter().take_while(|h| h.0 >= threshold).map(|h| h.1).collect();
        let crowd = f.characters() >= 5;
        for (i, h) in f.heads.iter().enumerate() {
            let Some(class) = character(h) else { continue };
            let got = usize::from(kept.contains(&Hit::Head(i)));
            for n in [&mut t.all, t.class.entry(class).or_default()] {
                *n = (n.0 + got, n.1 + 1);
            }
            for s in strata(h, crowd) {
                let n = t.stratum.entry(s).or_default();
                *n = (n.0 + got, n.1 + 1);
            }
        }
        for hit in kept.into_iter().filter(|&h| !ignored(f, h)) {
            match hit {
                Hit::Head(i) if character(&f.heads[i]).is_none() => {
                    let class = &f.heads[i].class;
                    let class = class.split_once(':').map_or(class.as_str(), |(c, _)| c);
                    *t.other.entry(class.to_string()).or_default() += 1
                }
                Hit::Head(_) => {}
                Hit::Duplicate(_) => t.duplicate += 1,
                Hit::Stray => t.stray += 1,
            }
        }
    }
    t
}

/// Every detection best first, as `(score, frame, found a character head)`.
fn ranked(frames: &[Frame], hits: &[Vec<(f64, Hit)>]) -> Vec<(f64, usize, bool)> {
    let mut r: Vec<(f64, usize, bool)> = hits
        .iter()
        .enumerate()
        .flat_map(|(f, hits)| {
            hits.iter()
                .filter(move |&&(_, hit)| !ignored(&frames[f], hit))
                .map(move |&(s, hit)| (s, f, matches!(hit, Hit::Head(i) if character(&frames[f].heads[i]).is_some())))
        })
        .collect();
    r.sort_by(|a, b| b.0.total_cmp(&a.0));
    r
}

/// A project's keyframes and its shots (`start..end`, sorted).
type Project<'a> = (&'a [Frame], Option<&'a [(u32, u32)]>);

/// Keyframes of one or more projects, numbered across them.
struct Layout {
    /// Per frame, its project and its character heads.
    project: Vec<usize>,
    heads: Vec<f64>,
    /// Per project its shots, per shot its frames: the bootstrap's clusters.
    shots: Vec<Vec<Vec<usize>>>,
}

impl Layout {
    /// A keyframe in no shot (every keyframe, without shots) is a cluster of
    /// its own.
    fn new(projects: &[Project]) -> Self {
        let mut l = Self { project: Vec::new(), heads: Vec::new(), shots: Vec::new() };
        for (p, &(frames, shots)) in projects.iter().enumerate() {
            let mut clusters: BTreeMap<(bool, u32), Vec<usize>> = BTreeMap::new();
            for f in frames {
                let shot = shots.and_then(|s| {
                    let i = s.partition_point(|r| r.0 <= f.at).checked_sub(1)?;
                    (f.at < s[i].1).then_some(i as u32)
                });
                clusters.entry(shot.map_or((false, f.at), |i| (true, i))).or_default().push(l.project.len());
                l.project.push(p);
                l.heads.push(f.characters() as f64);
            }
            l.shots.push(clusters.into_values().collect());
        }
        l
    }

    /// Draw counts per frame for one resample: each project's shots drawn
    /// with replacement, as many as it has.
    fn draw(&self, rng: &mut Rng) -> Vec<u32> {
        let mut w = vec![0; self.project.len()];
        for shots in &self.shots {
            for _ in 0..shots.len() {
                for &f in &shots[rng.below(shots.len())] {
                    w[f] += 1;
                }
            }
        }
        w
    }
}

/// The highest macro mean recall over the projects (one without character
/// heads left out) at a threshold where every project keeps precision ≥ `p`,
/// and the highest threshold that reaches it, with frame `f` counted
/// `weight[f]` times (1 for the sample itself, its draw count in a
/// bootstrap). `None` when no threshold finds a head that way.
fn at_precision(ranked: &[(f64, usize, bool)], layout: &Layout, weight: &[u32], p: f64) -> Option<(f64, f64)> {
    let n = layout.shots.len();
    let mut heads = vec![0.0; n];
    for (f, (&j, &h)) in layout.project.iter().zip(&layout.heads).enumerate() {
        heads[j] += h * f64::from(weight[f]);
    }
    let counted = heads.iter().filter(|&&h| h > 0.0).count() as f64;
    let (mut right, mut wrong, mut best) = (vec![0.0; n], vec![0.0; n], None::<(f64, f64)>);
    for (i, &(s, f, ok)) in ranked.iter().enumerate() {
        let (w, j) = (f64::from(weight[f]), layout.project[f]);
        if ok {
            right[j] += w;
        } else {
            wrong[j] += w;
        }
        // A threshold keeps all detections of one score or none of them.
        if ranked.get(i + 1).is_some_and(|n| n.0 == s) {
            continue;
        }
        let precise = (0..n).all(|j| right[j] >= p * (right[j] + wrong[j]) - 1e-9);
        let recall = (0..n).filter(|&j| heads[j] > 0.0).map(|j| right[j] / heads[j]).sum::<f64>() / counted;
        if precise && recall > 0.0 && best.is_none_or(|b| recall > b.0) {
            best = Some((recall, s));
        }
    }
    best
}

/// Wilson score interval (95 %) of `k` out of `n`.
fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (z, n, p) = (1.959964, n as f64, k as f64 / n as f64);
    let d = 1.0 + z * z / n;
    let centre = (p + z * z / (2.0 * n)) / d;
    let half = z / d * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt();
    ((centre - half).max(0.0), (centre + half).min(1.0))
}

fn share((k, n): (usize, usize)) -> String {
    if n == 0 {
        return "–".into();
    }
    let (lo, hi) = wilson(k, n);
    format!("{:.3} [{lo:.3}, {hi:.3}] ({k}/{n})", k as f64 / n as f64)
}

/// `NAME=<a>,<b>,…`: one file per project, or none.
fn per_project(name: &str, projects: usize) -> Vec<Option<String>> {
    match std::env::var(name) {
        Err(_) => vec![None; projects],
        Ok(v) => {
            let files: Vec<Option<String>> = v.split(',').map(|f| Some(f.to_string())).collect();
            assert_eq!(files.len(), projects, "{name}: one file per project, comma-separated");
            files
        }
    }
}

fn spans_of(file: Option<&str>, keyframes: &str) -> Spans {
    file.map_or_else(
        || {
            println!("{keyframes}: no SPANS, so keyframes in OP/ED and other repeated spans count");
            Spans::default()
        },
        Spans::read,
    )
}

fn score(a: &[String]) {
    let spans = spans_of(per_project("SPANS", 1)[0].as_deref(), &a[0]);
    let (all, _) = load(&a[0], &a[1], &spans);
    // Overall numbers come from the random keyframes alone; strata from all,
    // targeted frames included (README: rare strata are filled on purpose).
    let (targeted, frames): (Vec<Frame>, Vec<Frame>) = all.into_iter().partition(|f| f.targeted);
    let threshold: Option<f64> = a.get(2).map(|v| v.parse().unwrap());
    let hits: Vec<_> = frames.iter().map(|f| match_frame(&f.heads, &f.dets)).collect();
    let extra: Vec<_> = targeted.iter().map(|f| match_frame(&f.heads, &f.dets)).collect();
    let report = |t: &Tally| {
        let other: String = t.other.iter().map(|(c, n)| format!(", {n} on {c}")).collect();
        println!("  recall    {} (random keyframes)", share(t.all));
        println!("  precision {}", share(t.precision()));
        println!("  false alarms {}: {} on no head, {} duplicates{other}", t.false_alarms(), t.stray, t.duplicate);
    };
    let t = tally(&frames, &hits, threshold.unwrap_or(f64::NEG_INFINITY));
    match threshold {
        Some(s) => println!("at score ≥ {s}:"),
        None => println!("every detection:"),
    }
    report(&t);
    println!("  by class:");
    for c in HEADS {
        println!("    {c:<8} {}", share(t.class.get(c).copied().unwrap_or_default()));
    }
    println!("  by stratum (random and {} targeted keyframes):", targeted.len());
    let te = tally(&targeted, &extra, threshold.unwrap_or(f64::NEG_INFINITY));
    for s in STRATA {
        let (a, b) = (t.stratum.get(s).copied().unwrap_or_default(), te.stratum.get(s).copied().unwrap_or_default());
        println!("    {s:<8} {}", share((a.0 + b.0, a.1 + b.1)));
    }

    let ranked = ranked(&frames, &hits);
    let layout = Layout::new(&[(&frames, None)]);
    for p in [0.95, 0.97] {
        match at_precision(&ranked, &layout, &vec![1; frames.len()], p) {
            Some((_, s)) => {
                println!("precision ≥ {p}: at score ≥ {s}");
                report(&tally(&frames, &hits, s));
            }
            None => println!("precision ≥ {p}: reached at no threshold"),
        }
    }
}

/// SplitMix64: a small seeded generator, so a bootstrap repeats exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next()) * n as u128) >> 64) as usize
    }
}

/// A paired bootstrap of the recall gap first − second at a precision.
struct Boot {
    /// The gap on the sample itself.
    full: f64,
    /// The gap of each resample, sorted.
    gaps: Vec<f64>,
    /// Resamples where a variant misses the precision (its recall counts 0).
    short: usize,
}

/// `boot` resamples of [`Layout::draw`], the threshold of each variant chosen
/// anew in each by [`at_precision`] at `p`.
fn bootstrap(ranks: [&[(f64, usize, bool)]; 2], layout: &Layout, p: f64, boot: usize, seed: u64) -> Boot {
    let gap = |w: &[u32]| {
        let [first, second] = ranks.map(|r| at_precision(r, layout, w, p));
        (first.map_or(0.0, |x| x.0) - second.map_or(0.0, |x| x.0), first.is_none() || second.is_none())
    };
    let full = gap(&vec![1; layout.project.len()]).0;
    let (mut rng, mut short) = (Rng(seed), 0);
    let mut gaps: Vec<f64> = (0..boot)
        .map(|_| {
            let (g, missed) = gap(&layout.draw(&mut rng));
            short += usize::from(missed);
            g
        })
        .collect();
    gaps.sort_by(f64::total_cmp);
    Boot { full, gaps, short }
}

/// The `x` quantile of sorted `v`, at the nearest rank.
fn quantile(v: &[f64], x: f64) -> f64 {
    v[(x * (v.len() - 1) as f64).round() as usize]
}

/// E1's call on the second variant, from the gap on the sample itself and the
/// upper 95 % bound (both in points): within 2 points of the first when the
/// bound is below 2. No call when a variant misses the precision, or when the
/// first isn't ahead (the variants came in the wrong order).
fn verdict(full: f64, upper: f64, reached: bool) -> &'static str {
    if !reached {
        "no verdict, a variant misses the precision"
    } else if full < 0.0 {
        "no verdict: first isn't the best, swap them"
    } else if upper < 2.0 {
        "yes"
    } else {
        "no"
    }
}

fn paired(a: &[String]) {
    assert!(
        a.len() >= 3 && a.len() % 3 != 2,
        "paired <keyframes.json> <best.dets.json> <other.dets.json> [<keyframes.json> <best> <other> …] [precision]"
    );
    let p: f64 = if a.len() % 3 == 1 { a[a.len() - 1].parse().unwrap() } else { 0.95 };
    let boot: usize = std::env::var("BOOT").map_or(2000, |v| v.parse().unwrap());
    assert!(boot >= 1, "BOOT={boot}: at least one resample");
    let seed: u64 = std::env::var("SEED").map_or(1, |v| v.parse().unwrap());
    let (projects, _) = a.as_chunks::<3>();
    let (spans, segments) = (per_project("SPANS", projects.len()), per_project("SEGMENTS", projects.len()));
    // Both variants read the same keyframes, so frame `i` is the same in each.
    let mut runs: [Vec<Frame>; 2] = Default::default();
    let mut names: [Vec<String>; 2] = Default::default();
    let mut parts = Vec::new();
    for (j, x) in projects.iter().enumerate() {
        let spans = spans_of(spans[j].as_deref(), &x[0]);
        let start = runs[0].len();
        for v in 0..2 {
            let (frames, name) = load(&x[0], &x[1 + v], &spans);
            let frames: Vec<Frame> = frames.into_iter().filter(|f| !f.targeted).collect();
            runs[v].extend(frames);
            names[v].push(name);
        }
        parts.push((start..runs[0].len(), segments[j].as_deref().map(read_shots)));
    }
    let layout = Layout::new(&parts.iter().map(|(r, s)| (&runs[0][r.clone()], s.as_deref())).collect::<Vec<_>>());
    let hits = runs.each_ref().map(|frames| frames.iter().map(|f| match_frame(&f.heads, &f.dets)).collect::<Vec<_>>());
    let ranks = [0, 1].map(|v| ranked(&runs[v], &hits[v]));
    let one = vec![1; layout.project.len()];
    let mut reached = true;
    for (v, word) in ["first", "second"].into_iter().enumerate() {
        match at_precision(&ranks[v], &layout, &one, p) {
            Some((rec, s)) => {
                println!(
                    "{word}: macro mean recall {rec:.4} over {} projects (random keyframes of each) at \
                     precision ≥ {p} in each, score ≥ {s}",
                    projects.len()
                );
                for ((r, _), name) in parts.iter().zip(&names[v]) {
                    let t = tally(&runs[v][r.clone()], &hits[v][r.clone()], s);
                    println!("  {name}: recall {}, precision {}", share(t.all), share(t.precision()));
                }
            }
            None => {
                println!("{word}: precision {p} in every project reached at no threshold");
                reached = false;
            }
        }
    }
    let b = bootstrap([&ranks[0], &ranks[1]], &layout, p, boot, seed);
    let q = |x: f64| 100.0 * quantile(&b.gaps, x);
    let upper = q(0.95);
    let clusters: usize = layout.shots.iter().map(Vec::len).sum();
    let by = if segments.iter().all(Option::is_some) {
        "shots"
    } else {
        "shots (keyframes where SEGMENTS is missing: too narrow when they share a shot)"
    };
    println!(
        "gap first − second: {:+.2} points; bootstrap of {boot} over {clusters} {by}, drawn within each project \
         (seed {seed}, {} resamples where a variant misses precision {p}): 95 % interval [{:+.2}, {:+.2}], upper \
         95 % bound {upper:+.2}",
        100.0 * b.full,
        b.short,
        q(0.025),
        q(0.975)
    );
    println!("second within 2 points of first (upper bound < 2): {}", verdict(100.0 * b.full, upper, reached));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(bbox: [f64; 4], class: &str, flags: &[&str]) -> Head {
        Head { bbox, class: class.into(), flags: flags.iter().map(|f| f.to_string()).collect() }
    }

    fn frame(heads: &[Head], dets: &[[f64; 5]]) -> Frame {
        Frame { at: 0, targeted: false, heads: heads.to_vec(), dets: dets.to_vec() }
    }

    /// Frames `0..` at frames `at`, frame `i` with `heads[i]` character heads.
    fn frames(at: &[u32], heads: &[usize]) -> Vec<Frame> {
        at.iter()
            .zip(heads)
            .map(|(&at, &n)| Frame {
                at,
                targeted: false,
                heads: vec![head([0.0, 0.0, 0.1, 0.1], "H", &[]); n],
                dets: vec![],
            })
            .collect()
    }

    #[test]
    fn iou_of_boxes() {
        assert_eq!(iou(&[0.0, 0.0, 0.2, 0.2], &[0.0, 0.0, 0.2, 0.2]), 1.0);
        assert_eq!(iou(&[0.0, 0.0, 0.1, 0.1], &[0.2, 0.2, 0.3, 0.3]), 0.0);
        assert!((iou(&[0.0, 0.0, 0.2, 0.1], &[0.1, 0.0, 0.3, 0.1]) - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn best_score_takes_the_head_and_later_ones_are_duplicates() {
        let heads = [head([0.1, 0.1, 0.3, 0.3], "H", &[]), head([0.6, 0.1, 0.8, 0.3], "H", &[])];
        // The weaker first detection fits head 0 better, but the stronger one comes first.
        let dets = [[0.1, 0.1, 0.3, 0.3, 0.4], [0.11, 0.11, 0.31, 0.31, 0.9], [0.6, 0.1, 0.8, 0.3, 0.5]];
        let hits = match_frame(&heads, &dets);
        assert_eq!(hits, vec![(0.9, Hit::Head(0)), (0.5, Hit::Head(1)), (0.4, Hit::Duplicate(0))]);
        let frames = [frame(&heads, &dets)];
        let t = tally(&frames, std::slice::from_ref(&hits), f64::NEG_INFINITY);
        assert_eq!((t.all, t.duplicate, t.stray, t.precision()), ((2, 2), 1, 0, (2, 3)));
        // A threshold above the duplicate drops it and keeps both heads.
        let t = tally(&frames, &[hits], 0.45);
        assert_eq!((t.all, t.false_alarms()), ((2, 2), 0));
    }

    #[test]
    fn a_free_head_beats_a_taken_one() {
        // Two overlapping heads: the second detection takes the free one
        // although it overlaps the taken one more.
        let heads = [head([0.0, 0.0, 0.2, 0.2], "H", &[]), head([0.02, 0.0, 0.22, 0.2], "Hb", &[])];
        let dets = [[0.0, 0.0, 0.2, 0.2, 0.9], [0.005, 0.0, 0.205, 0.2, 0.8]];
        assert_eq!(match_frame(&heads, &dets), vec![(0.9, Hit::Head(0)), (0.8, Hit::Head(1))]);
    }

    #[test]
    fn of_two_free_heads_the_closer_one_wins_wherever_it_is_listed() {
        // IoU 0.82 with the first head, 1 with the second.
        let heads = [head([0.0, 0.0, 0.2, 0.2], "H", &[]), head([0.02, 0.0, 0.22, 0.2], "H", &[])];
        assert_eq!(match_frame(&heads, &[[0.02, 0.0, 0.22, 0.2, 0.9]]), vec![(0.9, Hit::Head(1))]);
    }

    #[test]
    fn a_character_head_comes_before_other_boxes() {
        // IoU 0.74 with the character's head, 0.90 with a second box on it:
        // the detection finds the character, the next one lands on the box.
        let heads = [head([0.0, 0.0, 0.2, 0.2], "H", &[]), head([0.02, 0.0, 0.22, 0.2], "S:1", &[])];
        let dets = [[0.03, 0.0, 0.23, 0.2, 0.9], [0.02, 0.0, 0.22, 0.2, 0.8]];
        let hits = match_frame(&heads, &dets);
        assert_eq!(hits, vec![(0.9, Hit::Head(0)), (0.8, Hit::Head(1))]);
        let t = tally(&[frame(&heads, &dets)], &[hits], f64::NEG_INFINITY);
        // Second boxes count together as `S`.
        assert_eq!((t.all, t.other.get("S").copied()), ((1, 1), Some(1)));
    }

    #[test]
    fn posters_and_strays_are_false_alarms() {
        let heads = [head([0.1, 0.1, 0.3, 0.3], "O", &[]), head([0.5, 0.5, 0.7, 0.7], "Hk", &["chibi"])];
        let dets = [[0.1, 0.1, 0.3, 0.3, 0.9], [0.8, 0.8, 0.9, 0.9, 0.8], [0.52, 0.52, 0.72, 0.72, 0.7]];
        let hits = match_frame(&heads, &dets);
        assert_eq!(hits, vec![(0.9, Hit::Head(0)), (0.8, Hit::Stray), (0.7, Hit::Head(1))]);
        let frames = [frame(&heads, &dets)];
        let t = tally(&frames, std::slice::from_ref(&hits), f64::NEG_INFINITY);
        assert_eq!((t.all, t.stray, t.other.get("O").copied()), ((1, 1), 1, Some(1)));
        assert_eq!(t.precision(), (1, 3));
        assert_eq!((t.class.get("Hk"), t.class.get("H")), (Some(&(1, 1)), None));
        assert_eq!(ranked(&frames, &[hits]).iter().map(|r| r.2).collect::<Vec<_>>(), [false, false, true]);
    }

    #[test]
    fn strata_by_flag_class_size_and_crowd() {
        assert_eq!(strata(&head([0.0, 0.0, 0.1, 0.05], "Hb", &["back", "flashback"]), false), ["back", "small"]);
        assert_eq!(strata(&head([0.0, 0.0, 0.1, 0.08], "H", &[]), true), ["mid", "crowd"]);
        assert_eq!(strata(&head([0.0, 0.5, 0.3, 0.84], "Hm", &["mask", "graded"]), false), ["mask", "graded", "close"]);
        // The class alone: the back of a head, chibi, a creature; not a mask.
        assert_eq!(strata(&head([0.0, 0.0, 0.1, 0.1], "Hb", &[]), false), ["back", "mid"]);
        assert_eq!(strata(&head([0.0, 0.0, 0.1, 0.1], "Hk", &[]), false), ["chibi", "mid"]);
        assert_eq!(strata(&head([0.0, 0.0, 0.1, 0.1], "Hn", &[]), false), ["creature", "mid"]);
        assert_eq!(strata(&head([0.0, 0.0, 0.1, 0.1], "Hm", &[]), false), ["mid"]);
        // Heights on the bounds, whatever the float subtraction leaves.
        assert_eq!(strata(&head([0.0, 0.1, 0.1, 0.43], "H", &[]), false), ["close"]);
        assert_eq!(strata(&head([0.0, 0.07, 0.1, 0.4], "H", &[]), false), ["close"]);
        assert_eq!(strata(&head([0.0, 0.1, 0.1, 0.42], "H", &[]), false), ["mid"]);
        assert_eq!(strata(&head([0.0, 0.04, 0.1, 0.12], "H", &[]), false), ["mid"]);
        assert_eq!(strata(&head([0.0, 0.04, 0.1, 0.119], "H", &[]), false), ["small"]);
        // A crowd counts character heads only: four characters and a poster aren't one.
        let small = |x: f64, class: &str| head([x, 0.0, x + 0.05, 0.1], class, &[]);
        let mut heads: Vec<Head> = (0..4).map(|i| small(0.1 * f64::from(i), "H")).collect();
        heads.push(small(0.5, "O"));
        let t = tally(&[frame(&heads, &[])], &[vec![]], 0.0);
        assert_eq!((t.all, t.stratum.get("crowd")), ((0, 4), None));
        heads.push(small(0.6, "Hn"));
        let t = tally(&[frame(&heads, &[])], &[vec![]], 0.0);
        assert_eq!((t.all, t.stratum.get("crowd"), t.stratum.get("mid")), ((0, 5), Some(&(0, 5)), Some(&(0, 5))));
    }

    #[test]
    fn wilson_interval() {
        let (lo, hi) = wilson(8, 10);
        assert!((lo - 0.4902).abs() < 1e-4 && (hi - 0.9433).abs() < 1e-4, "{lo} {hi}");
        let (lo, hi) = wilson(0, 10);
        assert!(lo == 0.0 && (hi - 0.2775).abs() < 1e-4, "{hi}");
        let (lo, hi) = wilson(10, 10);
        assert!((lo - 0.7225).abs() < 1e-4 && (hi - 1.0).abs() < 1e-12, "{lo} {hi}");
        assert_eq!(wilson(0, 0), (0.0, 1.0));
        assert_eq!(share((0, 0)), "–");
    }

    #[test]
    fn recall_at_precision_cuts_between_scores() {
        // Best first: right, right, wrong, then a right and a wrong one tied.
        let r = [(0.9, 0, true), (0.8, 0, true), (0.7, 1, false), (0.6, 1, true), (0.6, 1, false)];
        let layout = Layout::new(&[(&frames(&[0, 1], &[2, 2]), None)]);
        let one = [1, 1];
        // Precision 1, 1, 2/3, then 3/5 with the tie taken whole.
        assert_eq!(at_precision(&r, &layout, &one, 0.95), Some((0.5, 0.8)));
        assert_eq!(at_precision(&r, &layout, &one, 0.6), Some((0.75, 0.6)));
        // Split, the tie would give 3/4 at 0.6.
        assert_eq!(at_precision(&r, &layout, &one, 0.7), Some((0.5, 0.8)));
        assert_eq!(at_precision(&r[2..3], &layout, &one, 0.5), None);
        // Frame 1 drawn twice and frame 0 never: 2 right out of 6 at best.
        assert_eq!(at_precision(&r, &layout, &[0, 2], 0.5), None);
        assert_eq!(at_precision(&r, &layout, &[0, 2], 0.3), Some((0.5, 0.6)));
    }

    #[test]
    fn recall_is_a_macro_mean_with_precision_in_every_project() {
        // Project 0 (frame 0, 2 heads), project 1 (frame 1, 4 heads).
        let (a, b) = (frames(&[0], &[2]), frames(&[0], &[4]));
        let layout = Layout::new(&[(&a, None), (&b, None)]);
        let r = [(0.9, 0, true), (0.8, 1, false), (0.7, 0, true), (0.6, 1, true)];
        // At 0.6 project 1 has 1/2: enough for 0.5, the mean of 2/2 and 1/4.
        assert_eq!(at_precision(&r, &layout, &[1, 1], 0.5), Some((0.625, 0.6)));
        // Pooled, 3/4 would reach 0.6 at 0.6; project 1 alone doesn't.
        assert_eq!(at_precision(&r, &layout, &[1, 1], 0.6), Some((0.25, 0.9)));
    }

    #[test]
    fn shots_are_drawn_whole_within_each_project() {
        let (a, b) = (frames(&[10, 20, 30], &[1, 1, 1]), frames(&[5], &[1]));
        // Frames 10 and 20 share a shot, 30 is in none, project 1 has no shots.
        let layout = Layout::new(&[(&a, Some(&[(0, 25), (40, 50)])), (&b, None)]);
        assert_eq!(layout.shots, vec![vec![vec![2], vec![0, 1]], vec![vec![3]]]);
        let mut rng = Rng(7);
        for _ in 0..100 {
            let w = layout.draw(&mut rng);
            assert_eq!((w[0], w[0] + w[2], w[3]), (w[1], 2, 1), "{w:?}");
        }
    }

    #[test]
    fn bootstrap_counts_a_missing_variant_as_recall_zero() {
        let layout = Layout::new(&[(&frames(&[0, 1, 2, 3], &[1, 1, 1, 1]), None)]);
        let all = [(0.9, 0, true), (0.8, 1, true), (0.7, 2, true), (0.6, 3, true)];
        let b = bootstrap([&all, &[]], &layout, 0.95, 50, 1);
        assert_eq!((b.full, b.short, b.gaps.len()), (1.0, 50, 50));
        assert!(b.gaps.iter().all(|&g| g == 1.0));
        let b = bootstrap([&all, &all], &layout, 0.95, 50, 1);
        assert_eq!((b.full, b.short), (0.0, 0));
        assert!(b.gaps.iter().all(|&g| g == 0.0));
        // Seeded: the same draws again; and the gaps are sorted.
        let half = [(0.9, 0, true), (0.8, 1, true)];
        let (x, y) = (bootstrap([&all, &half], &layout, 0.95, 200, 3), bootstrap([&all, &half], &layout, 0.95, 200, 3));
        assert_eq!((x.full, &x.gaps), (0.5, &y.gaps));
        assert!(x.gaps.windows(2).all(|w| w[0] <= w[1]) && x.gaps[0] < x.gaps[199]);
    }

    #[test]
    fn quantiles_and_verdict() {
        let v: Vec<f64> = (0..=100).map(f64::from).collect();
        assert_eq!([0.025, 0.95, 0.975].map(|x| quantile(&v, x)), [3.0, 95.0, 98.0]);
        assert_eq!(verdict(1.0, 1.9, true), "yes");
        assert_eq!(verdict(0.0, 2.0, true), "no");
        assert_eq!(verdict(-50.0, 0.0, true), "no verdict: first isn't the best, swap them");
        assert_eq!(verdict(1.0, 1.0, false), "no verdict, a variant misses the precision");
    }

    #[test]
    fn rng_is_seeded_and_in_range() {
        let draw = |seed| {
            let mut r = Rng(seed);
            (0..1000).map(|_| r.below(7)).collect::<Vec<_>>()
        };
        assert_eq!(draw(1), draw(1));
        assert_ne!(draw(1), draw(2));
        assert!(draw(3).iter().all(|&x| x < 7));
        assert!((0..7).all(|v| draw(3).contains(&v)));
    }

    /// `load` on these files, with `spans`.
    fn load_json(keyframes: &str, dets: &str, spans: Spans) -> Vec<Frame> {
        let dir = tempfile::tempdir().unwrap();
        let (k, d) = (dir.path().join("k.json"), dir.path().join("d.json"));
        std::fs::write(&k, keyframes).unwrap();
        std::fs::write(&d, dets).unwrap();
        load(k.to_str().unwrap(), d.to_str().unwrap(), &spans).0
    }

    const ONE: &str = r#"{"frames": {"5": {"heads": [{"box": [0.1, 0.2, 0.2, 0.4], "class": "H"}]}}}"#;

    #[test]
    fn load_leaves_out_repeated_spans() {
        let k = r#"{"frames": {"5": {"heads": [{"box": [0.1, 0.2, 0.2, 0.4], "class": "S:2"}]},
            "150": {"heads": [{"box": [0.1, 0.2, 0.2, 0.4], "class": "Hb", "flags": ["back"]}]}}}"#;
        let d = r#"{"frames": {"5": [[0.1, 0.2, 0.2, 0.4, 0.9]], "150": [[0.1, 0.2, 0.2, 0.4, 0.9]], "7": []}}"#;
        let f = load_json(k, d, Spans(vec![(100, 200, "op".into())]));
        assert_eq!(f.iter().map(|f| (f.at, f.dets.len())).collect::<Vec<_>>(), [(5, 1)]);
        assert!(known_class("N") && known_class("S:2") && !known_class("S:") && !known_class("h"));
    }

    #[test]
    #[should_panic(expected = "unknown class")]
    fn load_refuses_an_unknown_class() {
        load_json(&ONE.replace("\"H\"", "\"h\""), r#"{"frames": {}}"#, Spans::default());
    }

    #[test]
    #[should_panic(expected = "unknown flag")]
    fn load_refuses_an_unknown_flag() {
        load_json(&ONE.replace("\"H\"", "\"H\", \"flags\": [\"bakc\"]"), r#"{"frames": {}}"#, Spans::default());
    }

    #[test]
    #[should_panic(expected = "shares of the frame")]
    fn load_refuses_boxes_in_pixels() {
        load_json(ONE, r#"{"frames": {"5": [[100, 50, 200, 150, 0.9]]}}"#, Spans::default());
    }

    #[test]
    #[should_panic(expected = "same frame grid")]
    fn load_refuses_detections_on_another_frame_grid() {
        load_json(ONE, r#"{"frames": {"1005": [[0.1, 0.2, 0.2, 0.4, 0.9]]}}"#, Spans::default());
    }
    #[test]
    fn ignore_regions_count_neither_way() {
        let f = frame(
            &[head([0.0, 0.0, 0.1, 0.1], "I", &[]), head([0.5, 0.5, 0.6, 0.6], "H", &[])],
            &[[0.0, 0.0, 0.1, 0.1, 0.9], [0.0, 0.0, 0.1, 0.1, 0.8], [0.5, 0.5, 0.6, 0.6, 0.7]],
        );
        let hits = vec![match_frame(&f.heads, &f.dets)];
        let frames = [f];
        let t = tally(&frames, &hits, 0.0);
        assert_eq!((t.all, t.false_alarms()), ((1, 1), 0));
        assert_eq!(ranked(&frames, &hits).len(), 1);
    }
}
