//! Frame-exact clip rendering.
//!
//! For clip `[s, e)` on a media's frame grid:
//! - video: `trim` from frame `s`'s own timestamp to frame `e`'s, on absolute
//!   timestamps (`-copyts`), in microseconds that round back onto exactly
//!   those frames in any time base (see `FrameIndex::trim_us`);
//! - audio: `atrim` to `[t(s), t(e))` and re-timed against `t(s)` itself —
//!   not against the first sample that happens to exist — so a stream that
//!   starts late or has gaps gets silence where it has no sound and stays in
//!   sync; then padded/cut to the exact sample count, so A/V length matches;
//! - `-fps_mode passthrough` forbids frame duplication/dropping, and VFR
//!   sources keep their own time base so every frame keeps its timing;
//! - the encoder logs each frame's *original decoder timestamp* via
//!   `-stats_enc_pre {ptsi}`, and we check it equals `pts[s..e)` exactly.
//!
//! If verification fails (unusual seek behaviour in a broken file), we retry
//! with a longer pre-roll and finally with no seek at all.

use std::path::{Path, PathBuf};

use as_core::{CancelToken, FrameIndex, FrameRange, Rational, Timecode};
use as_media::filters::display_chain;
use as_media::{Caps, CodecFamily, HdrKind, MediaInfo, Source, Tools, Vendor, run_ffmpeg};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::preset::{
    AudioCodec, Container, DnxProfile, EncoderPref, ExportPreset, Gop, HdrMode, ProResProfile, VideoCodec,
};
use crate::{ExportError, ExportResult};

const AUDIO_RATE: u32 = 48_000;

/// What to cut and where to write it.
#[derive(Clone, Debug)]
pub struct ClipJob<'a> {
    pub src: Source<'a>,
    pub range: FrameRange,
    /// Audio stream ordinal (`0:a:N`); `None` → default track.
    pub audio_ordinal: Option<u32>,
    pub preset: &'a ExportPreset,
    /// Output path *without* extension; the preset's container decides it.
    /// Dots in the name are kept ("Mr. Robot" stays "Mr. Robot.mov").
    pub out_stem: PathBuf,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClipOutcome {
    pub path: String,
    pub frames: u32,
    /// Every output frame was checked against the source frame grid.
    pub verified: bool,
    pub encoder: String,
    pub bytes: f64,
}

/// Which encoder and arguments a preset resolves to on this machine.
struct Encoding {
    encoder: String,
    args: Vec<String>,
    pix_fmt: &'static str,
}

fn gop_args(gop: Gop, fps: f64, hardware: Option<Vendor>, args: &mut Vec<String>) {
    match gop {
        Gop::Editing => {
            args.extend(["-g".into(), format!("{}", fps.round().max(1.0) as u32), "-bf".into(), "0".into()])
        }
        // Hardware encoders reject `-g 1` (NVENC needs GOP > B-frames + 1); asking
        // for a keyframe on every frame is encoder-agnostic.
        Gop::AllIntra if hardware.is_some() => {
            args.extend(["-bf", "0", "-force_key_frames", "expr:1"].map(String::from));
            if hardware == Some(Vendor::Nvidia) {
                args.extend(["-forced-idr".into(), "1".into()]);
            }
        }
        Gop::AllIntra => args.extend(["-g".into(), "1".into(), "-bf".into(), "0".into()]),
        Gop::Delivery => {}
    }
}

fn resolve_encoding(codec: &VideoCodec, caps: &Caps, fps: f64) -> ExportResult<Encoding> {
    let sw = |name: &str| -> ExportResult<String> {
        if caps.has_sw(name) { Ok(name.to_string()) } else { Err(ExportError::EncoderMissing(name.into())) }
    };
    Ok(match codec {
        VideoCodec::Prores { profile } => {
            let (p, pix) = match profile {
                ProResProfile::Proxy => ("0", "yuv422p10le"),
                ProResProfile::Lt => ("1", "yuv422p10le"),
                ProResProfile::Standard => ("2", "yuv422p10le"),
                ProResProfile::Hq => ("3", "yuv422p10le"),
                ProResProfile::P4444 => ("4", "yuv444p10le"),
            };
            Encoding {
                encoder: sw("prores_ks")?,
                args: ["-c:v", "prores_ks", "-profile:v", p, "-vendor", "apl0"].map(String::from).to_vec(),
                pix_fmt: pix,
            }
        }
        VideoCodec::Dnxhr { profile } => {
            let (p, pix) = match profile {
                DnxProfile::Hq => ("dnxhr_hq", "yuv422p"),
                DnxProfile::Hqx => ("dnxhr_hqx", "yuv422p10le"),
            };
            Encoding {
                encoder: sw("dnxhd")?,
                args: ["-c:v", "dnxhd", "-profile:v", p].map(String::from).to_vec(),
                pix_fmt: pix,
            }
        }
        VideoCodec::H264 { quality, gop, encoder } | VideoCodec::Hevc { quality, gop, encoder, .. } => {
            let hevc = matches!(codec, VideoCodec::Hevc { .. });
            let ten = matches!(codec, VideoCodec::Hevc { ten_bit: true, .. });
            let family = if hevc { CodecFamily::Hevc } else { CodecFamily::H264 };
            let hw = (*encoder == EncoderPref::Auto).then(|| caps.hw_encoder(family)).flatten();
            let q = quality.to_string();
            let mut args = Vec::new();
            let (name, pix_fmt): (String, &'static str) = match hw {
                Some(e) => {
                    args.extend(["-c:v".into(), e.name.clone()]);
                    match e.vendor {
                        Vendor::Nvidia => args.extend(
                            ["-preset", "p6", "-tune", "hq", "-rc", "vbr", "-cq", &q, "-b:v", "0", "-spatial-aq", "1"]
                                .map(String::from),
                        ),
                        Vendor::Amd => args.extend(
                            ["-quality", "quality", "-rc", "cqp", "-qp_i", &q, "-qp_p", &q, "-qp_b", &q]
                                .map(String::from),
                        ),
                        Vendor::Intel => args.extend(["-global_quality", &q, "-look_ahead", "1"].map(String::from)),
                        Vendor::Apple => {
                            let vt = (100 - (*quality as i32) * 2).clamp(1, 100).to_string();
                            args.extend(["-q:v", &vt, "-allow_sw", "1"].map(String::from));
                        }
                    }
                    let pix = match (ten, e.vendor) {
                        (true, _) => "p010le",
                        (false, Vendor::Intel | Vendor::Apple) => "nv12",
                        _ => "yuv420p",
                    };
                    (e.name.clone(), pix)
                }
                None if hevc => {
                    let n = sw("libx265")?;
                    args.extend(["-c:v", "libx265", "-preset", "medium", "-crf", &q].map(String::from));
                    (n, if ten { "yuv420p10le" } else { "yuv420p" })
                }
                None => {
                    let n = sw("libx264")?;
                    args.extend(
                        ["-c:v", "libx264", "-preset", "slow", "-crf", &q, "-profile:v", "high"].map(String::from),
                    );
                    (n, "yuv420p")
                }
            };
            gop_args(*gop, fps, hw.map(|e| e.vendor), &mut args);
            if hevc {
                // Apple/QuickTime-compatible HEVC sample entry.
                args.extend(["-tag:v".into(), "hvc1".into()]);
            }
            Encoding { encoder: name, args, pix_fmt }
        }
    })
}

/// Colour tags for the output: keep the source's when we keep its colour,
/// BT.709 when we tone map.
fn color_args(info: &MediaInfo, tonemapped: bool) -> Vec<String> {
    let v = &info.video;
    let pairs: [(&str, Option<String>); 4] = if tonemapped {
        [
            ("-color_primaries", Some("bt709".into())),
            ("-color_trc", Some("bt709".into())),
            ("-colorspace", Some("bt709".into())),
            ("-color_range", Some("tv".into())),
        ]
    } else {
        [
            ("-color_primaries", v.color_primaries.clone()),
            ("-color_trc", v.color_transfer.clone()),
            ("-colorspace", v.color_space.clone()),
            ("-color_range", v.color_range.clone()),
        ]
    };
    pairs.into_iter().filter_map(|(k, v)| v.map(|v| [k.to_string(), v])).flatten().collect()
}

fn source_timecode(index: &FrameIndex, f: u32) -> String {
    let fps = index.nominal_fps;
    let ntsc = fps.den == 1001;
    Timecode::from_frame(f as u64, Rational::new(fps.num, fps.den), ntsc).to_string()
}

pub fn export_clip(
    tools: &Tools,
    job: &ClipJob,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
) -> ExportResult<ClipOutcome> {
    let ClipJob { src: Source { path: src, info, index }, range, preset, .. } = job;
    if range.is_empty() || range.end > index.len() {
        return Err(ExportError::BadRange(format!("{range:?} outside 0..{}", index.len())));
    }
    if preset.container == Container::Mp4 && preset.audio == AudioCodec::Pcm24 {
        return Err(ExportError::BadPreset("uncompressed audio needs a MOV file".into()));
    }
    let n = range.len();
    let (s, e) = (range.start, range.end);
    let fps = index.nominal_fps.to_f64();
    let enc = resolve_encoding(&preset.video, tools.caps(), fps)?;

    let hdr_in = info.video.hdr != HdrKind::None;
    let tonemap = hdr_in && (preset.hdr == HdrMode::ToneMap || !preset.video.supports_hdr());

    let (trim_start, trim_end) = index.trim_us(*range);
    let mut vchain = vec![format!("trim=start={trim_start}us:end={trim_end}us"), "setpts=PTS-STARTPTS".to_string()];
    vchain.extend(display_chain(info, preset.max_height, tonemap));
    vchain.push(format!("format={}", enc.pix_fmt));
    let mut graph = format!("[0:{}]{}[v]", info.video.index, vchain.join(","));

    let audio = match preset.audio {
        AudioCodec::None => None,
        codec => job.audio_ordinal.or_else(|| info.default_audio().map(|a| a.ordinal)).map(|ord| (ord, codec)),
    };
    if let Some((ord, _)) = audio {
        let samples = ((index.t(e) - index.t(s)) * AUDIO_RATE as f64).round() as u64;
        // `first_pts=0` + `async` make aresample insert silence wherever the
        // source has no samples (late start, dropouts) instead of pulling
        // later sound forward.
        graph += &format!(
            ";[0:a:{ord}]atrim=start={ts:.9}:end={te:.9},asetpts=PTS-{ts:.9}/TB,aresample={AUDIO_RATE}:async=1:first_pts=0,apad=whole_len={samples},atrim=end_sample={samples}{downmix}[a]",
            ts = index.t(s),
            te = index.t(e),
            // Standard ITU matrix: centre and surrounds at -3 dB, LFE dropped.
            downmix = if preset.stereo { ",aformat=channel_layouts=stereo" } else { "" },
        );
    }
    let audio_language = audio
        .and_then(|(ord, _)| info.audio.iter().find(|a| a.ordinal == ord))
        .and_then(|a| a.language.as_deref())
        .and_then(as_media::lang::canonical);

    let ext = preset.container.ext();
    let out = with_suffix(&job.out_stem, &format!(".{ext}"));
    let part = with_suffix(&job.out_stem, &format!(".part.{ext}"));
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let stats = with_suffix(&job.out_stem, ".stats.txt");

    let build = |preroll: Option<f64>| -> Vec<String> {
        let mut a: Vec<String> = ["-v", "error", "-y", "-copyts"].map(String::from).to_vec();
        if let Some(pre) = preroll {
            let seek = (index.t(s) - index.t0() - pre).max(0.0);
            a.extend(["-noaccurate_seek".into(), "-ss".into(), format!("{seek:.6}")]);
        }
        a.extend(["-i".into(), src.display().to_string(), "-filter_complex".into(), graph.clone()]);
        a.extend(["-map".into(), "[v]".into()]);
        if audio.is_some() {
            a.extend(["-map".into(), "[a]".into()]);
        }
        // No `-frames:v`: it ends the mux as soon as video is done and clips the
        // interleaved audio tail. `trim` bounds the frames; `verify` proves it.
        a.extend(["-fps_mode".into(), "passthrough".into()]);
        a.extend(as_media::proxy::timing_args(index));
        a.extend(enc.args.iter().cloned());
        a.extend(color_args(info, tonemap));
        match audio.map(|(_, c)| c) {
            Some(AudioCodec::Pcm24) => a.extend(["-c:a", "pcm_s24le"].map(String::from)),
            Some(AudioCodec::Aac { kbps }) => {
                a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), format!("{kbps}k")])
            }
            _ => {}
        }
        a.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(String::from));
        if let Some(lang) = &audio_language {
            a.extend(["-metadata:s:a:0".into(), format!("language={lang}")]);
        }
        if preset.container == Container::Mov {
            a.extend(["-timecode".into(), source_timecode(index, s), "-movflags".into(), "+write_colr".into()]);
        } else {
            a.extend(["-movflags".into(), "+faststart".into()]);
        }
        if let Some(t) = &job.title {
            a.extend(["-metadata".into(), format!("title={t}")]);
        }
        a.extend([
            "-stats_enc_pre:v:0".into(),
            stats.display().to_string(),
            "-stats_enc_pre_fmt:v:0".into(),
            "{ptsi}".into(),
            "-f".into(),
            if preset.container == Container::Mov { "mov".into() } else { "mp4".into() },
            part.display().to_string(),
        ]);
        a
    };

    let mut last_err = String::new();
    for preroll in [Some(3.0), Some(10.0), None] {
        let _ = std::fs::remove_file(&stats);
        run_ffmpeg(tools, build(preroll), cancel, |p| progress((p.frame as f64 / n as f64).clamp(0.0, 1.0)))
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&part);
                let _ = std::fs::remove_file(&stats);
            })?;
        match verify(&stats, index, *range) {
            Ok(()) => {
                let _ = std::fs::remove_file(&stats);
                std::fs::rename(&part, &out)?;
                let bytes = std::fs::metadata(&out).map(|m| m.len() as f64).unwrap_or(0.0);
                return Ok(ClipOutcome {
                    path: out.display().to_string(),
                    frames: n,
                    verified: true,
                    encoder: enc.encoder,
                    bytes,
                });
            }
            Err(why) => {
                tracing::warn!(?preroll, %why, clip = %out.display(), "export verification failed; retrying");
                last_err = why;
            }
        }
    }
    // Keep the best effort but say so: the UI marks it "unverified".
    let _ = std::fs::remove_file(&stats);
    std::fs::rename(&part, &out)?;
    tracing::error!(clip = %out.display(), reason = %last_err, "clip could not be verified");
    let bytes = std::fs::metadata(&out).map(|m| m.len() as f64).unwrap_or(0.0);
    Ok(ClipOutcome { path: out.display().to_string(), frames: n, verified: false, encoder: enc.encoder, bytes })
}

/// `stem` + `suffix` as a path. Unlike `with_extension`, never cuts a stem
/// at its own dots ("Mr. Robot" + ".mov", not "Mr.mov").
fn with_suffix(stem: &Path, suffix: &str) -> PathBuf {
    let mut s = stem.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Every encoded frame must be source frame `s + i`, in order. Frames whose
/// decoder timestamp was missing or out of order got a repaired slot in the
/// index (one tick after the previous frame, see `as_media::index`); those are
/// checked by position instead of by value.
fn verify(stats: &Path, index: &FrameIndex, range: FrameRange) -> Result<(), String> {
    let text = std::fs::read_to_string(stats).map_err(|e| format!("no stats: {e}"))?;
    let got: Vec<Option<i64>> = text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().parse().ok()).collect();
    if got.len() != range.len() as usize {
        return Err(format!("{} frames encoded, expected {}", got.len(), range.len()));
    }
    let mut prev_raw: Option<i64> = None;
    for (i, &p) in got.iter().enumerate() {
        let f = range.start + i as u32;
        let want = index.pts(f);
        let repaired_slot = f > 0 && want == index.pts(f - 1) + 1;
        match p {
            Some(p) if p == want => {}
            Some(p) if repaired_slot && prev_raw.is_some_and(|prev| p <= prev) => {}
            None if repaired_slot => {}
            Some(p) => return Err(format!("frame {i}: source pts {p}, expected {want}")),
            None => return Err(format!("frame {i}: no source pts, expected {want}")),
        }
        if p.is_some() {
            prev_raw = p;
        }
    }
    Ok(())
}
