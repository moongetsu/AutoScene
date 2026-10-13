//! Import → ingest → preview mapping → export, on barcoded fixtures.
//! Needs `cargo xtask fixtures`; skipped (with a note) otherwise.

use std::path::Path;
use std::process::Stdio;

use as_api::{
    AppSettings, ClipRequest, ContentKind, ExportRequest, ExportStatus, FrameMap, MediaStatus, NewProject, ProjectKind,
    ProxyState,
};
use as_core::{AppPaths, CancelToken};
use as_export::builtin_presets;
use as_export::preset::{EncoderPref, Gop, VideoCodec};
use as_media::Tools;
use as_project::Library;
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
    let Some(truth) = load_truth() else {
        eprintln!("fixtures missing — run `cargo xtask fixtures`; skipping");
        return;
    };
    let tools = Tools::locate(None).expect("ffmpeg on PATH");
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::open(&AppPaths::at(tmp.path().join("data"))).unwrap();
    lib.update_settings(&AppSettings {
        library_root: tmp.path().join("Library").display().to_string(),
        export_preset_id: "prores-hq".into(),
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
            added.iter().map(|&id| s.spawn(move || p.ingest(t, id, c, &|_, _| {}).unwrap())).collect();
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
    assert!(summary.poster.is_some());

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
        let req = ExportRequest { preset_id: preset.id.clone(), out_dir: None, clips: clips.clone() };
        let ex = project.create_export(&req, preset, &out_dir).unwrap();
        assert_eq!(ex.status, ExportStatus::Queued);
        let ex = project.run_export(&tools, ex.id, &cancel, &|_, _| {}).unwrap();
        assert_eq!(ex.status, ExportStatus::Done);
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
    let req = ExportRequest { preset_id: prores.id.clone(), out_dir: None, clips: clips[..1].to_vec() };
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
    let m = project.ingest(&tools, id, &CancelToken::new(), &|_, _| {}).unwrap();
    assert_eq!(m.label, "E01");
    let preset = builtin_presets().remove(0);
    let req = |s, e| ExportRequest {
        preset_id: preset.id.clone(),
        out_dir: None,
        clips: vec![ClipRequest { media_id: id, start: s, end: e, name: Some("x".into()) }],
    };
    let out = tmp.path().join("out");
    assert!(project.create_export(&req(10, 10), preset.clone(), &out).is_err(), "empty range");
    assert!(project.create_export(&req(0, m.frames.unwrap() + 1), preset.clone(), &out).is_err(), "past the end");

    std::fs::remove_file(&copy).unwrap();
    assert_eq!(project.media(id).unwrap().status, MediaStatus::Missing);
    assert!(project.create_export(&req(0, 10), preset, &out).is_err(), "missing source");
    let err = project.ingest(&tools, id, &CancelToken::new(), &|_, _| {}).unwrap_err();
    assert!(err.to_string().contains("can't be found"), "{err}");
}
