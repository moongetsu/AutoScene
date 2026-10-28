//! Bit-Perfect export: every frame decodes to exactly the source's pixels and
//! every sample is the source's, checked here against a full decode of the
//! source (not the export's own hashes). Needs `cargo xtask fixtures`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use as_core::{CancelToken, FrameIndex, FrameRange};
use as_export::{
    ClipJob, ExportPreset, JoinJob, JoinPart, LosslessCodec, MergeSource, Part, builtin_presets, exact_format,
    export_clip, join_parts, lossless_merge_blocker, plan_lossless_merge,
};
use as_media::{MediaInfo, Source, Tools, build_frame_index, idet, probe};
use as_testkit::{Truth, fixtures_dir, load_truth};

struct Fixture {
    path: PathBuf,
    info: MediaInfo,
    index: FrameIndex,
}

fn setup() -> Option<(Tools, Truth)> {
    let truth = load_truth()?;
    Some((Tools::locate(None).expect("ffmpeg on PATH"), truth))
}

/// Probe and index like the app's ingest does.
fn load(tools: &Tools, path: &Path, work: &Path) -> Fixture {
    let mut info = probe(tools, path).unwrap();
    if info.video.field_order.is_none()
        && let Some(order) = idet::detect_field_order(tools, path, &info).unwrap()
    {
        info.video.field_order = Some(order.into());
    }
    let index = build_frame_index(tools, path, &info, work, &CancelToken::new(), |_| {}).unwrap();
    Fixture { path: path.to_path_buf(), info, index }
}

fn bit_perfect() -> ExportPreset {
    builtin_presets().into_iter().find(|p| p.id == "bit-perfect").unwrap()
}

/// Per-frame MD5s of `file`'s decoded frames, as stored (not turned upright).
fn frame_md5s(tools: &Tools, file: &Path) -> Vec<String> {
    let out = tools
        .ffmpeg()
        .args(["-v", "error", "-noautorotate", "-i"])
        .arg(file)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-f", "framemd5", "-"])
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.rsplit(',').next().map(|h| h.trim().to_string()))
        .collect()
}

/// `stream=<entry>` of `file`'s first stream of `kind` (`v` or `a`).
fn stream_entry(tools: &Tools, file: &Path, kind: &str, entry: &str) -> String {
    let out = tools
        .ffprobe()
        .args(["-select_streams", &format!("{kind}:0"), "-show_entries", &format!("stream={entry}")])
        .args(["-of", "default=nw=1:nk=1"])
        .arg(file)
        .output()
        .unwrap();
    // A MOV's stream group (FFmpeg 9) repeats its streams: the first.
    String::from_utf8(out.stdout).unwrap().lines().next().unwrap_or_default().trim().to_string()
}

/// The raw format `ffmpeg -f` writes a PCM codec's samples in.
fn raw_format(codec: &str) -> &'static str {
    match codec {
        "pcm_f32le" => "f32le",
        "pcm_f64le" => "f64le",
        "pcm_s16le" => "s16le",
        "pcm_s24le" => "s24le",
        "pcm_s32le" => "s32le",
        other => panic!("not a lossless PCM codec: {other}"),
    }
}

/// `file`'s first sound track as raw samples of `codec`.
fn raw_sound(tools: &Tools, file: &Path, codec: &str, trim: Option<(f64, f64)>, track: u32) -> Vec<u8> {
    let mut cmd = tools.ffmpeg();
    cmd.args(["-v", "error", "-copyts", "-i"]).arg(file).args(["-map", &format!("0:a:{track}")]);
    if let Some((s, e)) = trim {
        cmd.args(["-af", &format!("atrim=start={s:.9}:end={e:.9}")]);
    }
    let out = cmd.args(["-c:a", codec, "-f", raw_format(codec), "-"]).stdout(Stdio::piped()).output().unwrap();
    out.stdout
}

/// Export `range` of `fx` Bit-Perfect; the export checks itself, and here
/// its frames are compared with the source's own (decoded whole) and its
/// sound with the source's samples. Returns the file and its codec.
fn check(tools: &Tools, fx: &Fixture, range: FrameRange, source_md5s: &[String], work: &Path) -> PathBuf {
    let name = fx.path.file_name().unwrap().to_string_lossy().replace('.', "_");
    let preset = bit_perfect();
    let job = ClipJob {
        src: Source { path: &fx.path, info: &fx.info, index: &fx.index },
        range,
        audio_ordinal: None,
        preset: &preset,
        out_stem: work.join(format!("{name}_{}_{}", range.start, range.end)),
        title: None,
        part: None,
    };
    let out = export_clip(tools, &job, &CancelToken::new(), |_| {})
        .unwrap_or_else(|e| panic!("{name} {range:?}: {e} ({e:?})"));
    assert!(out.verified, "{name} {range:?}");
    let file = PathBuf::from(&out.path);
    let want = &source_md5s[range.start as usize..range.end as usize];
    assert_eq!(frame_md5s(tools, &file), want, "{name} {range:?}: decoded frames");
    if !fx.info.audio.is_empty() {
        let codec = stream_entry(tools, &file, "a", "codec_name");
        let got = raw_sound(tools, &file, &codec, None, 0);
        let (s, e) = (fx.index.t(range.start), fx.index.t(range.end));
        let ord = fx.info.default_audio().map_or(0, |a| a.ordinal);
        // The source's sound around the clip, a second either side.
        let margin = s.min(1.0);
        let src = raw_sound(tools, &fx.path, &codec, Some((s - margin, e + 1.0)), ord);
        let unit = match codec.as_str() {
            "pcm_s16le" => 2,
            "pcm_s24le" => 3,
            "pcm_f64le" => 8,
            _ => 4,
        } * fx.info.audio[0].channels as usize;
        let rate = f64::from(fx.info.audio[0].sample_rate);
        // The clip's samples are a run of the source's, the same ones (silence
        // pads it where the source's sound ends): where in the source?
        let n = got.len() / unit;
        let probe = n.min(512);
        let at = (0..=(src.len() / unit).saturating_sub(probe))
            .filter(|&i| got[..probe * unit] == src[i * unit..(i + probe) * unit])
            .min_by_key(|&i| (i as f64 - margin * rate).abs() as i64);
        let at = at.unwrap_or_else(|| panic!("{name} {range:?}: its sound isn't the source's samples ({codec})"));
        let have = (src.len() / unit - at).min(n);
        assert_eq!(got[..have * unit], src[at * unit..(at + have) * unit], "{name} {range:?}: sound ({codec})");
        assert!(got[have * unit..].iter().all(|&b| b == 0), "{name} {range:?}: only silence after the source's sound");
        // On the frame's time: to the sample (one either way where a frame's
        // edge falls between two), or as close as an MKV's millisecond
        // timestamps say (a seek reads them, a full decode counts samples).
        let shift = at as f64 - margin * rate;
        let slack = if fx.info.container.contains("matroska") { 0.0005 * rate + 1.0 } else { 1.0 };
        assert!(shift.abs() <= slack, "{name} {range:?}: sound {shift:+} samples off the picture");
    }
    file
}

#[test]
fn every_variant_is_kept_exactly() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    for v in &truth.variants {
        let fx = load(&tools, &fixtures_dir().join(&v.file), work.path());
        let md5s = frame_md5s(&tools, &fx.path);
        assert_eq!(md5s.len(), fx.index.len() as usize, "{}: the index counts every frame", v.file);
        let n = fx.index.len();
        // Within a GOP (not starting on a keyframe), one frame, the end.
        for range in [FrameRange::new(301, 341), FrameRange::new(268, 269), FrameRange::new(n - 11, n)] {
            let file = check(&tools, &fx, range, &md5s, work.path());
            let codec = stream_entry(&tools, &file, "v", "codec_name");
            let exact = exact_format(&tools, &fx.path, fx.info.video.index).unwrap();
            // Every one of them (interlaced too) in HEVC, which editors open.
            assert_eq!(codec, "hevc", "{} {range:?}", v.file);
            assert_eq!(stream_entry(&tools, &file, "v", "pix_fmt"), exact.pix_fmt.replace("yuvj", "yuv"), "{}", v.file);
        }
    }
}

#[test]
fn hdr10_keeps_its_mastering_display_and_light_level() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let w = work.path();
    // An HDR10 episode as a release carries it: SEI on the frames, boxes in the file.
    let hdr10 = variant_of(
        &tools,
        w,
        "hdr10.mov",
        &[],
        &[
            "-t",
            "4",
            "-vf",
            "format=yuv420p10le",
            "-c:v",
            "libx265",
            "-x265-params",
            "log-level=error:hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
            "-tag:v",
            "hvc1",
            "-c:a",
            "copy",
        ],
    );
    let fx = load(&tools, &hdr10, w);
    let want = exact_format(&tools, &fx.path, fx.info.video.index).unwrap().hdr_metadata;
    assert_eq!(want, ["Content light level metadata", "Mastering display metadata"]);
    // On any machine (NVENC would drop them): kept, and the check knows.
    let file = check(&tools, &fx, FrameRange::new(20, 50), &frame_md5s(&tools, &hdr10), w);
    let out = probe(&tools, &file).unwrap();
    assert_eq!(as_media::probe_exact(&tools, &file, out.video.index).unwrap().hdr_metadata, want);
}

#[test]
fn hdr_keeps_its_colour() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let fx = load(&tools, &fixtures_dir().join("hevc10_hdr.mkv"), work.path());
    let md5s = frame_md5s(&tools, &fx.path);
    let file = check(&tools, &fx, FrameRange::new(100, 130), &md5s, work.path());
    let out = probe(&tools, &file).unwrap();
    assert_eq!(out.video.pix_fmt, "yuv420p10le");
    assert_eq!(out.video.color_transfer.as_deref(), Some("smpte2084"));
    assert_eq!(out.video.color_primaries.as_deref(), Some("bt2020"));
    assert_eq!(out.video.color_space.as_deref(), Some("bt2020nc"));
    assert_eq!(out.video.hdr, as_media::HdrKind::Pq);
}

/// A copy of the 24 fps fixture made by `args` (`ffmpeg … <args> <out>`).
fn variant_of(tools: &Tools, work: &Path, name: &str, input: &[&str], args: &[&str]) -> PathBuf {
    let out = work.join(name);
    let status = tools
        .ffmpeg()
        .args(["-v", "error", "-y"])
        .args(input)
        .arg("-i")
        .arg(fixtures_dir().join("cfr24_h264.mp4"))
        .args(args)
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success(), "making {name}");
    out
}

#[test]
fn rotation_range_and_odd_sizes_stay_as_they_are() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let w = work.path();
    // A phone video standing up: the rotation stays a tag, the pixels as stored.
    let rotated = variant_of(&tools, w, "rotated.mp4", &["-display_rotation", "90"], &["-map", "0", "-c", "copy"]);
    let fx = load(&tools, &rotated, w);
    assert_eq!(fx.info.video.rotation, 90);
    let file = check(&tools, &fx, FrameRange::new(301, 341), &frame_md5s(&tools, &rotated), w);
    assert_eq!(probe(&tools, &file).unwrap().video.rotation, 90);
    // Full range (JPEG-style levels) stays full range.
    let full = variant_of(
        &tools,
        w,
        "full.mp4",
        &[],
        &[
            "-t",
            "6",
            "-vf",
            "scale=out_range=pc",
            "-color_range",
            "pc",
            "-c:v",
            "libx264",
            "-crf",
            "20",
            "-c:a",
            "copy",
        ],
    );
    let fx = load(&tools, &full, w);
    let file = check(&tools, &fx, FrameRange::new(30, 60), &frame_md5s(&tools, &full), w);
    assert_eq!(probe(&tools, &file).unwrap().video.color_range.as_deref(), Some("pc"));
    // An odd size HEVC can't hold in 4:2:0: FFV1.
    let odd =
        variant_of(&tools, w, "odd.mkv", &[], &["-t", "6", "-vf", "scale=640:357", "-c:v", "ffv1", "-c:a", "copy"]);
    let fx = load(&tools, &odd, w);
    let file = check(&tools, &fx, FrameRange::new(30, 60), &frame_md5s(&tools, &odd), w);
    assert_eq!(stream_entry(&tools, &file, "v", "codec_name"), "ffv1");
    assert_eq!(stream_entry(&tools, &file, "v", "height"), "357");
}

#[test]
fn a_joined_file_is_its_clips_exactly() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let a = load(&tools, &fixtures_dir().join("cfr24_h264.mp4"), work.path());
    let b = load(&tools, &fixtures_dir().join("cfr23976_h264.mp4"), work.path());
    let clips = [(&a, FrameRange::new(140, 160)), (&b, FrameRange::new(301, 341)), (&a, FrameRange::new(575, 586))];
    let sources: Vec<MergeSource> = clips
        .iter()
        .map(|(fx, range)| MergeSource { info: &fx.info, index: &fx.index, range: *range, audio_ordinal: None })
        .collect();
    assert_eq!(lossless_merge_blocker(&sources), None);
    let exact: Vec<_> =
        clips.iter().map(|(fx, _)| exact_format(&tools, &fx.path, fx.info.video.index).unwrap()).collect();
    let plan = plan_lossless_merge(tools.caps(), &sources, &exact).unwrap();
    let preset = bit_perfect();
    let mut parts = Vec::new();
    let mut want = Vec::new();
    for (i, ((fx, range), timing)) in clips.iter().zip(&plan.parts).enumerate() {
        let job = ClipJob {
            src: Source { path: &fx.path, info: &fx.info, index: &fx.index },
            range: *range,
            audio_ordinal: None,
            preset: &preset,
            out_stem: work.path().join(format!("part-{i}")),
            title: None,
            part: Some(Part { shape: &plan.shape, samples: timing.samples }),
        };
        let out = export_clip(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
        parts.push(JoinPart { path: out.path.into(), frames: out.frames, timing: *timing, title: format!("Shot {i}") });
        want.extend_from_slice(&frame_md5s(&tools, &fx.path)[range.start as usize..range.end as usize]);
    }
    let job = JoinJob {
        parts: &parts,
        shape: &plan.shape,
        preset: &preset,
        out_stem: work.path().join("joined"),
        work_dir: work.path(),
        title: Some("Pack".into()),
    };
    let out = join_parts(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
    assert!(out.verified);
    let file = PathBuf::from(&out.path);
    assert_eq!(file.extension().unwrap(), LosslessCodec::Hevc.ext());
    assert_eq!(frame_md5s(&tools, &file), want);
    let samples: u64 = plan.parts.iter().map(|p| p.samples).sum();
    assert_eq!(stream_entry(&tools, &file, "a", "duration_ts").parse::<u64>().unwrap(), samples);
}

#[test]
fn a_joined_file_of_ffv1_parts_is_its_clips_exactly() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let w = work.path();
    // An odd height: HEVC can't hold it as it is, so the parts are FFV1, the file an MKV.
    let odd =
        variant_of(&tools, w, "odd.mkv", &[], &["-t", "8", "-vf", "scale=640:357", "-c:v", "ffv1", "-c:a", "copy"]);
    let fx = load(&tools, &odd, w);
    let clips = [FrameRange::new(10, 34), FrameRange::new(100, 130), FrameRange::new(150, 151)];
    let sources: Vec<MergeSource> = clips
        .iter()
        .map(|range| MergeSource { info: &fx.info, index: &fx.index, range: *range, audio_ordinal: None })
        .collect();
    let exact = vec![exact_format(&tools, &fx.path, fx.info.video.index).unwrap(); clips.len()];
    let plan = plan_lossless_merge(tools.caps(), &sources, &exact).unwrap();
    let preset = bit_perfect();
    let all = frame_md5s(&tools, &odd);
    let mut parts = Vec::new();
    let mut want = Vec::new();
    for (i, (range, timing)) in clips.iter().zip(&plan.parts).enumerate() {
        let job = ClipJob {
            src: Source { path: &fx.path, info: &fx.info, index: &fx.index },
            range: *range,
            audio_ordinal: None,
            preset: &preset,
            out_stem: w.join(format!("part-{i}")),
            title: None,
            part: Some(Part { shape: &plan.shape, samples: timing.samples }),
        };
        let out = export_clip(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
        parts.push(JoinPart { path: out.path.into(), frames: out.frames, timing: *timing, title: format!("Shot {i}") });
        want.extend_from_slice(&all[range.start as usize..range.end as usize]);
    }
    let job = JoinJob {
        parts: &parts,
        shape: &plan.shape,
        preset: &preset,
        out_stem: w.join("joined"),
        work_dir: w,
        title: Some("Pack".into()),
    };
    let out = join_parts(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
    let file = PathBuf::from(&out.path);
    assert_eq!(file.extension().unwrap(), LosslessCodec::Ffv1.ext());
    assert_eq!(frame_md5s(&tools, &file), want);
}

#[test]
fn a_clip_never_replaces_a_file_that_came_meanwhile() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let fx = load(&tools, &fixtures_dir().join("cfr24_h264.mp4"), work.path());
    let preset = bit_perfect();
    let stem = work.path().join("Walter");
    std::fs::write(work.path().join("Walter.mov"), b"the user's").unwrap();
    let job = ClipJob {
        src: Source { path: &fx.path, info: &fx.info, index: &fx.index },
        range: FrameRange::new(10, 20),
        audio_ordinal: None,
        preset: &preset,
        out_stem: stem,
        title: None,
        part: None,
    };
    let out = export_clip(&tools, &job, &CancelToken::new(), |_| {}).unwrap();
    assert!(out.path.ends_with("Walter_2.mov"), "{}", out.path);
    assert_eq!(std::fs::read(work.path().join("Walter.mov")).unwrap(), b"the user's");
}

#[test]
fn episodes_unlike_each_other_share_no_file() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let a = load(&tools, &fixtures_dir().join("cfr24_h264.mp4"), work.path());
    let b = load(&tools, &fixtures_dir().join("hevc10_hdr.mkv"), work.path());
    let sources = [
        MergeSource { info: &a.info, index: &a.index, range: FrameRange::new(0, 24), audio_ordinal: None },
        MergeSource { info: &b.info, index: &b.index, range: FrameRange::new(0, 24), audio_ordinal: None },
    ];
    assert!(lossless_merge_blocker(&sources).is_some());
    let exact = [
        exact_format(&tools, &a.path, a.info.video.index).unwrap(),
        exact_format(&tools, &b.path, b.info.video.index).unwrap(),
    ];
    assert!(matches!(plan_lossless_merge(tools.caps(), &sources, &exact), Err(as_export::ExportError::Inexact(_))));
}
