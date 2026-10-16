//! Dev tool: what bodies add to the face pass on a labelled episode
//! (`bench/bodies`) — how many labelled faces sit on a detected body, how
//! clean body tracks are, how many faces their bodies name, how well
//! clothes find a person again, and how much more of the cast is found.
//! `cargo run --release -p as-analysis --example bodies_bench -- <cmd> <faces.tracks> <run.bodies> <bb_dir>
//! <labels.tsv> [out_dir]` with `<cmd>`: `recall`, `tracks`, `score`, `reid`, `presence`, `sheets`.
//! `bodies_bench audit <items.tsv> <verdicts.tsv>` scores the verdicts given on the sheets.
//!
//! Env: `BODY_STRIDE=3|6|12` uses every 1st/2nd/4th body sample; `HALF=first|second` keeps
//! the queries of one half of the episode (`reid`); `WINDOW_S` (180), `REID_MIN` (0.65),
//! `REID_MARGIN` (0.15), `RESCUE_UNSURE` (1), `RESCUE_BELOW` (0.45) set the propagation;
//! `VETO`, `FAR_COS` the tracker; `DETAIL=1` lists cases.

#[path = "common/bodyfile.rs"]
mod bodyfile;
#[path = "common/labels.rs"]
mod labels;

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use as_analysis::bodies::{BodyDet, BodyObs, BodyTrack};
use as_analysis::body_tracker::{BodyTrackerParams, track_bodies};
use as_analysis::link::{LinkParams, fit, link_shot};
use as_analysis::tracks::{self, FaceTrack};
use as_core::FrameRange;
use as_identity::bodies::{BodyAssign, BodyIn, BodyOutcome, BodyParams, FaceOn, propagate};
use as_identity::metrics::bcubed;
use as_identity::{Outcome, Params, TrackIn, identify};
use labels::{FPS, Label, STRIDE, character_names, found_faces, input, label_screen, members, names_of};

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().map_or(default, |v| v.parse().ok().unwrap_or_else(|| panic!("{name}")))
}

fn detail() -> bool {
    std::env::var_os("DETAIL").is_some()
}

/// Tiny deterministic random numbers (xorshift64*), for samples to look at.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn pick<T: Clone>(&mut self, v: &[T], n: usize) -> Vec<T> {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        for i in (1..idx.len()).rev() {
            idx.swap(i, (self.next() % (i as u64 + 1)) as usize);
        }
        let mut idx: Vec<usize> = idx.into_iter().take(n).collect();
        idx.sort_unstable();
        idx.into_iter().map(|i| v[i].clone()).collect()
    }
}

/// Everything derived once per run.
struct World<'a> {
    tracks: Vec<FaceTrack>,
    input: Vec<TrackIn>,
    outcome: Outcome,
    got: Vec<Option<usize>>,
    truth: Vec<Option<&'a str>>,
    char_name: HashMap<usize, &'a str>,
    /// Main cast: named in the labels with 30 s or more.
    main: HashSet<&'a str>,
    shots: Vec<FrameRange>,
    samples: Vec<bodyfile::Sample>,
    body_stride: u32,
    bodies: Vec<BodyTrack>,
    /// Per body: (face track, samples tied).
    faces_on: Vec<Vec<(usize, u32)>>,
    body_in: Vec<BodyIn>,
    res: BodyOutcome,
    params: BodyParams,
}

fn read_shots(dir: &Path) -> Vec<FrameRange> {
    std::fs::read_to_string(dir.join("bb.segments.tsv"))
        .unwrap()
        .lines()
        .filter_map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            (v[2].trim() == "shot").then(|| FrameRange::new(v[0].parse().unwrap(), v[1].parse().unwrap()))
        })
        .collect()
}

/// The propagation as the study recommends it, each value overridable.
fn body_params() -> BodyParams {
    BodyParams {
        rescue_unsure: env("RESCUE_UNSURE", 1) != 0,
        rescue_below: env("RESCUE_BELOW", Params::default().core_quality),
        window: (env("WINDOW_S", 180.0) * FPS) as u32,
        reid_min: env("REID_MIN", 0.65),
        reid_margin: env("REID_MARGIN", 0.15),
    }
}

fn build<'a>(tracks_path: &str, bodies_path: &str, dir: &Path, labels: &'a [Label]) -> World<'a> {
    let tracks = labels::read_tracks(tracks_path);
    let truth = names_of(&tracks, labels);
    let input = input(&tracks);
    let outcome = identify(&input, &Params::default());
    let got = members(&outcome);
    let char_name = character_names(&tracks, &truth, &got);
    let screen = label_screen(labels);
    let main: HashSet<&str> =
        screen.iter().filter(|(n, f)| **n != "?" && **n != "x" && **f as f64 / FPS >= 30.0).map(|(n, _)| *n).collect();
    let shots = read_shots(dir);
    let body_stride: u32 = env("BODY_STRIDE", 3);
    assert!(body_stride.is_multiple_of(STRIDE));
    let file = bodyfile::read(Path::new(bodies_path));
    let samples: Vec<bodyfile::Sample> =
        file.samples.into_iter().filter(|s| s.frame.is_multiple_of(body_stride)).collect();

    let mut tp = BodyTrackerParams::new(body_stride);
    tp.veto_cos = env("VETO", tp.veto_cos);
    tp.far_cos = env("FAR_COS", tp.far_cos);
    tp.max_gap = tp.max_gap.max(2 * body_stride);
    let lp = LinkParams { max_interp: 24.max(2 * body_stride), ..LinkParams::default() };
    let gap = (24 / body_stride).max(1);
    let (mut bodies, mut faces_on) = (Vec::new(), Vec::new());
    let mut s = 0;
    for shot in &shots {
        while s < samples.len() && samples[s].frame < shot.start {
            s += 1;
        }
        let mut here = Vec::new();
        while s < samples.len() && samples[s].frame < shot.end {
            let obs = samples[s]
                .people
                .iter()
                .map(|p| BodyObs {
                    det: BodyDet {
                        frame: samples[s].frame,
                        bbox: p.bbox,
                        visible: p.visible,
                        score: p.score,
                        sharpness: p.sharpness,
                        luma: p.luma,
                    },
                    reid: p.reid.clone(),
                })
                .collect();
            here.push((samples[s].frame, obs));
            s += 1;
        }
        let found = track_bodies(*shot, here, &tp);
        let face_ids: Vec<usize> = (0..tracks.len()).filter(|&i| tracks[i].shot == shot.start).collect();
        let fs: Vec<&FaceTrack> = face_ids.iter().map(|&i| &tracks[i]).collect();
        let bs: Vec<&BodyTrack> = found.iter().collect();
        let links = link_shot(&fs, &bs, &lp);
        let base = bodies.len();
        faces_on.extend((0..found.len()).map(|_| Vec::new()));
        for l in links {
            faces_on[base + l.body].push((face_ids[l.face], l.samples));
        }
        bodies.extend(found);
    }
    let body_in: Vec<BodyIn> = bodies
        .iter()
        .zip(&faces_on)
        .map(|(b, fo)| {
            let face_spans: Vec<(u32, u32)> = fo.iter().flat_map(|&(f, _)| input[f].spans.clone()).collect();
            let has_face = |d: &BodyDet| face_spans.iter().any(|&(a, e)| a <= d.frame && d.frame < e);
            BodyIn {
                media: 0,
                spans: b.spans(body_stride, gap),
                faces: fo.iter().map(|&(track, samples)| FaceOn { track, samples }).collect(),
                reid: b.appearance(|_| true),
                reid_away: b.appearance(|d| !has_face(d)),
            }
        })
        .collect();
    let params = body_params();
    let res = propagate(&input, &outcome, &body_in, &params);
    World {
        tracks,
        input,
        outcome,
        got,
        truth,
        char_name,
        main,
        shots,
        samples,
        body_stride,
        bodies,
        faces_on,
        body_in,
        res,
        params,
    }
}

impl World<'_> {
    /// The label name of a body: the one most of its tied faces' samples have.
    fn body_truth(&self, b: usize) -> Option<&str> {
        let mut by: HashMap<&str, u32> = HashMap::new();
        for &(f, n) in &self.faces_on[b] {
            if let Some(name) = self.truth[f] {
                *by.entry(name).or_default() += n;
            }
        }
        by.into_iter().max_by_key(|&(n, v)| (v, std::cmp::Reverse(n))).map(|(n, _)| n)
    }

    fn character_of(&self, b: usize) -> Option<usize> {
        match self.res.bodies[b] {
            BodyAssign::Face(c) | BodyAssign::Reid { character: c, .. } => Some(c),
            BodyAssign::None | BodyAssign::Conflict => None,
        }
    }

    fn name(&self, c: usize) -> &str {
        self.char_name.get(&c).copied().unwrap_or("?")
    }

    fn shot_of(&self, frame: u32) -> Option<FrameRange> {
        let i = self.shots.partition_point(|s| s.end <= frame);
        self.shots.get(i).filter(|s| s.contains(frame)).copied()
    }
}

fn pct(a: usize, b: usize) -> String {
    if b == 0 { "   -  ".into() } else { format!("{:5.1}%", 100.0 * a as f64 / b as f64) }
}

/// Labelled faces sitting on a detected body.
fn recall(w: &World, labels: &[Label]) {
    let lp = LinkParams::default();
    let at: HashMap<u32, &bodyfile::Sample> = w.samples.iter().map(|s| (s.frame, s)).collect();
    let buckets = [(0.0, 0.08, "< 8 %"), (0.08, 0.15, "8-15 %"), (0.15, 0.3, "15-30 %"), (0.3, 2.0, ">= 30 %")];
    println!("labelled main-cast faces on a detected body (sample frames of the body run only):");
    for th in [0.3f32, 0.4, 0.5] {
        let (mut all, mut on, mut unique) = (0usize, 0usize, 0usize);
        let mut by_bucket = [(0usize, 0usize); 4];
        let mut by_name: HashMap<&str, (usize, usize)> = HashMap::new();
        for l in labels.iter().filter(|l| w.main.contains(l.name.as_str())) {
            for (frame, fb) in &l.boxes {
                let Some(s) = at.get(frame) else { continue };
                let fits: Vec<f32> =
                    s.people.iter().filter(|p| p.score >= th).filter_map(|p| fit(fb, &p.visible, &lp)).collect();
                let h = fb[3] - fb[1];
                let k = buckets.iter().position(|b| (b.0..b.1).contains(&h)).unwrap();
                all += 1;
                by_bucket[k].0 += 1;
                let e = by_name.entry(l.name.as_str()).or_default();
                e.0 += 1;
                if !fits.is_empty() {
                    on += 1;
                    by_bucket[k].1 += 1;
                    e.1 += 1;
                    let mut f = fits.clone();
                    f.sort_by(|a, b| b.total_cmp(a));
                    unique += usize::from(f.len() == 1 || f[0] - f[1] >= lp.rival_margin);
                }
            }
        }
        let buck: Vec<String> =
            buckets.iter().zip(&by_bucket).map(|(b, (n, k))| format!("{} {} of {n}", b.2, pct(*k, *n))).collect();
        println!(
            "  score >= {th}: {} of {all} ({} told apart) | by face height: {}",
            pct(on, all),
            pct(unique, all),
            buck.join(", ")
        );
        if th == 0.5 {
            let mut names: Vec<(&&str, &(usize, usize))> = by_name.iter().collect();
            names.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
            let v: Vec<String> = names.iter().map(|(n, (a, k))| format!("{n} {}", pct(*k, *a))).collect();
            println!("    by character: {}", v.join(", "));
        }
    }
    // Backs of heads and gas masks the face detector found: on a body?
    let (mut xs, mut xon) = (0, 0);
    for l in labels.iter().filter(|l| l.name == "x") {
        for (frame, fb) in &l.boxes {
            if let Some(s) = at.get(frame) {
                xs += 1;
                xon += usize::from(s.people.iter().any(|p| p.score >= 0.5 && fit(fb, &p.visible, &lp).is_some()));
            }
        }
    }
    println!("  'x' faces (masks, backs of heads) on a body: {} of {xs}", pct(xon, xs));
    // People without any face.
    let mut faces_at: HashMap<u32, Vec<[f32; 4]>> = HashMap::new();
    for t in &w.tracks {
        for d in &t.dets {
            faces_at.entry(d.frame).or_default().push(d.bbox);
        }
    }
    let (mut people, mut faceless) = (0usize, 0usize);
    for s in &w.samples {
        for p in s.people.iter().filter(|p| p.score >= 0.5) {
            people += 1;
            let f = faces_at.get(&s.frame).map_or(&[][..], Vec::as_slice);
            faceless += usize::from(!f.iter().any(|fb| fit(fb, &p.visible, &lp).is_some()));
        }
    }
    println!(
        "people (score >= 0.5): {:.2} per sample, {} of them without a face of the face pass",
        people as f64 / w.samples.len() as f64,
        pct(faceless, people)
    );
    let with_face: HashSet<u32> = w.tracks.iter().map(|t| t.shot).collect();
    let bare: Vec<&FrameRange> = w.shots.iter().filter(|s| !with_face.contains(&s.start)).collect();
    let bodied = bare.iter().filter(|s| w.bodies.iter().any(|b| b.shot == s.start)).count();
    let secs: u32 = w.bodies.iter().filter(|b| !with_face.contains(&b.shot)).map(|b| b.end - b.start).sum();
    println!(
        "shots without any face track: {}; with a body track: {bodied} ({:.0} s of body tracks there)",
        bare.len(),
        secs as f64 / FPS
    );
}

/// How clean body tracks are, judged by the labels of the faces on them.
fn tracks_cmd(w: &World) {
    let n = w.bodies.len();
    let linked = w.faces_on.iter().filter(|f| !f.is_empty()).count();
    let named = |f: usize| w.truth[f].filter(|n| *n != "?" && *n != "x");
    let (mut one, mut two, mut impure, mut links, mut wrong) = (0, 0, 0, 0, 0);
    let mut list = Vec::new();
    for (b, fo) in w.faces_on.iter().enumerate() {
        let names: Vec<&str> = fo.iter().filter_map(|&(f, _)| named(f)).collect();
        if names.is_empty() {
            continue;
        }
        one += 1;
        if names.len() < 2 {
            continue;
        }
        two += 1;
        let major = w.body_truth(b).unwrap_or("?");
        links += names.len();
        let off = names.iter().filter(|&&x| x != major).count();
        wrong += off;
        if off > 0 {
            impure += 1;
            list.push(format!("{}-{} {:?}", w.bodies[b].start, w.bodies[b].end, names));
        }
    }
    let conflicts = w.res.bodies.iter().filter(|a| **a == BodyAssign::Conflict).count();
    let mut frag: HashMap<(u32, &str), HashSet<usize>> = HashMap::new();
    for (b, fo) in w.faces_on.iter().enumerate() {
        for &(f, _) in fo {
            if let Some(name) = named(f) {
                frag.entry((w.bodies[b].shot, name)).or_default().insert(b);
            }
        }
    }
    let split = frag.values().filter(|s| s.len() > 1).count();
    let dets: usize = w.bodies.iter().map(|b| b.dets.len()).sum();
    println!(
        "{n} body tracks (body stride {}; {:.1} sightings each); {linked} carry a face track ({})",
        w.body_stride,
        dets as f64 / n as f64,
        pct(linked, n)
    );
    println!(
        "  with a named face: {one}; with 2+ named faces: {two}, of them mixed: {impure} ({}); faces on the wrong body: {wrong} of {links} ({})",
        pct(impure, two),
        pct(wrong, links)
    );
    println!(
        "  conflicts (faces of different characters): {conflicts}; (shot, person) split over several bodies: {split}"
    );
    if detail() {
        for l in list {
            println!("    mixed: {l}");
        }
    }
}

/// BCubed of the main cast per track and per screen time.
fn bcubed_main(w: &World, got: &[Option<usize>]) -> (f32, f32, f32, f32) {
    let keep: Vec<usize> = (0..w.tracks.len()).filter(|&i| w.truth[i].is_some_and(|n| n != "x")).collect();
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let truth: Vec<usize> = keep
        .iter()
        .map(|&i| match w.truth[i] {
            Some(n) if n != "?" => {
                let k = ids.len();
                *ids.entry(n).or_insert(k)
            }
            _ => 1_000_000 + i,
        })
        .collect();
    let main_ids: HashSet<usize> = ids.iter().filter(|(n, _)| w.main.contains(*n)).map(|(_, &k)| k).collect();
    let sub: Vec<Option<usize>> = keep.iter().map(|&i| got[i]).collect();
    let frames: Vec<f32> = keep.iter().map(|&i| (w.tracks[i].end - w.tracks[i].start) as f32).collect();
    let ones = vec![1f32; keep.len()];
    let counted = |k: usize| main_ids.contains(&truth[k]);
    let a = bcubed(&sub, &truth, &ones, counted);
    let b = bcubed(&sub, &truth, &frames, counted);
    (a.precision, a.recall, a.f, b.f)
}

fn score(w: &World, labels: &[Label]) {
    let show = |what: &str, got: &[Option<usize>]| {
        let (p, r, f, tf) = bcubed_main(w, got);
        println!("{what:34} main cast tracks P {p:.3} R {r:.3} F {f:.3} | screen time F {tf:.3}");
        found_faces(&w.tracks, labels, &w.truth, got);
    };
    show("faces only", &w.got);
    for rescue_unsure in [false, true] {
        let p = BodyParams { rescue_unsure, ..w.params };
        let res = propagate(&w.input, &w.outcome, &w.body_in, &p);
        let mut got = w.got.clone();
        for &(t, c) in &res.promoted {
            got[t] = Some(c);
        }
        let wrong: Vec<String> = res
            .promoted
            .iter()
            .filter(|&&(t, c)| w.truth[t].is_some_and(|n| n != "?" && n != "x" && n != w.name(c)))
            .map(|&(t, c)| {
                format!("{}@{}-{} -> {}", w.truth[t].unwrap(), w.tracks[t].start, w.tracks[t].end, w.name(c))
            })
            .collect();
        let what = if rescue_unsure { "+ bodies name unassigned + unsure" } else { "+ bodies name unassigned faces" };
        println!("{} faces named by their body, {} of them wrong:", res.promoted.len(), wrong.len());
        show(what, &got);
        if detail() {
            for x in wrong {
                println!("    wrong: {x}");
            }
        }
    }
}

/// A query's answer (character, score, runner-up's score) and its true name.
type Ranked = (Option<(usize, f32, f32)>, String);

/// A body a query compares against: character, appearance, when.
struct Anchor<'v> {
    body: usize,
    character: usize,
    shot: u32,
    first: u32,
    last: u32,
    reid: [Option<&'v Vec<f32>>; 2],
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Best character for a query and its score, and the runner-up's score.
#[allow(clippy::too_many_arguments)]
fn rank(
    q: &[f32],
    first: u32,
    last: u32,
    shot: u32,
    own: usize,
    anchors: &[Anchor],
    window: u32,
    skip_shot: bool,
) -> Option<(usize, f32, f32)> {
    let mut by: Vec<(usize, f32)> = Vec::new();
    for a in anchors {
        if a.body == own || (skip_shot && a.shot == shot) {
            continue;
        }
        let gap = a.first.saturating_sub(last).max(first.saturating_sub(a.last));
        if gap > window {
            continue;
        }
        let s = a.reid.iter().flatten().map(|e| dot(q, e)).fold(f32::NEG_INFINITY, f32::max);
        match by.iter_mut().find(|v| v.0 == a.character) {
            Some(v) => v.1 = v.1.max(s),
            None => by.push((a.character, s)),
        }
    }
    by.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let &(c, s) = by.first()?;
    Some((c, s, by.get(1).map_or(-1.0, |v| v.1)))
}

/// Calibrate finding people again by their clothes, on bodies whose
/// person the labels know.
fn reid(w: &World) {
    let half = std::env::var("HALF").unwrap_or_default();
    let mid = w.shots.last().map_or(0, |s| s.end) / 2;
    let in_half = |b: &BodyTrack| match half.as_str() {
        "first" => b.start < mid,
        "second" => b.start >= mid,
        _ => true,
    };
    let anchors: Vec<Anchor> = (0..w.bodies.len())
        .filter_map(|b| match w.res.bodies[b] {
            BodyAssign::Face(c) => Some(Anchor {
                body: b,
                character: c,
                shot: w.bodies[b].shot,
                first: w.bodies[b].start,
                last: w.bodies[b].end,
                reid: [w.body_in[b].reid.as_ref(), w.body_in[b].reid_away.as_ref()],
            }),
            _ => None,
        })
        .collect();
    // Q1: bodies with a face, asked by their sightings without one.
    // Q2: bodies whose faces the face pass couldn't name, the labels can.
    // N: bodies of extras (only faces labelled `?`): any answer is wrong.
    let mut q1 = Vec::new();
    let mut q2 = Vec::new();
    let mut neg = Vec::new();
    for (b, body) in w.bodies.iter().enumerate() {
        if !in_half(body) {
            continue;
        }
        let truth = w.body_truth(b);
        match (w.res.bodies[b], truth) {
            (BodyAssign::Face(_), Some(n)) if w.main.contains(n) => {
                let away = body
                    .dets
                    .iter()
                    .zip(&body.reids)
                    .filter(|(d, e)| {
                        e.is_some()
                            && !w.faces_on[b]
                                .iter()
                                .any(|&(f, _)| w.input[f].spans.iter().any(|&(a, z)| a <= d.frame && d.frame < z))
                    })
                    .count();
                if let Some(v) = w.body_in[b].reid_away.as_ref().filter(|_| away >= 3) {
                    q1.push((b, n, v));
                }
            }
            (BodyAssign::None, Some(n)) if w.main.contains(n) => {
                if let Some(v) = w.body_in[b].reid.as_ref() {
                    q2.push((b, n, v));
                }
            }
            (BodyAssign::None, Some("?")) => {
                if let Some(v) = w.body_in[b].reid.as_ref() {
                    neg.push((b, "?", v));
                }
            }
            _ => {}
        }
    }
    println!(
        "queries ({}): Q1 hidden face {}, Q2 face unnamed {}, N extras {}; anchors {}",
        if half.is_empty() { "all" } else { half.as_str() },
        q1.len(),
        q2.len(),
        neg.len(),
        anchors.len()
    );
    let skip_shot = std::env::var_os("SKIP_SHOT").is_some();
    for window_s in [60.0, 180.0, 600.0, 1e6] {
        let window = (window_s * FPS) as u32;
        let ranked = |qs: &[(usize, &str, &Vec<f32>)]| -> Vec<Ranked> {
            qs.iter()
                .map(|&(b, n, v)| {
                    let body = &w.bodies[b];
                    (rank(v, body.start, body.end, body.shot, b, &anchors, window, skip_shot), n.to_string())
                })
                .collect()
        };
        let (r1, r2, rn) = (ranked(&q1), ranked(&q2), ranked(&neg));
        println!("window {}:", if window_s > 1e5 { "episode".to_string() } else { format!("{window_s} s") });
        for margin in [0.05f32, 0.1, 0.15] {
            let mut line = format!("  margin {margin:.2}:");
            for th in [0.6f32, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9] {
                let count = |r: &[Ranked]| {
                    let (mut right, mut wrong) = (0, 0);
                    for (x, n) in r {
                        if let Some((c, s, s2)) = x
                            && *s >= th
                            && s - s2 >= margin
                        {
                            if w.name(*c) == n.as_str() { right += 1 } else { wrong += 1 }
                        }
                    }
                    (right, wrong)
                };
                let ((a1, b1), (a2, b2), (_, fp)) = (count(&r1), count(&r2), count(&rn));
                let p = |a: usize, b: usize| if a + b == 0 { 1.0 } else { a as f64 / (a + b) as f64 };
                let _ = write!(
                    line,
                    " | {th:.2} Q1 {:.2}/{:.2} Q2 {:.2}/{:.2} N {fp}",
                    p(a1, b1),
                    a1 as f64 / r1.len().max(1) as f64,
                    p(a2, b2),
                    a2 as f64 / r2.len().max(1) as f64
                );
            }
            println!("{line}");
        }
    }
}

fn union_len(mut v: Vec<(u32, u32)>) -> u32 {
    v.sort_unstable();
    let (mut total, mut end) = (0u32, 0u32);
    for (a, b) in v {
        let a = a.max(end);
        if b > a {
            total += b - a;
            end = b;
        }
    }
    total
}

/// How much more of the main cast is found: screen time and shots.
fn presence(w: &World, out: Option<&Path>) {
    let mut names: Vec<&str> = w.main.iter().copied().collect();
    names.sort_by_key(|n| {
        std::cmp::Reverse(
            w.tracks.iter().zip(&w.truth).filter(|(_, t)| **t == Some(*n)).map(|(t, _)| t.end - t.start).sum::<u32>(),
        )
    });
    let mut tsv = String::from("name\tsource\tshot\tstart\tend\theight\tforeground\tdark\tscore\n");
    println!(
        "{:10} {:>9} {:>6} | {:>8} {:>8} {:>8} | {:>9} {:>9} | (c) new shots by visible height >=50/30/15/<15 %, foreground, dark",
        "", "face s", "shots", "+(a) s", "+(b) s", "+(c) s", "+(c) shots", "of them"
    );
    let (mut tot_face, mut tot_new) = (0usize, 0usize);
    for n in names {
        let chars: HashSet<usize> = w.char_name.iter().filter(|(_, x)| **x == n).map(|(c, _)| *c).collect();
        let face: Vec<usize> = (0..w.tracks.len()).filter(|&i| w.got[i].is_some_and(|c| chars.contains(&c))).collect();
        let face_spans: Vec<(u32, u32)> = face.iter().flat_map(|&i| w.input[i].spans.clone()).collect();
        let face_shots: HashSet<u32> = face.iter().map(|&i| w.tracks[i].shot).collect();
        let a_bodies: Vec<usize> = (0..w.bodies.len())
            .filter(|&b| matches!(w.res.bodies[b], BodyAssign::Face(c) if chars.contains(&c)))
            .collect();
        let a_spans: Vec<(u32, u32)> = a_bodies.iter().flat_map(|&b| w.body_in[b].spans.clone()).collect();
        let b_spans: Vec<(u32, u32)> = w
            .res
            .promoted
            .iter()
            .filter(|(_, c)| chars.contains(c))
            .flat_map(|&(t, _)| w.input[t].spans.clone())
            .collect();
        let c_bodies: Vec<usize> = (0..w.bodies.len())
            .filter(|&b| matches!(w.res.bodies[b], BodyAssign::Reid { character: c, .. } if chars.contains(&c)))
            .collect();
        let c_spans: Vec<(u32, u32)> = c_bodies.iter().flat_map(|&b| w.body_in[b].spans.clone()).collect();
        let base = union_len(face_spans.clone());
        let with_a = union_len([face_spans.clone(), a_spans.clone()].concat());
        let with_b = union_len([face_spans.clone(), a_spans.clone(), b_spans.clone()].concat());
        let with_c = union_len([face_spans, a_spans, b_spans, c_spans].concat());
        let a_shots: HashSet<u32> = a_bodies.iter().map(|&b| w.bodies[b].shot).collect();
        let mut new_shots: HashMap<u32, Vec<usize>> = HashMap::new();
        for &b in &c_bodies {
            let s = w.bodies[b].shot;
            if !face_shots.contains(&s) && !a_shots.contains(&s) {
                new_shots.entry(s).or_default().push(b);
            }
        }
        let mut hb = [0usize; 4];
        let (mut fg, mut dark) = (0, 0);
        for bs in new_shots.values() {
            let best =
                bs.iter().copied().max_by(|&x, &y| w.bodies[x].height().total_cmp(&w.bodies[y].height())).unwrap();
            let h = w.bodies[best].height();
            hb[if h >= 0.5 {
                0
            } else if h >= 0.3 {
                1
            } else if h >= 0.15 {
                2
            } else {
                3
            }] += 1;
            let (f, d) = flags(w, best);
            fg += usize::from(f);
            dark += usize::from(d);
        }
        for &b in &c_bodies {
            let (f, d) = flags(w, b);
            let BodyAssign::Reid { score, .. } = w.res.bodies[b] else { unreachable!() };
            let body = &w.bodies[b];
            let _ = writeln!(
                tsv,
                "{n}\tc\t{}\t{}\t{}\t{:.3}\t{}\t{}\t{score:.3}",
                body.shot,
                body.start,
                body.end,
                body.height(),
                u8::from(f),
                u8::from(d)
            );
        }
        tot_face += face_shots.len();
        tot_new += new_shots.len();
        println!(
            "{n:10} {:9.1} {:6} | {:8.1} {:8.1} {:8.1} | {:9} {:>9} | {:?} fg {fg} dark {dark}",
            base as f64 / FPS,
            face_shots.len(),
            (with_a - base) as f64 / FPS,
            (with_b - with_a) as f64 / FPS,
            (with_c - with_b) as f64 / FPS,
            new_shots.len(),
            pct(new_shots.len(), face_shots.len()),
            hb
        );
        if detail() && !new_shots.is_empty() {
            let mut v: Vec<&u32> = new_shots.keys().collect();
            v.sort();
            println!("    new shots: {v:?}");
        }
    }
    println!("main cast: {tot_face} shots with a face, {tot_new} more by clothes ({})", pct(tot_new, tot_face));
    if let Some(out) = out {
        std::fs::create_dir_all(out).unwrap();
        std::fs::write(out.join("presence.tsv"), tsv).unwrap();
    }
}

/// A body that is a blurred shoulder in the foreground, and one that is dark.
fn flags(w: &World, b: usize) -> (bool, bool) {
    let body = &w.bodies[b];
    let mut fg = 0;
    for d in &body.dets {
        let v = d.visible;
        let edge = v[3] >= 0.98 && (v[0] <= 0.02 || v[2] >= 0.98);
        // Blurred next to the sharpest other person in the frame (crops of one size).
        let sample = w.samples.binary_search_by_key(&d.frame, |s| s.frame).ok().map(|i| &w.samples[i]);
        let sharpest = sample.map_or(0.0, |s| {
            s.people.iter().filter(|p| p.visible != d.visible).map(|p| p.sharpness).fold(0.0, f32::max)
        });
        let blurred = d.sharpness < 0.5 * sharpest;
        fg += usize::from(edge && d.height() >= 0.45 && blurred);
    }
    let mut lumas: Vec<f32> = body.dets.iter().map(|d| d.luma).collect();
    lumas.sort_by(f32::total_cmp);
    (2 * fg > body.dets.len(), lumas[lumas.len() / 2] < 35.0)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a[0] == "audit" {
        return audit(&a[1], &a[2]);
    }
    let labels = labels::read_labels(&a[4]);
    let t = std::time::Instant::now();
    let w = build(&a[1], &a[2], Path::new(&a[3]), &labels);
    let found = w.res.bodies.iter().filter(|x| matches!(x, BodyAssign::Reid { .. })).count();
    let face = w.res.bodies.iter().filter(|x| matches!(x, BodyAssign::Face(_))).count();
    eprintln!(
        "built in {:.1?}: {} body tracks, {face} by face, {found} by clothes, {} faces named by their body",
        t.elapsed(),
        w.bodies.len(),
        w.res.promoted.len()
    );
    let out = a.get(5).map(Path::new);
    match a[0].as_str() {
        "recall" => recall(&w, &labels),
        "tracks" => tracks_cmd(&w),
        "score" => score(&w, &labels),
        "reid" => reid(&w),
        "presence" => presence(&w, out),
        "sheets" => sheets(&w, out.expect("out_dir")),
        other => panic!("unknown command {other}"),
    }
}

/// Cells for looking at: frames with boxes and small pictures of who the
/// pipeline says it is (drawn by `bodies_render`).
fn sheets(w: &World, out: &Path) {
    std::fs::create_dir_all(out).unwrap();
    let mut rng = Rng(0x5EED_B0D1E5);
    let mut items = String::from("sheet\tcell\tstratum\tframe\tboxes\tinsets\tinfo\n");
    let mut cell = 0usize;
    let mut add = |stratum: &str,
                   frame: u32,
                   boxes: Vec<([f32; 4], &str, u8)>,
                   insets: Vec<(u32, [f32; 4])>,
                   info: String| {
        let b: Vec<String> =
            boxes.iter().map(|(x, c, t)| format!("{:.4},{:.4},{:.4},{:.4},{c},{t}", x[0], x[1], x[2], x[3])).collect();
        let i: Vec<String> =
            insets.iter().map(|(f, x)| format!("{f}:{:.4},{:.4},{:.4},{:.4}", x[0], x[1], x[2], x[3])).collect();
        let _ =
            writeln!(items, "{}\t{}\t{stratum}\t{frame}\t{}\t{}\t{info}", cell / 6, cell % 6, b.join(";"), i.join(";"));
        cell += 1;
    };
    // The clearest face of each character: its picture.
    let mut clearest: HashMap<usize, (f32, u32, [f32; 4])> = HashMap::new();
    for (i, t) in w.tracks.iter().enumerate() {
        let (Some(c), Some(d)) = (w.got[i], tracks::clearest(&t.dets)) else { continue };
        let e = clearest.entry(c).or_insert((d.quality, d.frame, d.bbox));
        if d.quality > e.0 {
            *e = (d.quality, d.frame, d.bbox);
        }
    }
    let face_of: HashMap<usize, (u32, [f32; 4])> = clearest.into_iter().map(|(c, (_, f, b))| (c, (f, b))).collect();
    let has_face_at =
        |b: usize, f: u32| w.faces_on[b].iter().any(|&(t, _)| w.input[t].spans.iter().any(|&(a, z)| a <= f && f < z));
    let mid_det = |b: usize, pick: &dyn Fn(&BodyDet) -> bool| -> Option<BodyDet> {
        let v: Vec<&BodyDet> = w.bodies[b].dets.iter().filter(|d| pick(d)).collect();
        v.get(v.len() / 2).map(|d| **d)
    };
    // S1 (a): a body with a face, at a moment without one.
    let s1: Vec<usize> = (0..w.bodies.len())
        .filter(|&b| {
            matches!(w.res.bodies[b], BodyAssign::Face(c) if w.main.contains(w.name(c)))
                && w.bodies[b].dets.iter().any(|d| !has_face_at(b, d.frame))
        })
        .collect();
    for b in rng.pick(&s1, 30) {
        let BodyAssign::Face(c) = w.res.bodies[b] else { unreachable!() };
        let d = mid_det(b, &|d| !has_face_at(b, d.frame)).unwrap();
        let tf = w.faces_on[b].iter().find(|(t, _)| w.got[*t] == Some(c)).map_or(w.faces_on[b][0].0, |x| x.0);
        let fd = tracks::clearest(&w.tracks[tf].dets).unwrap();
        add(
            "a",
            d.frame,
            vec![(d.visible, "ff8800", 4)],
            vec![(fd.frame, fd.bbox)],
            format!("body {b} = {} (truth {})", w.name(c), w.body_truth(b).unwrap_or("-")),
        );
    }
    // S2 (b): faces their body named.
    let s2: Vec<(usize, usize)> = w.res.promoted.clone();
    for (t, c) in rng.pick(&s2, 42) {
        let d = tracks::clearest(&w.tracks[t].dets).unwrap();
        let reference = face_of.get(&c).map(|&(f, x)| vec![(f, x)]).unwrap_or_default();
        add(
            "b",
            d.frame,
            vec![(d.bbox, "ff8800", 3)],
            reference,
            format!("face {t} = {} (truth {})", w.name(c), w.truth[t].unwrap_or("-")),
        );
    }
    // S3 (c): bodies found by their clothes, with the body that matched.
    let anchors: Vec<Anchor> = (0..w.bodies.len())
        .filter_map(|b| match w.res.bodies[b] {
            BodyAssign::Face(c) => Some(Anchor {
                body: b,
                character: c,
                shot: w.bodies[b].shot,
                first: w.bodies[b].start,
                last: w.bodies[b].end,
                reid: [w.body_in[b].reid.as_ref(), w.body_in[b].reid_away.as_ref()],
            }),
            _ => None,
        })
        .collect();
    let s3: Vec<usize> = (0..w.bodies.len()).filter(|&b| matches!(w.res.bodies[b], BodyAssign::Reid { .. })).collect();
    let s3 = if s3.len() <= 150 { s3 } else { rng.pick(&s3, 130) };
    for b in s3 {
        let BodyAssign::Reid { character: c, score, runner_up } = w.res.bodies[b] else { unreachable!() };
        let d = mid_det(b, &|_| true).unwrap();
        let q = w.body_in[b].reid.as_ref().unwrap();
        // The anchor that matched best.
        let best = anchors
            .iter()
            .filter(|a| a.character == c)
            .max_by(|x, y| {
                let s = |a: &Anchor| a.reid.iter().flatten().map(|e| dot(q, e)).fold(f32::NEG_INFINITY, f32::max);
                s(x).total_cmp(&s(y))
            })
            .map(|a| a.body);
        let mut insets = Vec::new();
        if let Some(a) = best {
            let ad = mid_det(a, &|_| true).unwrap();
            insets.push((ad.frame, ad.visible));
        }
        if let Some(&(f, x)) = face_of.get(&c) {
            insets.push((f, x));
        }
        add(
            "c",
            d.frame,
            vec![(d.visible, "ff8800", 4)],
            insets,
            format!("body {b} = {} by clothes {score:.3} (next {runner_up:.3})", w.name(c)),
        );
    }
    // S4: shots, their middle sample, everyone coloured by who the pipeline says they are.
    let palette = [
        "e6194b", "3cb44b", "4363d8", "f58231", "911eb4", "42d4f4", "f032e6", "bfef45", "fabed4", "469990", "dcbeff",
        "9a6324",
    ];
    let mut colour: HashMap<usize, &str> = HashMap::new();
    let mut cast: Vec<usize> = w.char_name.keys().copied().filter(|c| w.main.contains(w.name(*c))).collect();
    cast.sort_unstable();
    for (k, c) in cast.iter().enumerate() {
        colour.insert(*c, palette[k % palette.len()]);
    }
    let body_shots: Vec<u32> = {
        let mut v: Vec<u32> = (0..w.bodies.len())
            .filter(|&b| {
                matches!(w.res.bodies[b], BodyAssign::Reid { .. })
                    || (matches!(w.res.bodies[b], BodyAssign::Face(_))
                        && w.bodies[b].dets.iter().any(|d| !has_face_at(b, d.frame)))
            })
            .map(|b| w.bodies[b].shot)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let all_shots: Vec<u32> = w.shots.iter().filter(|s| s.len() >= 24).map(|s| s.start).collect();
    let mut chosen = rng.pick(&body_shots, 20);
    chosen.extend(rng.pick(&all_shots, 20));
    chosen.sort_unstable();
    chosen.dedup();
    for s in chosen {
        let shot = w.shot_of(s).unwrap();
        let frames: Vec<u32> = w.samples.iter().map(|x| x.frame).filter(|f| shot.contains(*f)).collect();
        if frames.is_empty() {
            continue;
        }
        for k in [frames.len() / 2] {
            let f = frames[k];
            let mut boxes = Vec::new();
            let mut who = Vec::new();
            for (i, t) in w.tracks.iter().enumerate().filter(|(_, t)| t.shot == s) {
                if let Some(d) = t.dets.iter().find(|d| d.frame == f) {
                    let c = w.got[i].and_then(|c| colour.get(&c).copied()).unwrap_or("808080");
                    boxes.push((d.bbox, c, 2u8));
                }
            }
            for b in (0..w.bodies.len()).filter(|&b| w.bodies[b].shot == s) {
                let Some(x) = w.bodies[b].box_at(f, 24) else { continue };
                let c = w.character_of(b);
                if let Some(c) = c {
                    who.push(w.name(c).to_string());
                }
                let col = c.and_then(|c| colour.get(&c).copied()).unwrap_or("808080");
                boxes.push((x, col, if matches!(w.res.bodies[b], BodyAssign::Reid { .. }) { 6 } else { 4 }));
            }
            for (i, _) in w.tracks.iter().enumerate().filter(|(_, t)| t.shot == s) {
                if let Some(c) = w.got[i] {
                    who.push(w.name(c).to_string());
                }
            }
            who.sort();
            who.dedup();
            let legend: Vec<(u32, [f32; 4])> = who
                .iter()
                .filter_map(|n| {
                    w.char_name.iter().find(|(_, x)| **x == n.as_str()).and_then(|(c, _)| face_of.get(c).copied())
                })
                .take(4)
                .collect();
            add("s", f, boxes, legend, format!("shot {s}: says {}", who.join(",")));
        }
    }
    std::fs::write(out.join("items.tsv"), items).unwrap();
    println!("{cell} cells in {} sheets → {}", cell.div_ceil(6), out.join("items.tsv").display());
}

/// Wilson score interval (95 %) of `k` successes in `n`.
fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (p, n, z) = (k as f64 / n as f64, n as f64, 1.96f64);
    let d = 1.0 + z * z / n;
    let c = (p + z * z / (2.0 * n)) / d;
    let h = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / d;
    (c - h, c + h)
}

/// Precision per stratum from verdicts (`sheet cell y|n|u [note]`).
fn audit(items: &str, verdicts: &str) {
    let strata: HashMap<(String, String), String> = std::fs::read_to_string(items)
        .unwrap()
        .lines()
        .skip(1)
        .map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            ((v[0].to_string(), v[1].to_string()), v[2].to_string())
        })
        .collect();
    let mut by: HashMap<String, (usize, usize, usize)> = HashMap::new();
    for l in std::fs::read_to_string(verdicts).unwrap().lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let v: Vec<&str> = l.split('\t').collect();
        let Some(s) = strata.get(&(v[0].to_string(), v[1].to_string())) else { panic!("no cell {} {}", v[0], v[1]) };
        let e = by.entry(s.clone()).or_default();
        match v[2].trim() {
            "y" => e.0 += 1,
            "n" => e.1 += 1,
            _ => e.2 += 1,
        }
    }
    let mut keys: Vec<&String> = by.keys().collect();
    keys.sort();
    for k in keys {
        let (y, n, u) = by[k];
        let (lo, hi) = wilson(y, y + n);
        println!(
            "{k}: {y} right, {n} wrong, {u} unclear → precision {} (95 % {:.1}-{:.1} %)",
            pct(y, y + n),
            100.0 * lo,
            100.0 * hi
        );
    }
}
