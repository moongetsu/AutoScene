//! Frame-exact export, proven with barcodes. Needs `cargo xtask fixtures`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use as_core::{CancelToken, FrameIndex, FrameRange};
use as_export::preset::{EncoderPref, Gop, VideoCodec};
use as_export::{ClipJob, ExportPreset, builtin_presets, export_clip};
use as_media::{MediaInfo, Source, Tools, build_frame_index, idet, probe};
use as_testkit::{Truth, barcode, fixtures_dir, load_truth};

struct Fixture {
    file: &'static str,
    info: MediaInfo,
    index: FrameIndex,
}

fn setup() -> Option<(Tools, Truth)> {
    let truth = load_truth()?;
    Some((Tools::locate(None).expect("ffmpeg on PATH"), truth))
}

/// Probe and index like the app's ingest does (including interlace detection
/// for files whose container doesn't say).
fn load(tools: &Tools, file: &'static str, work: &Path) -> Fixture {
    let path = fixtures_dir().join(file);
    let mut info = probe(tools, &path).unwrap();
    if info.video.field_order.is_none()
        && let Some(order) = idet::detect_field_order(tools, &path, &info).unwrap()
    {
        info.video.field_order = Some(order.into());
    }
    let index = build_frame_index(tools, &path, &info, work, &CancelToken::new(), |_| {}).unwrap();
    Fixture { file, info, index }
}

fn preset(id: &str) -> ExportPreset {
    builtin_presets().into_iter().find(|p| p.id == id).unwrap()
}

fn software_h264() -> ExportPreset {
    let mut p = preset("h264-edit");
    p.video = VideoCodec::H264 { quality: 16, gop: Gop::Editing, encoder: EncoderPref::Software };
    p
}

fn barcodes(tools: &Tools, file: &Path) -> Vec<u32> {
    let out = tools
        .ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-vf", "scale=640:360,format=gray", "-f", "rawvideo", "-"])
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    out.stdout
        .as_chunks::<{ 640 * 360 }>()
        .0
        .iter()
        .enumerate()
        .map(|(i, f)| {
            barcode::read_luma(f, 640, 360, 640)
                .unwrap_or_else(|| panic!("{}: no barcode in output frame {i}", file.display()))
                .frame as u32
        })
        .collect()
}

fn audio_pcm(tools: &Tools, file: &Path) -> Vec<i16> {
    let out = tools
        .ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "s16le", "-"])
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

/// Dominant frequency of a pure tone from its zero-crossing rate.
fn tone_hz(samples: &[i16]) -> f64 {
    let crossings = samples.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    crossings as f64 / 2.0 / (samples.len() as f64 / 48_000.0)
}

/// Edge cases plus deterministic pseudo-random spans.
fn ranges(truth: &Truth) -> Vec<FrameRange> {
    let n = truth.frame_count;
    let mut r = vec![
        FrameRange::new(0, 12),
        FrameRange::new(1, 2),
        FrameRange::new(140, 160),  // across the dissolve
        FrameRange::new(268, 269),  // the 1-frame shot
        FrameRange::new(330, 350),  // across the wipe
        FrameRange::new(n - 11, n), // media end
        FrameRange::new(200, 201),
    ];
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..5 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let s = (x >> 33) as u32 % (n - 40);
        let len = 1 + ((x >> 13) as u32 % 39);
        r.push(FrameRange::new(s, s + len));
    }
    r
}

/// Export `range`, prove it with barcodes, return the clip's path.
fn check(tools: &Tools, fx: &Fixture, range: FrameRange, preset: &ExportPreset, work: &Path) -> PathBuf {
    let src = fixtures_dir().join(fx.file);
    let job = ClipJob {
        src: Source { path: &src, info: &fx.info, index: &fx.index },
        range,
        audio_ordinal: None,
        preset,
        out_stem: work.join(format!("{}_{}_{}_{}", fx.file.replace('.', "_"), preset.id, range.start, range.end)),
        title: None,
    };
    let out = export_clip(tools, &job, &CancelToken::new(), |_| {}).unwrap();
    assert!(out.verified, "{} {range:?} {}: self-verification failed", fx.file, preset.id);
    let codes = barcodes(tools, Path::new(&out.path));
    let want: Vec<u32> = (range.start..range.end).collect();
    assert_eq!(codes, want, "{} {range:?} {}: decoded frames", fx.file, preset.id);
    PathBuf::from(out.path)
}

#[test]
fn software_exports_are_frame_exact_on_every_variant() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let presets = [preset("prores-hq"), software_h264()];
    for file in [
        "cfr24_h264.mp4",
        "cfr23976_h264.mp4",
        "offset725_h264.mp4",
        "transport_h264.ts",
        "hevc10_hdr.mkv",
        "vfr_h264.mkv",
        "two_audio_h264.mkv",
        "late_audio_h264.mkv",
        "interlaced_h264.ts",
        "interlaced_xvid.avi",
        "prores.mov",
    ] {
        let fx = load(&tools, file, work.path());
        for range in ranges(&truth) {
            for p in &presets {
                check(&tools, &fx, range, p, work.path());
            }
        }
    }
}

/// Presentation time of every video frame (edit lists applied).
fn frame_times(tools: &Tools, file: &Path) -> Vec<f64> {
    let out = tools
        .ffprobe()
        .args(["-select_streams", "v:0", "-show_entries", "frame=pts_time", "-of", "csv=p=0"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().trim_end_matches(',').parse().ok()).collect()
}

#[test]
fn vfr_exports_keep_every_frames_timing() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let fx = load(&tools, "vfr_h264.mkv", work.path());
    // Spans crossing the 24 → 60 → 30 fps changes.
    for (s, e) in [(180u32, 260u32), (300, 420)] {
        let range = FrameRange::new(s, e);
        for p in [preset("prores-hq"), software_h264()] {
            check(&tools, &fx, range, &p, work.path());
            let out = work.path().join(format!("vfr_h264_mkv_{}_{s}_{e}.{}", p.id, p.container.ext()));
            let got = frame_times(&tools, &out);
            assert_eq!(got.len() as u32, range.len());
            for (i, t) in got.iter().enumerate() {
                let want = fx.index.t(s + i as u32) - fx.index.t(s);
                assert!(
                    (t - got[0] - want).abs() < 1.5e-3,
                    "{} frame {}: at {t:.4}, want {want:.4}",
                    p.id,
                    s + i as u32
                );
            }
        }
    }
}

#[test]
fn hardware_and_dnxhr_exports_are_frame_exact() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let mut presets = vec![preset("dnxhr-hq"), preset("hevc10")];
    if tools.caps().hw_encoder(as_media::CodecFamily::H264).is_some() {
        presets.push(preset("h264-edit"));
        presets.push(preset("h264-intra"));
    } else {
        eprintln!("no hardware H.264 encoder on this machine; testing software paths only");
    }
    for file in ["offset725_h264.mp4", "hevc10_hdr.mkv"] {
        let fx = load(&tools, file, work.path());
        for range in ranges(&truth).into_iter().take(6) {
            for p in &presets {
                check(&tools, &fx, range, p, work.path());
            }
        }
    }
}

#[test]
fn audio_is_cut_with_the_video() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let p = preset("prores-hq");
    for file in ["cfr24_h264.mp4", "offset725_h264.mp4", "transport_h264.ts"] {
        let fx = load(&tools, file, work.path());
        // Clips that start on a hard cut and run into the next shot.
        for shot in truth.shots().filter(|s| s.end - s.start >= 12 && s.tone_hz.is_some()) {
            let range = FrameRange::new(shot.start, (shot.start + 30).min(truth.frame_count));
            let src = fixtures_dir().join(file);
            let job = ClipJob {
                src: Source { path: &src, info: &fx.info, index: &fx.index },
                range,
                audio_ordinal: None,
                preset: &p,
                out_stem: work.path().join(format!("a_{}_{}", file.replace('.', "_"), shot.start)),
                title: None,
            };
            let out = export_clip(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
            let pcm = audio_pcm(&tools, Path::new(&out.path));
            let expect = ((fx.index.t(range.end) - fx.index.t(range.start)) * 48_000.0).round() as usize;
            assert_eq!(pcm.len(), expect, "{file} {range:?}: audio length");
            // AAC sources carry encoder ramps; skip the first 3 ms, measure 20 ms.
            let hz = tone_hz(&pcm[144..144 + 960]);
            let want = shot.tone_hz.unwrap() as f64;
            assert!((hz - want).abs() / want < 0.12, "{file} shot '{}': tone {hz:.0} Hz, want {want}", shot.label);
        }
    }
}

/// Names come from titles and file names; dots in them ("Mr. Robot",
/// "Blade.Runner.2049") must survive, and two clips must never share a file.
#[test]
fn dotted_names_get_their_own_files() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let fx = load(&tools, "cfr24_h264.mp4", work.path());
    let src = fixtures_dir().join(fx.file);
    let p = preset("prores-hq");
    let mut paths = Vec::new();
    for (stem, s) in [("Mr. Robot S01E01 00-00-00-00", 0u32), ("Mr. Robot S01E01 00-00-01-00", 24), ("shot 1.5", 48)] {
        let job = ClipJob {
            src: Source { path: &src, info: &fx.info, index: &fx.index },
            range: FrameRange::new(s, s + 6),
            audio_ordinal: None,
            preset: &p,
            out_stem: work.path().join(stem),
            title: None,
        };
        let out = export_clip(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
        assert!(out.verified);
        assert_eq!(Path::new(&out.path).file_name().unwrap().to_string_lossy(), format!("{stem}.mov"));
        assert_eq!(barcodes(&tools, Path::new(&out.path)), (s..s + 6).collect::<Vec<_>>(), "{stem}");
        paths.push(out.path);
    }
    paths.dedup();
    assert_eq!(paths.len(), 3, "every clip has its own file");
    let leftovers: Vec<_> = std::fs::read_dir(work.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".part.") || n.ends_with(".stats.txt"))
        .collect();
    assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
}

/// Audio whose stream starts 0.5 s after the video: silence before it, the
/// right tone after it, and the A/V length still exact.
#[test]
fn late_audio_stays_in_sync() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let file = "late_audio_h264.mkv";
    let variant = truth.variant(file).expect("late_audio variant; run `cargo xtask fixtures`");
    assert!((variant.audio_start_sec - 0.5).abs() < 1e-9);
    let fx = load(&tools, file, work.path());
    let src = fixtures_dir().join(file);
    let first = truth.shots().next().unwrap();
    let tone = first.tone_hz.unwrap() as f64;
    for p in [preset("prores-hq"), software_h264()] {
        // [6, 30) = 0.25..1.25 s: a quarter second of nothing, then the first shot's tone.
        let range = FrameRange::new(6, 30);
        let job = ClipJob {
            src: Source { path: &src, info: &fx.info, index: &fx.index },
            range,
            audio_ordinal: None,
            preset: &p,
            out_stem: work.path().join(format!("late_{}", p.id)),
            title: None,
        };
        let out = export_clip(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
        assert!(out.verified);
        let pcm = audio_pcm(&tools, Path::new(&out.path));
        let expect = ((fx.index.t(range.end) - fx.index.t(range.start)) * 48_000.0).round() as usize;
        // AAC adds its own priming; only the PCM export must match to the sample.
        if p.id == "prores-hq" {
            assert_eq!(pcm.len(), expect, "{}: audio length", p.id);
        } else {
            assert!(pcm.len().abs_diff(expect) < 2048, "{}: audio length {} vs {expect}", p.id, pcm.len());
        }
        let peak = |a: &[i16]| a.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        // 0.00-0.22 s: silence (well before the 0.25 s onset, past AAC ramps).
        assert!(peak(&pcm[480..10_560]) < 300, "{}: sound before the audio stream starts", p.id);
        // 0.30-0.50 s: the first shot's tone, in place.
        let hz = tone_hz(&pcm[14_400..24_000]);
        assert!((hz - tone).abs() / tone < 0.05, "{}: tone {hz:.0} Hz, want {tone}", p.id);
        let onset = pcm.iter().position(|v| v.unsigned_abs() > 2000).unwrap() as f64 / 48_000.0;
        assert!((onset - 0.25).abs() < 0.012, "{}: tone starts at {onset:.4} s, want 0.25 s", p.id);
    }
}

/// Phone editors get stereo: a 5.1 source is mixed down, the length stays exact.
#[test]
fn mobile_preset_mixes_surround_to_stereo() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("surround.mkv");
    let status = tools
        .ffmpeg()
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=320x180:r=24:d=2", "-f", "lavfi"])
        .args([
            "-i",
            "sine=f=440:r=48000:d=2",
            "-filter_complex",
            "[1:a]pan=5.1|FL=c0|FR=c0|FC=c0|LFE=c0|BL=c0|BR=c0[a]",
        ])
        .args(["-map", "0:v", "-map", "[a]", "-c:v", "libx264", "-c:a", "flac"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success());
    let info = probe(&tools, &src).unwrap();
    assert_eq!(info.audio[0].channels, 6);
    let index = build_frame_index(&tools, &src, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    for (id, channels) in [("h264-mobile", 2), ("prores-hq", 6)] {
        let mut p = preset(id);
        if let VideoCodec::H264 { quality, gop, .. } = p.video {
            p.video = VideoCodec::H264 { quality, gop, encoder: EncoderPref::Software };
        }
        let job = ClipJob {
            src: Source { path: &src, info: &info, index: &index },
            range: FrameRange::new(0, 24),
            audio_ordinal: None,
            preset: &p,
            out_stem: work.path().join(format!("surround_{id}")),
            title: None,
        };
        let out = export_clip(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
        assert!(out.verified);
        let out_info = probe(&tools, Path::new(&out.path)).unwrap();
        assert_eq!(out_info.audio[0].channels, channels, "{id}");
    }
}

/// Interlaced sources come out as whole, combing-free frames — one per source
/// frame, flagged or not — and still on the exact frames asked for.
#[test]
fn interlaced_sources_are_deinterlaced() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    // idet's verdict over a whole file: (interlaced frames, progressive frames).
    let fields = |file: &Path| -> (u64, u64) {
        let out = tools
            .ffmpeg()
            .args(["-v", "info", "-nostats", "-i"])
            .arg(file)
            .args(["-map", "0:v:0", "-vf", "idet", "-f", "null", "-"])
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&out.stderr);
        let line = log.lines().rev().find(|l| l.contains("Multi frame detection")).expect("idet summary").to_string();
        let count = |key: &str| -> u64 {
            line.split(key).nth(1).and_then(|r| r.split_whitespace().next()).unwrap().parse().unwrap()
        };
        (count("TFF:") + count("BFF:"), count("Progressive:"))
    };
    for file in ["interlaced_h264.ts", "interlaced_xvid.avi"] {
        let fx = load(&tools, file, work.path());
        assert!(fx.info.is_interlaced(), "{file}: detected as interlaced");
        let (combed, _) = fields(&fixtures_dir().join(file));
        assert!(combed > 400, "{file}: the source itself is interlaced ({combed} combed frames)");
        let range = FrameRange::new(100, 340);
        let out = check(&tools, &fx, range, &software_h264(), work.path());
        let (combed, progressive) = fields(&out);
        assert!(combed * 20 < progressive, "{file}: export still combed ({combed} vs {progressive} progressive)");
    }
}
