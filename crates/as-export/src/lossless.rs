//! Bit-Perfect: clips whose every frame decodes to exactly the pixels the
//! source's own frames decode to, with the source's own sound samples.
//!
//! Nothing between decoder and encoder may touch a frame: no deinterlacing,
//! no square pixels, no height cap, no tone mapping, no other pixel format,
//! no turning upright (the rotation stays a tag). The encoder is chosen by
//! the format the source decodes to, so it stores exactly that format:
//! - HEVC without loss, in a MOV: Main or Main 10 for 4:2:0 pictures, the
//!   range extensions for 4:2:2, 4:4:4 and 12 bits. After Effects shows 8-
//!   and 10-bit ones exactly as it shows the source (October 2026, AE 25.5);
//!   any editor that reads HEVC reads it. NVENC makes it where it can hold
//!   the picture (5 to 8 times as fast as x265), x265 otherwise. Interlaced
//!   pictures keep their lines as they are, their field order a tag.
//! - FFV1 (in an MKV) for what HEVC can't hold as it is: 4:2:0 or 4:2:2
//!   pictures of an odd size, 4:1:1, alpha, more than 12 bits. DaVinci
//!   Resolve and VLC read it; After Effects doesn't.
//!
//! A source that doesn't say what colour it is gets the tags editors assume
//! for it (see [`colour`]): the pixels stay, and the copy looks like the
//! source.
//!
//! Sound keeps its rate, channels and sample format, as PCM: a decoder that
//! gives floating-point samples (AAC, AC-3, Opus) gets 32-bit float PCM, the
//! only kind that holds them exactly.
//!
//! That a clip came out exact is checked, not assumed: the frames and sound
//! handed to the encoder are hashed while it encodes them, the finished file
//! is decoded and hashed again, and every frame and sample must match
//! ([`check_file`]). "Exact" is as FFmpeg decodes the source: for H.264,
//! HEVC and AV1 every decoder gives the same pixels, for older codecs
//! (MPEG-4 Part 2, ProRes) decoders may differ in the last bit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use as_core::CancelToken;
use as_media::{Caps, CodecFamily, ExactAudio, ExactFormat, Tools, Vendor, run_ffmpeg};

use crate::clip::Encoding;
use crate::{ExportError, ExportResult};

/// Pixel formats x265 stores without loss (this build's list, less alpha).
const HEVC_FORMATS: &[&str] = &[
    "yuv420p",
    "yuvj420p",
    "yuv422p",
    "yuvj422p",
    "yuv444p",
    "yuvj444p",
    "gbrp",
    "yuv420p10le",
    "yuv422p10le",
    "yuv444p10le",
    "gbrp10le",
    "yuv420p12le",
    "yuv422p12le",
    "yuv444p12le",
    "gbrp12le",
    "gray",
    "gray10le",
    "gray12le",
];

/// Pixel formats FFV1 stores without loss (this build's list).
const FFV1_FORMATS: &[&str] = &[
    "yuv420p",
    "yuva420p",
    "yuva422p",
    "yuv444p",
    "yuva444p",
    "yuv440p",
    "yuv422p",
    "yuv411p",
    "yuv410p",
    "bgr0",
    "bgra",
    "yuv420p16le",
    "yuv422p16le",
    "yuv444p16le",
    "yuv444p9le",
    "yuv422p9le",
    "yuv420p9le",
    "yuv420p10le",
    "yuv422p10le",
    "yuv444p10le",
    "yuv420p12le",
    "yuv422p12le",
    "yuv444p12le",
    "yuva444p16le",
    "yuva422p16le",
    "yuva420p16le",
    "yuva444p12le",
    "yuva422p12le",
    "yuva444p10le",
    "yuva422p10le",
    "yuva420p10le",
    "yuva444p9le",
    "yuva422p9le",
    "yuva420p9le",
    "gray16le",
    "gray",
    "gbrp9le",
    "gbrp10le",
    "gbrp12le",
    "gbrp14le",
    "gbrap14le",
    "gbrap10le",
    "gbrap12le",
    "ya8",
    "gray10le",
    "gray12le",
    "gray14le",
    "gbrp16le",
    "rgb48le",
    "gbrap16le",
    "rgba64le",
    "gray9le",
    "yuv420p14le",
    "yuv422p14le",
    "yuv444p14le",
    "yuv440p10le",
    "yuv440p12le",
];

/// What `path` decodes to exactly ([`as_media::probe_exact`]), probed once
/// per file as it is on disk: a scenepack cuts many clips from one episode.
pub fn exact_format(tools: &Tools, path: &Path, video_index: u32) -> ExportResult<ExactFormat> {
    type Key = (PathBuf, u32, u64, Option<SystemTime>);
    static SEEN: LazyLock<Mutex<HashMap<Key, ExactFormat>>> = LazyLock::new(Default::default);
    let meta = std::fs::metadata(path)?;
    let key = (path.to_path_buf(), video_index, meta.len(), meta.modified().ok());
    if let Some(e) = SEEN.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return Ok(e.clone());
    }
    let exact = as_media::probe_exact(tools, path, video_index)?;
    SEEN.lock().unwrap_or_else(|p| p.into_inner()).insert(key, exact.clone());
    Ok(exact)
}

/// The codec a Bit-Perfect clip's picture is kept in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LosslessCodec {
    /// HEVC without loss, in a MOV.
    Hevc,
    /// FFV1, in an MKV.
    Ffv1,
}

impl LosslessCodec {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Hevc => "mov",
            Self::Ffv1 => "mkv",
        }
    }

    /// The muxer (`-f`).
    pub(crate) fn format(self) -> &'static str {
        match self {
            Self::Hevc => "mov",
            Self::Ffv1 => "matroska",
        }
    }
}

/// How a source's picture is kept without loss on a machine.
#[derive(Clone, Debug)]
pub(crate) struct Keeper {
    pub(crate) codec: LosslessCodec,
    /// Encoder, its options (with the colour tags) and the pixel format it
    /// is handed.
    pub(crate) encoding: Encoding,
    /// The filter that writes the colour tags on every frame (the hashed
    /// ones too): FFmpeg 9 converts frames whose tags differ from the file's.
    pub(crate) tag: String,
    /// The layout the encoder takes the frames in when it isn't theirs: a
    /// lossless repacking (NVENC takes 10 bits as `p010le`).
    pub(crate) repack: Option<String>,
}

/// How many pixels share a chroma sample across and down in `pix_fmt`.
pub(crate) fn chroma_step(pix_fmt: &str) -> (u32, u32) {
    let layout = ["420", "422", "411", "410", "440"].into_iter().find(|l| pix_fmt.contains(l));
    match layout {
        Some("420") => (2, 2),
        Some("422") => (2, 1),
        Some("411") => (4, 1),
        Some("410") => (4, 4),
        Some("440") => (1, 2),
        _ => (1, 1),
    }
}

/// A picture's colour, every tag given.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Colour {
    range: String,
    primaries: String,
    transfer: String,
    space: String,
    chroma: Option<String>,
}

/// The colour `exact`'s frames are. Where the source doesn't say, the copy
/// says what editors take it to be, so they show it as they show the
/// source: BT.709 for H.264, HEVC, AV1 and VP9 pictures (After Effects reads
/// an untagged H.264 that way at any size) and HD ones, BT.601 for older SD
/// codecs (525 or 625 lines); limited range unless the format is full range.
/// A tag that isn't a plain name counts as not said (it would go into the
/// filter graph, see [`crate::merge::graph_name`]).
fn colour(exact: &ExactFormat, source_codec: &str, height: u32) -> Colour {
    let modern = matches!(source_codec, "h264" | "hevc" | "av1" | "vp9") || height >= 720;
    let (primaries, transfer, space) = match () {
        _ if modern => ("bt709", "bt709", "bt709"),
        _ if height > 480 => ("bt470bg", "smpte170m", "bt470bg"),
        _ => ("smpte170m", "smpte170m", "smpte170m"),
    };
    let full = exact.pix_fmt.starts_with("yuvj");
    let or = |v: &Option<String>, d: &str| {
        v.clone().filter(|v| crate::merge::graph_name(v, false)).unwrap_or_else(|| d.to_string())
    };
    Colour {
        range: or(&exact.color_range, if full { "pc" } else { "tv" }),
        primaries: or(&exact.color_primaries, primaries),
        transfer: or(&exact.color_transfer, transfer),
        space: or(&exact.color_space, space),
        chroma: exact.chroma_location.clone(),
    }
}

/// The colour tags a Bit-Perfect copy of `info`'s picture gets (as
/// [`keeper`] writes them), from what its probe says: alike for two sources
/// whose copies can share a file.
pub(crate) fn assumed_colour(info: &as_media::MediaInfo) -> String {
    let v = &info.video;
    let exact = ExactFormat {
        pix_fmt: v.pix_fmt.clone(),
        color_range: v.color_range.clone(),
        color_primaries: v.color_primaries.clone(),
        color_transfer: v.color_transfer.clone(),
        color_space: v.color_space.clone(),
        ..ExactFormat::default()
    };
    colour(&exact, &v.codec, v.height).filter()
}

impl Colour {
    /// Written on the frames: tags only, the pixels stay.
    fn filter(&self) -> String {
        format!(
            "setparams=range={}:color_primaries={}:color_trc={}:colorspace={}",
            self.range, self.primaries, self.transfer, self.space
        )
    }

    /// The file's tags (output options).
    fn args(&self) -> Vec<String> {
        let mut a: Vec<String> = [
            ("-color_range", &self.range),
            ("-color_primaries", &self.primaries),
            ("-color_trc", &self.transfer),
            ("-colorspace", &self.space),
        ]
        .into_iter()
        .flat_map(|(k, v)| [k.to_string(), v.clone()])
        .collect();
        if let Some(c) = &self.chroma {
            a.extend(["-chroma_sample_location".into(), c.clone()]);
        }
        a
    }
}

/// The layout NVENC takes a picture of `pix_fmt` in without loss: 10 bits
/// moved to the top of 16 (`p010le`, `yuv444p16le`) is a repacking, not a
/// conversion. `None`: NVENC can't hold it exactly (4:2:2, more than 10
/// bits, gray, RGB; it would cut them down without a word).
fn nvenc_input(pix_fmt: &str) -> Option<&'static str> {
    Some(match pix_fmt {
        "yuv420p" | "yuvj420p" => "yuv420p",
        "yuv420p10le" => "p010le",
        "yuv444p" | "yuvj444p" => "yuv444p",
        "yuv444p10le" => "yuv444p16le",
        _ => return None,
    })
}

/// How a picture of `exact` from a `source_codec` stream of `size` pixels is
/// kept without loss with the encoders of `caps`: NVENC's lossless HEVC
/// where it holds the picture exactly (several times faster), else x265's,
/// else FFV1.
pub(crate) fn keeper(caps: &Caps, exact: &ExactFormat, source_codec: &str, (w, h): (u32, u32)) -> ExportResult<Keeper> {
    let fmt = exact.pix_fmt.as_str();
    let c = colour(exact, source_codec, h);
    let (sx, sy) = chroma_step(fmt);
    let whole = w % sx == 0 && h % sy == 0;
    // NVENC pads an odd size and spoils an odd height's chroma, silently.
    // NVENC writes no HDR metadata (mastering display, light level, Dolby
    // Vision): HDR goes to x265, which keeps it.
    let hdr =
        !exact.hdr_metadata.is_empty() || matches!(exact.color_transfer.as_deref(), Some("smpte2084" | "arib-std-b67"));
    let nvenc = caps
        .hw_encoder(CodecFamily::Hevc)
        .filter(|e| e.vendor == Vendor::Nvidia && w % 2 == 0 && h % 2 == 0 && !hdr)
        .and_then(|e| nvenc_input(fmt).map(|input| (e.name.clone(), input)))
        .filter(|(_, input)| caps.nvenc_lossless.iter().any(|f| f == input));
    if let Some((name, input)) = nvenc {
        let mut args: Vec<String> =
            ["-c:v", &name, "-preset", "p4", "-tune", "lossless", "-pix_fmt", input, "-tag:v", "hvc1"]
                .map(String::from)
                .to_vec();
        args.extend(c.args());
        let repack = (input != fmt).then(|| input.to_string());
        let encoding = Encoding { encoder: name, args, pix_fmt: input.to_string() };
        return Ok(Keeper { codec: LosslessCodec::Hevc, encoding, tag: c.filter(), repack });
    }
    if HEVC_FORMATS.contains(&fmt) && whole && caps.has_sw("libx265") {
        let mut args: Vec<String> = [
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-x265-params",
            "lossless=1:log-level=error",
            "-pix_fmt",
            fmt,
            // Apple/QuickTime-compatible HEVC sample entry.
            "-tag:v",
            "hvc1",
        ]
        .map(String::from)
        .to_vec();
        args.extend(c.args());
        let encoding = Encoding { encoder: "libx265".into(), args, pix_fmt: fmt.to_string() };
        return Ok(Keeper { codec: LosslessCodec::Hevc, encoding, tag: c.filter(), repack: None });
    }
    // FFV1 has no full-range formats of its own: the same samples, tagged full.
    let ffv1 = fmt.strip_prefix("yuvj").map(|rest| format!("yuv{rest}")).unwrap_or_else(|| fmt.to_string());
    if FFV1_FORMATS.contains(&ffv1.as_str()) && caps.has_sw("ffv1") {
        // Every frame a keyframe, so any frame can start a cut or a part.
        let mut args: Vec<String> =
            ["-c:v", "ffv1", "-level", "3", "-g", "1", "-coder", "1", "-context", "0", "-slicecrc", "1", "-pix_fmt"]
                .map(String::from)
                .to_vec();
        args.push(ffv1.clone());
        args.extend(c.args());
        let encoding = Encoding { encoder: "ffv1".into(), args, pix_fmt: ffv1 };
        return Ok(Keeper { codec: LosslessCodec::Ffv1, encoding, tag: c.filter(), repack: None });
    }
    if !caps.has_sw("libx265") && !caps.has_sw("ffv1") {
        return Err(ExportError::EncoderMissing("ffv1".into()));
    }
    Err(ExportError::Inexact(format!("its pictures ({fmt}) can't be kept as they are")))
}

/// The codec a picture of `pix_fmt` and `size` will be kept in, as
/// [`keeper`] chooses it (where NVENC or x265 keeps it, HEVC; else FFV1):
/// for saying so before an export.
pub fn codec_for(pix_fmt: &str, (w, h): (u32, u32)) -> LosslessCodec {
    let (sx, sy) = chroma_step(pix_fmt);
    if HEVC_FORMATS.contains(&pix_fmt) && w % sx == 0 && h % sy == 0 {
        LosslessCodec::Hevc
    } else {
        LosslessCodec::Ffv1
    }
}

/// The picture's graph from stream `stream`: `chain` (the cut and its
/// timing), the frames tagged, then handed to the encoder (`[v]`, repacked
/// if it takes another layout) and to the hashes (`[vref]`).
pub(crate) fn picture_graph(stream: u32, chain: &[String], tag: &str, repack: Option<&str>) -> String {
    let chain = chain.iter().map(String::as_str).chain([tag]).collect::<Vec<_>>().join(",");
    match repack {
        Some(f) => format!("[0:{stream}]{chain},split[w][vref];[w]format={f}[v]"),
        None => format!("[0:{stream}]{chain},split[v][vref]"),
    }
}

/// The PCM codec that stores `a`'s decoded samples as they are.
pub(crate) fn pcm_codec(a: &ExactAudio) -> ExportResult<&'static str> {
    let fmt = a.sample_fmt.as_str();
    Ok(match fmt.strip_suffix('p').unwrap_or(fmt) {
        "flt" => "pcm_f32le",
        "dbl" => "pcm_f64le",
        "s16" => "pcm_s16le",
        // 24 significant bits in a 32-bit sample (24-bit PCM, FLAC, TrueHD).
        "s32" if (1..=24).contains(&a.bits) => "pcm_s24le",
        "s32" => "pcm_s32le",
        "u8" => "pcm_u8",
        _ => return Err(ExportError::Inexact(format!("its sound ({fmt}) can't be kept as it is"))),
    })
}

/// What a clip's frames and sound were when the encoder got them.
#[derive(Clone, Debug)]
pub(crate) struct Reference {
    /// The PCM codec the sound is stored (and hashed) as; `None`: no sound.
    pub(crate) audio: Option<&'static str>,
    /// The source's HDR metadata (`ExactFormat::hdr_metadata`): the file
    /// must carry every kind too.
    pub(crate) hdr: Vec<String>,
}

/// Per-frame hashes of a framemd5 file, with its picture size and shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameHashes {
    pub(crate) dimensions: Option<String>,
    pub(crate) sar: Option<String>,
    pub(crate) hashes: Vec<String>,
}

pub(crate) fn read_framemd5(text: &str) -> FrameHashes {
    let mut h = FrameHashes::default();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix('#') {
            let (k, v) = head.split_once(':').map(|(k, v)| (k.trim(), v.trim())).unwrap_or((head.trim(), ""));
            match k {
                "dimensions 0" => h.dimensions = Some(v.to_string()),
                "sar 0" => h.sar = Some(v.to_string()),
                _ => {}
            }
        } else if let Some(hash) = line.rsplit(',').next().map(str::trim).filter(|s| !s.is_empty()) {
            h.hashes.push(hash.to_string());
        }
    }
    h
}

fn read_md5(text: &str) -> Option<String> {
    text.lines().find_map(|l| l.trim().strip_prefix("MD5=").map(str::to_string))
}

/// Where a part's frames are left, as checked, next to it: the file it
/// joins is checked against them.
pub(crate) fn checked_frames(part: &Path) -> PathBuf {
    crate::clip::with_suffix(part, ".frames.txt")
}

/// Where a clip's reference hashes go, next to its file in the making.
pub(crate) fn reference_paths(part: &Path) -> (PathBuf, PathBuf) {
    (crate::clip::with_suffix(part, ".ref-frames.txt"), crate::clip::with_suffix(part, ".ref-sound.txt"))
}

/// The hashes of `file`'s frames as decoded (not turned upright), written to
/// `frames`, and of its sound stored as `audio`; `progress` follows the
/// frames out of `n`.
pub(crate) fn decode_hashes(
    tools: &Tools,
    file: &Path,
    frames: &Path,
    audio: Option<(&'static str, &Path)>,
    n: u32,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
) -> ExportResult<()> {
    let mut a: Vec<String> = ["-v", "error", "-y", "-noautorotate", "-i"].map(String::from).to_vec();
    a.push(file.display().to_string());
    a.extend(["-map", "0:v:0", "-fps_mode", "passthrough", "-f", "framemd5"].map(String::from));
    a.push(frames.display().to_string());
    if let Some((codec, path)) = audio {
        a.extend(["-map", "0:a:0", "-c:a", codec, "-f", "md5"].map(String::from));
        a.push(path.display().to_string());
    }
    run_ffmpeg(tools, a, cancel, |p| progress((p.frame as f64 / f64::from(n.max(1))).clamp(0.0, 1.0)))?;
    Ok(())
}

/// Does `part` decode to exactly the frames and sound hashed as it was
/// encoded (`reference_paths(part)`)? `Err(Ok(why))` when it doesn't,
/// `Err(Err(e))` when it couldn't be checked. Its frames' hashes are left
/// at `frames_out` (a joined file is checked against its parts' ones).
pub(crate) fn check_file(
    tools: &Tools,
    part: &Path,
    reference: &Reference,
    frames_out: &Path,
    n: u32,
    cancel: &CancelToken,
    progress: impl FnMut(f64),
) -> Result<(), Result<String, ExportError>> {
    let (ref_frames, ref_sound) = reference_paths(part);
    let sound_out = sound_hashes(frames_out);
    let audio = reference.audio.map(|c| (c, sound_out.as_path()));
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| Err(ExportError::Io(e)));
    let hashed = (|| {
        decode_hashes(tools, part, frames_out, audio, n, cancel, progress).map_err(Err)?;
        let want = read_framemd5(&read(&ref_frames)?);
        let got = read_framemd5(&read(frames_out)?);
        let sound = match reference.audio {
            Some(_) => Some((read_md5(&read(&ref_sound)?), read_md5(&read(&sound_out)?))),
            None => None,
        };
        Ok((want, got, sound))
    })();
    // Gone on every way out, a cancel too: it would keep the work folder.
    let _ = std::fs::remove_file(&sound_out);
    let (want, got, sound) = hashed?;
    compare(&want, &got, sound).map_err(Ok)?;
    keeps_hdr(tools, part, &reference.hdr)
}

/// Where the sound's hashes of a check that leaves its frames' at `frames`
/// go (a clip's scratch file).
pub(crate) fn sound_hashes(frames: &Path) -> PathBuf {
    crate::clip::with_suffix(frames, ".sound.txt")
}

/// Does `file` carry every kind of HDR metadata in `want`?
pub(crate) fn keeps_hdr(tools: &Tools, file: &Path, want: &[String]) -> Result<(), Result<String, ExportError>> {
    if want.is_empty() {
        return Ok(());
    }
    let out = as_media::proc::output(
        tools
            .ffprobe()
            .args(["-select_streams", "v:0", "-read_intervals", "%+#64", "-show_entries"])
            .args(["stream_side_data:frame_side_data", "-of", "json"])
            .arg(file),
    )
    .map_err(|e| Err(ExportError::Io(e)))?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let mut sides: Vec<&serde_json::Value> = Vec::new();
    for key in ["streams", "frames"] {
        sides.extend(v.get(key).and_then(serde_json::Value::as_array).into_iter().flatten().take(1));
    }
    let have = as_media::hdr_metadata(sides);
    let lost: Vec<&String> = want.iter().filter(|k| !have.contains(k)).collect();
    if lost.is_empty() {
        Ok(())
    } else {
        Err(Ok(format!(
            "its HDR metadata didn't come along ({})",
            lost.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        )))
    }
}

/// Frames (and sound) `got` against what was encoded, `want`.
pub(crate) fn compare(
    want: &FrameHashes,
    got: &FrameHashes,
    sound: Option<(Option<String>, Option<String>)>,
) -> Result<(), String> {
    if want.hashes.is_empty() {
        return Err("no frames were hashed".into());
    }
    if want.dimensions != got.dimensions || want.sar != got.sar {
        return Err(format!(
            "the picture is {:?} {:?}, the source's {:?} {:?}",
            got.dimensions, got.sar, want.dimensions, want.sar
        ));
    }
    if want.hashes.len() != got.hashes.len() {
        return Err(format!("{} frames, the source {}", got.hashes.len(), want.hashes.len()));
    }
    if let Some(i) = want.hashes.iter().zip(&got.hashes).position(|(a, b)| a != b) {
        let differ = want.hashes.iter().zip(&got.hashes).filter(|(a, b)| a != b).count();
        return Err(format!("{differ} frames differ from the source's, the first frame {i}"));
    }
    if let Some((want, got)) = sound {
        match (want, got) {
            (Some(w), Some(g)) if w == g => {}
            (w, g) => return Err(format!("the sound differs from the source's ({g:?} vs {w:?})")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(pix_fmt: &str) -> ExactFormat {
        ExactFormat { pix_fmt: pix_fmt.into(), ..ExactFormat::default() }
    }

    fn caps() -> Caps {
        Caps { sw_encoders: vec!["libx265".into(), "ffv1".into()], ..Caps::default() }
    }

    fn codec(e: &ExactFormat, size: (u32, u32)) -> Option<LosslessCodec> {
        keeper(&caps(), e, "h264", size).ok().map(|k| k.codec)
    }

    /// A machine with an NVIDIA card.
    fn nvidia() -> Caps {
        let hevc = as_media::HwEncoder { name: "hevc_nvenc".into(), family: CodecFamily::Hevc, vendor: Vendor::Nvidia };
        let lossless = ["yuv420p", "p010le", "yuv444p", "yuv444p16le"].map(String::from).to_vec();
        Caps { hw_encoders: vec![hevc], nvenc_lossless: lossless, ..caps() }
    }

    #[test]
    fn nvenc_keeps_no_hdr_and_only_what_the_card_does_without_loss() {
        let hd = (1920, 1080);
        let encoder = |caps: &Caps, e: &ExactFormat| keeper(caps, e, "hevc", hd).unwrap().encoding.encoder;
        // HDR: NVENC writes no mastering display or light level; x265 does.
        let pq = ExactFormat { color_transfer: Some("smpte2084".into()), ..exact("yuv420p10le") };
        assert_eq!(encoder(&nvidia(), &pq), "libx265");
        let hlg = ExactFormat { color_transfer: Some("arib-std-b67".into()), ..exact("yuv420p10le") };
        assert_eq!(encoder(&nvidia(), &hlg), "libx265");
        let metadata = ExactFormat { hdr_metadata: vec!["Mastering display metadata".into()], ..exact("yuv420p10le") };
        assert_eq!(encoder(&nvidia(), &metadata), "libx265");
        assert_eq!(encoder(&nvidia(), &exact("yuv420p10le")), "hevc_nvenc");
        // A card that can't do it without loss (or not in 10 bits): x265.
        let old = Caps { nvenc_lossless: vec!["yuv420p".into()], ..nvidia() };
        assert_eq!(encoder(&old, &exact("yuv420p")), "hevc_nvenc");
        assert_eq!(encoder(&old, &exact("yuv420p10le")), "libx265");
        let none = Caps { nvenc_lossless: Vec::new(), ..nvidia() };
        assert_eq!(encoder(&none, &exact("yuv420p")), "libx265");
    }

    #[test]
    fn the_container_is_known_before_the_export() {
        assert_eq!(codec_for("yuv420p", (1920, 1080)), LosslessCodec::Hevc);
        assert_eq!(codec_for("yuv420p", (1920, 803)), LosslessCodec::Ffv1);
        assert_eq!(codec_for("yuv411p", (720, 480)), LosslessCodec::Ffv1);
        for (f, size) in [("yuv420p10le", (3840, 2160)), ("yuv422p10le", (1920, 1080)), ("yuv420p", (641, 357))] {
            let e = exact(f);
            assert_eq!(codec_for(f, size), keeper(&caps(), &e, "h264", size).unwrap().codec, "{f}");
            assert_eq!(codec_for(f, size), keeper(&nvidia(), &e, "h264", size).unwrap().codec, "{f}");
        }
    }

    #[test]
    fn hevc_keeps_what_it_can_ffv1_the_rest() {
        let hd = (1920, 1080);
        for f in ["yuv420p", "yuvj420p", "yuv420p10le", "yuv422p10le", "yuv444p12le", "gbrp", "gray"] {
            assert_eq!(codec(&exact(f), hd), Some(LosslessCodec::Hevc), "{f}");
        }
        // Odd sizes for subsampled chroma, 4:1:1 (DV), alpha, 16 bits.
        assert_eq!(codec(&exact("yuv420p"), (640, 357)), Some(LosslessCodec::Ffv1));
        assert_eq!(codec(&exact("yuv422p"), (641, 360)), Some(LosslessCodec::Ffv1));
        assert_eq!(codec(&exact("yuv444p"), (641, 357)), Some(LosslessCodec::Hevc));
        for f in ["yuv411p", "yuva444p12le", "yuv444p16le"] {
            assert_eq!(codec(&exact(f), hd), Some(LosslessCodec::Ffv1), "{f}");
        }
        // Interlaced pictures keep their lines in HEVC (editors open it).
        let woven = ExactFormat { interlaced: Some(false), ..exact("yuv420p") };
        assert_eq!(codec(&woven, (720, 576)), Some(LosslessCodec::Hevc));
        // A format neither stores as it is.
        assert!(matches!(keeper(&caps(), &exact("rgb24"), "h264", hd), Err(ExportError::Inexact(_))));
        assert!(matches!(keeper(&Caps::default(), &exact("yuv420p"), "h264", hd), Err(ExportError::EncoderMissing(_))));
    }

    #[test]
    fn nvenc_takes_only_what_it_holds_exactly() {
        let nv = |f: &str, size| keeper(&nvidia(), &exact(f), "h264", size).unwrap();
        let k = nv("yuv420p", (1920, 1080));
        assert_eq!((k.encoding.encoder.as_str(), k.repack), ("hevc_nvenc", None));
        assert!(k.encoding.args.join(" ").starts_with("-c:v hevc_nvenc -preset p4 -tune lossless -pix_fmt yuv420p"));
        // 10 bits repacked to the top of 16, the same samples.
        let k = nv("yuv420p10le", (3840, 2160));
        assert_eq!((k.encoding.pix_fmt.as_str(), k.repack.as_deref()), ("p010le", Some("p010le")));
        assert_eq!(nv("yuv444p10le", (1920, 1080)).repack.as_deref(), Some("yuv444p16le"));
        // What NVENC would spoil or cut down without a word goes to x265.
        for (f, size) in [
            ("yuv420p", (640, 357)),
            ("yuv420p", (641, 360)),
            ("yuv422p10le", (1920, 1080)),
            ("yuv420p12le", (1920, 1080)),
        ] {
            assert_ne!(nv(f, size).encoding.encoder, "hevc_nvenc", "{f} {size:?}");
        }
    }

    #[test]
    fn the_encoder_gets_the_decoded_format_and_its_tags() {
        let hdr = ExactFormat {
            color_range: Some("tv".into()),
            color_primaries: Some("bt2020".into()),
            color_transfer: Some("smpte2084".into()),
            color_space: Some("bt2020nc".into()),
            chroma_location: Some("left".into()),
            ..exact("yuv420p10le")
        };
        let k = keeper(&caps(), &hdr, "hevc", (3840, 2160)).unwrap();
        assert_eq!(
            k.encoding.args.join(" "),
            "-c:v libx265 -preset ultrafast -x265-params lossless=1:log-level=error -pix_fmt yuv420p10le \
             -tag:v hvc1 -color_range tv -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc \
             -chroma_sample_location left"
        );
        assert_eq!(k.tag, "setparams=range=tv:color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc");
        // A full-range picture in FFV1: the same samples, tagged full range.
        let full = ExactFormat { chroma_location: Some("center".into()), ..exact("yuvj420p") };
        let k = keeper(&caps(), &full, "mjpeg", (639, 360)).unwrap();
        assert_eq!(k.codec, LosslessCodec::Ffv1);
        assert_eq!(k.encoding.pix_fmt, "yuv420p");
        assert!(k.encoding.args.join(" ").contains("-pix_fmt yuv420p -color_range pc"));
        assert!(k.encoding.args.join(" ").ends_with("-chroma_sample_location center"));
    }

    #[test]
    fn an_untagged_source_gets_the_colour_editors_assume() {
        let tags = |codec: &str, h: u32| keeper(&caps(), &exact("yuv420p"), codec, (720, h)).unwrap().tag;
        // H.264 and HD pictures: BT.709, limited range.
        assert_eq!(tags("h264", 360), "setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709");
        assert_eq!(tags("mpeg2video", 720), tags("h264", 360));
        // Older SD codecs: BT.601, 625 or 525 lines.
        assert_eq!(
            tags("mpeg2video", 576),
            "setparams=range=tv:color_primaries=bt470bg:color_trc=smpte170m:colorspace=bt470bg"
        );
        assert_eq!(
            tags("mpeg4", 480),
            "setparams=range=tv:color_primaries=smpte170m:color_trc=smpte170m:colorspace=smpte170m"
        );
        // What the source says, it keeps.
        let partly = ExactFormat { color_primaries: Some("bt2020".into()), ..exact("yuv420p") };
        assert!(keeper(&caps(), &partly, "h264", (720, 360)).unwrap().tag.contains("color_primaries=bt2020:"));
        // A tag that would reshape the graph isn't one.
        let odd = ExactFormat { color_primaries: Some("bt2020,movie=x".into()), ..exact("yuv420p") };
        assert_eq!(keeper(&caps(), &odd, "h264", (720, 360)).unwrap().tag, tags("h264", 360));
    }

    #[test]
    fn the_graph_hashes_the_frames_the_encoder_gets() {
        let cut = ["trim=start=0us:end=41708us".to_string(), "setpts=PTS-STARTPTS".to_string()];
        assert_eq!(
            picture_graph(0, &cut, "setparams=range=tv", None),
            "[0:0]trim=start=0us:end=41708us,setpts=PTS-STARTPTS,setparams=range=tv,split[v][vref]"
        );
        assert_eq!(
            picture_graph(1, &cut, "setparams=range=tv", Some("p010le")),
            "[0:1]trim=start=0us:end=41708us,setpts=PTS-STARTPTS,setparams=range=tv,split[w][vref];[w]format=p010le[v]"
        );
    }

    #[test]
    fn sound_is_stored_in_its_own_sample_format() {
        let a = |fmt: &str, bits| ExactAudio { sample_fmt: fmt.into(), bits, ..ExactAudio::default() };
        assert_eq!(pcm_codec(&a("fltp", 0)).unwrap(), "pcm_f32le");
        assert_eq!(pcm_codec(&a("flt", 0)).unwrap(), "pcm_f32le");
        assert_eq!(pcm_codec(&a("s16p", 16)).unwrap(), "pcm_s16le");
        assert_eq!(pcm_codec(&a("s32", 24)).unwrap(), "pcm_s24le");
        assert_eq!(pcm_codec(&a("s32p", 0)).unwrap(), "pcm_s32le");
        assert_eq!(pcm_codec(&a("dblp", 0)).unwrap(), "pcm_f64le");
        assert_eq!(pcm_codec(&a("u8", 8)).unwrap(), "pcm_u8");
        assert!(pcm_codec(&a("s64", 64)).is_err());
    }

    #[test]
    fn frames_are_compared_by_their_hashes() {
        let file = |hashes: &[&str]| {
            let mut s = String::from("#format: frame checksums\n#version: 2\n#hash: MD5\n#tb 0: 1/24\n");
            s += "#media_type 0: video\n#codec_id 0: rawvideo\n#dimensions 0: 640x360\n#sar 0: 1/1\n";
            s += "#stream#, dts,        pts, duration,     size, hash\n";
            for (i, h) in hashes.iter().enumerate() {
                s += &format!("0, {i}, {i}, 1, 345600, {h}\n");
            }
            read_framemd5(&s)
        };
        let want = file(&["a1", "b2", "c3"]);
        assert_eq!(want.hashes, ["a1", "b2", "c3"]);
        assert_eq!(want.dimensions.as_deref(), Some("640x360"));
        assert!(compare(&want, &file(&["a1", "b2", "c3"]), None).is_ok());
        assert!(compare(&want, &file(&["a1", "xx", "c3"]), None).unwrap_err().contains("first frame 1"));
        assert!(compare(&want, &file(&["a1", "b2"]), None).is_err());
        let md5 = |s: &str| Some(s.to_string());
        assert!(compare(&want, &want.clone(), Some((md5("s"), md5("s")))).is_ok());
        assert!(compare(&want, &want.clone(), Some((md5("s"), md5("t")))).is_err());
        assert_eq!(read_md5("MD5=0123abc\n").as_deref(), Some("0123abc"));
    }
}
