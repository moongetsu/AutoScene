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
//!
//! A clip can also be rendered as a *part* of one file made of many clips
//! ([`Part`], see [`crate::merge`]): the same cut, in a picture, colour,
//! sound and time base every part shares, so the parts join without being
//! encoded again.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use as_core::paths::name;
use as_core::{CancelToken, FrameIndex, FrameRange, Rational, Timecode};
use as_media::filters::{deinterlace, picture_chain};
use as_media::{Caps, CodecFamily, HdrKind, MediaInfo, Source, Tools, Vendor, run_ffmpeg};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::lossless::{self, LosslessCodec, Reference};
use crate::merge::{LosslessShape, MergeShape, natural_size};
use crate::preset::{
    AudioCodec, Container, DnxProfile, EncoderPref, ExportPreset, Gop, HdrMode, ProResProfile, VideoCodec,
};
use crate::{ExportError, ExportResult};

pub(crate) const AUDIO_RATE: u32 = 48_000;

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
    /// Render it as a part of one file made of many clips (always a MOV).
    pub part: Option<Part<'a>>,
}

/// How a clip is rendered as a part of a merged file.
#[derive(Clone, Copy, Debug)]
pub struct Part<'a> {
    pub shape: &'a MergeShape,
    /// Audio samples the part carries (see [`crate::merge::PartTiming`]).
    pub samples: u64,
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
#[derive(Clone, Debug)]
pub(crate) struct Encoding {
    pub(crate) encoder: String,
    pub(crate) args: Vec<String>,
    pub(crate) pix_fmt: String,
}

/// NVENC's constant quality for a preset's CRF-like `quality`. The scales
/// differ, so NVENC gets a higher number: the one that looks like x264 (and
/// x265) on film grain, where the difference shows. Measured on 1080p
/// Breaking Bad (VMAF, PSNR): x264 CRF 14 all-intra ≈ CQ 21–23, CRF 16 with a
/// keyframe a second ≈ CQ 17–19, x265 CRF 18 ≈ CQ 18–20, the same steps
/// apart from CRF 10 to 22. A clean picture then looks a little better than
/// x264's and takes more (there CQ +10 and +6 would match).
pub(crate) fn nvenc_cq(quality: u8, gop: Gop) -> u8 {
    let offset = if gop == Gop::AllIntra { 7 } else { 1 };
    quality.saturating_add(offset).min(51)
}

/// The most bits a second NVENC may write of `w`×`h` pictures at `fps`.
/// Without a cap of its own it keeps to the highest rate of the lowest
/// H.264/HEVC level that fits the picture (1080p24: 25 Mbit/s H.264, 12
/// HEVC), whatever the quality asked for; with one it picks the level that
/// allows it (1080p24: H.264 level 4.1 with a GOP and 5 all-intra, HEVC 5
/// High tier). Three bits a pixel all-intra and 1.5 with a GOP are well
/// above what the presets take (film grain at 1080p: 1.2 and 0.6), and
/// level 5.2's rate the most: above it NVENC picks levels made for 8K.
pub(crate) fn nvenc_maxrate(gop: Gop, hevc: bool, (w, h): (u32, u32), fps: f64) -> f64 {
    let bpp = if gop == Gop::AllIntra { 3.0 } else { 1.5 };
    let top = if hevc { 240e6 } else { 300e6 };
    (bpp * f64::from(w) * f64::from(h) * fps).clamp(1e6, top)
}

/// The frame rate ffmpeg tells the encoder of an episode at `fps`: the
/// stream's own when it is higher (a field rate, a variable-rate file), so a
/// cap per second leaves every frame its share.
pub(crate) fn encoder_fps(info: &MediaInfo, fps: f64) -> f64 {
    fps.max(info.video.r_frame_rate.to_f64())
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

/// x265 dates its first frames back by the delay its B-frames add, which it
/// works out from the third frame and before that reads from a field it
/// never sets: a clip of one or two frames gets a decode time off by
/// whatever that memory held (macOS: often), which a MOV refuses ("pts/dts
/// pair unsupported") or keeps wrong. So few frames have nothing to reorder.
pub(crate) fn x265_in_order_when_short(encoder: &str, frames: u32, args: &mut Vec<String>) {
    if encoder == "libx265" && frames < 3 && !args.iter().any(|a| a == "-bf") {
        args.extend(["-bf".into(), "0".into()]);
    }
}

/// Which encoder and arguments `codec` resolves to on a machine with `caps`,
/// for `size` pictures with keyframes spaced by `fps`. `fastest` is the
/// highest frame rate an encoder of them is told ([`encoder_fps`]): NVENC's
/// cap follows it.
pub(crate) fn resolve_encoding(
    codec: &VideoCodec,
    caps: &Caps,
    fps: f64,
    size: (u32, u32),
    fastest: f64,
) -> ExportResult<Encoding> {
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
                pix_fmt: pix.into(),
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
                pix_fmt: pix.into(),
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
                        Vendor::Nvidia => {
                            let cq = nvenc_cq(*quality, *gop).to_string();
                            let kbps = nvenc_maxrate(*gop, hevc, size, fastest) / 1000.0;
                            let (max, buf) = (format!("{kbps:.0}k"), format!("{:.0}k", 2.0 * kbps));
                            args.extend(
                                ["-preset", "p6", "-tune", "hq", "-rc", "vbr", "-cq", &cq, "-b:v", "0"]
                                    .map(String::from),
                            );
                            args.extend(["-maxrate", &max, "-bufsize", &buf, "-spatial-aq", "1"].map(String::from));
                        }
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
            Encoding { encoder: name, args, pix_fmt: pix_fmt.into() }
        }
        // Chosen per source, by what it decodes to (`crate::lossless::keeper`).
        VideoCodec::Lossless => return Err(ExportError::BadPreset("lossless encoding follows each source".into())),
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

/// The ffmpeg recipe of a clip: what is filtered and encoded how.
struct Recipe {
    encoder: String,
    /// Options for the source (before `-i`).
    input: Vec<String>,
    graph: String,
    /// The sound's own filter graph (Bit-Perfect): a graph fed by the
    /// picture alone hands the picture's rotation on to the file.
    audio_graph: Option<String>,
    audio: bool,
    /// Everything between the maps and the stats file: timing, codecs, tags, metadata.
    args: Vec<String>,
    format: &'static str,
    ext: &'static str,
    /// Bit-Perfect: the graph also hands the frames (`[vref]`) and sound
    /// (`[aref]`) to hashes, and the file is checked against them.
    reference: Option<Reference>,
}

/// The sound's language tag, when the track has a known one.
fn audio_language(info: &MediaInfo, ordinal: Option<u32>) -> Option<String> {
    ordinal
        .and_then(|ord| info.audio.iter().find(|a| a.ordinal == ord))
        .and_then(|a| a.language.as_deref())
        .and_then(as_media::lang::canonical)
}

/// A Bit-Perfect clip as a file of its own: the source's frames and samples
/// as they are, in the codec that keeps them (see [`crate::lossless`]).
fn lossless_recipe(tools: &Tools, job: &ClipJob) -> ExportResult<Recipe> {
    let ClipJob { src: Source { path, info, index }, range, .. } = job;
    let exact = lossless::exact_format(tools, path, info.video.index)?;
    let v = &info.video;
    let keeper = lossless::keeper(tools.caps(), &exact, &v.codec, (v.width, v.height))?;
    let (s, e) = (range.start, range.end);
    let (trim_start, trim_end) = index.trim_us(*range);
    let cut = [format!("trim=start={trim_start}us:end={trim_end}us"), "setpts=PTS-STARTPTS".to_string()];
    let graph = lossless::picture_graph(v.index, &cut, &keeper.tag, keeper.repack.as_deref());
    let ordinal = job.audio_ordinal.or_else(|| info.default_audio().map(|a| a.ordinal));
    let audio = match ordinal {
        Some(ord) => {
            let a = exact
                .audio
                .iter()
                .find(|a| a.ordinal == ord)
                .ok_or_else(|| ExportError::Inexact(format!("its sound track {} can't be read", ord + 1)))?;
            Some((ord, a.sample_rate.max(1), lossless::pcm_codec(a)?))
        }
        None => None,
    };
    // As a clip of any preset, at the source's own rate: gaps in the source's
    // sound become silence, every sample there stays as it is.
    let audio_graph = audio.map(|(ord, rate, _)| {
        let samples = ((index.t(e) - index.t(s)) * f64::from(rate)).round() as u64;
        format!(
            "[0:a:{ord}]atrim=start={ts:.9}:end={te:.9},asetpts=PTS-{ts:.9}/TB,aresample={rate}:{fill}:first_pts=0,apad=whole_len={samples},atrim=end_sample={samples},asplit[a][aref]",
            fill = as_media::proxy::AUDIO_GAP_FILL,
            ts = index.t(s),
            te = index.t(e),
        )
    });
    let mut a: Vec<String> = vec!["-fps_mode".into(), "passthrough".into()];
    a.extend(as_media::proxy::timing_args(index));
    a.extend(keeper.encoding.args.iter().cloned());
    if let Some((_, _, codec)) = audio {
        a.extend(["-c:a".into(), codec.into()]);
    }
    a.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(String::from));
    if let Some(lang) = audio_language(info, ordinal).filter(|_| audio.is_some()) {
        a.extend(["-metadata:s:a:0".into(), format!("language={lang}")]);
    }
    if keeper.codec == LosslessCodec::Hevc {
        a.extend(["-timecode".into(), source_timecode(index, s), "-movflags".into(), "+write_colr".into()]);
    }
    if let Some(t) = &job.title {
        a.extend(["-metadata".into(), format!("title={t}")]);
    }
    Ok(Recipe {
        encoder: keeper.encoding.encoder,
        // The picture as stored: a rotation stays a tag of the file.
        input: vec!["-noautorotate".into()],
        graph,
        audio_graph,
        audio: audio.is_some(),
        args: a,
        format: keeper.codec.format(),
        ext: keeper.codec.ext(),
        reference: Some(Reference { audio: audio.map(|(.., codec)| codec), hdr: exact.hdr_metadata }),
    })
}

/// A clip as a file of its own, in the preset's container.
fn clip_recipe(tools: &Tools, job: &ClipJob) -> ExportResult<Recipe> {
    if job.preset.video == VideoCodec::Lossless {
        return lossless_recipe(tools, job);
    }
    let ClipJob { src: Source { info, index, .. }, range, preset, .. } = job;
    let (s, e) = (range.start, range.end);
    let fps = index.nominal_fps.to_f64();
    let size = natural_size(info, preset.max_height);
    let enc = resolve_encoding(&preset.video, tools.caps(), fps, size, encoder_fps(info, fps))?;

    let hdr_in = info.video.hdr != HdrKind::None;
    let tonemap = hdr_in && (preset.hdr == HdrMode::ToneMap || !preset.video.supports_hdr());

    let (trim_start, trim_end) = index.trim_us(*range);
    // Deinterlace before the trim: the clip's first and last frames are made
    // from the fields around them, like every other frame (a one-frame clip
    // would otherwise come out as two fields woven together).
    let mut vchain: Vec<String> = deinterlace(info).into_iter().collect();
    vchain.extend([format!("trim=start={trim_start}us:end={trim_end}us"), "setpts=PTS-STARTPTS".to_string()]);
    vchain.extend(picture_chain(info, preset.max_height, tonemap));
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
        // later sound forward (see `as_media::proxy::AUDIO_GAP_FILL`).
        graph += &format!(
            ";[0:a:{ord}]atrim=start={ts:.9}:end={te:.9},asetpts=PTS-{ts:.9}/TB,aresample={AUDIO_RATE}:{fill}:first_pts=0,apad=whole_len={samples},atrim=end_sample={samples}{downmix}[a]",
            fill = as_media::proxy::AUDIO_GAP_FILL,
            ts = index.t(s),
            te = index.t(e),
            // Standard ITU matrix: centre and surrounds at -3 dB, LFE dropped.
            downmix = if preset.stereo { ",aformat=channel_layouts=stereo" } else { "" },
        );
    }
    let audio_language = audio_language(info, audio.map(|(ord, _)| ord));

    // No `-frames:v`: it ends the mux as soon as video is done and clips the
    // interleaved audio tail. `trim` bounds the frames; `verify` proves it.
    let mut a: Vec<String> = vec!["-fps_mode".into(), "passthrough".into()];
    a.extend(as_media::proxy::timing_args(index));
    a.extend(enc.args.iter().cloned());
    a.extend(color_args(info, tonemap));
    match audio.map(|(_, c)| c) {
        Some(AudioCodec::Pcm24) => a.extend(["-c:a", "pcm_s24le"].map(String::from)),
        Some(AudioCodec::Aac { kbps }) => a.extend(["-c:a".into(), "aac".into(), "-b:a".into(), format!("{kbps}k")]),
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
    Ok(Recipe {
        encoder: enc.encoder,
        input: Vec::new(),
        graph,
        audio_graph: None,
        audio: audio.is_some(),
        args: a,
        format: if preset.container == Container::Mov { "mov" } else { "mp4" },
        ext: preset.container.ext(),
        reference: None,
    })
}

/// A Bit-Perfect clip as a part of a merged file: its frames and samples as
/// they are (every part's are alike, see `crate::merge::plan_merge`), on the
/// file's time base, silence when its episode has no sound.
fn lossless_part_recipe(job: &ClipJob, part: Part, keep: &LosslessShape) -> Recipe {
    let ClipJob { src: Source { info, index, .. }, range, .. } = job;
    let Part { shape, samples } = part;
    let (s, e) = (range.start, range.end);
    let (trim_start, trim_end) = index.trim_us(*range);
    let mut vchain = vec![format!("trim=start={trim_start}us:end={trim_end}us"), "setpts=PTS-STARTPTS".to_string()];
    if !index.is_vfr() {
        let g = crate::merge::grid(index);
        vchain.push(format!("settb={}/{}", g.num, g.den));
    }
    let graph = lossless::picture_graph(info.video.index, &vchain, &keep.tag, keep.repack.as_deref());
    let audio_graph = match (&shape.audio, keep.audio) {
        (Some(layout), Some((_, rate))) => Some(match crate::merge::audio_stream(info, job.audio_ordinal) {
            Some(stream) => format!(
                "[0:a:{ord}]atrim=start={ts:.9}:end={te:.9},asetpts=PTS-{ts:.9}/TB,aresample={rate}:{fill}:first_pts=0,apad=whole_len={samples},atrim=end_sample={samples},asplit[a][aref]",
                ord = stream.ordinal,
                fill = as_media::proxy::AUDIO_GAP_FILL,
                ts = index.t(s),
                te = index.t(e),
            ),
            None => format!("anullsrc=r={rate}:cl={layout},atrim=end_sample={samples},asplit[a][aref]"),
        }),
        _ => None,
    };
    let mut a: Vec<String> = vec!["-fps_mode".into(), "passthrough".into()];
    a.extend(["-enc_time_base:v".into(), format!("1/{}", shape.timescale)]);
    a.extend(shape.encoding.args.iter().cloned());
    if let Some((codec, _)) = keep.audio.filter(|_| shape.audio.is_some()) {
        a.extend(["-c:a".into(), codec.into()]);
    }
    a.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(String::from));
    if keep.codec == LosslessCodec::Hevc {
        a.extend(["-video_track_timescale".into(), shape.timescale.to_string()]);
        a.extend(["-movflags", "+write_colr"].map(String::from));
    }
    let audio = shape.audio.is_some() && keep.audio.is_some();
    Recipe {
        encoder: shape.encoding.encoder.clone(),
        input: vec!["-noautorotate".into()],
        graph,
        audio_graph,
        audio,
        args: a,
        format: keep.codec.format(),
        ext: keep.codec.ext(),
        reference: Some(Reference {
            audio: keep.audio.map(|(codec, _)| codec).filter(|_| audio),
            hdr: keep.hdr.clone(),
        }),
    }
}

/// A clip as a part of a merged file: in the shape every part shares, with
/// PCM sound (silence when its episode has none), no timecode and no names.
fn part_recipe(job: &ClipJob, part: Part) -> Recipe {
    if let Some(keep) = &part.shape.lossless {
        return lossless_part_recipe(job, part, keep);
    }
    let ClipJob { src: Source { info, index, .. }, range, .. } = job;
    let Part { shape, samples } = part;
    let (s, e) = (range.start, range.end);
    let (trim_start, trim_end) = index.trim_us(*range);
    let mut vchain: Vec<String> = deinterlace(info).into_iter().collect();
    vchain.extend([format!("trim=start={trim_start}us:end={trim_end}us"), "setpts=PTS-STARTPTS".to_string()]);
    vchain.extend(shape.picture(info, index));
    let mut graph = format!("[0:{}]{}[v]", info.video.index, vchain.join(","));
    if let Some(layout) = &shape.audio {
        match crate::merge::audio_stream(info, job.audio_ordinal) {
            Some(stream) => {
                graph += &format!(
                    ";[0:a:{ord}]atrim=start={ts:.9}:end={te:.9},asetpts=PTS-{ts:.9}/TB,aresample={AUDIO_RATE}:{fill}:first_pts=0{upmix},aformat=channel_layouts={layout},apad=whole_len={samples},atrim=end_sample={samples}[a]",
                    ord = stream.ordinal,
                    fill = as_media::proxy::AUDIO_GAP_FILL,
                    ts = index.t(s),
                    te = index.t(e),
                    // Mono at full level on both sides, as loud as the stereo
                    // episodes next to it (a plain upmix is 3 dB quieter).
                    upmix = if stream.channels == 1 && layout != "mono" { ",pan=stereo|c0=c0|c1=c0" } else { "" },
                )
            }
            // An episode without sound is silent in the file, for exactly as long.
            None => graph += &format!(";anullsrc=r={AUDIO_RATE}:cl={layout},atrim=end_sample={samples}[a]"),
        }
    }
    let mut a: Vec<String> = vec!["-fps_mode".into(), "passthrough".into()];
    // Every part's frames on whole ticks of one time base: the join needs it.
    a.extend(["-enc_time_base:v".into(), format!("1/{}", shape.timescale)]);
    a.extend(shape.encoding.args.iter().cloned());
    a.extend(shape.color_args());
    if shape.audio.is_some() {
        a.extend(["-c:a", "pcm_s24le"].map(String::from));
    }
    a.extend(["-map_metadata", "-1", "-map_chapters", "-1"].map(String::from));
    a.extend(["-video_track_timescale".into(), shape.timescale.to_string()]);
    a.extend(["-movflags", "+write_colr"].map(String::from));
    Recipe {
        encoder: shape.encoding.encoder.clone(),
        input: Vec::new(),
        graph,
        audio_graph: None,
        audio: shape.audio.is_some(),
        args: a,
        format: "mov",
        ext: "mov",
        reference: None,
    }
}

pub fn export_clip(
    tools: &Tools,
    job: &ClipJob,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
) -> ExportResult<ClipOutcome> {
    let ClipJob { src: Source { path: src, index, .. }, range, preset, .. } = job;
    if range.is_empty() || range.end > index.len() {
        return Err(ExportError::BadRange(format!("{range:?} outside 0..{}", index.len())));
    }
    if job.part.is_none() && preset.container == Container::Mp4 && preset.audio == AudioCodec::Pcm24 {
        return Err(ExportError::BadPreset("uncompressed audio needs a MOV file".into()));
    }
    let n = range.len();
    let s = range.start;
    let mut recipe = match job.part {
        Some(part) => part_recipe(job, part),
        None => clip_recipe(tools, job)?,
    };
    // A part's encoding is its file's: the plan already took its shortest clip.
    x265_in_order_when_short(&recipe.encoder, n, &mut recipe.args);

    let out = with_suffix(&job.out_stem, &format!(".{}", recipe.ext));
    // Work files live in a hidden folder next to the clips, so no clip name
    // (not even "X.part") can collide with another clip's unfinished file.
    let out_dir = out.parent().map(Path::to_path_buf).unwrap_or_default();
    let work = out_dir.join(PARTIAL_DIR);
    std::fs::create_dir_all(&work)?;
    let stem_name = job.out_stem.file_name().map(PathBuf::from).unwrap_or_default();
    let part = with_suffix(&work.join(&stem_name), &format!(".{}", recipe.ext));
    let stats = with_suffix(&work.join(&stem_name), ".stats.txt");

    let (ref_frames, ref_sound) = lossless::reference_paths(&part);
    // A clip of its own never replaces a file that came there meanwhile: it
    // takes a free name (`_2`…), as a joined file does. A part is named by
    // its export.
    let place = |part: &Path| -> ExportResult<PathBuf> {
        let to = if job.part.is_none() { crate::merge::free_path(&job.out_stem, recipe.ext) } else { out.clone() };
        std::fs::rename(part, &to)?;
        Ok(to)
    };
    let checked = with_suffix(&part, ".frames.txt");
    let checked_sound = lossless::sound_hashes(&checked);
    // Paths go to ffmpeg as they are: a file name needn't be valid Unicode.
    let build = |preroll: Option<f64>| -> Vec<OsString> {
        let mut a: Vec<OsString> = ["-v", "error", "-y", "-copyts"].map(OsString::from).to_vec();
        a.extend(recipe.input.iter().map(OsString::from));
        if let Some(pre) = preroll {
            let seek = (index.t(s) - index.t0() - pre).max(0.0);
            a.extend(["-noaccurate_seek".into(), "-ss".into(), format!("{seek:.6}").into()]);
        }
        a.extend(["-i".into(), src.as_os_str().into(), "-filter_complex".into(), (&recipe.graph).into()]);
        if let Some(g) = &recipe.audio_graph {
            a.extend(["-filter_complex".into(), g.into()]);
        }
        a.extend(["-map", "[v]"].map(OsString::from));
        if recipe.audio {
            a.extend(["-map", "[a]"].map(OsString::from));
        }
        a.extend(recipe.args.iter().map(OsString::from));
        a.extend([
            "-stats_enc_pre:v:0".into(),
            stats.as_os_str().into(),
            "-stats_enc_pre_fmt:v:0".into(),
            "{ptsi}".into(),
            "-f".into(),
            recipe.format.into(),
            part.as_os_str().into(),
        ]);
        // What the encoder got, hashed as it goes: the frames, and the sound
        // as the file stores it.
        if let Some(reference) = &recipe.reference {
            a.extend(["-map", "[vref]", "-fps_mode", "passthrough", "-f", "framemd5"].map(OsString::from));
            a.push(ref_frames.as_os_str().into());
            if let Some(codec) = reference.audio {
                a.extend(["-map", "[aref]", "-c:a", codec, "-f", "md5"].map(OsString::from));
                a.push(ref_sound.as_os_str().into());
            }
        }
        a
    };

    // Best effort: other exports may still be using the folder.
    let tidy = || {
        let _ = std::fs::remove_dir(&work);
    };
    let scratch = || {
        for f in [&stats, &ref_frames, &ref_sound, &checked, &checked_sound] {
            let _ = std::fs::remove_file(f);
        }
    };
    // A Bit-Perfect clip is checked frame by frame after it is made: a fifth
    // of the way.
    let encoding = if recipe.reference.is_some() { 0.8 } else { 1.0 };
    let mut last_err = String::new();
    // A retry starts its frames again; what was shown holds until it passes.
    let mut shown = 0.0f64;
    for preroll in [Some(3.0), Some(10.0), None] {
        scratch();
        run_ffmpeg(tools, build(preroll), cancel, |p| {
            shown = shown.max(encoding * (p.frame as f64 / n as f64).clamp(0.0, 1.0));
            progress(shown)
        })
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&part);
            scratch();
            tidy();
        })?;
        if let Err(why) = verify(&stats, index, *range) {
            tracing::warn!(?preroll, %why, clip = %name(&out), "export verification failed; retrying");
            last_err = why;
            continue;
        }
        if let Some(reference) = &recipe.reference {
            let check = lossless::check_file(tools, &part, reference, &checked, n, cancel, |p| {
                shown = shown.max(encoding + (1.0 - encoding) * p);
                progress(shown)
            });
            match check {
                Ok(()) => {}
                // The hashes are of the very frames the encoder got: another
                // seek can't change what it made of them.
                Err(Ok(why)) => {
                    tracing::warn!(?preroll, %why, clip = %name(&out), "the clip isn't the source exactly");
                    last_err = why;
                    break;
                }
                Err(Err(e)) => {
                    let _ = std::fs::remove_file(&part);
                    scratch();
                    tidy();
                    return Err(e);
                }
            }
        }
        let out = place(&part)?;
        // A part's frames, as checked, for the check of the file it joins.
        if recipe.reference.is_some() && job.part.is_some() {
            std::fs::rename(&checked, lossless::checked_frames(&out))?;
        }
        scratch();
        tidy();
        let bytes = std::fs::metadata(&out).map(|m| m.len() as f64).unwrap_or(0.0);
        return Ok(ClipOutcome {
            path: out.display().to_string(),
            frames: n,
            verified: true,
            encoder: recipe.encoder,
            bytes,
        });
    }
    scratch();
    if recipe.reference.is_some() {
        // Bit-Perfect promises the source: nothing else is kept.
        let _ = std::fs::remove_file(&part);
        tidy();
        tracing::error!(clip = %name(&out), reason = %last_err, "clip isn't the source exactly");
        return Err(ExportError::Inexact(format!("it didn't come out exactly as the source ({last_err})")));
    }
    // Keep the best effort but say so: the UI marks it "unverified".
    let out = place(&part)?;
    tidy();
    tracing::error!(clip = %name(&out), reason = %last_err, "clip could not be verified");
    let bytes = std::fs::metadata(&out).map(|m| m.len() as f64).unwrap_or(0.0);
    Ok(ClipOutcome { path: out.display().to_string(), frames: n, verified: false, encoder: recipe.encoder, bytes })
}

/// Hidden folder (inside the export folder) for unfinished clips.
pub const PARTIAL_DIR: &str = ".autoscene-partial";

/// `stem` + `suffix` as a path. Unlike `with_extension`, never cuts a stem
/// at its own dots ("Mr. Robot" + ".mov", not "Mr.mov").
pub(crate) fn with_suffix(stem: &Path, suffix: &str) -> PathBuf {
    let mut s = stem.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Every encoded frame must be source frame `s + i`, in order. Frames whose
/// decoder timestamp was missing or out of order got a repaired slot in the
/// index (one tick after the previous frame, see `as_media::index`); those are
/// checked by position instead of by value. A one-tick step only marks a
/// repair when a frame normally lasts several ticks: in 1/fps time bases
/// (AVI) every step is one tick, and guessing would wave through a missing
/// frame followed by a duplicate.
fn verify(stats: &Path, index: &FrameIndex, range: FrameRange) -> Result<(), String> {
    let ticks_per_frame = index.tb.den as f64 / (index.tb.num as f64 * index.nominal_fps.to_f64().max(1e-6));
    let text = std::fs::read_to_string(stats).map_err(|e| format!("no stats: {e}"))?;
    let got: Vec<Option<i64>> = text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim().parse().ok()).collect();
    if got.len() != range.len() as usize {
        return Err(format!("{} frames encoded, expected {}", got.len(), range.len()));
    }
    let mut prev_raw: Option<i64> = None;
    for (i, &p) in got.iter().enumerate() {
        let f = range.start + i as u32;
        let want = index.pts(f);
        let repaired_slot = ticks_per_frame >= 2.0 && f > 0 && want == index.pts(f - 1) + 1;
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

#[cfg(test)]
mod tests {
    use as_media::HwEncoder;

    use super::*;
    use crate::builtin_presets;

    const FILM: f64 = 24000.0 / 1001.0;

    /// A machine with an NVIDIA card (the encoders a smoke test finds there).
    fn nvidia() -> Caps {
        let hw = |name: &str, family| HwEncoder { name: name.into(), family, vendor: Vendor::Nvidia };
        Caps {
            hw_encoders: vec![hw("h264_nvenc", CodecFamily::H264), hw("hevc_nvenc", CodecFamily::Hevc)],
            sw_encoders: vec!["libx264".into(), "libx265".into()],
            ..Caps::default()
        }
    }

    fn video(id: &str) -> VideoCodec {
        builtin_presets().into_iter().find(|p| p.id == id).unwrap_or_else(|| panic!("preset {id}")).video
    }

    fn args(id: &str, size: (u32, u32), fps: f64) -> String {
        resolve_encoding(&video(id), &nvidia(), fps, size, fps).unwrap().args.join(" ")
    }

    #[test]
    fn nvenc_gets_its_own_quality_and_a_cap_above_it() {
        // 1080p23.976: CQ 14 + 7 all-intra, + 1 with a GOP; at most 3 and
        // 1.5 bits a pixel, twice that buffered.
        let hd = (1920, 1080);
        assert_eq!(
            args("h264-intra", hd, FILM),
            "-c:v h264_nvenc -preset p6 -tune hq -rc vbr -cq 21 -b:v 0 -maxrate 149150k -bufsize 298300k \
             -spatial-aq 1 -bf 0 -force_key_frames expr:1 -forced-idr 1"
        );
        assert_eq!(
            args("h264-edit", hd, FILM),
            "-c:v h264_nvenc -preset p6 -tune hq -rc vbr -cq 17 -b:v 0 -maxrate 74575k -bufsize 149150k \
             -spatial-aq 1 -g 24 -bf 0"
        );
        assert_eq!(
            args("h264-mobile", hd, FILM),
            "-c:v h264_nvenc -preset p6 -tune hq -rc vbr -cq 19 -b:v 0 -maxrate 74575k -bufsize 149150k \
             -spatial-aq 1 -g 24 -bf 0"
        );
        assert_eq!(
            args("hevc10", hd, FILM),
            "-c:v hevc_nvenc -preset p6 -tune hq -rc vbr -cq 19 -b:v 0 -maxrate 74575k -bufsize 149150k \
             -spatial-aq 1 -g 24 -bf 0 -tag:v hvc1"
        );
        // The cap follows the picture and frame rate, from 1 Mbit/s up to
        // level 5.2's rate.
        assert!(args("h264-edit", (1280, 720), 25.0).contains("-maxrate 34560k -bufsize 69120k"));
        assert!(args("h264-edit", (160, 96), 24.0).contains("-maxrate 1000k -bufsize 2000k"));
        assert!(args("h264-intra", (3840, 2160), 60.0).contains("-maxrate 300000k -bufsize 600000k"));
        assert!(args("hevc10", (3840, 2160), 60.0).contains("-maxrate 240000k -bufsize 480000k"));
    }

    #[test]
    fn the_cap_follows_the_rate_the_encoder_is_told() {
        let mut info: MediaInfo = serde_json::from_value(serde_json::json!({
            "container": "mpegts", "durationSec": null, "startSec": 0.0, "sizeBytes": null, "bitRate": null,
            "video": {
                "index": 0, "codec": "h264", "profile": null, "pixFmt": "yuv420p", "bitDepth": 8,
                "width": 1920, "height": 1080, "sar": Rational::new(1, 1), "rFrameRate": Rational::new(50, 1),
                "avgFrameRate": null, "timeBase": Rational::new(1, 90_000), "colorPrimaries": null,
                "colorTransfer": null, "colorSpace": null, "colorRange": null, "fieldOrder": "tt",
                "rotation": 0, "hdr": "none"
            },
            "audio": [], "subtitles": []
        }))
        .unwrap();
        // An interlaced stream at its field rate: ffmpeg tells the encoder 50.
        assert_eq!(encoder_fps(&info, 25.0), 50.0);
        info.video.r_frame_rate = Rational::new(25, 1);
        assert_eq!(encoder_fps(&info, 25.0), 25.0);
        // A stream without a rate of its own keeps its episode's.
        info.video.r_frame_rate = Rational::new(0, 1);
        assert_eq!(encoder_fps(&info, 25.0), 25.0);
    }

    #[test]
    fn nvenc_quality_stays_on_its_scale() {
        assert_eq!(nvenc_cq(0, Gop::AllIntra), 7);
        assert_eq!(nvenc_cq(18, Gop::Delivery), 19);
        assert_eq!(nvenc_cq(50, Gop::AllIntra), 51);
        assert_eq!(nvenc_cq(u8::MAX, Gop::Editing), 51);
    }

    #[test]
    fn the_software_encoders_keep_their_crf() {
        let mut v = video("h264-intra");
        if let VideoCodec::H264 { encoder, .. } = &mut v {
            *encoder = EncoderPref::Software;
        }
        let x264 = resolve_encoding(&v, &nvidia(), FILM, (1920, 1080), FILM).unwrap().args.join(" ");
        assert_eq!(x264, "-c:v libx264 -preset slow -crf 14 -profile:v high -g 1 -bf 0");
    }

    #[test]
    fn x265_reorders_no_clip_of_one_or_two_frames() {
        let with = |encoder: &str, frames, args: &[&str]| {
            let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            x265_in_order_when_short(encoder, frames, &mut a);
            a.join(" ")
        };
        assert_eq!(with("libx265", 1, &["-c:v", "libx265"]), "-c:v libx265 -bf 0");
        assert_eq!(with("libx265", 2, &["-c:v", "libx265"]), "-c:v libx265 -bf 0");
        // From the third frame on x265 times its B-frames right.
        assert_eq!(with("libx265", 3, &["-c:v", "libx265"]), "-c:v libx265");
        // A GOP that already has none stays as it is; other encoders aren't touched.
        assert_eq!(with("libx265", 1, &["-g", "1", "-bf", "0"]), "-g 1 -bf 0");
        assert_eq!(with("libx264", 1, &["-c:v", "libx264"]), "-c:v libx264");
        assert_eq!(with("hevc_nvenc", 1, &["-c:v", "hevc_nvenc"]), "-c:v hevc_nvenc");
    }
}
