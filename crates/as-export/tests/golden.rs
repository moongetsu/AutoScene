//! Frame-exact export, proven with barcodes. Needs `cargo xtask fixtures`.

use std::path::Path;
use std::process::Stdio;

use as_core::{CancelToken, FrameIndex, FrameRange};
use as_export::preset::{EncoderPref, Gop, VideoCodec};
use as_export::{ClipJob, ExportPreset, builtin_presets, export_clip};
use as_media::{MediaInfo, Source, Tools, build_frame_index, probe};
use as_testkit::{Truth, barcode, fixtures_dir, load_truth};

struct Fixture {
    file: &'static str,
    info: MediaInfo,
    index: FrameIndex,
}

fn setup() -> Option<(Tools, Truth)> {
    let Some(truth) = load_truth() else {
        eprintln!("fixtures missing — run `cargo xtask fixtures`; skipping");
        return None;
    };
    Some((Tools::locate(None).expect("ffmpeg on PATH"), truth))
}

fn load(tools: &Tools, file: &'static str, work: &Path) -> Fixture {
    let path = fixtures_dir().join(file);
    let info = probe(tools, &path).unwrap();
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
        .chunks_exact(640 * 360)
        .map(|f| barcode::read_luma(f, 640, 360, 640).expect("barcode").frame as u32)
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
    out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
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

fn check(tools: &Tools, fx: &Fixture, range: FrameRange, preset: &ExportPreset, work: &Path) {
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
