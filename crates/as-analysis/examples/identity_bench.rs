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
//!
//! One person, one character:
//! - `stress <file.tracks> <labels.tsv>` makes the clustering hard on
//!   purpose (mirror copies of faces, reflections shot after shot, the
//!   episode cut into episodes, faces left out, other thresholds, a cast
//!   built episode by episode) and counts people split over characters and
//!   characters holding two people: the second must stay 0, the first 0 up
//!   to 30 mirror copies (more is as often as twins seen together, who must
//!   stay two). `scale <..> <n>`
//!   does it for a series of `n` simulated episodes (with timing; `DETAIL=1`
//!   shows where split people's faces are).
//! - `chars` lists who is in each character (`DUMP=<file>`: per track, its
//!   character and label) and the pairs of tracks that keep two characters
//!   of one person apart; `pairs <..> [link]` compares groups of one person
//!   with groups of two (mean faces, average linkage, faces seen together);
//!   `overlap` the faces seen in the same frames (where, how alike);
//!   `copresence` characters on screen together (how often, how alike);
//!   `dets <file.tracks> <track>…` a track's detections.
//!
//! The anime bench (`bench/anime/README.md`, files as `FORMATS.md`): label
//! names are gallery codes, `?e` and `n` (each someone of their own), `?u`
//! and `x` (left out); older labels' `?` is `?e`. From the environment:
//! - `GALLERY=<gallery.tsv>` and `LEVEL=look|figure|person` (default
//!   `look`: names as labelled) map names to the level before every
//!   subcommand scores them.
//! - The main cast is fixed once from the labels: figures with at least
//!   `MAIN_SECS` (30) on screen by shot presence (`PRESENCE=<presence.tsv>`),
//!   else by the labels' spans. `FPS=<num>/<den>` (24000/1001).
//! - `SPANS=<spans.tsv>`: tracks mostly inside an OP/ED, eyecatch, recap or
//!   preview are left out of the numbers and reported apart.
//! - `SEGMENTS=<bb.segments.tsv>`: each main name's scenepack (the shots
//!   their character is seen in, ranked as the app does) against the shots
//!   they are in (by presence, else by their labels): clip precision and
//!   recall, precision at 10 and 25 (a shorter pack: over all its shots).
//!   With truth from the labels, a pack's shots where the character holds
//!   only `x`, `?u` or unlabelled faces are left out (reported).
//! - `score` also gives each main name's coverage over time (a track holds
//!   their labelled frames; one of a character named like them does), the
//!   characters needed for 90 % of their frames in characters ("merges to
//!   clean"; per episode with `EPISODES=<frame>,…`, the first frame of each
//!   episode after the first on the project's timeline) and the share of
//!   their frames in no character.
//!   `BOOT=<n>` adds 95 % intervals (figures drawn with replacement, the
//!   looks of a figure together);
//!   `ITEMS=<file>` writes what each figure adds, and
//!   `paired <itemsA> <itemsB>` gives the paired-bootstrap interval of B − A.
//! - `stress`: `STRESS_COPY_LO/HI` (0.5 – 0.75), how alike a mirror copy is;
//!   `STRESS_REFL_LO/HI` (0.53 – 0.65), the share of a reflection's own face.

#[path = "common/anime.rs"]
mod anime;
#[path = "common/labels.rs"]
mod labels;
#[path = "common/scoring.rs"]
mod scoring;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anime::{Gallery, Level, Presence, Spans, at_level, read_presence, read_shots};
use as_analysis::tracks::FaceTrack;
use as_identity::metrics::{BCubed, bcubed};
use as_identity::{Assign, Params, Space, identify};
use labels::{
    Kind, Label, character_names, found_faces_of, fps, input, kind, members, named, names_of, read_labels, read_tracks,
};
use scoring::{Item, PackScore};

/// What the bench reads beside the tracks and labels, from the environment
/// (see the module).
struct Setup {
    gallery: Option<Gallery>,
    level: Level,
    presence: Option<Vec<Presence>>,
    spans: Spans,
    shots: Option<Vec<(u32, u32)>>,
    main_secs: f64,
    /// The first frame of each episode after the first, on the project's
    /// timeline (empty: one episode).
    episodes: Vec<u32>,
}

/// The main cast, fixed once from the labels.
struct Cast {
    /// Main figures and their seconds on screen, most first.
    figures: Vec<(String, f64)>,
    /// Their names at the scoring level (every look of theirs at `look`).
    names: HashSet<String>,
    from: &'static str,
}

impl Cast {
    fn is_main(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

impl Setup {
    fn from_env() -> Self {
        let path = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let level = Level::from_env();
        let gallery = path("GALLERY").map(|p| Gallery::read(&p));
        assert!(gallery.is_some() || level == Level::Look, "LEVEL={} needs GALLERY=<gallery.tsv>", level.word());
        Self {
            gallery,
            level,
            presence: path("PRESENCE").map(|p| read_presence(&p)),
            spans: path("SPANS").map_or_else(Spans::default, |p| Spans::read(&p)),
            shots: path("SEGMENTS").map(|p| read_shots(&p)),
            main_secs: std::env::var("MAIN_SECS").ok().map_or(30.0, |v| v.parse().expect("MAIN_SECS=<seconds>")),
            episodes: path("EPISODES").map_or(Vec::new(), |v| {
                let mut at: Vec<u32> = v.split(',').map(|f| f.trim().parse().expect("EPISODES=<frame>,…")).collect();
                at.sort_unstable();
                at
            }),
        }
    }

    fn at(&self, name: &str, level: Level) -> String {
        at_level(name, self.gallery.as_ref(), level)
    }

    /// The episode `frame` is in (0 the first).
    fn episode(&self, frame: u32) -> usize {
        self.episodes.partition_point(|&f| f <= frame)
    }

    /// The labels of `path` with their names at the scoring level, and the
    /// main cast (fixed from them first).
    fn labels(&self, path: &str) -> (Vec<Label>, Cast) {
        self.name_labels(read_labels(path))
    }

    /// `labels` with their names at the scoring level, and the main cast.
    fn name_labels(&self, mut labels: Vec<Label>) -> (Vec<Label>, Cast) {
        if let Some(g) = &self.gallery {
            let mut unknown: Vec<&str> =
                labels.iter().filter_map(|l| named(&l.name)).filter(|n| !g.contains(n)).collect();
            unknown.sort_unstable();
            unknown.dedup();
            if !unknown.is_empty() {
                eprintln!("not in the gallery (scored as they are): {}", unknown.join(", "));
            }
        }
        let cast = self.cast(&labels);
        for l in &mut labels {
            l.name = self.at(&l.name, self.level);
        }
        (labels, cast)
    }

    /// Figures with at least `main_secs` on screen, outside the spans: by
    /// shot presence, else by the spans of their labels.
    fn cast(&self, labels: &[Label]) -> Cast {
        let mut secs: HashMap<String, f64> = HashMap::new();
        let from = match &self.presence {
            Some(rows) => {
                for r in rows.iter().filter(|r| self.spans.mostly_in(r.start, r.end).is_none()) {
                    let mut figures: Vec<String> =
                        r.codes.iter().filter(|c| named(c).is_some()).map(|c| self.at(c, Level::Figure)).collect();
                    figures.sort_unstable();
                    figures.dedup();
                    for f in figures {
                        *secs.entry(f).or_default() += f64::from(r.end - r.start) / fps();
                    }
                }
                "shot presence"
            }
            None => {
                for l in labels.iter().filter(|l| self.spans.mostly_in(l.span.0, l.span.1).is_none()) {
                    if let Some(n) = named(&l.name) {
                        *secs.entry(self.at(n, Level::Figure)).or_default() += f64::from(l.span.1 - l.span.0) / fps();
                    }
                }
                "the labels' spans"
            }
        };
        let mut figures: Vec<(String, f64)> = secs.into_iter().filter(|(_, s)| *s >= self.main_secs).collect();
        figures.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let main: HashSet<&str> = figures.iter().map(|f| f.0.as_str()).collect();
        let codes = labels
            .iter()
            .map(|l| l.name.as_str())
            .chain(self.gallery.iter().flat_map(Gallery::codes))
            .chain(self.presence.iter().flatten().flat_map(|r| r.codes.iter().map(String::as_str)));
        let names = codes
            .filter(|c| named(c).is_some() && main.contains(self.at(c, Level::Figure).as_str()))
            .map(|c| self.at(c, self.level))
            .collect();
        Cast { figures, names, from }
    }

    /// Shot presence with its codes at the scoring level.
    fn presence_at_level(&self) -> Option<Vec<(u32, u32, Vec<String>)>> {
        let rows = self.presence.as_ref()?;
        Some(rows.iter().map(|r| (r.start, r.end, r.codes.iter().map(|c| self.at(c, self.level)).collect())).collect())
    }
}

/// The labels of `path`, names at the scoring level (`GALLERY`, `LEVEL`).
fn load_labels(path: &str) -> Vec<Label> {
    Setup::from_env().labels(path).0
}

/// A person per track: each known name one, everyone else someone of their
/// own; and the known names by their person.
fn people<'a>(truth_name: &[Option<&'a str>]) -> (Vec<usize>, HashMap<usize, &'a str>) {
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let truth: Vec<usize> = truth_name
        .iter()
        .enumerate()
        .map(|(i, n)| match n.and_then(named) {
            Some(n) => {
                let k = ids.len();
                *ids.entry(n).or_insert(k)
            }
            None => 1_000_000 + i,
        })
        .collect();
    (truth, ids.into_iter().map(|(n, k)| (k, n)).collect())
}

/// The tracks scored: labelled, not left out (`x`, `?u`), not mostly
/// inside a span.
fn scored_tracks(tracks: &[FaceTrack], truth_name: &[Option<&str>], spans: &Spans) -> Vec<usize> {
    (0..tracks.len())
        .filter(|&i| truth_name[i].is_some_and(|n| kind(n) != Kind::Excluded))
        .filter(|&i| spans.mostly_in(tracks[i].start, tracks[i].end).is_none())
        .collect()
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
    // Faces in the space the centroids are in (the fitted one, for animation).
    let in_space = |e: &[f32]| match &o.space {
        Space::Fitted(f) => f.embed(e),
        Space::Faces => e.to_vec(),
    };
    let mut write = |name: &str, mut ids: Vec<usize>, centroid: Option<&[f32]>| {
        // Least typical first: that's where mistakes are.
        if let Some(c) = centroid {
            let cos: HashMap<usize, f32> =
                ids.iter().map(|&i| (i, as_vision::arcface::cosine(&in_space(&tracks[i].embedding), c))).collect();
            ids.sort_by(|a, b| cos[a].total_cmp(&cos[b]));
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
        println!("c{rank:02}: {} tracks, {:.0} s", ids.len(), screen(c) as f64 / fps());
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
        println!("{n:12} {k:5} tracks {:7.1} s", f as f64 / fps());
    }
}

/// Embedder alone: same-person vs different-person track similarity.
fn separation(tracks: &[FaceTrack], labels_path: &str) {
    let labels = load_labels(labels_path);
    let names = names_of(tracks, &labels);
    let named: Vec<usize> = (0..tracks.len()).filter(|&i| names[i].and_then(named).is_some()).collect();
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
    let setup = Setup::from_env();
    let (labels, cast) = setup.labels(labels_path);
    let truth_name = names_of(tracks, &labels);
    let matched = truth_name.iter().filter(|n| n.is_some()).count();
    let keep = scored_tracks(tracks, &truth_name, &setup.spans);
    // Unnamed faces are each someone of their own.
    let (truth, named) = people(&truth_name);
    let mut screen: HashMap<usize, u32> = HashMap::new();
    for (i, t) in tracks.iter().enumerate() {
        *screen.entry(truth[i]).or_default() += t.end - t.start;
    }
    // The main cast, fixed from the labels.
    let main = |k: usize| named.get(&k).is_some_and(|n| cast.is_main(n));

    let t0 = std::time::Instant::now();
    let o = identify(&input(tracks), p);
    let ms = t0.elapsed().as_millis();
    let got = members(&o);
    let show = |what: &str, ids: &[usize], counted: &dyn Fn(usize) -> bool| {
        let sub_got: Vec<Option<usize>> = ids.iter().map(|&i| got[i]).collect();
        let sub_truth: Vec<usize> = ids.iter().map(|&i| truth[i]).collect();
        let frames: Vec<f32> = ids.iter().map(|&i| (tracks[i].end - tracks[i].start) as f32).collect();
        let counted = |k: usize| counted(ids[k]);
        let by_track = bcubed(&sub_got, &sub_truth, &vec![1f32; ids.len()], counted);
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
    show(&format!("main cast (>= {} s)", setup.main_secs), &keep, &|i| main(truth[i]));
    show("everyone named", &keep, &|i| named.contains_key(&truth[i]));
    found_faces_of(tracks, &labels, &truth_name, &got, |n| cast.is_main(n), |f| !setup.spans.holds(f));
    // Per main character: where its tracks went.
    let mut mains: Vec<usize> = named.keys().copied().filter(|&k| main(k)).collect();
    mains.sort_by_key(|k| (std::cmp::Reverse(screen[k]), named[k]));
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
            screen[&k] as f64 / fps(),
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

    // The anime bench's numbers.
    let mut flags: HashMap<&str, usize> = HashMap::new();
    labels.iter().flat_map(|l| &l.flags).for_each(|f| *flags.entry(f.as_str()).or_default() += 1);
    let mut flags: Vec<String> = flags.into_iter().map(|(f, n)| format!("{f} {n}")).collect();
    flags.sort();
    println!(
        "\nscored at the {} level; left out: {} tracks `x`, {} `?u`{}",
        setup.level.word(),
        truth_name.iter().filter(|n| *n == &Some("x")).count(),
        truth_name.iter().filter(|n| *n == &Some("?u")).count(),
        if flags.is_empty() { String::new() } else { format!("; labels flagged: {}", flags.join(", ")) }
    );
    let figures: Vec<String> = cast.figures.iter().map(|(n, s)| format!("{n} {s:.0} s")).collect();
    println!(
        "main cast (figures with >= {} s by {}, frozen): {}",
        setup.main_secs,
        cast.from,
        if figures.is_empty() { "none".into() } else { figures.join(", ") }
    );
    if !setup.spans.0.is_empty() {
        let apart: Vec<usize> = (0..tracks.len())
            .filter(|&i| truth_name[i].is_some_and(|n| kind(n) != Kind::Excluded))
            .filter(|&i| setup.spans.mostly_in(tracks[i].start, tracks[i].end).is_some())
            .collect();
        let mut by_kind: HashMap<&str, usize> = HashMap::new();
        for t in tracks {
            if let Some(k) = setup.spans.mostly_in(t.start, t.end) {
                *by_kind.entry(k).or_default() += 1;
            }
        }
        let mut by_kind: Vec<String> = by_kind.into_iter().map(|(k, n)| format!("{k} {n}")).collect();
        by_kind.sort();
        // Characters made of span tracks alone (OP/ED characters nobody is).
        let only: usize = (0..o.characters.len())
            .filter(|&c| {
                let mut of = (0..tracks.len()).filter(|&i| got[i] == Some(c));
                of.all(|i| setup.spans.mostly_in(tracks[i].start, tracks[i].end).is_some())
            })
            .count();
        println!(
            "in OP/ED, eyecatch, recap, preview (left out above): {}; {only} characters only there",
            by_kind.join(", ")
        );
        show("  of them, everyone named", &apart, &|i| named.contains_key(&truth[i]));
    }
    let r = report(tracks, &labels, &truth_name, &got, &cast, &setup);
    r.print(setup.presence.is_some());
    if let Some(n) = std::env::var("BOOT").ok().map(|v| v.parse::<usize>().expect("BOOT=<resamples>")) {
        let items: Vec<Item> = r.items.iter().map(|x| x.1).collect();
        let iv = scoring::bootstrap(&items, n, BOOT_SEED);
        let clip = r.clips().map_or(f64::NAN, |c| c.0);
        let at = [f64::from(r.track.f), f64::from(r.time.f), clip];
        let shown: Vec<String> =
            (0..3).map(|k| format!("{} {:.3} [{:.3}, {:.3}]", scoring::METRICS[k], at[k], iv[k].0, iv[k].1)).collect();
        println!("95 % intervals ({n} resamples of {} main figures): {}", items.len(), shown.join(", "));
    }
    if let Some(path) = std::env::var_os("ITEMS") {
        scoring::write_items(&path.to_string_lossy(), &r.items);
    }
}

/// Seed of every bootstrap: the same draws on every run.
const BOOT_SEED: u64 = 0x5eed_2026;

/// One main name's numbers beside BCubed.
struct Row {
    name: String,
    /// Labelled frames, held by a track, held by a track of their character.
    cover: [f64; 3],
    /// Characters holding 90 % of their frames in characters, over the
    /// project and per episode (`EPISODES`).
    groups: usize,
    per_episode: Vec<usize>,
    /// The share of their scored tracks' frames in no character (left out
    /// of `groups`).
    unassigned: f64,
    pack: Option<PackScore>,
    /// Shots of their pack left out of it: truth from the labels, and only
    /// `x`, `?u` or unlabelled faces of the character there.
    unknown: usize,
}

/// The bench's numbers for one grouping (`got`) beside BCubed.
struct Report {
    /// The main cast, BCubed per track and per screen time (as `score`).
    track: BCubed,
    time: BCubed,
    rows: Vec<Row>,
    /// What each main figure adds (its looks together at `LEVEL=look`), for
    /// the bootstrap.
    items: Vec<(String, Item)>,
}

impl Report {
    fn held(&self) -> (f64, f64) {
        let sum = |k: usize| self.rows.iter().map(|r| r.cover[k]).sum::<f64>();
        (scoring::ratio(sum(1), sum(0)), scoring::ratio(sum(2), sum(0)))
    }

    fn merges(&self) -> usize {
        self.rows.iter().map(|r| r.groups.saturating_sub(1)).sum()
    }

    /// The most merges to clean of one name in one episode.
    fn worst_episode(&self) -> usize {
        self.rows.iter().flat_map(|r| &r.per_episode).map(|g| g.saturating_sub(1)).max().unwrap_or(0)
    }

    /// Clip precision and recall over the main names' packs, the shots in
    /// them, and the shots of unknown truth left out.
    fn clips(&self) -> Option<(f64, f64, usize, usize)> {
        let packs: Vec<PackScore> = self.rows.iter().filter_map(|r| r.pack).collect();
        if packs.is_empty() {
            return None;
        }
        let sum = |f: fn(&PackScore) -> usize| packs.iter().map(f).sum::<usize>() as f64;
        Some((
            scoring::ratio(sum(|p| p.right), sum(|p| p.shots)),
            scoring::ratio(sum(|p| p.right), sum(|p| p.truth)),
            sum(|p| p.shots) as usize,
            self.rows.iter().map(|r| r.unknown).sum(),
        ))
    }

    fn print(&self, by_presence: bool) {
        let (held, theirs) = self.held();
        let episodes = self.rows.first().map_or(1, |r| r.per_episode.len());
        println!(
            "main cast over time           in a track {held:.3} | on their character {theirs:.3} | {} merges to clean{}",
            self.merges(),
            if episodes > 1 {
                format!(" (at most {} per name and episode, {episodes} episodes)", self.worst_episode())
            } else {
                String::new()
            }
        );
        if let Some((p, r, n, unknown)) = self.clips() {
            let from = if by_presence { "shot presence" } else { "labels" };
            let left = if unknown > 0 {
                format!("; {unknown} with only `x`, `?u` or unlabelled faces left out")
            } else {
                String::new()
            };
            println!("main cast clips (truth: {from}) precision {p:.3} recall {r:.3} ({n} shots in their packs{left})");
        }
        for row in &self.rows {
            let [all, held, theirs] = row.cover;
            let pack = row.pack.map_or(String::new(), |p| {
                // A pack shorter than 10 (25) shots: over all of them.
                let at = |(right, n): (usize, usize)| scoring::ratio(right as f64, n as f64);
                format!(
                    " | clips {}: P {:.3} R {:.3} P@10 {:.2} P@25 {:.2}",
                    p.shots,
                    scoring::ratio(p.right as f64, p.shots as f64),
                    scoring::ratio(p.right as f64, p.truth as f64),
                    at(p.at10),
                    at(p.at25)
                )
            });
            let per_episode = if row.per_episode.len() > 1 {
                let m: Vec<String> = row.per_episode.iter().map(|g| g.saturating_sub(1).to_string()).collect();
                format!(" ({} per episode)", m.join("/"))
            } else {
                String::new()
            };
            println!(
                "  {:12} {:7.1} s labelled: in a track {:.3} | theirs {:.3} | {} characters for 90 %, {} merges{per_episode}, {:.0} % unassigned{pack}",
                row.name,
                all / fps(),
                scoring::ratio(held, all),
                scoring::ratio(theirs, all),
                row.groups,
                row.groups.saturating_sub(1),
                100.0 * row.unassigned
            );
        }
    }
}

/// The bench's numbers beside BCubed for one grouping (`got`, a character
/// per track) of `tracks` named `truth_name` from `labels`.
fn report(
    tracks: &[FaceTrack],
    labels: &[Label],
    truth_name: &[Option<&str>],
    got: &[Option<usize>],
    cast: &Cast,
    setup: &Setup,
) -> Report {
    let keep = scored_tracks(tracks, truth_name, &setup.spans);
    let kept: HashSet<usize> = keep.iter().copied().collect();
    let (truth, _) = people(truth_name);
    let main_of = |i: usize| truth_name[i].and_then(named).filter(|n| cast.is_main(n));
    let sub_got: Vec<Option<usize>> = keep.iter().map(|&i| got[i]).collect();
    let sub_truth: Vec<usize> = keep.iter().map(|&i| truth[i]).collect();
    let frames: Vec<f32> = keep.iter().map(|&i| (tracks[i].end - tracks[i].start) as f32).collect();
    let ones = vec![1f32; keep.len()];
    let counted = |k: usize| main_of(keep[k]).is_some();
    let (track, time) = (bcubed(&sub_got, &sub_truth, &ones, counted), bcubed(&sub_got, &sub_truth, &frames, counted));
    let (by_track, by_time) =
        (scoring::bcubed_terms(&sub_got, &sub_truth, &ones), scoring::bcubed_terms(&sub_got, &sub_truth, &frames));

    let char_name = character_names(tracks, truth_name, got);
    let cover =
        scoring::coverage(tracks, labels, got, &char_name, |l| cast.is_main(&l.name), |f| !setup.spans.holds(f));
    let groups = scoring::groups_for(tracks, truth_name, got, |i| kept.contains(&i), 0.9);
    // The same per episode (a track is in the one it starts in).
    let per_episode: Vec<HashMap<&str, usize>> = (0..=setup.episodes.len())
        .map(|e| {
            let of_e = |i: usize| kept.contains(&i) && setup.episode(tracks[i].start) == e;
            scoring::groups_for(tracks, truth_name, got, of_e, 0.9)
        })
        .collect();
    // Each main name's pack: the character named like them holding most of
    // their frames.
    let mut frames_in: HashMap<(&str, usize), u32> = HashMap::new();
    for &i in &keep {
        if let (Some(n), Some(c)) = (main_of(i), got[i])
            && char_name.get(&c) == Some(&n)
        {
            *frames_in.entry((n, c)).or_default() += tracks[i].end - tracks[i].start;
        }
    }
    let mut pack_of: HashMap<&str, (u32, usize)> = HashMap::new();
    for (&(n, c), &f) in &frames_in {
        let e = pack_of.entry(n).or_insert((f, c));
        if (f, std::cmp::Reverse(c)) > (e.0, std::cmp::Reverse(e.1)) {
            *e = (f, c);
        }
    }
    let packs = setup.shots.as_ref().map(|shots| {
        let ok = |s: usize| setup.spans.mostly_in(shots[s].0, shots[s].1).is_none();
        let packs = scoring::packs(tracks, got, shots, ok, fps());
        let rows = setup.presence_at_level();
        let by_presence = rows.is_some();
        let mut truth = match rows {
            Some(rows) => scoring::true_shots_presence(&rows, shots),
            None => scoring::true_shots_labels(labels, shots),
        };
        truth.values_mut().for_each(|s| s.retain(|&s| ok(s)));
        (packs, truth, scoring::in_shots(tracks, got, shots), by_presence)
    });

    let mut names: Vec<&str> = cast.names.iter().map(String::as_str).collect();
    let all = |n: &str| cover.get(n).map_or(0.0, |c| c[0]);
    names.sort_by(|a, b| all(b).total_cmp(&all(a)).then(a.cmp(b)));
    let none = HashSet::new();
    let mut rows = Vec::new();
    let mut items: Vec<(String, Item)> = Vec::new();
    for n in names {
        let pack = packs.as_ref().map(|(packs, truth, in_shot, by_presence)| {
            let truth = truth.get(n).unwrap_or(&none);
            let c = pack_of.get(n).map(|x| x.1);
            let mine = c.and_then(|c| packs.get(&c)).map_or(&[][..], Vec::as_slice);
            // Labels only know the faces they name: a shot where the
            // character holds only faces nobody could name (`x`, `?u`) or
            // nobody labelled may still be theirs. It is left out, not
            // counted wrong.
            let (known, unknown): (Vec<usize>, Vec<usize>) = mine.iter().partition(|&&s| {
                *by_presence
                    || truth.contains(&s)
                    || in_shot[s]
                        .iter()
                        .any(|&i| got[i] == c && truth_name[i].is_some_and(|t| kind(t) != Kind::Excluded))
            });
            (PackScore::of(&known, truth), unknown.len())
        });
        let mut item = Item::default();
        let (mut mine, mut unassigned) = (0.0, 0.0);
        for (k, &i) in keep.iter().enumerate().filter(|&(_, &i)| main_of(i) == Some(n)) {
            let w = f64::from(frames[k]);
            item.track[0] += by_track[k].0;
            item.track[1] += by_track[k].1;
            item.track[2] += 1.0;
            item.time[0] += w * by_time[k].0;
            item.time[1] += w * by_time[k].1;
            item.time[2] += w;
            mine += w;
            if got[i].is_none() {
                unassigned += w;
            }
        }
        if let Some((p, _)) = pack {
            item.clip = [p.right as f64, p.shots as f64];
        }
        // A look of a main figure nobody labelled or saw.
        if all(n) == 0.0 && item.track[2] == 0.0 && pack.is_none_or(|(p, _)| p.truth == 0) {
            continue;
        }
        // The bootstrap draws figures: the looks of one figure go together.
        let figure = setup.at(n, Level::Figure);
        match items.iter_mut().find(|x| x.0 == figure) {
            Some(x) => x.1.add(&item),
            None => items.push((figure, item)),
        }
        rows.push(Row {
            name: n.to_string(),
            cover: cover.get(n).copied().unwrap_or_default(),
            groups: groups.get(n).copied().unwrap_or(0),
            per_episode: per_episode.iter().map(|g| g.get(n).copied().unwrap_or(0)).collect(),
            unassigned: scoring::ratio(unassigned, mine),
            pack: pack.map(|x| x.0),
            unknown: pack.map_or(0, |x| x.1),
        });
    }
    Report { track, time, rows, items }
}

/// `paired <itemsA> <itemsB>`: per metric, B − A over the figures both
/// runs have, with its 95 % paired-bootstrap interval (`BOOT`, 2,000
/// resamples by default).
fn paired(a_path: &str, b_path: &str) {
    let (a, b) = (scoring::read_items(a_path), scoring::read_items(b_path));
    let of_b: HashMap<&str, Item> = b.iter().map(|(n, i)| (n.as_str(), *i)).collect();
    let both: Vec<(Item, Item)> = a.iter().filter_map(|(n, i)| Some((*i, *of_b.get(n.as_str())?))).collect();
    let only = a.len() + b.len() - 2 * both.len();
    if only > 0 {
        eprintln!("{only} figures in one run only: left out");
    }
    let n = std::env::var("BOOT").ok().map_or(2000, |v| v.parse().expect("BOOT=<resamples>"));
    let (xa, xb): (Vec<Item>, Vec<Item>) = both.into_iter().unzip();
    let (ma, mb) = (scoring::metrics(&xa), scoring::metrics(&xb));
    let diff = scoring::paired(&xa, &xb, n, BOOT_SEED);
    println!("{} figures, {n} paired resamples", xa.len());
    for (k, m) in scoring::METRICS.iter().enumerate() {
        let (d, lo, hi) = diff[k];
        println!("  {m:15} A {:.3} B {:.3}: B − A {d:+.4}, 95 % [{lo:+.4}, {hi:+.4}]", ma[k], mb[k]);
    }
}

/// Who is in each character (labels), and how alike characters of the same
/// person are: their average-linkage similarity and the tracks of theirs on
/// screen together.
fn chars(tracks: &[FaceTrack], labels_path: &str) {
    let labels = load_labels(labels_path);
    let truth_name = names_of(tracks, &labels);
    let input = input(tracks);
    let o = identify(&input, &env_params());
    let got = members(&o);
    let n = o.characters.len();
    let secs = |ids: &[usize]| ids.iter().map(|&i| (tracks[i].end - tracks[i].start) as f64).sum::<f64>() / fps();
    let of: Vec<Vec<usize>> = (0..n).map(|c| (0..tracks.len()).filter(|&i| got[i] == Some(c)).collect()).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| secs(&of[b]).total_cmp(&secs(&of[a])));
    let mut major: Vec<&str> = vec!["-"; n];
    for (rank, &c) in order.iter().enumerate() {
        let mut by: HashMap<&str, (usize, f64)> = HashMap::new();
        for &i in &of[c] {
            let e = by.entry(truth_name[i].unwrap_or("-")).or_default();
            e.0 += 1;
            e.1 += (tracks[i].end - tracks[i].start) as f64 / fps();
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
        major[c] = v.iter().find(|(k, _)| *k != "-" && named(k).is_some()).map_or("?", |x| x.0);
        let what: Vec<String> = v.iter().map(|(k, (t, s))| format!("{k} {t}/{s:.0}s")).collect();
        let core = of[c].iter().filter(|&&i| matches!(o.tracks[i], (Assign::Auto(_), s) if s >= 1.0)).count();
        println!(
            "C{:02} [{rank:2}] {:4} tracks ({core} core) {:6.1} s: {}",
            c,
            of[c].len(),
            secs(&of[c]),
            what.join(", ")
        );
    }
    if let Some(path) = std::env::var_os("DUMP") {
        // Per track: `start end quality character label x0 y0` (first box).
        let mut tsv = String::new();
        for (i, t) in tracks.iter().enumerate() {
            let c = got[i].map_or(-1, |c| c as i64);
            let b = t.dets[0].bbox;
            tsv += &format!(
                "{}\t{}\t{:.4}\t{c}\t{}\t{:.4}\t{:.4}\t{:?}\n",
                t.start,
                t.end,
                t.quality,
                truth_name[i].unwrap_or("-"),
                b[0],
                b[1],
                o.tracks[i]
            );
        }
        std::fs::write(path, tsv).unwrap();
    }
    let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    println!("\nSame person in several characters:");
    for a in 0..n {
        for b in a + 1..n {
            if major[a] != major[b] || major[a] == "?" {
                continue;
            }
            let (mut sum, mut cnt, mut best) = (0f64, 0usize, f32::MIN);
            let mut together = 0;
            for &i in &of[a] {
                for &j in &of[b] {
                    let s = cos(&tracks[i].embedding, &tracks[j].embedding);
                    sum += s as f64;
                    cnt += 1;
                    best = best.max(s);
                    let (x, y) = (&input[i], &input[j]);
                    let shared = x.spans.iter().any(|&(s1, e1)| y.spans.iter().any(|&(s2, e2)| s1 < e2 && s2 < e1));
                    if x.start < y.end && y.start < x.end && shared {
                        together += 1;
                        println!(
                            "    together: {i} {}-{} q {:.2} {:?} [{}] | {j} {}-{} q {:.2} {:?} [{}] | cos {s:.3}",
                            x.start,
                            x.end,
                            x.quality,
                            tracks[i].dets[0].bbox,
                            truth_name[i].unwrap_or("-"),
                            y.start,
                            y.end,
                            y.quality,
                            tracks[j].dets[0].bbox,
                            truth_name[j].unwrap_or("-")
                        );
                    }
                }
            }
            println!(
                "  {}: C{a:02} ({}) + C{b:02} ({}): centroid cos {:.3}, mean pair {:.3}, best pair {:.3}, overlapping pairs {together}",
                major[a],
                of[a].len(),
                of[b].len(),
                cos(&o.characters[a].centroid, &o.characters[b].centroid),
                sum / cnt.max(1) as f64,
                best
            );
        }
    }
}

/// How groups of the same person differ from groups of two people: the
/// core tracks clustered at `link`, then every pair of pure groups with
/// their average-linkage similarity, centroid similarity, best pairs and
/// tracks on screen together.
fn pairs(tracks: &[FaceTrack], labels_path: &str, link: f32) {
    let labels = load_labels(labels_path);
    let truth_name = names_of(tracks, &labels);
    let input = input(tracks);
    let p = Params::default();
    let core: Vec<usize> =
        (0..input.len()).filter(|&i| input[i].quality >= p.core_quality && input[i].dets >= p.core_dets).collect();
    let groups = as_identity::cluster(&input, &core, &Params { link, ..p });
    let w = |i: usize| input[i].quality.max(0.05);
    let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    struct G {
        members: Vec<usize>,
        name: String,
        purity: f32,
        sum: Vec<f32>,
        weight: f32,
        cen: Vec<f32>,
        coh: f32,
    }
    let gs: Vec<G> = groups
        .into_iter()
        .filter(|g| g.len() >= 2)
        .map(|g| {
            let mut by: HashMap<&str, u32> = HashMap::new();
            for &i in &g {
                *by.entry(truth_name[i].unwrap_or("-")).or_default() += tracks[i].end - tracks[i].start;
            }
            let total: u32 = by.values().sum();
            let (name, f) = by.into_iter().max_by_key(|&(n, f)| (f, std::cmp::Reverse(n))).unwrap();
            let mut sum = vec![0f32; input[g[0]].embedding.len()];
            let mut weight = 0.0;
            let mut w2 = 0.0;
            for &i in &g {
                for (s, x) in sum.iter_mut().zip(&input[i].embedding) {
                    *s += w(i) * x;
                }
                weight += w(i);
                w2 += w(i) * w(i);
            }
            let norm = sum.iter().map(|x| x * x).sum::<f32>();
            let coh = (norm - w2) / (weight * weight - w2).max(1e-9);
            let cen: Vec<f32> = sum.iter().map(|x| x / norm.sqrt()).collect();
            G { members: g, name: name.to_string(), purity: f as f32 / total as f32, sum, weight, cen, coh }
        })
        .collect();
    let named = |g: &G| g.purity >= 0.8 && g.name != "-" && named(&g.name).is_some();
    println!(
        "link {link}: {} groups of 2+ tracks, {} pure and named",
        gs.len(),
        gs.iter().filter(|g| named(g)).count()
    );
    let mut rows: Vec<(bool, f32, f32, f32, usize, String)> = Vec::new();
    for a in 0..gs.len() {
        for b in a + 1..gs.len() {
            let (ga, gb) = (&gs[a], &gs[b]);
            if !named(ga) || !named(gb) {
                continue;
            }
            let avg = cos(&ga.sum, &gb.sum) / (ga.weight * gb.weight);
            let cen = cos(&ga.cen, &gb.cen);
            let mut cross: Vec<f32> = Vec::new();
            let mut together = 0;
            for &i in &ga.members {
                for &j in &gb.members {
                    cross.push(cos(&input[i].embedding, &input[j].embedding));
                    let (x, y) = (&input[i], &input[j]);
                    if x.start < y.end
                        && y.start < x.end
                        && x.spans.iter().any(|&(s1, e1)| y.spans.iter().any(|&(s2, e2)| s1 < e2 && s2 < e1))
                    {
                        together += 1;
                        if ga.name == gb.name {
                            let b = |t: &FaceTrack| t.dets[0].bbox.map(|v| (v * 100.0).round() / 100.0);
                            println!(
                                "  {} together: track {i} {}-{} q {:.2} h {:.2} {:?} | track {j} {}-{} q {:.2} h {:.2} {:?} | cos {:.3}",
                                ga.name,
                                x.start,
                                x.end,
                                x.quality,
                                tracks[i].face_height(),
                                b(&tracks[i]),
                                y.start,
                                y.end,
                                y.quality,
                                tracks[j].face_height(),
                                b(&tracks[j]),
                                cos(&x.embedding, &y.embedding)
                            );
                        }
                    }
                }
            }
            cross.sort_by(|x, y| y.total_cmp(x));
            let best = cross[0];
            let same = ga.name == gb.name;
            let what = format!(
                "{} ({}, coh {:.2}) + {} ({}, coh {:.2})",
                ga.name,
                ga.members.len(),
                ga.coh,
                gb.name,
                gb.members.len(),
                gb.coh
            );
            rows.push((same, avg, cen, best, together, what));
        }
    }
    let show = |title: &str, mut v: Vec<&(bool, f32, f32, f32, usize, String)>, key: usize, n: usize| {
        let k = |r: &(bool, f32, f32, f32, usize, String)| match key {
            1 => r.1,
            2 => r.2,
            _ => r.3,
        };
        v.sort_by(|x, y| k(y).total_cmp(&k(x)));
        println!("{title}");
        for r in v.into_iter().take(n) {
            println!("  avg {:.3} cen {:.3} best {:.3} together {:3}  {}", r.1, r.2, r.3, r.4, r.5);
        }
    };
    let pos: Vec<_> = rows.iter().filter(|r| r.0).collect();
    let neg: Vec<_> = rows.iter().filter(|r| !r.0).collect();
    println!("{} same-person pairs, {} different-person pairs", pos.len(), neg.len());
    let mut lo = pos.clone();
    lo.sort_by(|x, y| x.2.total_cmp(&y.2));
    println!("same person, least alike centroids first:");
    for r in lo.iter().take(40) {
        println!("  avg {:.3} cen {:.3} best {:.3} together {:3}  {}", r.1, r.2, r.3, r.4, r.5);
    }
    show("different people, most alike centroids:", neg.clone(), 2, 25);
    show("different people, highest average linkage:", neg.clone(), 1, 10);
    show("different people, best single pair:", neg, 3, 10);
}

/// Faces on screen at the same time: how much their boxes overlap in the
/// frames both were seen (intersection over the smaller box), with their
/// labels — duplicates of one face sit at the same place, two people don't.
fn overlap(tracks: &[FaceTrack], labels_path: &str) {
    let labels = load_labels(labels_path);
    let truth_name = names_of(tracks, &labels);
    let contain = |a: &[f32; 4], b: &[f32; 4]| {
        let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
        let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
        let small = ((a[2] - a[0]) * (a[3] - a[1])).min((b[2] - b[0]) * (b[3] - b[1])).max(1e-9);
        iw * ih / small
    };
    let mut rows = Vec::new();
    for i in 0..tracks.len() {
        for j in i + 1..tracks.len() {
            let (a, b) = (&tracks[i], &tracks[j]);
            if !(a.start < b.end && b.start < a.end) {
                continue;
            }
            let at: HashMap<u32, [f32; 4]> = b.dets.iter().map(|d| (d.frame, d.bbox)).collect();
            let mut c: Vec<f32> =
                a.dets.iter().filter_map(|d| at.get(&d.frame).map(|bb| contain(&d.bbox, bb))).collect();
            if c.is_empty() {
                continue;
            }
            c.sort_by(f32::total_cmp);
            let (med, max) = (c[c.len() / 2], c[c.len() - 1]);
            let (na, nb) = (truth_name[i].unwrap_or("-"), truth_name[j].unwrap_or("-"));
            rows.push((med, max, c.len(), i, j, na, nb));
        }
    }
    rows.sort_by(|x, y| y.0.total_cmp(&x.0));
    println!("{} pairs of faces seen in the same frames", rows.len());
    // Their identity similarity: copies of one person (same name) vs two people.
    let mut by_cos: Vec<(f32, bool, &str, &str, usize, usize, usize)> = rows
        .iter()
        .filter(|r| [r.5, r.6].iter().all(|n| *n != "-" && kind(n) != Kind::Excluded))
        .map(|r| {
            let cos = as_vision::arcface::cosine(&tracks[r.3].embedding, &tracks[r.4].embedding);
            (cos, r.5 == r.6 && named(r.5).is_some(), r.5, r.6, r.3, r.4, r.2)
        })
        .collect();
    by_cos.sort_by(|x, y| y.0.total_cmp(&x.0));
    println!("most alike faces seen together (same name = copy of one person):");
    for (cos, same, na, nb, i, j, n) in by_cos.iter().take(25) {
        let q = |k: usize| tracks[k].quality;
        println!(
            "  cos {cos:.3} {} {na} + {nb} ({i} q {:.2}, {j} q {:.2}, {n} frames)",
            if *same { "SAME" } else { "    " },
            q(*i),
            q(*j)
        );
    }
    for (med, max, n, i, j, na, nb) in rows.iter().filter(|r| r.1 > 0.0) {
        let cos = as_vision::arcface::cosine(&tracks[*i].embedding, &tracks[*j].embedding);
        println!(
            "  median {med:.2} max {max:.2} over {n:3} frames: {i} {}-{} [{na}] + {j} {}-{} [{nb}] cos {cos:.2}",
            tracks[*i].start, tracks[*i].end, tracks[*j].start, tracks[*j].end
        );
    }
}

/// Characters seen together: per pair, the share of the smaller one's
/// sightings with the bigger one in the same frame, and how alike their mean
/// faces are — how far apart copies (a reflection) and people who are
/// always together are.
fn copresence(tracks: &[FaceTrack], labels_path: &str) {
    let labels = load_labels(labels_path);
    let names = names_of(tracks, &labels);
    let ins = input(tracks);
    let o = identify(&ins, &Params::default());
    let got = members(&o);
    let n = o.characters.len();
    let of: Vec<Vec<usize>> = (0..n).map(|c| (0..tracks.len()).filter(|&i| got[i] == Some(c)).collect()).collect();
    let seen: Vec<std::collections::HashSet<u32>> =
        of.iter().map(|g| g.iter().flat_map(|&i| ins[i].seen.iter().copied()).collect()).collect();
    let frames = |g: &[usize]| g.iter().map(|&i| tracks[i].end - tracks[i].start).sum::<u32>();
    let name = |g: &[usize]| {
        let mut by: HashMap<&str, u32> = HashMap::new();
        for &i in g {
            *by.entry(names[i].unwrap_or("-")).or_default() += tracks[i].end - tracks[i].start;
        }
        by.into_iter().max_by_key(|&(n, f)| (f, std::cmp::Reverse(n))).map_or("-", |x| x.0)
    };
    let mut rows = Vec::new();
    for a in 0..n {
        for b in 0..n {
            if a == b || (frames(&of[a]), a) <= (frames(&of[b]), b) {
                continue;
            }
            let share = seen[b].iter().filter(|f| seen[a].contains(f)).count() as f32 / seen[b].len().max(1) as f32;
            let cos: f32 = o.characters[a].centroid.iter().zip(&o.characters[b].centroid).map(|(x, y)| x * y).sum();
            // Each of b's faces against the face of a's it is seen with.
            let mut partner: Vec<f32> = of[b]
                .iter()
                .filter_map(|&i| {
                    of[a]
                        .iter()
                        .filter(|&&j| ins[j].seen.iter().any(|f| ins[i].seen.binary_search(f).is_ok()))
                        .map(|&j| tracks[i].embedding.iter().zip(&tracks[j].embedding).map(|(x, y)| x * y).sum::<f32>())
                        .max_by(f32::total_cmp)
                })
                .collect();
            partner.sort_by(f32::total_cmp);
            let median = partner.get(partner.len() / 2).copied().unwrap_or(f32::NAN);
            rows.push((share, cos, name(&of[a]), name(&of[b]), of[a].len(), of[b].len(), median));
        }
    }
    rows.sort_by(|x, y| y.0.total_cmp(&x.0));
    for (share, cos, na, nb, ka, kb, med) in rows.iter().take(30) {
        println!(
            "  {nb} ({kb}) with {na} ({ka}): {:.0}% of the time, mean faces {cos:.3}, with the face beside {med:.3}",
            share * 100.0
        );
    }
    rows.sort_by(|x, y| y.1.total_cmp(&x.1));
    println!("most alike mean faces:");
    for (share, cos, na, nb, ka, kb, med) in rows.iter().take(8) {
        println!(
            "  {nb} ({kb}) with {na} ({ka}): {:.0}% of the time, mean faces {cos:.3}, with the face beside {med:.3}",
            share * 100.0
        );
    }
    rows.retain(|r| r.0 >= 0.5 && !r.6.is_nan());
    rows.sort_by(|x, y| y.6.total_cmp(&x.6));
    println!("seen together at least half the time, most alike to the face beside them:");
    for (share, cos, na, nb, ka, kb, med) in rows.iter().take(8) {
        println!(
            "  {nb} ({kb}) with {na} ({ka}): {:.0}% of the time, mean faces {cos:.3}, with the face beside {med:.3}",
            share * 100.0
        );
    }
}

/// Small deterministic generator for the stress scenarios.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }
    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.next(), self.next());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

/// `e` turned away from itself to cosine `c` (a copy of the face: a mirror,
/// a screen).
fn copy_of(rng: &mut Rng, e: &[f32], c: f32) -> Vec<f32> {
    let mut n: Vec<f32> = e.iter().map(|_| rng.gauss()).collect();
    let along: f32 = n.iter().zip(e).map(|(x, y)| x * y).sum();
    n.iter_mut().zip(e).for_each(|(x, y)| *x -= along * y);
    let len = n.iter().map(|x| x * x).sum::<f32>().sqrt();
    let k = (1.0 - c * c).sqrt();
    e.iter().zip(&n).map(|(y, x)| c * y + k * x / len).collect()
}

/// Per named person (with at least 20 s on screen): how many characters
/// they are most of; and how many characters hold two such people.
fn splits_and_merges(o: &as_identity::Outcome, names: &[Option<&str>], frames: &[u32]) -> (usize, usize, Vec<String>) {
    let got = members(o);
    let mut person_frames: HashMap<&str, u32> = HashMap::new();
    for (i, n) in names.iter().enumerate() {
        if let Some(n) = n.and_then(named) {
            *person_frames.entry(n).or_default() += frames[i];
        }
    }
    let big = |n: &str| person_frames.get(n).is_some_and(|&f| f as f64 / fps() >= 20.0);
    let mut by_char: HashMap<usize, HashMap<&str, u32>> = HashMap::new();
    for (i, c) in got.iter().enumerate() {
        if let (Some(c), Some(n)) = (c, names[i]) {
            *by_char.entry(*c).or_default().entry(n).or_default() += frames[i];
        }
    }
    let mut majority: HashMap<&str, usize> = HashMap::new();
    let mut merges = 0;
    let mut notes = Vec::new();
    for (c, m) in &by_char {
        let total: u32 = m.values().sum();
        let (top, f) = m.iter().max_by_key(|(n, f)| (**f, std::cmp::Reverse(**n))).map(|(n, f)| (*n, *f)).unwrap();
        if big(top) && f * 2 > total {
            *majority.entry(top).or_default() += 1;
        }
        let strong = m.iter().filter(|(n, f)| big(n) && **f as f32 >= 0.2 * total as f32).count();
        if strong >= 2 {
            merges += 1;
            notes.push(format!("char {c} mixes {m:?}"));
        }
    }
    let mut splits = 0;
    for (n, k) in &majority {
        if *k > 1 {
            splits += k - 1;
            notes.push(format!("{n} is {k} characters"));
        }
    }
    (splits, merges, notes)
}

/// The clustering on purpose made hard, on real faces: mirror copies of
/// faces, the episode cut into episodes, faces left out, other thresholds,
/// a cast built up episode by episode. Counts people split over characters
/// and characters holding two people.
fn stress(tracks: &[FaceTrack], labels_path: &str) {
    let labels = load_labels(labels_path);
    let names = names_of(tracks, &labels);
    let base = input(tracks);
    let frames: Vec<u32> = tracks.iter().map(|t| t.end - t.start).collect();
    let mut total = (0, 0, 0);
    let mut run = |what: &str, ins: &[as_identity::TrackIn], nm: &[Option<&str>], fr: &[u32], p: &Params| {
        let o = identify(ins, p);
        let (s, m, notes) = splits_and_merges(&o, nm, fr);
        total.0 += 1;
        total.1 += s;
        total.2 += m;
        println!("{what:44} {:3} characters, {s} split, {m} merged {}", o.characters.len(), notes.join("; "));
    };
    let p = env_params();
    run("as found", &base, &names, &frames, &p);
    // The person with the most clear faces (Walter in Breaking Bad).
    let lead: &str = {
        let mut by: HashMap<&str, usize> = HashMap::new();
        for (i, n) in names.iter().enumerate() {
            if let Some(n) = n.and_then(named)
                && base[i].quality >= 0.6
            {
                *by.entry(n).or_default() += 1;
            }
        }
        by.into_iter().max_by_key(|&(n, c)| (c, std::cmp::Reverse(n))).map_or("?", |(n, _)| n)
    };

    // How alike a copy is to its face (`STRESS_COPY_LO/HI`), and the share
    // of its own face in a reflection (`STRESS_REFL_LO/HI`: the four
    // reflections below spread over that range as over 0.53 – 0.65).
    let var = |k: &str, d: f32| std::env::var(k).ok().map_or(d, |v| v.parse().unwrap_or_else(|_| panic!("{k}=<cos>")));
    let (copy_lo, copy_hi) = (var("STRESS_COPY_LO", 0.5), var("STRESS_COPY_HI", 0.75));
    let (refl_lo, refl_hi) = (var("STRESS_REFL_LO", 0.53), var("STRESS_REFL_HI", 0.65));
    // Without either variable the shares are used as they are, not mapped
    // onto the same range: the mapping's rounding would change the default
    // output in the last bits.
    let refl_set = ["STRESS_REFL_LO", "STRESS_REFL_HI"].iter().any(|k| std::env::var_os(k).is_some());
    let own_share = |own: f32| {
        if refl_set { refl_lo + (refl_hi - refl_lo) * (own - 0.53) / (0.65 - 0.53) } else { own }
    };

    // Mirror copies: a face shown twice at once (another place, cos 0.5–0.75).
    for seed in 1..=6u64 {
        let mut rng = Rng(0x9e37_79b9 ^ (seed * 7919));
        let (mut ins, mut nm, mut fr) = (base.clone(), names.clone(), frames.clone());
        let n = [3, 8, 15, 30, 60, 120][seed as usize - 1];
        for _ in 0..n {
            let i = (rng.next() * base.len() as f32) as usize % base.len();
            if base[i].quality < 0.3 {
                continue;
            }
            let c = copy_lo + (copy_hi - copy_lo) * rng.next();
            let mut t = base[i].clone();
            t.embedding = copy_of(&mut rng, &base[i].embedding, c);
            t.quality = (base[i].quality * (0.7 + 0.3 * rng.next())).max(0.05);
            ins.push(t);
            nm.push(names[i]);
            fr.push(frames[i]);
        }
        run(&format!("{n} mirror copies (seed {seed})"), &ins, &nm, &fr, &p);
    }
    // A run of copies in one place: a person and their reflection, shot
    // after shot. Each reflection is like its own face at that moment (real
    // ones: 0.53 – 0.64) and like the other reflections (the same glass, the
    // same light): its face, a share of the mirror's look, and noise.
    for (seed, (own, glass)) in [(0.55f32, 0.6f32), (0.55, 0.75), (0.53, 0.75), (0.65, 0.5)].into_iter().enumerate() {
        let mut rng = Rng(77 + seed as u64);
        let own = own_share(own);
        let (mut ins, mut nm, mut fr) = (base.clone(), names.clone(), frames.clone());
        let theirs: Vec<usize> =
            (0..base.len()).filter(|&i| names[i] == Some(lead) && base[i].quality >= 0.6).skip(10).take(8).collect();
        let mirror: Vec<f32> = copy_of(&mut rng, &base[theirs[0]].embedding, 0.0);
        // Own face, glass and noise are orthogonal unit parts: with more than
        // the whole in the first two, the reflection's likeness to its face
        // would no longer follow `own`.
        assert!(
            own * own + glass * glass <= 1.0,
            "STRESS_REFL_LO/HI: own share {own:.3} and glass {glass} make more than a whole face"
        );
        let rest = (1.0 - own * own - glass * glass).max(0.0).sqrt();
        let mut sims = Vec::new();
        for &i in &theirs {
            let noise = copy_of(&mut rng, &base[i].embedding, 0.0);
            let mut e: Vec<f32> =
                (0..mirror.len()).map(|d| own * base[i].embedding[d] + glass * mirror[d] + rest * noise[d]).collect();
            let len = e.iter().map(|x| x * x).sum::<f32>().sqrt();
            e.iter_mut().for_each(|x| *x /= len);
            sims.push(e.iter().zip(&base[i].embedding).map(|(x, y)| x * y).sum::<f32>());
            let mut t = base[i].clone();
            t.embedding = e;
            ins.push(t);
            nm.push(Some(lead));
            fr.push(frames[i]);
        }
        let mean = sims.iter().sum::<f32>() / sims.len() as f32;
        run(&format!("8 reflections in a row ({mean:.2} like the face, glass {glass})"), &ins, &nm, &fr, &p);
    }
    // Episodes: the episode cut into 2..8 media.
    for parts in [2u32, 4, 8] {
        let last = base.iter().map(|t| t.end).max().unwrap_or(1);
        let ins: Vec<_> = base
            .iter()
            .cloned()
            .map(|mut t| {
                t.media = t.start * parts / last;
                t
            })
            .collect();
        run(&format!("cut into {parts} episodes"), &ins, &names, &frames, &p);
    }
    // Faces left out.
    for seed in 1..=4u64 {
        let mut rng = Rng(seed * 104_729);
        let keep: Vec<usize> = (0..base.len()).filter(|_| rng.next() < 0.7).collect();
        let ins: Vec<_> = keep.iter().map(|&i| base[i].clone()).collect();
        let nm: Vec<_> = keep.iter().map(|&i| names[i]).collect();
        let fr: Vec<_> = keep.iter().map(|&i| frames[i]).collect();
        run(&format!("70% of the faces (seed {seed})"), &ins, &nm, &fr, &p);
    }
    // Other thresholds.
    for (link, copy_link) in [(0.25, 0.6), (0.35, 0.6), (0.4, 0.6), (0.3, 0.5), (0.3, 0.7)] {
        let q = Params { link, copy_link, ..p };
        run(&format!("link {link}, copy_link {copy_link}"), &base, &names, &frames, &q);
    }
    // The cast built up episode by episode, characters kept from one run to the next.
    {
        let parts = 4u32;
        let last = base.iter().map(|t| t.end).max().unwrap_or(1);
        let all: Vec<_> = base
            .iter()
            .cloned()
            .map(|mut t| {
                t.media = t.start * parts / last;
                t
            })
            .collect();
        let mut keys: HashMap<usize, u32> = HashMap::new();
        let mut next_key = 0u32;
        for upto in 1..=parts {
            let idx: Vec<usize> = (0..all.len()).filter(|&i| all[i].media < upto).collect();
            let ins: Vec<_> = idx
                .iter()
                .map(|&i| {
                    let mut t = all[i].clone();
                    t.previous = keys.get(&i).copied();
                    t
                })
                .collect();
            let o = identify(&ins, &p);
            let key_of: Vec<u32> = o
                .characters
                .iter()
                .map(|c| {
                    c.key.unwrap_or_else(|| {
                        next_key += 1;
                        next_key + 999
                    })
                })
                .collect();
            keys.clear();
            for (k, &i) in idx.iter().enumerate() {
                if let Some(c) = o.tracks[k].0.character() {
                    keys.insert(i, key_of[c]);
                }
            }
            let nm: Vec<_> = idx.iter().map(|&i| names[i]).collect();
            let fr: Vec<_> = idx.iter().map(|&i| frames[i]).collect();
            run(&format!("episode by episode, after {upto}"), &ins, &nm, &fr, &p);
        }
        println!("  characters ever made: {next_key}");
    }
    println!("{} runs: {} people split, {} characters merged", total.0, total.1, total.2);
}

/// A series: the episode `n` times over as `n` episodes. In each new one a
/// named person's face keeps how much it says about who they are (its part
/// along their mean face) but looks like another shot of theirs (the rest
/// of the face, from a random other track of theirs): the same people, new
/// shots — no odd shot repeated in every episode. Unnamed faces are others
/// each time. How long grouping takes, and whether everyone is still one
/// character.
fn scale(tracks: &[FaceTrack], labels_path: &str, n: u32) {
    let labels = load_labels(labels_path);
    let names = names_of(tracks, &labels);
    let base = input(tracks);
    let frames: Vec<u32> = tracks.iter().map(|t| t.end - t.start).collect();
    let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let named = |i: usize| names[i].and_then(labels::named);
    let mut of: HashMap<&str, Vec<usize>> = HashMap::new();
    for i in 0..base.len() {
        if let Some(name) = named(i) {
            of.entry(name).or_default().push(i);
        }
    }
    let mean: HashMap<&str, Vec<f32>> = of
        .iter()
        .map(|(name, ids)| {
            let mut m = vec![0f32; base[0].embedding.len()];
            for &i in ids {
                m.iter_mut().zip(&base[i].embedding).for_each(|(x, y)| *x += y);
            }
            let len = dot(&m, &m).sqrt().max(1e-6);
            m.iter_mut().for_each(|x| *x /= len);
            (*name, m)
        })
        .collect();
    let rest = |e: &[f32], m: &[f32]| -> (f32, Vec<f32>) {
        let a = dot(e, m);
        (a, e.iter().zip(m).map(|(x, y)| x - a * y).collect())
    };
    // One face seen twice at once (a mirror): in each new episode the second
    // stays a copy of the first, as alike as here.
    let mut copies: Vec<(usize, usize, f32)> = Vec::new();
    for i in 0..base.len() {
        for j in i + 1..base.len() {
            let (a, b) = (&base[i], &base[j]);
            if named(i).is_some() && named(i) == named(j) && a.seen.iter().any(|f| b.seen.binary_search(f).is_ok()) {
                copies.push((i, j, dot(&a.embedding, &b.embedding)));
            }
        }
    }
    let mut rng = Rng(4242);
    let (mut ins, mut nm, mut fr) = (Vec::new(), Vec::new(), Vec::new());
    for m in 0..n {
        let first = ins.len();
        for (i, t) in base.iter().enumerate() {
            let mut t = t.clone();
            t.media = m;
            if m > 0 {
                t.embedding = match named(i) {
                    Some(name) => {
                        let (mu, ids) = (&mean[name], &of[name]);
                        let (a, own) = rest(&t.embedding, mu);
                        let j = ids[(rng.next() * ids.len() as f32) as usize % ids.len()];
                        let (_, other) = rest(&base[j].embedding, mu);
                        let k = dot(&own, &own).sqrt() / dot(&other, &other).sqrt().max(1e-6);
                        let mut e: Vec<f32> = mu.iter().zip(&other).map(|(x, y)| a * x + k * y).collect();
                        let len = dot(&e, &e).sqrt().max(1e-6);
                        e.iter_mut().for_each(|x| *x /= len);
                        e
                    }
                    None => copy_of(&mut rng, &t.embedding, 0.5),
                };
            }
            ins.push(t);
            nm.push(names[i]);
            fr.push(frames[i]);
        }
        if m > 0 {
            for &(i, j, c) in &copies {
                let e = copy_of(&mut rng, &ins[first + i].embedding, c);
                ins[first + j].embedding = e;
            }
        }
    }
    let t0 = std::time::Instant::now();
    let o = identify(&ins, &Params::default());
    let ms = t0.elapsed().as_millis();
    let (s, m, notes) = splits_and_merges(&o, &nm, &fr);
    println!(
        "{n} episodes, {} faces: {} characters in {ms} ms, {s} split, {m} merged {}",
        ins.len(),
        o.characters.len(),
        notes.join("; ")
    );
    if let Ok(detail) = std::env::var("DETAIL") {
        // Where the people who are split are: per character, their tracks
        // per episode. `DETAIL=<name>,…` names the people to show.
        let got = members(&o);
        let mut by: HashMap<(&str, usize), Vec<u32>> = HashMap::new();
        for (i, c) in got.iter().enumerate() {
            if let (Some(c), Some(name)) = (c, nm[i]) {
                let v = by.entry((name, *c)).or_insert_with(|| vec![0; n as usize]);
                v[ins[i].media as usize] += 1;
            }
        }
        let shown: HashSet<&str> = match detail.as_str() {
            "1" => {
                // Named people on screen 20 s or more (as `splits_and_merges`)
                // with two tracks or more in each of two characters or more.
                let mut secs: HashMap<&str, f64> = HashMap::new();
                for (i, name) in nm.iter().enumerate() {
                    if let Some(name) = name.and_then(labels::named) {
                        *secs.entry(name).or_default() += f64::from(fr[i]) / fps();
                    }
                }
                let mut of: HashMap<&str, usize> = HashMap::new();
                for ((name, _), per) in &by {
                    if per.iter().sum::<u32>() >= 2 && secs.get(name).is_some_and(|&s| s >= 20.0) {
                        *of.entry(name).or_default() += 1;
                    }
                }
                of.into_iter().filter(|&(_, k)| k >= 2).map(|(n, _)| n).collect()
            }
            names => names.split(',').map(str::trim).collect(),
        };
        let mut rows: Vec<_> = by.into_iter().filter(|((name, _), _)| shown.contains(name)).collect();
        rows.sort();
        let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let mut main: HashMap<&str, usize> = HashMap::new();
        for ((name, c), per) in &rows {
            let total: u32 = per.iter().sum();
            let e = main.entry(name).or_insert(*c);
            let best: u32 =
                rows.iter().filter(|((n, k), _)| n == name && k == e).map(|(_, p)| p.iter().sum::<u32>()).sum();
            if total > best {
                *e = *c;
            }
        }
        for ((name, c), per) in &rows {
            if per.iter().sum::<u32>() >= 2 {
                let m = main[name];
                println!(
                    "  {name:8} character {c:3}: {per:?} mean faces with their main one {:.3}",
                    cos(&o.characters[*c].centroid, &o.characters[m].centroid)
                );
            }
        }
    }
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

/// Default parameters (SFace's with `SFACE=1`), with experiments from the
/// environment.
fn env_params() -> Params {
    let mut p = if std::env::var_os("SFACE").is_some() { Params::sface() } else { Params::default() };
    let var = |k: &str| std::env::var(k).ok().map(|v| v.parse::<f32>().unwrap());
    if let Some(v) = var("CALIBRATE_FROM") {
        p.calibrate_from = v as usize;
    }
    if let Some(v) = var("MIN_COH") {
        p.min_cohesion = v;
    }
    if let Some(v) = var("LINK") {
        p.link = v;
    }
    let knobs: [(&str, &mut f32); 7] = [
        ("ASSIGN", &mut p.assign),
        ("MARGIN", &mut p.margin),
        ("UNSURE", &mut p.unsure),
        ("COPY_LINK", &mut p.copy_link),
        ("COPY_ALIKE", &mut p.copy_alike),
        ("COPY_SHARE", &mut p.copy_share),
        ("INHERIT", &mut p.inherit),
    ];
    for (k, f) in knobs {
        if let Some(v) = var(k) {
            *f = v;
        }
    }
    p.animated = std::env::var_os("ANIMATED").is_some();
    p.adapt = std::env::var_os("ADAPT").is_none_or(|v| v != "0");
    if let Some(v) = var("CORE_QUALITY") {
        p.core_quality = v;
    }
    if let Some(v) = var("STRANGERS_ALONE") {
        p.strangers_alone = v;
    }
    if let Some(v) = var("STRANGERS_STYLE") {
        p.strangers_style = v;
    }
    p
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a[0] == "paired" {
        return paired(&a[1], &a[2]);
    }
    let tracks = read_tracks(&a[1]);
    match a[0].as_str() {
        "separation" => separation(&tracks, &a[2]),
        "chars" => chars(&tracks, &a[2]),
        "overlap" => overlap(&tracks, &a[2]),
        "stress" => stress(&tracks, &a[2]),
        "scale" => scale(&tracks, &a[2], a.get(3).map_or(10, |s| s.parse().unwrap())),
        "copresence" => copresence(&tracks, &a[2]),
        "dets" => {
            for i in a[2..].iter().map(|s| s.parse::<usize>().unwrap()) {
                let t = &tracks[i];
                println!("track {i} {}-{} q {:.2}", t.start, t.end, t.quality);
                for d in &t.dets {
                    println!(
                        "  {} {:?} score {:.2} q {:.2}",
                        d.frame,
                        d.bbox.map(|v| (v * 1000.0).round() / 1000.0),
                        d.score,
                        d.quality
                    );
                }
            }
        }
        "pairs" => pairs(&tracks, &a[2], a.get(3).map_or(0.5, |s| s.parse().unwrap())),
        "norms" => norms(&tracks, &read_tracks(&a[2]), a[3].parse().unwrap(), a[4].parse().unwrap()),
        "sheets" => sheets(&tracks, Path::new(&a[2])),
        "labels" => labels(&tracks, Path::new(&a[2]), &a[3], &a[4]),
        "score" => {
            let mut p = env_params();
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
            let ins = input(&tracks);
            let fitted = p.adapt.then(|| as_identity::fit(&ins, &p)).flatten();
            // `ZOUT=<file>`: the faces in the fitted space, as an EMB_OVERRIDE file.
            if let (Some(path), Some((f, _))) = (std::env::var_os("ZOUT"), &fitted) {
                let z: Vec<Vec<f32>> = ins.iter().map(|t| f.embed(&t.embedding)).collect();
                let mut b = Vec::new();
                b.extend_from_slice(&(z.len() as u32).to_le_bytes());
                b.extend_from_slice(&(z[0].len() as u32).to_le_bytes());
                z.iter().flatten().for_each(|x| b.extend_from_slice(&x.to_le_bytes()));
                std::fs::write(path, b).unwrap();
            }
            match fitted {
                Some((f, st)) => println!(
                    "fitted space: strangers {} alike {:.5} share {:.4} | seeds: bar {:.5} tracks {} groups {} | pairs same {} other {} | k {} weight {:.4} tail {:.5} → link = unsure {:.5} copy_link {:.5} copy_alike {:.5}",
                    st.strangers,
                    st.alike,
                    st.share,
                    st.seed_bar,
                    st.seed_tracks,
                    st.seed_groups,
                    st.same_pairs,
                    st.other_pairs,
                    f.k,
                    f.weight,
                    st.tail,
                    f.link,
                    f.copy_link,
                    f.copy_alike
                ),
                None => {
                    let f = p.fitted(&ins);
                    println!(
                        "strangers alike {:?} → link {:.3} assign {:.3} unsure {:.3} cohesion {:.3}",
                        as_identity::strangers(&ins, &p),
                        f.link,
                        f.assign,
                        f.unsure,
                        f.min_cohesion
                    );
                }
            }
            score(&tracks, &a[2], &p, std::env::var_os("DETAIL").is_some())
        }
        other => panic!("unknown command {other}"),
    }
}

/// Mutation tests: from synthetic tracks grouped perfectly, each mistake
/// moves the number it is about and leaves the others as they were.
#[cfg(test)]
mod tests {
    use super::*;
    use labels::tests::{label_of, track};

    /// Frames per shot: a track per shot, each 12.5 s.
    const SHOT: u32 = 300;
    /// Where each figure's head is, and where `n` (not a head) is.
    const PLACE: [[f32; 4]; 4] =
        [[0.10, 0.10, 0.30, 0.40], [0.40, 0.10, 0.60, 0.40], [0.70, 0.10, 0.90, 0.40], [0.40, 0.50, 0.60, 0.80]];
    const NOT_A_HEAD: [f32; 4] = [0.05, 0.60, 0.15, 0.75];

    struct World {
        tracks: Vec<FaceTrack>,
        labels: Vec<Label>,
        got: Vec<Option<usize>>,
    }

    fn setup(shots: Option<Vec<(u32, u32)>>) -> Setup {
        Setup {
            gallery: None,
            level: Level::Look,
            presence: None,
            spans: Spans::default(),
            shots,
            main_secs: 30.0,
            episodes: Vec::new(),
        }
    }

    impl World {
        fn empty() -> Self {
            World { tracks: Vec::new(), labels: Vec::new(), got: Vec::new() }
        }

        /// Main figures A (8 tracks, 100 s), B (6) and C (5), and D (one
        /// track, not main), each their own character.
        fn perfect() -> Self {
            let mut w = World::empty();
            for (f, (name, n)) in [("A", 8), ("B", 6), ("C", 5), ("D", 1)].into_iter().enumerate() {
                for _ in 0..n {
                    w.add(name, PLACE[f], Some(f));
                }
            }
            w
        }

        /// A track in a shot of its own, labelled `name`, in character `c`.
        fn add(&mut self, name: &str, bbox: [f32; 4], c: Option<usize>) {
            let start = SHOT * self.labels.len() as u32;
            let t = track(start, start + SHOT, bbox);
            self.labels.push(label_of(&t, name, bbox));
            self.tracks.push(t);
            self.got.push(c);
        }

        /// The tracks labelled `name`.
        fn of(&self, name: &str) -> Vec<usize> {
            let at: HashSet<u32> = self.labels.iter().filter(|l| l.name == name).map(|l| l.span.0).collect();
            (0..self.tracks.len()).filter(|&i| at.contains(&self.tracks[i].start)).collect()
        }

        /// A shot per label.
        fn shots(&self) -> Vec<(u32, u32)> {
            (0..self.labels.len() as u32).map(|k| (k * SHOT, (k + 1) * SHOT)).collect()
        }

        fn report(&self, shots: bool) -> Report {
            let setup = setup(shots.then(|| self.shots()));
            let cast = setup.cast(&self.labels);
            assert_eq!(cast.figures.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["A", "B", "C"]);
            self.report_with(&setup)
        }

        /// As `score` with `setup`: names at its level, its main cast.
        fn report_with(&self, setup: &Setup) -> Report {
            let (labels, cast) = setup.name_labels(self.labels.clone());
            let truth = names_of(&self.tracks, &labels);
            report(&self.tracks, &labels, &truth, &self.got, &cast, setup)
        }

        fn numbers(&self) -> Numbers {
            Self::numbers_of(&self.report(true), self)
        }

        fn numbers_with(&self, setup: &Setup) -> Numbers {
            Self::numbers_of(&self.report_with(setup), self)
        }

        fn numbers_of(r: &Report, w: &World) -> Numbers {
            let (held, theirs) = r.held();
            let (clip_precision, clip_recall, _, _) = r.clips().unwrap();
            Numbers {
                labelled: names_of(&w.tracks, &w.labels).iter().flatten().count(),
                precision: r.track.precision,
                recall: r.track.recall,
                held,
                theirs,
                merges: r.merges(),
                clip_precision,
                clip_recall,
            }
        }
    }
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Numbers {
        labelled: usize,
        precision: f32,
        recall: f32,
        held: f64,
        theirs: f64,
        merges: usize,
        clip_precision: f64,
        clip_recall: f64,
    }

    const PERFECT: Numbers = Numbers {
        labelled: 20,
        precision: 1.0,
        recall: 1.0,
        held: 1.0,
        theirs: 1.0,
        merges: 0,
        clip_precision: 1.0,
        clip_recall: 1.0,
    };

    #[test]
    fn a_perfect_grouping_scores_perfectly() {
        assert_eq!(World::perfect().numbers(), PERFECT);
    }

    #[test]
    fn two_main_figures_merged_cost_precision_alone() {
        let mut w = World::perfect();
        for i in w.of("B") {
            w.got[i] = Some(0);
        }
        let n = w.numbers();
        assert!(n.precision < 1.0 && n.clip_precision < 1.0, "{n:?}");
        assert!(n.theirs < 1.0, "B's frames are on A's character: {n:?}");
        assert_eq!((n.labelled, n.recall, n.held, n.merges), (20, 1.0, 1.0, 0));
    }

    #[test]
    fn a_figure_split_costs_recall_and_a_merge() {
        let mut w = World::perfect();
        for &i in &w.of("A")[4..] {
            w.got[i] = Some(9);
        }
        let n = w.numbers();
        assert!(n.recall < 1.0 && n.clip_recall < 1.0, "{n:?}");
        assert_eq!(n.merges, 1);
        assert_eq!((n.labelled, n.precision, n.held, n.theirs, n.clip_precision), (20, 1.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn not_a_head_on_a_main_figure_costs_precision_alone() {
        let mut w = World::perfect();
        for _ in 0..3 {
            w.add("n", NOT_A_HEAD, Some(0));
        }
        let n = w.numbers();
        assert!(n.precision < 1.0 && n.clip_precision < 1.0, "{n:?}");
        assert_eq!(n.labelled, 23);
        assert_eq!((n.recall, n.held, n.theirs, n.merges, n.clip_recall), (1.0, 1.0, 1.0, 0, 1.0));
    }

    #[test]
    fn dropped_tracks_cost_coverage_alone() {
        let mut w = World::perfect();
        for &i in w.of("A")[..3].iter().rev() {
            w.tracks.remove(i);
            w.got.remove(i);
        }
        let n = w.numbers();
        assert!(n.held < 1.0 && n.theirs < 1.0 && n.clip_recall < 1.0, "{n:?}");
        assert_eq!(n.labelled, 17);
        assert_eq!((n.precision, n.recall, n.merges, n.clip_precision), (1.0, 1.0, 0, 1.0));
    }

    #[test]
    fn face_boxes_inside_head_boxes_are_those_heads() {
        // The tracks box faces, the labels heads: a face is well inside its
        // head box but overlaps it too little for IoU alone.
        let face = |b: [f32; 4]| {
            let (w, h) = (b[2] - b[0], b[3] - b[1]);
            [b[0] + 0.2 * w, b[1] + 0.25 * h, b[2] - 0.2 * w, b[3] - 0.1 * h]
        };
        assert!(labels::box_iou(&PLACE[0], &face(PLACE[0])) < 0.5);
        let mut w = World::perfect();
        w.tracks.iter_mut().flat_map(|t| &mut t.dets).for_each(|d| d.bbox = face(d.bbox));
        assert_eq!(w.numbers(), PERFECT);
        // Moved out of the head box (its centre outside): no longer that head.
        for i in w.of("C") {
            w.tracks[i].dets.iter_mut().for_each(|d| d.bbox = [d.bbox[0] + 0.2, d.bbox[1], d.bbox[2] + 0.2, d.bbox[3]]);
        }
        let n = w.numbers();
        assert!(n.held < 1.0 && n.theirs < 1.0 && n.clip_recall < 1.0, "{n:?}");
        assert_eq!(n.labelled, 15);
        assert_eq!((n.precision, n.recall, n.merges, n.clip_precision), (1.0, 1.0, 0, 1.0));
    }

    #[test]
    fn items_add_up_to_the_numbers_and_the_bootstrap_resamples_figures() {
        let items = |w: &World| -> Vec<Item> { w.report(false).items.into_iter().map(|x| x.1).collect() };
        let mut merged = World::perfect();
        for i in merged.of("B") {
            merged.got[i] = Some(0);
        }
        let r = merged.report(false);
        let m = scoring::metrics(r.items.iter().map(|x| &x.1));
        assert!((m[0] - f64::from(r.track.f)).abs() < 1e-6 && (m[1] - f64::from(r.time.f)).abs() < 1e-6);
        let (a, b) = (items(&World::perfect()), items(&merged));
        assert_eq!(scoring::bootstrap(&a, 200, BOOT_SEED)[0], (1.0, 1.0));
        // A and B lost precision, C didn't: the resamples spread round the
        // point (all C: 1.0, all B: 0.6).
        let iv = scoring::bootstrap(&b, 2000, BOOT_SEED);
        for k in 0..2 {
            let (lo, hi) = iv[k];
            assert!(lo < m[k] && m[k] < hi, "{} {} not inside {iv:?}", scoring::METRICS[k], m[k]);
        }
        // No packs without shots: clip precision is NaN.
        assert!(iv[2].0.is_nan() && iv[2].1.is_nan());
        // Paired: one draw of figures for both runs, so a run against itself
        // never differs, however much its figures do.
        let same = scoring::paired(&b, &b, 2000, BOOT_SEED);
        assert_eq!((same[0], same[1]), ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0)));
        let worse = scoring::paired(&a, &b, 2000, BOOT_SEED);
        let (d, lo, hi) = worse[0];
        assert!(lo < d && d < hi && hi <= 0.0, "{worse:?}");
        assert!((d - (m[0] - 1.0)).abs() < 1e-12);
    }

    /// Tanjiro as himself (4 tracks) and in his uniform (3), both one
    /// figure, behind his mask (3, a figure of its own), and Nezuko (6).
    /// Each figure is a character; at `LEVEL=look` both of Tanjiro's looks
    /// are one.
    fn looks() -> World {
        let mut w = World::empty();
        for (name, n, c) in [("TAN", 4, 0), ("TAN_U", 3, 0), ("TAN_M", 3, 1), ("NEZ", 6, 2)] {
            for _ in 0..n {
                w.add(name, PLACE[c], Some(c));
            }
        }
        w
    }

    fn at(level: Level) -> Setup {
        Setup { gallery: Some(anime::tests::gallery()), level, ..setup(None) }
    }

    #[test]
    fn figures_hold_a_persons_looks_but_not_their_masks() {
        let pr = |w: &World, level: Level| {
            let r = w.report_with(&at(level));
            (r.track.precision, r.track.recall)
        };
        let mut w = looks();
        let cast = at(Level::Figure).cast(&w.labels);
        let figures: Vec<&str> = cast.figures.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(figures, ["tanjiro", "nezuko", "TAN_M"]);
        assert_eq!(pr(&w, Level::Figure), (1.0, 1.0));
        let (p, r) = pr(&w, Level::Look);
        assert!(p < 1.0 && r == 1.0, "two looks in one character: {p} {r}");
        let (p, r) = pr(&w, Level::Person);
        assert!(p == 1.0 && r < 1.0, "the mask apart from its person: {p} {r}");
        // At the look level the bootstrap still draws figures.
        let look = w.report_with(&at(Level::Look));
        let mut rows: Vec<&str> = look.rows.iter().map(|r| r.name.as_str()).collect();
        let mut items: Vec<&str> = look.items.iter().map(|x| x.0.as_str()).collect();
        rows.sort_unstable();
        items.sort_unstable();
        assert_eq!((rows, items), (vec!["NEZ", "TAN", "TAN_M", "TAN_U"], vec!["TAN_M", "nezuko", "tanjiro"]));
        let sum = scoring::metrics(look.items.iter().map(|x| &x.1));
        assert!((sum[0] - f64::from(look.track.f)).abs() < 1e-6);
        // The mask put on Tanjiro: a merge of two figures, one person.
        for i in w.of("TAN_M") {
            w.got[i] = Some(0);
        }
        let (p, r) = pr(&w, Level::Figure);
        assert!(p < 1.0 && r == 1.0, "{p} {r}");
        assert_eq!(pr(&w, Level::Person), (1.0, 1.0));
    }

    #[test]
    fn tracks_in_op_ed_spans_are_left_out() {
        let mut w = World::perfect();
        let first = w.labels.len() as u32;
        // B in the opening, on A's character.
        for _ in 0..3 {
            w.add("B", PLACE[1], Some(0));
        }
        let span = (SHOT * first, SHOT * w.labels.len() as u32);
        assert!(w.numbers_with(&setup(Some(w.shots()))).precision < 1.0);
        let s = Setup { spans: Spans(vec![(span.0, span.1, "op".into())]), ..setup(Some(w.shots())) };
        assert_eq!(w.numbers_with(&s), Numbers { labelled: 23, ..PERFECT });
    }

    #[test]
    fn shot_presence_fixes_the_cast_and_the_true_shots() {
        let w = World::perfect();
        // D, labelled in one shot, is seen in A's 8 shots too (from behind):
        // 112.5 s by presence, the most of all.
        let rows: Vec<Presence> = w
            .labels
            .iter()
            .map(|l| {
                let mut codes = vec![l.name.clone()];
                if l.name == "A" {
                    codes.push("D".into());
                }
                Presence { start: l.span.0, end: l.span.1, codes }
            })
            .collect();
        let s = Setup { presence: Some(rows), ..setup(Some(w.shots())) };
        let cast = s.cast(&w.labels);
        assert_eq!(cast.from, "shot presence");
        assert_eq!(cast.figures.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["D", "A", "B", "C"]);
        let r = w.report_with(&s);
        let d = r.rows.iter().find(|r| r.name == "D").unwrap().pack.unwrap();
        assert_eq!((d.shots, d.right, d.truth), (1, 1, 9));
        let n = w.numbers_with(&s);
        assert!(n.clip_recall < 1.0, "{n:?}");
        assert_eq!((n.precision, n.recall, n.held, n.theirs, n.merges, n.clip_precision), (1.0, 1.0, 1.0, 1.0, 0, 1.0));
    }

    #[test]
    fn shots_of_faces_nobody_could_name_leave_clip_precision() {
        let mut w = World::perfect();
        for name in ["x", "?u"] {
            w.add(name, PLACE[3], Some(0));
        }
        assert_eq!(w.numbers(), Numbers { labelled: 22, ..PERFECT });
        assert_eq!(w.report(true).clips().unwrap().3, 2);
        // Someone of their own there is a wrong shot.
        w.add("?e", PLACE[3], Some(0));
        let n = w.numbers();
        assert!(n.precision < 1.0 && n.clip_precision < 1.0, "{n:?}");
    }

    #[test]
    fn merges_to_clean_per_episode() {
        let mut w = World::perfect();
        // A's last 4 tracks (from frame 1200) in a character of their own.
        for &i in &w.of("A")[4..] {
            w.got[i] = Some(9);
        }
        let mut s = setup(Some(w.shots()));
        let r = w.report_with(&s);
        assert_eq!((r.merges(), r.worst_episode(), &r.rows[0].per_episode[..]), (1, 1, &[2][..]));
        // Cut where A's second character starts: each episode holds one.
        s.episodes = vec![4 * SHOT];
        let r = w.report_with(&s);
        assert_eq!((r.rows[0].name.as_str(), r.merges(), r.worst_episode()), ("A", 1, 0));
        assert_eq!((&r.rows[0].per_episode[..], &r.rows[1].per_episode[..]), (&[1, 1][..], &[0, 1][..]));
        // Cut earlier: the second episode holds both.
        s.episodes = vec![2 * SHOT];
        assert_eq!(w.report_with(&s).rows[0].per_episode, [1, 2]);
    }

    #[test]
    fn unassigned_tracks_cost_no_merge_but_show() {
        let mut w = World::perfect();
        for &i in &w.of("A")[..6] {
            w.got[i] = None;
        }
        let r = w.report(true);
        let a = &r.rows[0];
        assert_eq!((a.name.as_str(), a.groups, a.unassigned), ("A", 1, 0.75));
        assert!(r.held().1 < 1.0);
    }
}
