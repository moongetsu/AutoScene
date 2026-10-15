//! Import → ingest → preview mapping → export, on barcoded fixtures.
//! Needs `cargo xtask fixtures`; skipped (with a note) otherwise.

use std::path::Path;
use std::process::Stdio;

use as_analysis::FaceDet;
use as_analysis::tracks::{self, FaceTrack};
use as_api::{
    AppSettings, CastEdit, ClipRequest, ContentKind, ExportRequest, ExportStatus, FrameMap, MediaStatus, NewProject,
    ProjectKind, ProxyState, Relink, RelinkSummary, TrackFilter,
};
use as_core::{AppPaths, CancelToken};
use as_export::builtin_presets;
use as_export::preset::{EncoderPref, Gop, VideoCodec};
use as_media::Tools;
use as_project::{Library, Portrait, Project};
use as_testkit::{barcode, fixtures_dir, load_truth};

const FILES: [&str; 3] = ["cfr24_h264.mp4", "transport_h264.ts", "vfr_h264.mkv"];

/// Every decoded frame of `file` as `(presentation time, barcode frame number)`.
fn frames_with_times(tools: &Tools, file: &Path) -> Vec<(f64, u32)> {
    let probe = tools
        .ffprobe()
        .args(["-select_streams", "v:0", "-show_entries", "frame=pts_time", "-of", "csv=p=0"])
        .arg(file)
        .output()
        .unwrap();
    let times: Vec<f64> = String::from_utf8_lossy(&probe.stdout)
        .lines()
        .filter_map(|l| l.trim().trim_end_matches(',').parse().ok())
        .collect();
    let decoded = tools
        .ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-vf", "scale=640:360,format=gray", "-f", "rawvideo", "-"])
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    let codes: Vec<u32> = decoded
        .stdout
        .as_chunks::<{ 640 * 360 }>()
        .0
        .iter()
        .map(|f| barcode::read_luma(f, 640, 360, 640).expect("barcode").frame as u32)
        .collect();
    assert_eq!(times.len(), codes.len(), "{}: ffprobe and decoder disagree on frame count", file.display());
    times.into_iter().zip(codes).collect()
}

/// What a `<video>` element shows when seeked to frame `f` the way the UI does
/// (middle of the frame's display interval): the last frame starting at or before that time.
fn shown_at(frames: &[(f64, u32)], t: f64) -> u32 {
    let i = frames.partition_point(|(pt, _)| *pt <= t);
    frames[i.saturating_sub(1)].1
}

fn player_time(map: &FrameMap, f: u32) -> f64 {
    let rel = |f: u32| match (&map.times, map.step) {
        (Some(t), _) => t[f as usize],
        (None, Some(step)) => f as f64 * step,
        _ => unreachable!("frame map without times or step"),
    };
    map.offset + 0.5 * (rel(f) + rel(f + 1))
}

#[test]
fn import_preview_and_export_three_episodes() {
    let Some(truth) = load_truth() else { return };
    let tools = Tools::locate(None).expect("ffmpeg on PATH");
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(&AppPaths::at(tmp.path().join("data"))).unwrap();
    lib.update_settings(&AppSettings {
        library_root: tmp.path().join("Library").display().to_string(),
        export_preset_id: "prores-hq".into(),
        audio_language: "eng".into(),
    })
    .unwrap();

    // Import: three files, one of each playback situation.
    let files: Vec<String> = FILES.iter().map(|f| fixtures_dir().join(f).display().to_string()).collect();
    let (_, project, added) = lib
        .create_project(&NewProject {
            title: "Fixture Show".into(),
            kind: ProjectKind::Series,
            content: ContentKind::Live,
            files: files.clone(),
        })
        .unwrap();
    assert_eq!(added.len(), 3);
    assert!(project.media_list().unwrap().iter().all(|m| m.status == MediaStatus::Queued));

    // Ingest all three at once, like the app's parallel media workers:
    // probe, frame index, poster, proxy decision.
    let cancel = CancelToken::new();
    let (p, t, c) = (&project, &tools, &cancel);
    let ingested: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> =
            added.iter().map(|&id| s.spawn(move || p.ingest(t, id, "eng", c, &|_, _| {}).unwrap())).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut pending = Vec::new();
    for m in ingested {
        let id = m.id;
        assert_eq!(m.status, MediaStatus::Ready, "{}", m.file_name);
        assert_eq!(m.frames, Some(truth.frame_count), "{}", m.file_name);
        assert!(m.poster.as_deref().is_some_and(|p| Path::new(p).is_file()), "{}: poster", m.file_name);
        let want = if m.file_name == "cfr24_h264.mp4" { ProxyState::NotNeeded } else { ProxyState::Pending };
        assert_eq!(m.proxy, want, "{}", m.file_name);
        assert_eq!(m.vfr, m.file_name == "vfr_h264.mkv");
        if m.proxy == ProxyState::Pending {
            assert!(project.frame_map(id).is_err(), "no preview before the proxy exists");
            pending.push(id);
        }
    }
    std::thread::scope(|s| {
        for &id in &pending {
            s.spawn(move || p.build_proxy(t, id, c, &|_, _| {}).unwrap());
        }
    });
    let summary = lib.refresh_stats(&project).unwrap();
    assert_eq!((summary.media_count, summary.ready_count), (3, 3));
    assert!(summary.poster.is_some() && summary.poster_small.is_some());

    // Posters: 720p artwork and a small one, named after their sizes; an
    // ingest leaves nothing to make again.
    for m in project.media_list().unwrap() {
        for (p, name) in [(&m.poster, format!("{}-720.jpg", m.id)), (&m.poster_small, format!("{}-480x270.jpg", m.id))]
        {
            let p = Path::new(p.as_deref().expect("poster"));
            assert!(p.is_file() && p.file_name().unwrap() == name.as_str(), "{}: {name}", m.file_name);
        }
    }
    assert!(project.recover().unwrap().posters.is_empty());

    // A project from before the full-bleed artwork: a 360p poster under the
    // old name and no small one. The next launch makes both again; the old
    // file goes, and the Library card follows without moving to the front.
    let first = project.media_list().unwrap().remove(0);
    let old = project.thumbs_dir().join(format!("{}.jpg", first.id));
    std::fs::rename(first.poster.as_deref().unwrap(), &old).unwrap();
    std::fs::remove_file(first.poster_small.as_deref().unwrap()).unwrap();
    rusqlite::Connection::open(project.dir.join("project.db"))
        .unwrap()
        .execute(
            "UPDATE media SET poster = ?1, poster_small = NULL WHERE id = ?2",
            (old.display().to_string(), first.id.to_string()),
        )
        .unwrap();
    let before = lib.refresh_stats(&project).unwrap();
    assert_eq!((before.poster, before.poster_small), (Some(old.display().to_string()), None));
    assert_eq!(project.recover().unwrap().posters, vec![first.id]);

    assert!(project.refresh_posters(&tools, first.id, &cancel).unwrap());
    let m = project.media(first.id).unwrap();
    assert_eq!((&m.poster, &m.poster_small), (&first.poster, &first.poster_small));
    assert!([&m.poster, &m.poster_small].iter().all(|p| p.as_deref().is_some_and(|p| Path::new(p).is_file())));
    assert!(!old.exists(), "the old poster is removed");
    assert!(project.recover().unwrap().posters.is_empty());
    assert!(!project.refresh_posters(&tools, first.id, &cancel).unwrap(), "nothing left to do");

    assert!(lib.refresh_posters(&project).unwrap());
    let after = lib.summary(project.id).unwrap();
    assert_eq!((&after.poster, &after.poster_small), (&m.poster, &m.poster_small));
    assert_eq!(after.updated_at, before.updated_at);

    // Preview: seeking the player to frame f shows exactly frame f, for direct
    // playback, a proxy of an MPEG-TS with a start offset, and a VFR proxy.
    for m in project.media_list().unwrap() {
        let map = project.frame_map(m.id).unwrap();
        assert_eq!(map.frames, truth.frame_count);
        assert_eq!(map.is_proxy, m.file_name != "cfr24_h264.mp4");
        assert_eq!(map.times.is_some(), m.vfr);
        let shown = frames_with_times(&tools, Path::new(&map.src));
        assert_eq!(shown.len() as u32, truth.frame_count, "{}", m.file_name);
        for f in 0..map.frames {
            assert_eq!(shown_at(&shown, player_time(&map, f)), f, "{}: seek to frame {f}", m.file_name);
        }
    }

    // Export: mark in/out on each episode and render ProRes and software H.264.
    let media = project.media_list().unwrap();
    let ranges = [(100u32, 148u32), (199, 201), (0, 24)];
    let clips: Vec<ClipRequest> = media
        .iter()
        .zip(ranges)
        .map(|(m, (s, e))| ClipRequest { media_id: m.id, start: s, end: e, name: None })
        .collect();
    let mut h264 = builtin_presets().into_iter().find(|p| p.id == "h264-edit").unwrap();
    h264.video = VideoCodec::H264 { quality: 18, gop: Gop::Editing, encoder: EncoderPref::Software };
    let prores = builtin_presets().into_iter().find(|p| p.id == "prores-hq").unwrap();
    let out_dir = lib.default_export_dir(&summary).unwrap();
    for preset in [prores, h264] {
        let req = ExportRequest { preset_id: preset.id.clone(), out_dir: None, clips: clips.clone(), label: None };
        let ex = project.create_export(&req, preset, &out_dir).unwrap();
        assert_eq!(ex.status, ExportStatus::Queued);
        let updates = std::cell::Cell::new(0);
        let ex = project.run_export(&tools, ex.id, &cancel, &|_, _| {}, &|| updates.set(updates.get() + 1)).unwrap();
        assert_eq!(ex.status, ExportStatus::Done);
        assert_eq!(updates.get(), 2 * ranges.len(), "every clip reports its start and its end");
        for (item, (s, e)) in ex.items.iter().zip(ranges) {
            assert!(item.verified, "{}: not verified", item.name);
            let path = item.path.as_deref().unwrap();
            let got: Vec<u32> = frames_with_times(&tools, Path::new(path)).into_iter().map(|(_, c)| c).collect();
            assert_eq!(got, (s..e).collect::<Vec<_>>(), "{}", item.name);
        }
    }

    // Names are readable; files never overwrite each other.
    let exports = project.exports().unwrap();
    assert_eq!(exports.len(), 2);
    let items: Vec<_> = exports.iter().flat_map(|e| e.items.iter()).collect();
    assert!(items.iter().any(|i| i.name == "Fixture_Show_cfr24_h264_00-00-04-04"), "{items:?}");
    let paths: std::collections::HashSet<_> = items.iter().filter_map(|i| i.path.clone()).collect();
    assert_eq!(paths.len(), items.len());

    // Exporting the same range again with the same preset picks a new name.
    let prores = builtin_presets().into_iter().find(|p| p.id == "prores-hq").unwrap();
    let req = ExportRequest { preset_id: prores.id.clone(), out_dir: None, clips: clips[..1].to_vec(), label: None };
    let again = project.create_export(&req, prores.clone(), &out_dir).unwrap();
    assert_eq!(again.items[0].name, "Fixture_Show_cfr24_h264_00-00-04-04_2");
    // …and a second one queued before the first has written anything gets its own name too.
    let queued = project.create_export(&req, prores, &out_dir).unwrap();
    assert_eq!(queued.items[0].name, "Fixture_Show_cfr24_h264_00-00-04-04_3");
}

#[test]
fn bad_ranges_and_missing_files_are_refused() {
    let Some(_truth) = load_truth() else { return };
    let tools = Tools::locate(None).expect("ffmpeg on PATH");
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(&AppPaths::at(tmp.path().join("data"))).unwrap();
    lib.update_settings(&AppSettings {
        library_root: tmp.path().join("Library").display().to_string(),
        export_preset_id: "prores-hq".into(),
        audio_language: "eng".into(),
    })
    .unwrap();
    // A copy we can delete to simulate a disconnected drive.
    let copy = tmp.path().join("Show - 01.mp4");
    std::fs::copy(fixtures_dir().join("cfr24_h264.mp4"), &copy).unwrap();
    let (_, project, added) = lib
        .create_project(&NewProject {
            title: "Edge".into(),
            kind: ProjectKind::Series,
            content: ContentKind::Live,
            files: vec![copy.display().to_string()],
        })
        .unwrap();
    let id = added[0];
    let m = project.ingest(&tools, id, "eng", &CancelToken::new(), &|_, _| {}).unwrap();
    assert_eq!(m.label, "E01");
    let preset = builtin_presets().remove(0);
    let req = |s, e| ExportRequest {
        preset_id: preset.id.clone(),
        out_dir: None,
        clips: vec![ClipRequest { media_id: id, start: s, end: e, name: Some("x".into()) }],
        label: None,
    };
    let out = tmp.path().join("out");
    assert!(project.create_export(&req(10, 10), preset.clone(), &out).is_err(), "empty range");
    assert!(project.create_export(&req(0, m.frames.unwrap() + 1), preset.clone(), &out).is_err(), "past the end");

    std::fs::remove_file(&copy).unwrap();
    assert_eq!(project.media(id).unwrap().status, MediaStatus::Missing);
    assert!(project.create_export(&req(0, 10), preset, &out).is_err(), "missing source");
    let err = project.ingest(&tools, id, "eng", &CancelToken::new(), &|_, _| {}).unwrap_err();
    assert!(err.to_string().contains("can't be found"), "{err}");
}

/// `(width, height)` of a picture.
fn dimensions(tools: &Tools, file: &Path) -> (u32, u32) {
    let out = tools
        .ffprobe()
        .args(["-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(file)
        .output()
        .expect("ffprobe");
    let text = String::from_utf8_lossy(&out.stdout);
    let (w, h) = text.trim().split_once(',').expect("width,height");
    (w.parse().unwrap(), h.parse().unwrap())
}

/// A character's cover gets a sharp portrait: 3:4 from the exact frame of
/// its clearest face, named after that frame and its size. A portrait of
/// another size goes; a source that is gone makes none.
#[test]
fn covers_get_portraits_from_the_exact_frame() {
    let Some(_truth) = load_truth() else { return };
    let tools = Tools::locate(None).expect("ffmpeg on PATH");
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(&AppPaths::at(tmp.path().join("data"))).unwrap();
    lib.update_settings(&AppSettings {
        library_root: tmp.path().join("Library").display().to_string(),
        export_preset_id: "prores-hq".into(),
        audio_language: "eng".into(),
    })
    .unwrap();
    // A copy we can delete to simulate a disconnected drive.
    let copy = tmp.path().join("Show - 01.mkv");
    std::fs::copy(fixtures_dir().join("vfr_h264.mkv"), &copy).unwrap();
    let (_, project, added) = lib
        .create_project(&NewProject {
            title: "Portraits".into(),
            kind: ProjectKind::Series,
            content: ContentKind::Live,
            files: vec![copy.display().to_string()],
        })
        .unwrap();
    let cancel = CancelToken::new();
    let m = project.ingest(&tools, added[0], "eng", &cancel, &|_, _| {}).unwrap();

    // One face, seen at 191, 200 and 209: clearest at 200. Another whose
    // detections lie past the end of the file (as when a file comes back
    // shorter and is indexed again).
    let det = |frame, quality| FaceDet { frame, bbox: [0.4, 0.3, 0.55, 0.6], score: 0.9, norm: 22.0, quality };
    let face = |who: usize, start: u32, dets: Vec<FaceDet>| {
        let mut embedding = vec![0.0; tracks::DIM];
        embedding[who] = 1.0;
        FaceTrack {
            start,
            end: start + 21,
            shot: 0,
            dets,
            embedding,
            quality: 0.8,
            thumb: vec![0xFF, 0xD8, 0xFF, 0xD9],
        }
    };
    let frames = m.frames.unwrap();
    let faces = [
        face(0, 191, vec![det(191, 0.5), det(200, 0.9), det(209, 0.6)]),
        face(1, frames + 10, vec![det(frames + 10, 0.9)]),
    ];
    project.store_face_chunk(m.id, 0, frames, &tracks::encode(&faces), &[]).unwrap();
    project.finish_faces(m.id).unwrap();
    let id_at = |start| {
        project.cast_tracks(TrackFilter::Unassigned).unwrap().into_iter().find(|t| t.start == start).unwrap().id
    };
    let (track, past_the_end) = (id_at(191), id_at(frames + 10));
    project.cast_edit(&CastEdit::NewCharacter { tracks: vec![track], name: Some("Walter".into()) }).unwrap();
    project.cast_edit(&CastEdit::NewCharacter { tracks: vec![past_the_end], name: Some("Jesse".into()) }).unwrap();
    let (track, past_the_end) = (i64::from(track), i64::from(past_the_end));
    assert_eq!(project.missing_portraits().unwrap(), [track, past_the_end]);

    // A portrait of an earlier size.
    let faces = project.faces_dir().join(m.id.to_string());
    let old = faces.join(format!("{track}-200-300x400.jpg"));
    std::fs::write(&old, b"old").unwrap();

    let Portrait::Made(portrait) = project.portrait(&tools, track, &cancel).unwrap() else { panic!("a portrait") };
    let name = portrait.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with(&format!("{track}-200-")) && name.ends_with("-450x600.jpg"), "{name}");
    assert_eq!(portrait.parent(), Some(faces.as_path()));
    assert_eq!(dimensions(&tools, &portrait), (450, 600));
    assert!(!old.exists(), "the earlier size goes");
    assert!(faces.join(format!("{track}.jpg")).is_file(), "the thumbnail stays");
    let walter = project.cast().unwrap().characters.into_iter().find(|c| c.name.as_deref() == Some("Walter")).unwrap();
    assert_eq!(walter.cover.as_deref(), Some(portrait.display().to_string().as_str()));
    assert_eq!(walter.cover_small, Some(faces.join(format!("{track}.jpg")).display().to_string()));
    assert_eq!(project.portrait(&tools, track, &cancel).unwrap(), Portrait::Ready(portrait.clone()), "made once");

    // A face past the end of the file gets none, and isn't owed one again.
    assert_eq!(project.portrait(&tools, past_the_end, &cancel).unwrap(), Portrait::Unavailable);
    assert!(project.missing_portraits().unwrap().is_empty());

    // The source is gone: no portrait, and none owed.
    std::fs::remove_file(&portrait).unwrap();
    std::fs::remove_file(&copy).unwrap();
    assert_eq!(project.media(m.id).unwrap().status, MediaStatus::Missing);
    assert_eq!(project.portrait(&tools, track, &cancel).unwrap(), Portrait::Unavailable);
    assert!(project.missing_portraits().unwrap().is_empty());
}

#[test]
fn a_moved_episode_keeps_its_analysis() {
    let Some(_truth) = load_truth() else { return };
    let tools = Tools::locate(None).expect("ffmpeg on PATH");
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(&AppPaths::at(tmp.path().join("data"))).unwrap();
    lib.update_settings(&AppSettings {
        library_root: tmp.path().join("Library").display().to_string(),
        export_preset_id: "prores-hq".into(),
        audio_language: "eng".into(),
    })
    .unwrap();
    let file = tmp.path().join("Show - 01.mp4");
    std::fs::copy(fixtures_dir().join("cfr24_h264.mp4"), &file).unwrap();
    let (_, project, added) = lib
        .create_project(&NewProject {
            title: "Moves".into(),
            kind: ProjectKind::Series,
            content: ContentKind::Live,
            files: vec![file.display().to_string()],
        })
        .unwrap();
    let id = added[0];
    let cancel = CancelToken::new();
    let m = project.ingest(&tools, id, "eng", &cancel, &|_, _| {}).unwrap();

    // A face and a decision about it.
    let det = |frame| FaceDet { frame, bbox: [0.4, 0.3, 0.55, 0.6], score: 0.9, norm: 22.0, quality: 0.8 };
    let mut embedding = vec![0.0; tracks::DIM];
    embedding[0] = 1.0;
    let face = FaceTrack {
        start: 10,
        end: 40,
        shot: 0,
        dets: vec![det(10), det(25)],
        embedding,
        quality: 0.8,
        thumb: vec![0xFF, 0xD8, 0xFF, 0xD9],
    };
    project.store_face_chunk(id, 0, m.frames.unwrap(), &tracks::encode(&[face]), &[]).unwrap();
    project.finish_faces(id).unwrap();
    let track = project.cast_tracks(TrackFilter::Unassigned).unwrap()[0].id;
    let walter = project
        .cast_edit(&CastEdit::NewCharacter { tracks: vec![track], name: Some("Walter".into()) })
        .unwrap()
        .unwrap();
    let walters = |p: &Project| p.cast_tracks(TrackFilter::Character(walter)).unwrap().len();
    assert_eq!(walters(&project), 1);

    // The folder was renamed: missing, then found again where it went.
    let moved = tmp.path().join("Moved");
    std::fs::create_dir(&moved).unwrap();
    let there = moved.join("Show - 01.mp4");
    std::fs::rename(&file, &there).unwrap();
    assert_eq!(project.media(id).unwrap().status, MediaStatus::Missing);
    let (summary, reindex) = project.relink_folder(&tools, &moved).unwrap();
    assert_eq!(summary, RelinkSummary { same: 1, reindex: 0, missing: 0 });
    assert!(reindex.is_empty());
    let m = project.media(id).unwrap();
    assert_eq!((m.status, m.path.as_str()), (MediaStatus::Ready, there.display().to_string().as_str()));
    assert_eq!(walters(&project), 1);

    // Another video is refused unless taken anyway, and changes nothing.
    let other = tmp.path().join("Other.mkv");
    std::fs::copy(fixtures_dir().join("vfr_h264.mkv"), &other).unwrap();
    let r = project.relink(&tools, id, &other, false).unwrap();
    assert!(matches!(r, Relink::Different { .. }), "{r:?}");
    assert_eq!(project.media(id).unwrap().path, there.display().to_string());

    // A remuxed copy (another size, the same frames) is indexed again and
    // keeps its analysis.
    let copy = tmp.path().join("Show - 01 copy.mp4");
    let remux = tools
        .ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(&there)
        .args(["-map", "0", "-c", "copy", "-metadata", "comment=A copy of the episode"])
        .arg(&copy)
        .status()
        .unwrap();
    assert!(remux.success());
    assert_ne!(std::fs::metadata(&copy).unwrap().len(), std::fs::metadata(&there).unwrap().len());
    assert_eq!(project.relink(&tools, id, &copy, false).unwrap(), Relink::Reindex);
    project.ingest(&tools, id, "eng", &cancel, &|_, _| {}).unwrap();
    assert_eq!(walters(&project), 1, "the same frames keep their analysis");
    assert_eq!(project.cast().unwrap().characters[0].name.as_deref(), Some("Walter"));
}
