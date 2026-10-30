//! How the scenepack builder copes with a huge pack: a series of 60 episodes
//! with 6000 shots of one character. Not run with the other tests:
//!
//! ```text
//! cargo test -p as-project --release --lib -- --ignored --nocapture pack::bench
//! ```
//!
//! With `AUTOSCENE_PACK_BENCH_DIR` set, the library is only made, there, and
//! kept: open it in the app with `AUTOSCENE_DATA_DIR=<dir>/data`.
//! `AUTOSCENE_PACK_BENCH_PROJECT` names a project folder (a copy!) whose
//! character with the most shots is measured the same way.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use as_api::{AppSettings, ContentKind, NewProject, ProjectKind, ProxyState, Segment, SegmentKind, StageState};
use as_core::{AppPaths, ProjectId, Rational};
use as_db::{IngestRecord, NewTrack};

use super::*;

const EPISODES: usize = 60;
/// Shots of the character in each episode; a sixth of its shots are others'.
const SHOTS: usize = 100;
const FPS: (i64, i64) = (24000, 1001);

/// A small deterministic generator (xorshift): every run measures the same pack.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn between(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

fn face(shot: FrameIdx, start: FrameIdx, end: FrameIdx, quality: f32, face_h: f32) -> NewTrack {
    NewTrack { start, end, shot, quality, face_h, dets: vec![0; 64], emb: vec![0; 1024], rival: 0.0 }
}

/// A series of [`EPISODES`] episodes in `dir`, with Walter in [`SHOTS`]
/// shots of each — most of them clear and big, as real faces are (their
/// quality tops out at 1) — Jesse with him in some, Skyler in a few.
fn series(dir: &Path) -> (Arc<Project>, CharacterId) {
    let lib = crate::Library::open(&AppPaths::at(dir.join("data"))).unwrap();
    lib.update_settings(&AppSettings {
        library_root: dir.join("Library").display().to_string(),
        export_preset_id: "h264-edit".into(),
        audio_language: String::new(),
        find_bodies: false,
        fast_scan: false,
        anilist: false,
    })
    .unwrap();
    let files: Vec<String> = (1..=EPISODES)
        .map(|e| {
            let file = dir.join(format!("Show S01E{e:02}.mkv"));
            std::fs::write(&file, b"x").unwrap();
            file.display().to_string()
        })
        .collect();
    let new = NewProject { title: "Show".into(), kind: ProjectKind::Series, content: ContentKind::Live, files };
    let (_, project, ids) = lib.create_project(&new).unwrap();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let fps = Rational::new(FPS.0, FPS.1);
    let mut people: [Vec<i64>; 3] = Default::default();
    for &media in &ids {
        // The shots: theirs, and now and then someone else's.
        let mut shots = Vec::new();
        let mut at = 0;
        for i in 0..SHOTS * 6 / 5 {
            let r = rng.next();
            let len = if r < 0.02 {
                2
            } else if r < 0.10 {
                rng.between(6.0, 24.0) as u32
            } else if r < 0.80 {
                rng.between(24.0, 144.0) as u32
            } else {
                rng.between(144.0, 360.0) as u32
            };
            shots.push((Segment { start: at, end: at + len, kind: SegmentKind::Shot, tiny: len < 3 }, i % 6 != 5));
            at += len;
        }
        let frames = at;
        let index = FrameIndex::cfr(Rational::new(FPS.1, FPS.0), fps, 0, frames);
        let rec = IngestRecord {
            info: crate::analysis::tests::info(),
            frames,
            duration_sec: index.duration(),
            fps,
            vfr: false,
            interlaced: false,
            proxy: ProxyState::NotNeeded,
            poster: None,
            poster_small: None,
            index_blob: index.to_bytes(),
            audio_ordinal: None,
        };
        project.db().record_ingest(media, &rec).unwrap();
        project.db().store_segments(media, &shots.iter().map(|(s, _)| *s).collect::<Vec<_>>()).unwrap();
        // Found by the embedder of now (`as_models::ids::ARCFACE`): the app
        // doesn't look for them again.
        project.db().use_faces_model(media, "arcface-glintr100").unwrap();
        let (mut tracks, mut who) = (Vec::new(), Vec::new());
        for &(s, theirs) in &shots {
            let len = s.end - s.start;
            if theirs {
                // Their clearest face, on screen for most (often all) of the shot.
                let quality = if rng.next() < 0.7 { 1.0 } else { rng.between(0.3, 1.0) };
                let r = rng.next();
                let face_h = if r < 0.3 {
                    rng.between(0.45, 0.95)
                } else if r < 0.6 {
                    rng.between(0.25, 0.45)
                } else if r < 0.85 {
                    rng.between(0.15, 0.25)
                } else {
                    rng.between(0.03, 0.15)
                };
                let late = if rng.next() < 0.7 { 0.0 } else { rng.between(0.0, 0.4) };
                tracks.push(face(s.start, s.start + (len as f32 * late) as u32, s.end, quality, face_h));
                who.push(0);
                // Now and then a second, smaller view of them.
                if len > 24 && rng.next() < 0.3 {
                    tracks.push(face(s.start, s.start, s.start + len / 2, quality * 0.8, face_h * 0.7));
                    who.push(0);
                }
            }
            for (person, share) in [(1, if theirs { 0.4 } else { 0.8 }), (2, 0.1)] {
                if rng.next() < share {
                    tracks.push(face(s.start, s.start, s.end, rng.between(0.5, 1.0), rng.between(0.1, 0.4)));
                    who.push(person);
                }
            }
        }
        let sprites: Vec<_> = shots.iter().map(|(s, _)| (s.start, s.end, 8)).collect();
        let stored = project.db().put_face_chunk(media, 0, frames, &tracks, &sprites, |_| Ok(())).unwrap();
        project.db().set_faces_state(media, StageState::Ready, None).unwrap();
        for (id, person) in stored.into_iter().zip(who) {
            people[person].push(id);
        }
    }
    let mut cast = people.iter().zip(["Walter", "Jesse", "Skyler"]).map(|(tracks, name)| {
        let c = CharacterId::new();
        project.db().pin_tracks(tracks, c, Some(Some(name))).unwrap();
        c
    });
    let walter = cast.next().unwrap();
    cast.for_each(drop);
    project.db().set_meta(crate::cast::GROUPED_WITH, &as_identity::VERSION.to_string()).unwrap();
    (project, walter)
}

/// The times of `runs` runs of `f`, in milliseconds.
fn times<T>(runs: usize, mut f: impl FnMut() -> T) -> Vec<f64> {
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect()
}

/// The median time of `runs` runs of `f`, in milliseconds.
fn ms<T>(runs: usize, f: impl FnMut() -> T) -> f64 {
    let mut t = times(runs, f);
    t.sort_by(f64::total_cmp);
    t[runs / 2]
}

/// How big an answer is on its way to the app.
fn json<T: serde::Serialize>(x: &T) -> usize {
    serde_json::to_vec(x).unwrap().len()
}

fn kb(bytes: usize) -> f64 {
    bytes as f64 / 1024.0
}

fn row(what: &str, ms: f64) {
    println!("{what:<52}{ms:>9.2} ms");
}

/// What the builder costs `c`'s pack in `project`, printed.
fn measure(project: &Project, c: CharacterId) {
    let presets = as_export::builtin_presets();
    let caps = Caps::default();
    let naming = PackNaming::default();
    let spec = PackSpec::default();
    let best = PackSpec { order: PackOrder::Best, limit: Some(200), ..PackSpec::default() };
    let v = project.pack_preview(c, &spec).unwrap();
    let media: Vec<MediaId> = project.media_list().unwrap().iter().map(|m| m.id).collect();
    let episodes: std::collections::HashSet<_> = v.clips.iter().map(|c| c.shot.media_id).collect();
    println!("pack: {} shots, {} taken, {} episodes", v.clips.len(), v.totals.clips, episodes.len());
    let some = media[media.len() / 2];
    row("media(id)", ms(201, || project.media(some).unwrap()));
    row("ready_source(id)", ms(201, || project.ready_source(some).unwrap()));
    row("pack_state", ms(21, || project.pack_state(c).unwrap()));
    row("pack_preview (episodes)", ms(11, || project.pack_preview(c, &spec).unwrap()));
    row("  pack_rows", ms(11, || project.db().pack_rows(c).unwrap()));
    row("  media_list (counted)", ms(11, || project.media_list().unwrap()));
    row("  media_list_uncounted", ms(11, || project.media_list_uncounted().unwrap()));
    let rows = project.db().pack_rows(c).unwrap();
    row("  shots_of", ms(11, || shots_of(c, &rows, &[])));
    row("pack_preview (best, limit 200)", ms(11, || project.pack_preview(c, &best).unwrap()));
    row("pack_preview + its JSON", ms(11, || json(&project.pack_preview(c, &spec).unwrap())));
    row("pack_choices + its JSON", ms(11, || json(&project.pack_choices(c, &spec).unwrap())));
    row("pack_plan", ms(11, || project.pack_plan(c, &spec, &naming, &presets, &caps).unwrap()));
    let key = |i: usize| PackKey { media_id: v.clips[i].shot.media_id, shot: v.clips[i].shot.start };
    row(
        "set_pack_overrides (1 shot)",
        ms(11, || {
            project.set_pack_overrides(c, &[key(10)], Some(false)).unwrap();
            project.set_pack_overrides(c, &[key(10)], None).unwrap();
        }) / 2.0,
    );
    let n = v.clips.len();
    let range: Vec<PackKey> = (n / 60..n / 60 + n / 12).map(key).collect();
    row(
        &format!("set_pack_overrides ({} shots)", range.len()),
        ms(5, || {
            project.set_pack_overrides(c, &range, Some(false)).unwrap();
            project.set_pack_overrides(c, &range, None).unwrap();
        }) / 2.0,
    );
    let sizes = (
        json(&project.pack_preview(c, &spec).unwrap()),
        json(&project.pack_choices(c, &spec).unwrap()),
        json(&project.pack_plan(c, &spec, &naming, &presets, &caps).unwrap()),
    );
    println!("JSON: preview {:.0} kB, choices {:.0} kB, plan {:.1} kB", kb(sizes.0), kb(sizes.1), kb(sizes.2));
    // A click as the builder makes it: the pick answered with the pack's
    // choices, then its plan (names, sizes) once no other click follows.
    row(
        "a click: pick + choices + plan",
        ms(5, || {
            project.set_pack_overrides(c, &[key(10)], Some(false)).unwrap();
            json(&project.pack_choices(c, &spec).unwrap());
            json(&project.pack_plan(c, &spec, &naming, &presets, &caps).unwrap());
            project.set_pack_overrides(c, &[key(10)], None).unwrap();
        }),
    );
    println!("  bytes per click: {:.0} kB", kb(sizes.1 + sizes.2));
    // Many picks (every other shot), under a limit too.
    let half: Vec<PackKey> = (0..n).step_by(2).map(key).collect();
    project.set_pack_overrides(c, &half, Some(true)).unwrap();
    row(&format!("pack_preview with {} picks", half.len()), ms(11, || project.pack_preview(c, &spec).unwrap()));
    let limited = PackSpec { limit: Some(n as u32 / 4), ..best.clone() };
    row(&format!("  and a limit of {}", n / 4), ms(11, || project.pack_preview(c, &limited).unwrap()));
    project.set_pack_overrides(c, &half, None).unwrap();

    // Exports: one file, then a file per clip; each run after earlier ones.
    let out = project.dir.join("bench-exports");
    let preset = presets.iter().find(|p| p.id == "h264-edit").unwrap().clone();
    for merge in [true, false] {
        let req = PackExportRequest {
            spec: spec.clone(),
            naming: PackNaming { merge, ..PackNaming::default() },
            preset_id: preset.id.clone(),
            out_dir: Some(out.display().to_string()),
            stamp: v.stamp.clone(),
        };
        let what = if merge { "one file" } else { "a file per clip" };
        row(&format!("pack_export_request ({what})"), ms(3, || project.pack_export_request(c, &req, &out).unwrap()));
        let (request, dir) = project.pack_export_request(c, &req, &out).unwrap();
        let runs = times(3, || project.create_export(&request, preset.clone(), &dir).unwrap());
        let runs: Vec<String> = runs.iter().map(|t| format!("{t:.0}")).collect();
        println!("create_export ({} clips, {what}): {} ms, run after run", request.clips.len(), runs.join(" / "));
    }
    row("exports() with 6 exports in the list", ms(5, || project.exports().unwrap()));

    // Elsewhere, what reads every media but none of their counts.
    row("stats (the library's card)", ms(21, || project.db().stats().unwrap()));
    row("recover (each start)", ms(21, || project.recover().unwrap()));
    row("missing_portraits", ms(21, || project.missing_portraits().unwrap()));
    row("  cover_tracks", ms(21, || project.db().cover_tracks().unwrap()));
    // The cast as the app lists it: each character with its cover and counts.
    row("cast()", ms(21, || project.cast().unwrap()));
    row("  characters", ms(21, || project.db().characters().unwrap()));
    row("  cast_counts", ms(21, || project.db().cast_counts().unwrap()));
}

/// What `project.db` takes: all of it, and the tracks with each of their indexes.
fn db_sizes(project: &Project) {
    let db = rusqlite::Connection::open(project.dir.join("project.db")).unwrap();
    let mb = |bytes: i64| bytes as f64 / (1024.0 * 1024.0);
    let total: i64 = db
        .query_row("SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()", [], |r| r.get(0))
        .unwrap();
    let mut st = db
        .prepare(
            "SELECT name, SUM(pgsize) FROM dbstat
              WHERE name IN (SELECT name FROM sqlite_schema WHERE tbl_name = 'tracks') GROUP BY name ORDER BY name",
        )
        .unwrap();
    let parts: Vec<String> = st
        .query_map([], |r| Ok(format!("{} {:.2}", r.get::<_, String>(0)?, mb(r.get(1)?))))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    println!("project.db {:.2} MB; {} (MB)", mb(total), parts.join(", "));
}

#[test]
#[ignore = "a benchmark: run it in release"]
fn a_pack_of_6000_shots() {
    let temp = tempfile::tempdir().unwrap();
    let kept = std::env::var_os("AUTOSCENE_PACK_BENCH_DIR").map(PathBuf::from);
    let dir = kept.clone().unwrap_or_else(|| temp.path().to_path_buf());
    let t = Instant::now();
    let (project, walter) = series(&dir);
    println!("made {EPISODES} episodes in {:.1} s ({})", t.elapsed().as_secs_f64(), dir.display());
    db_sizes(&project);
    // Measuring exports some: a library to open stays as it was made.
    if kept.is_none() {
        measure(&project, walter);
    }
}

#[test]
#[ignore = "a benchmark: needs AUTOSCENE_PACK_BENCH_PROJECT"]
fn a_real_pack() {
    let Some(dir) = std::env::var_os("AUTOSCENE_PACK_BENCH_PROJECT").map(PathBuf::from) else { return };
    let top: String = rusqlite::Connection::open(dir.join("project.db"))
        .unwrap()
        .query_row(
            "SELECT character_id FROM tracks WHERE character_id IS NOT NULL AND assign IN ('auto', 'pinned')
              GROUP BY character_id ORDER BY COUNT(DISTINCT media_id || shot) DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let project = Project::open(ProjectId::new(), dir, Arc::new(AtomicBool::new(false))).unwrap();
    db_sizes(&project);
    measure(&project, top.parse().unwrap());
}
