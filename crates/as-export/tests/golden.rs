//! Frame-exact export, proven with barcodes. Needs `cargo xtask fixtures`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use as_core::{CancelToken, FrameIndex, FrameRange};
use as_export::preset::{EncoderPref, Gop, VideoCodec};
use as_export::{
    ClipJob, ExportPreset, JoinJob, JoinPart, MergePlan, MergeSource, Part, builtin_presets, export_clip, join_parts,
    plan_merge,
};
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
        part: None,
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
                part: None,
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
            part: None,
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
            part: None,
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
            part: None,
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
        // idet flags a few frames of any encode (how many depends on the
        // encoder's threads, so on the machine); a combed export is most of them.
        assert!(combed * 10 < progressive, "{file}: export still combed ({combed} vs {progressive} progressive)");
    }
}

/// However a clip of an interlaced source is cut, its frames look the same:
/// the first and last frame are deinterlaced from the fields around them,
/// like the frames inside a longer clip, and a one-frame clip isn't two
/// fields woven together (combed: neighbouring rows differ more than rows
/// two apart, from the same field).
#[test]
fn interlaced_clip_edges_look_like_the_frames_inside_a_clip() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    // Every frame of a 640×360 clip as grey.
    let luma = |file: &Path| -> Vec<Vec<u8>> {
        let out = tools
            .ffmpeg()
            .args(["-v", "error", "-i"])
            .arg(file)
            .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-vf", "format=gray", "-f", "rawvideo", "-"])
            .output()
            .unwrap();
        out.stdout.as_chunks::<{ 640 * 360 }>().0.iter().map(|f| f.to_vec()).collect()
    };
    let rows = |g: &[u8], gap: usize| -> u64 {
        (0..358)
            .flat_map(|y| (0..640).map(move |x| (y, x)))
            .map(|(y, x)| g[y * 640 + x].abs_diff(g[(y + gap) * 640 + x]) as u64)
            .sum()
    };
    let comb = |g: &[u8]| rows(g, 1) as f64 / rows(g, 2).max(1) as f64;
    let diff = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(&x, &y)| x.abs_diff(y) as f64).sum::<f64>() / a.len() as f64;

    let prores = preset("prores-hq");
    for file in ["interlaced_h264.ts", "interlaced_xvid.avi"] {
        let fx = load(&tools, file, work.path());
        let long = FrameRange::new(240, 270);
        let inside = luma(&check(&tools, &fx, long, &prores, work.path()));
        assert_eq!(inside.len(), 30);
        for range in [FrameRange::new(250, 258), FrameRange::new(255, 256)] {
            let clip = luma(&check(&tools, &fx, range, &prores, work.path()));
            for (i, f) in [(0, range.start), (clip.len() - 1, range.end - 1)] {
                let (got, want) = (&clip[i], &inside[(f - long.start) as usize]);
                let c = comb(got);
                assert!(c < 0.85, "{file} {range:?}: frame {f} combed ({c:.2})");
                let d = diff(got, want);
                assert!(d < 1.0, "{file} {range:?}: frame {f} differs from the same frame in a longer clip ({d:.2})");
            }
        }
    }
}

/// A lost audio frame (64 ms of nothing in the middle of the stream) must
/// not pull the rest of the sound forward — in exports and in previews.
#[test]
fn audio_dropouts_keep_sync() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("dropout.mkv");
    // Silence, then a tone from 2.0 s; the frames between 1.0 and 1.064 s are
    // gone but every other frame keeps its timestamp (PCM keeps frames as they are).
    let status = tools
        .ffmpeg()
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=320x180:r=24:d=4", "-f", "lavfi"])
        .args(["-i", "sine=f=1000:r=48000:d=4:samples_per_frame=1024"])
        .args(["-af", "aselect='not(between(t,1.0,1.06))',volume=enable='lt(t,2)':volume=0"])
        .args(["-map", "0:v", "-map", "1:a", "-c:v", "libx264", "-c:a", "pcm_s16le"])
        .arg(&src)
        .status()
        .unwrap();
    assert!(status.success());
    let info = probe(&tools, &src).unwrap();
    let index = build_frame_index(&tools, &src, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    let onset = |pcm: &[i16]| pcm.iter().position(|v| v.unsigned_abs() > 2000).unwrap() as f64 / 48_000.0;

    // Clip [0.5 s, 3.0 s): the tone must start 1.5 s in.
    let job = ClipJob {
        src: Source { path: &src, info: &info, index: &index },
        range: FrameRange::new(12, 72),
        audio_ordinal: None,
        preset: &preset("prores-hq"),
        out_stem: work.path().join("dropout_clip"),
        title: None,
        part: None,
    };
    let out = export_clip(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
    assert!(out.verified);
    let at = onset(&audio_pcm(&tools, Path::new(&out.path)));
    assert!((at - 1.5).abs() < 0.006, "export: tone starts at {at:.4} s, want 1.5 s");

    // The preview plays the same sound at the same moment.
    let proxy_out = work.path().join("dropout_proxy.mp4");
    let r = as_media::proxy::build_proxy(
        &tools,
        Source { path: &src, info: &info, index: &index },
        Some(0),
        &proxy_out,
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    let at = onset(&audio_pcm(&tools, &proxy_out));
    assert!((at - (2.0 + r.t0)).abs() < 0.006, "preview: tone starts at {at:.4} s, want {:.4} s", 2.0 + r.t0);
}

// -- merged exports -------------------------------------------------------------

/// An episode a merged export cuts from: a fixture, or a file made in a test.
struct Media {
    path: PathBuf,
    info: MediaInfo,
    index: FrameIndex,
}

fn media(tools: &Tools, path: PathBuf, work: &Path) -> Media {
    let mut info = probe(tools, &path).unwrap();
    if info.video.field_order.is_none()
        && let Some(order) = idet::detect_field_order(tools, &path, &info).unwrap()
    {
        info.video.field_order = Some(order.into());
    }
    let index = build_frame_index(tools, &path, &info, work, &CancelToken::new(), |_| {}).unwrap();
    Media { path, info, index }
}

fn fixture(tools: &Tools, file: &str, work: &Path) -> Media {
    media(tools, fixtures_dir().join(file), work)
}

/// A clip of a merged export: frames of an episode, and its audio track.
type Cut<'a> = (&'a Media, FrameRange, Option<u32>);

struct Merged {
    path: PathBuf,
    plan: MergePlan,
}

/// Render every clip as a part and join them, as a merged export does.
fn merge(tools: &Tools, cuts: &[Cut], preset: &ExportPreset, work: &Path, name: &str) -> Merged {
    let sources: Vec<MergeSource> = cuts
        .iter()
        .map(|(m, range, audio)| MergeSource { info: &m.info, index: &m.index, range: *range, audio_ordinal: *audio })
        .collect();
    let plan = plan_merge(tools.caps(), preset, &sources).unwrap();
    let dir = work.join(format!("{name}-parts"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut parts = Vec::new();
    for (i, ((m, range, audio), timing)) in cuts.iter().zip(&plan.parts).enumerate() {
        let job = ClipJob {
            src: Source { path: &m.path, info: &m.info, index: &m.index },
            range: *range,
            audio_ordinal: *audio,
            preset,
            out_stem: dir.join(format!("part-{:05}", i + 1)),
            title: None,
            part: Some(Part { shape: &plan.shape, samples: timing.samples }),
        };
        let out = export_clip(tools, &job, &CancelToken::new(), |_| {}).unwrap();
        assert!(out.verified, "{name}: part {} ({range:?}) failed its self-verification", i + 1);
        parts.push(JoinPart {
            path: out.path.into(),
            frames: out.frames,
            timing: *timing,
            title: format!("Shot {} = {}", i + 1, range.start),
        });
    }
    let job = JoinJob {
        parts: &parts,
        shape: &plan.shape,
        preset,
        out_stem: work.join(name),
        work_dir: &dir,
        title: Some(format!("{name} · scenepack")),
    };
    let out = join_parts(tools, &job, &CancelToken::new(), |_| {}).unwrap();
    assert!(out.verified, "{name}: the joined file failed its check");
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with("part-"))
        .collect();
    assert!(left.is_empty(), "{name}: join files left behind: {left:?}");
    Merged { path: out.path.into(), plan }
}

/// Where each cut starts in the merged file, in seconds.
fn starts(m: &Merged) -> Vec<f64> {
    let t = f64::from(m.plan.shape.timescale);
    let mut at = 0u64;
    m.plan
        .parts
        .iter()
        .map(|p| {
            let s = at as f64 / t;
            at += p.ticks;
            s
        })
        .collect()
}

/// Every frame of every cut, in order and at its own time; a chapter per cut.
fn check_merged(tools: &Tools, m: &Merged, cuts: &[Cut], name: &str) {
    let want: Vec<u32> = cuts.iter().flat_map(|(_, r, _)| r.start..r.end).collect();
    assert_eq!(barcodes(tools, &m.path), want, "{name}: decoded frames");
    let times = frame_times(tools, &m.path);
    assert_eq!(times.len(), want.len(), "{name}: frame count");
    let mut i = 0;
    for ((media, r, _), start) in cuts.iter().zip(starts(m)) {
        for f in r.start..r.end {
            let offset = if media.index.is_vfr() {
                media.index.t(f) - media.index.t(r.start)
            } else {
                f64::from(f - r.start) / media.index.nominal_fps.to_f64()
            };
            let want = start + offset;
            assert!((times[i] - want).abs() < 2e-6, "{name}: frame {f} of {r:?} at {:.6}, want {want:.6}", times[i]);
            i += 1;
        }
    }
    let out = tools
        .ffprobe()
        .args(["-show_entries", "chapter=start_time,end_time:chapter_tags=title", "-of", "json"])
        .arg(&m.path)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let chapters = v["chapters"].as_array().unwrap();
    assert_eq!(chapters.len(), cuts.len(), "{name}: a chapter per cut");
    for (k, (c, start)) in chapters.iter().zip(starts(m)).enumerate() {
        let at: f64 = c["start_time"].as_str().unwrap().parse().unwrap();
        assert!((at - start).abs() < 2e-6, "{name}: chapter {k} starts at {at}, want {start}");
        let title = format!("Shot {} = {}", k + 1, cuts[k].1.start);
        assert_eq!(c["tags"]["title"].as_str(), Some(title.as_str()), "{name}: chapter {k}'s title");
    }
}

/// The sound of every cut, where it belongs: exact length (to the sample
/// for PCM), silence for an episode without sound, each shot's own tone.
fn check_merged_audio(tools: &Tools, m: &Merged, cuts: &[Cut], truth: &Truth, pcm_exact: bool, name: &str) {
    let pcm = audio_pcm(tools, &m.path);
    let samples: u64 = m.plan.parts.iter().map(|p| p.samples).sum();
    if pcm_exact {
        assert_eq!(pcm.len() as u64, samples, "{name}: audio length");
    } else {
        assert!((pcm.len() as u64).abs_diff(samples) < 2048, "{name}: audio length {} vs {samples}", pcm.len());
    }
    let peak = |a: &[i16]| a.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
    let mut at = 0usize;
    for ((media, r, _), part) in cuts.iter().zip(&m.plan.parts) {
        let span = at..at + part.samples as usize;
        at = span.end;
        // Clear of the edges: the shot before, AAC's ramps.
        let inner = span.start + 1440..span.end.saturating_sub(1440).max(span.start + 1440);
        if media.info.audio.is_empty() {
            assert!(peak(&pcm[inner]) < 300, "{name}: sound in {r:?} of an episode without any");
            continue;
        }
        // Its sound starts half a second in: silent at first.
        let late = media.path.ends_with("late_audio_h264.mkv");
        let Some(shot) = truth.shots().find(|s| s.start <= r.start && r.end <= s.end && s.tone_hz.is_some()) else {
            continue;
        };
        if late || inner.len() < 960 {
            continue;
        }
        let hz = tone_hz(&pcm[inner.start..inner.start + 960]);
        let want = f64::from(shot.tone_hz.unwrap());
        assert!((hz - want).abs() / want < 0.12, "{name}: tone {hz:.0} Hz in {r:?}, want {want}");
    }
}

#[test]
fn merged_parts_join_frame_exact() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let n = truth.frame_count;
    let fx = |f| fixture(&tools, f, work.path());
    let (cfr24, ntsc, vfr, ts, two, late, prores, avi) = (
        fx("cfr24_h264.mp4"),
        fx("cfr23976_h264.mp4"),
        fx("vfr_h264.mkv"),
        fx("interlaced_h264.ts"),
        fx("two_audio_h264.mkv"),
        fx("late_audio_h264.mkv"),
        fx("prores.mov"),
        fx("interlaced_xvid.avi"),
    );
    let cuts: Vec<Cut> = vec![
        (&cfr24, FrameRange::new(0, 12), None),
        (&ntsc, FrameRange::new(90, 130), None),
        // No sound at all; across its 24 → 60 → 30 fps changes.
        (&vfr, FrameRange::new(180, 260), None),
        (&cfr24, FrameRange::new(268, 269), None),
        (&ts, FrameRange::new(250, 258), None),
        (&two, FrameRange::new(160, 200), Some(1)),
        (&late, FrameRange::new(6, 30), None),
        (&prores, FrameRange::new(346, 394), None),
        (&avi, FrameRange::new(100, 112), None),
        (&ntsc, FrameRange::new(n - 11, n), None),
    ];
    for (p, exact) in [(preset("prores-hq"), true), (software_h264(), false)] {
        let name = format!("merged-{}", p.id);
        let m = merge(&tools, &cuts, &p, work.path(), &name);
        // 24, 23.976 and MKV's milliseconds.
        assert_eq!(m.plan.shape.timescale, 24_000);
        assert!(m.path.extension().is_some_and(|e| e == p.container.ext()), "{name}: {}", m.path.display());
        check_merged(&tools, &m, &cuts, &name);
        check_merged_audio(&tools, &m, &cuts, &truth, exact, &name);
        // The second track was taken: it's the first at half the level.
        let pcm = audio_pcm(&tools, &m.path);
        let span = |k: usize| {
            let at: u64 = m.plan.parts[..k].iter().map(|p| p.samples).sum();
            let (a, b) = (at as usize + 2400, (at + m.plan.parts[k].samples) as usize - 2400);
            pcm[a..b].iter().map(|v| v.unsigned_abs()).max().unwrap() as f64
        };
        let (half, full) = (span(5), span(1));
        assert!((0.4..0.6).contains(&(half / full)), "{name}: the second track's level {half} vs {full}");
        let info = probe(&tools, &m.path).unwrap();
        assert_eq!((info.video.width, info.video.height), (640, 360));
        assert_eq!(info.audio.len(), 1, "{name}: one sound track");
        assert!(!info.is_interlaced(), "{name}: deinterlaced");
    }
    // A second join of the same name takes a free one.
    let again = merge(&tools, &cuts[..2], &preset("prores-hq"), work.path(), "merged-prores-hq");
    assert!(again.path.ends_with("merged-prores-hq_2.mov"), "{}", again.path.display());
}

/// Episodes of other sizes and pixel shapes come out in one frame: scaled to
/// fit, letterboxed or pillarboxed, square pixels; mono and stereo episodes
/// share one stereo track.
#[test]
fn merged_sizes_and_pixel_shapes_become_one() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let make = |name: &str, args: &[&str]| -> Media {
        let path = work.path().join(name);
        let status = tools.ffmpeg().args(["-v", "error", "-y"]).args(args).arg(&path).status().unwrap();
        assert!(status.success(), "{name}");
        media(&tools, path, work.path())
    };
    let master = fixtures_dir().join("cfr24_h264.mp4");
    let master = master.to_str().unwrap();
    let base = fixture(&tools, "cfr24_h264.mp4", work.path());
    let hd = make("hd.mp4", &["-i", master, "-vf", "scale=1280:720", "-c:v", "libx264", "-crf", "12", "-c:a", "copy"]);
    // Anamorphic: 480×360 stored, 640×360 shown.
    let squeezed = make(
        "squeezed.mkv",
        &["-i", master, "-vf", "scale=480:360,setsar=4/3", "-c:v", "libx264", "-crf", "12", "-c:a", "copy"],
    );
    assert_eq!(squeezed.info.video.sar, as_core::Rational::new(4, 3));
    // 4:3 with stereo sound and no barcode: pillarboxed.
    let four_three = make(
        "four_three.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=480x360:r=24:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:r=48000:d=2",
            "-ac",
            "2",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
        ],
    );
    let cuts: Vec<Cut> = vec![
        (&base, FrameRange::new(0, 40), None),
        (&hd, FrameRange::new(48, 60), None),
        (&squeezed, FrameRange::new(84, 100), None),
        (&four_three, FrameRange::new(0, 12), None),
    ];
    let p = preset("prores-hq");
    let m = merge(&tools, &cuts, &p, work.path(), "shapes");
    assert_eq!((m.plan.shape.width, m.plan.shape.height), (640, 360));
    assert_eq!(m.plan.shape.audio.as_deref(), Some("stereo"), "mono and stereo episodes");
    let info = probe(&tools, &m.path).unwrap();
    assert_eq!((info.video.width, info.video.height), (640, 360));
    assert_eq!(info.video.sar, as_core::Rational::new(1, 1));
    assert_eq!(info.audio[0].channels, 2);
    // Every frame as grey: the barcode parts read back, the 4:3 one is pillarboxed.
    let out = tools
        .ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(&m.path)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-vf", "format=gray", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    let frames: Vec<&[u8; 640 * 360]> = out.stdout.as_chunks::<{ 640 * 360 }>().0.iter().collect();
    assert_eq!(frames.len(), 40 + 12 + 16 + 12);
    let want: Vec<u32> = cuts[..3].iter().flat_map(|(_, r, _)| r.start..r.end).collect();
    for (i, (f, want)) in frames.iter().zip(&want).enumerate() {
        let code = barcode::read_luma(&f[..], 640, 360, 640).unwrap_or_else(|| panic!("frame {i}: no barcode"));
        assert_eq!(u32::from(code.frame), *want, "frame {i}");
    }
    for f in &frames[68..] {
        let bars = (0..360).flat_map(|y| (0..76).chain(564..640).map(move |x| f[y * 640 + x]));
        assert!(bars.max().unwrap() < 24, "pillarbox bars are black");
        assert!(f[180 * 640 + 320] > 24 || f[100 * 640 + 300] > 24, "the picture is inside");
    }
    let pcm = audio_pcm(&tools, &m.path);
    let start = m.plan.parts[..3].iter().map(|p| p.samples as usize).sum::<usize>();
    let hz = tone_hz(&pcm[start + 1440..start + 1440 + 4800]);
    assert!((hz - 440.0).abs() < 25.0, "the 4:3 episode's tone: {hz:.0} Hz");
    // A mono episode plays at its own level on both sides, not 3 dB down.
    let peak = |a: &[i16]| a.iter().map(|v| v.unsigned_abs()).max().unwrap() as f64;
    let source = audio_pcm(&tools, &base.path);
    let ratio = peak(&pcm[2400..72_000]) / peak(&source[2400..72_000]);
    assert!((0.9..1.1).contains(&ratio), "mono episode at {ratio:.2} of its level");
}

/// Folders named like people name them ("#1", "it's", accents) hold the
/// parts and the file like any other.
#[test]
fn merged_files_join_in_any_folder() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("Scenes #1 it's ü");
    std::fs::create_dir_all(&dir).unwrap();
    let fx = fixture(&tools, "cfr24_h264.mp4", work.path());
    let cuts: Vec<Cut> = vec![(&fx, FrameRange::new(0, 12), None), (&fx, FrameRange::new(48, 60), None)];
    let m = merge(&tools, &cuts, &preset("prores-hq"), &dir, "Show #1");
    assert_eq!(m.path, dir.join("Show #1.mov"));
    check_merged(&tools, &m, &cuts, "special folder");
}

/// HDR and SDR episodes in one file: tone mapped together (BT.709); HDR
/// episodes alone in a preset that keeps HDR stay HDR.
#[test]
fn merged_hdr_and_sdr_are_tone_mapped_together() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let hdr = fixture(&tools, "hevc10_hdr.mkv", work.path());
    assert_ne!(hdr.info.video.hdr, as_media::HdrKind::None, "the fixture is HDR");
    let sdr = fixture(&tools, "cfr24_h264.mp4", work.path());
    let mut hevc = preset("hevc10");
    hevc.video = VideoCodec::Hevc { quality: 18, gop: Gop::Editing, encoder: EncoderPref::Software, ten_bit: true };

    let mixed: Vec<Cut> = vec![(&hdr, FrameRange::new(0, 20), None), (&sdr, FrameRange::new(20, 30), None)];
    let m = merge(&tools, &mixed, &hevc, work.path(), "mixed");
    assert!(m.plan.shape.tonemap);
    check_merged(&tools, &m, &mixed, "mixed");
    let info = probe(&tools, &m.path).unwrap();
    assert_eq!(info.video.hdr, as_media::HdrKind::None);
    assert_eq!(info.video.color_transfer.as_deref(), Some("bt709"));
    assert_eq!(info.video.color_primaries.as_deref(), Some("bt709"));
    // No HDR light levels left on an SDR picture.
    let side = tools.ffprobe().args(["-show_entries", "stream_side_data_list", "-of", "json"]).arg(&m.path).output();
    let side = String::from_utf8_lossy(&side.unwrap().stdout).to_lowercase();
    assert!(!side.contains("light level") && !side.contains("mastering"), "HDR metadata left: {side}");

    let all_hdr: Vec<Cut> = vec![(&hdr, FrameRange::new(0, 20), None), (&hdr, FrameRange::new(300, 310), None)];
    let m = merge(&tools, &all_hdr, &preset("prores-hq"), work.path(), "hdr");
    assert!(!m.plan.shape.tonemap);
    check_merged(&tools, &m, &all_hdr, "hdr");
    let info = probe(&tools, &m.path).unwrap();
    assert_eq!(info.video.hdr, hdr.info.video.hdr, "HDR kept");
}

/// Hardware encoders write the frame rate into the stream headers, so parts
/// of 24 and 23.976 fps episodes differ there; the file still decodes cleanly.
#[test]
fn merged_hardware_parts_join() {
    let Some((tools, _)) = setup() else { return };
    if tools.caps().hw_encoder(as_media::CodecFamily::H264).is_none() {
        eprintln!("no hardware H.264 encoder on this machine");
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let a = fixture(&tools, "cfr24_h264.mp4", work.path());
    let b = fixture(&tools, "cfr23976_h264.mp4", work.path());
    let cuts: Vec<Cut> = vec![
        (&a, FrameRange::new(0, 30), None),
        (&b, FrameRange::new(30, 60), None),
        (&a, FrameRange::new(60, 70), None),
    ];
    for id in ["h264-edit", "hevc10"] {
        let name = format!("hw-{id}");
        let m = merge(&tools, &cuts, &preset(id), work.path(), &name);
        check_merged(&tools, &m, &cuts, &name);
        let out = tools.ffmpeg().args(["-v", "error", "-i"]).arg(&m.path).args(["-f", "null", "-"]).output().unwrap();
        let log = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success() && log.trim().is_empty(), "{name}: decoding complains: {log}");
    }
}
