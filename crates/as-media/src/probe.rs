//! `ffprobe` → typed [`MediaInfo`].

use std::path::Path;

use as_core::Rational;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{MediaError, MediaResult, Tools};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HdrKind {
    None,
    Pq,
    Hlg,
    DolbyVision,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VideoStream {
    /// Absolute stream index in the container.
    pub index: u32,
    pub codec: String,
    pub profile: Option<String>,
    pub pix_fmt: String,
    pub bit_depth: u32,
    pub width: u32,
    pub height: u32,
    /// Sample aspect ratio (1/1 for square pixels).
    pub sar: Rational,
    pub r_frame_rate: Rational,
    pub avg_frame_rate: Option<Rational>,
    pub time_base: Rational,
    pub color_primaries: Option<String>,
    pub color_transfer: Option<String>,
    pub color_space: Option<String>,
    pub color_range: Option<String>,
    /// `progressive`, `tt`, `bb`, `tb`, `bt` or unknown.
    pub field_order: Option<String>,
    /// Display rotation in degrees (0, 90, 180, 270).
    pub rotation: i32,
    pub hdr: HdrKind,
}

impl VideoStream {
    /// How wide a pixel is for its height: the sample aspect ratio, or 1
    /// (square) when the stored one isn't a positive number (a damaged or
    /// edited record; a probe never stores one).
    pub fn pixel_aspect(&self) -> f64 {
        let Rational { num, den } = self.sar;
        if num > 0 && den > 0 { self.sar.to_f64() } else { 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioStream {
    pub index: u32,
    /// Position among audio streams (`0:a:N`).
    pub ordinal: u32,
    pub codec: String,
    /// Codec profile when it matters to people, e.g. `DTS-HD MA`.
    pub profile: Option<String>,
    pub channels: u32,
    pub channel_layout: Option<String>,
    pub sample_rate: u32,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    /// Director's commentary or audio description, not the programme sound.
    pub commentary: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleStream {
    pub index: u32,
    pub ordinal: u32,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    /// Text subtitles (SRT/ASS/…) can feed dialogue search; bitmap ones can't.
    pub text_based: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// ffprobe `format_name`, e.g. `mov,mp4,m4a,3gp,3g2,mj2` or `matroska,webm`.
    pub container: String,
    pub duration_sec: Option<f64>,
    pub start_sec: f64,
    pub size_bytes: Option<f64>,
    pub bit_rate: Option<f64>,
    pub video: VideoStream,
    pub audio: Vec<AudioStream>,
    pub subtitles: Vec<SubtitleStream>,
}

impl MediaInfo {
    pub fn is_interlaced(&self) -> bool {
        matches!(self.video.field_order.as_deref(), Some("tt" | "bb" | "tb" | "bt"))
    }

    /// Default audio track: the one flagged default, else the first.
    pub fn default_audio(&self) -> Option<&AudioStream> {
        self.audio.iter().find(|a| a.default).or(self.audio.first())
    }

    /// The track to use when nobody has chosen one: the first programme
    /// track in `language` (ISO 639 code or name; empty = no preference),
    /// else the file's default. Commentary and audio description only win
    /// when nothing else exists. Files often flag every track "default", so
    /// the language decides before the flag does.
    pub fn preferred_audio(&self, language: &str) -> Option<&AudioStream> {
        let programme: Vec<&AudioStream> = self.audio.iter().filter(|a| !a.commentary).collect();
        let pool = if programme.is_empty() { self.audio.iter().collect() } else { programme };
        let in_language: Vec<&AudioStream> = if language.trim().is_empty() {
            Vec::new()
        } else {
            pool.iter()
                .copied()
                .filter(|a| a.language.as_deref().is_some_and(|l| crate::lang::same(l, language)))
                .collect()
        };
        let from = if in_language.is_empty() { pool } else { in_language };
        from.iter().copied().find(|a| a.default).or(from.first().copied())
    }
}

/// Whether `new` is very likely the file `old` was probed from — moved,
/// copied or remuxed into the same kind of file: the same container,
/// picture, frame rate, time base, start and length (within half a frame)
/// and audio tracks. `Err` says, for people, what differs.
///
/// Half a frame of the frames' real rate: phone and screen recordings
/// report their time base's tick rate as `r_frame_rate` (`90000/1`, where
/// half a tick is microseconds), so a lower average rate wins, and no
/// window is narrower than half a frame at 240 fps.
pub fn same_source(old: &MediaInfo, new: &MediaInfo) -> Result<(), String> {
    let (a, b) = (&old.video, &new.video);
    if old.container != new.container {
        return Err("It's another kind of video file.".into());
    }
    if a.codec != b.codec {
        return Err(format!("Its picture is stored as {} instead of {}.", b.codec, a.codec));
    }
    if (a.width, a.height) != (b.width, b.height) || a.sar != b.sar {
        return Err(format!("Its picture is {}×{} instead of {}×{}.", b.width, b.height, a.width, a.height));
    }
    if a.r_frame_rate != b.r_frame_rate || a.time_base != b.time_base {
        return Err("It plays at another frame rate.".into());
    }
    let rate = a.r_frame_rate.to_f64();
    let fps = a.avg_frame_rate.map(Rational::to_f64).filter(|&avg| avg > 0.0 && avg < rate).unwrap_or(rate);
    let half_frame = if fps > 0.0 { 0.5 / fps.min(MAX_FPS) } else { 0.5 };
    if (old.start_sec - new.start_sec).abs() > half_frame {
        return Err("It starts at another moment.".into());
    }
    if let (Some(x), Some(y)) = (old.duration_sec, new.duration_sec)
        && (x - y).abs() > half_frame
    {
        return Err(format!("It's {} long instead of {}.", clock(y), clock(x)));
    }
    let tracks = |i: &MediaInfo| i.audio.iter().map(|t| (t.codec.clone(), t.channels)).collect::<Vec<_>>();
    if tracks(old) != tracks(new) {
        return Err("Its audio tracks are different.".into());
    }
    Ok(())
}

/// The highest rate frames are filmed at (slow-motion phone video): a
/// higher one is a time base's ticks.
const MAX_FPS: f64 = 240.0;

/// 2531.4 → "42:11"; 3725 → "1:02:05".
fn clock(sec: f64) -> String {
    let s = sec.max(0.0).round() as u64;
    let (h, m, r) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{r:02}") } else { format!("{m}:{r:02}") }
}

pub fn probe(tools: &Tools, path: &Path) -> MediaResult<MediaInfo> {
    let err = |message: String| MediaError::Probe { path: path.display().to_string(), message };
    let out = crate::proc::output(
        tools.ffprobe().args(["-print_format", "json", "-show_format", "-show_streams"]).arg(path),
    )?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(err(if msg.is_empty() { "unreadable file".into() } else { msg }));
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| err(e.to_string()))?;
    let mut info = parse(&v).ok_or_else(|| MediaError::NoVideo(path.display().to_string()))?;
    let vs = &info.video;
    if vs.color_transfer.is_none() || vs.color_primaries.is_none() || vs.color_space.is_none() {
        fill_colour_from_first_frame(tools, path, &mut info.video);
    }
    Ok(info)
}

/// Containers don't always carry the colour description — FFmpeg 9's
/// Matroska muxer drops transfer and primaries, and so do many remuxes — but
/// the decoded frames always do. Without this an HDR10 file reads as SDR.
fn fill_colour_from_first_frame(tools: &Tools, path: &Path, video: &mut VideoStream) {
    let out = crate::proc::output(
        tools
            .ffprobe()
            .args(["-select_streams", &video.index.to_string(), "-read_intervals", "%+#16", "-show_entries"])
            .args(["frame=color_transfer,color_primaries,color_space,color_range", "-of", "json"])
            .arg(path),
    );
    let Ok(out) = out else { return };
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else { return };
    let Some(frame) = v.get("frames").and_then(Value::as_array).and_then(|f| f.first()) else { return };
    apply_frame_colour(video, frame);
}

/// What a source's streams decode to, exactly as the decoder hands them
/// over: what a lossless copy has to keep (see [`probe_exact`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExactFormat {
    /// The decoded frames' pixel format (`yuvj420p` for a full-range
    /// picture, as the decoder names it).
    pub pix_fmt: String,
    pub color_range: Option<String>,
    pub color_primaries: Option<String>,
    pub color_transfer: Option<String>,
    pub color_space: Option<String>,
    /// Where chroma samples sit (`left`, `topleft`, …).
    pub chroma_location: Option<String>,
    /// `Some(top field first)` when the first frame is interlaced.
    pub interlaced: Option<bool>,
    /// The HDR metadata the stream or its first frame carries, by ffprobe's
    /// name (`Mastering display metadata`, `Content light level metadata`,
    /// `DOVI configuration record`, `HDR Dynamic Metadata SMPTE2094-40
    /// (HDR10+)`); a light level of 0/0 (unknown) isn't counted.
    pub hdr_metadata: Vec<String>,
    pub audio: Vec<ExactAudio>,
}

/// An audio stream's decoded samples.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExactAudio {
    /// Position among audio streams (`0:a:N`).
    pub ordinal: u32,
    /// `fltp`, `s16`, `s32p`…
    pub sample_fmt: String,
    /// Significant bits of an integer sample (24 in an `s32` of a 24-bit
    /// source); 0 when unknown.
    pub bits: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub channel_layout: Option<String>,
}

/// The exact decoded formats of `path`'s video stream `video_index` and its
/// audio streams: the stream's word, the first frame's where the stream has
/// none (colour) or where only frames know (interlacing).
pub fn probe_exact(tools: &Tools, path: &Path, video_index: u32) -> MediaResult<ExactFormat> {
    let err = |message: String| MediaError::Probe { path: path.display().to_string(), message };
    let json = |cmd: &mut std::process::Command| -> MediaResult<Value> {
        let out = crate::proc::output(cmd.args(["-of", "json"]).arg(path))?;
        if !out.status.success() {
            return Err(err(String::from_utf8_lossy(&out.stderr).trim().to_string()));
        }
        serde_json::from_slice(&out.stdout).map_err(|e| err(e.to_string()))
    };
    let streams = json(tools.ffprobe().args([
        "-show_entries",
        "stream=index,codec_type,pix_fmt,color_range,color_primaries,color_transfer,color_space,chroma_location,\
         sample_fmt,bits_per_raw_sample,sample_rate,channels,channel_layout:stream_disposition=attached_pic:\
         stream_side_data",
    ]))?;
    // A few packets: a recording that starts between keyframes decodes no
    // frame from its first ones.
    let frame = json(tools.ffprobe().args(["-select_streams", &video_index.to_string(), "-read_intervals", "%+#64"]).args([
        "-show_entries",
        "frame=pix_fmt,color_range,color_primaries,color_transfer,color_space,chroma_location,interlaced_frame,top_field_first:frame_side_data",
    ]))?;
    let frame = frame.get("frames").and_then(Value::as_array).and_then(|f| f.first()).cloned().unwrap_or_default();
    let list = streams.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    let is = |st: &Value, ty: &str| st.get("codec_type").and_then(Value::as_str) == Some(ty);
    let video = list
        .iter()
        .find(|st| is(st, "video") && num::<u32>(st, "index") == Some(video_index))
        .ok_or_else(|| MediaError::NoVideo(path.display().to_string()))?;
    let either = |k: &str| s(video, k).or_else(|| s(&frame, k));
    let interlaced =
        (num::<i64>(&frame, "interlaced_frame") == Some(1)).then(|| num::<i64>(&frame, "top_field_first") == Some(1));
    let audio = list
        .iter()
        .filter(|st| is(st, "audio"))
        .enumerate()
        .map(|(i, st)| ExactAudio {
            ordinal: i as u32,
            sample_fmt: s(st, "sample_fmt").unwrap_or_default(),
            bits: num(st, "bits_per_raw_sample").unwrap_or(0),
            sample_rate: num(st, "sample_rate").unwrap_or(0),
            channels: num(st, "channels").unwrap_or(0),
            channel_layout: s(st, "channel_layout"),
        })
        .collect();
    Ok(ExactFormat {
        // The frame's word first: a stream may name the coded format.
        pix_fmt: s(&frame, "pix_fmt").or_else(|| s(video, "pix_fmt")).unwrap_or_default(),
        color_range: either("color_range"),
        color_primaries: either("color_primaries"),
        color_transfer: either("color_transfer"),
        color_space: either("color_space"),
        chroma_location: either("chroma_location").filter(|c| c != "unspecified"),
        interlaced,
        hdr_metadata: hdr_metadata([video, &frame]),
        audio,
    })
}

/// The kinds of HDR metadata in `sides` (streams' or frames' side data), as
/// [`ExactFormat::hdr_metadata`] lists them.
pub fn hdr_metadata<'a>(sides: impl IntoIterator<Item = &'a Value>) -> Vec<String> {
    const KINDS: [&str; 4] = [
        "Mastering display metadata",
        "Content light level metadata",
        "DOVI configuration record",
        "HDR Dynamic Metadata SMPTE2094-40 (HDR10+)",
    ];
    let mut kinds: Vec<String> = sides
        .into_iter()
        .filter_map(|v| v.get("side_data_list").and_then(Value::as_array))
        .flatten()
        .filter(|sd| {
            // A light level of 0/0 says nothing (muxers and encoders drop it).
            let unknown = sd.get("max_content").and_then(Value::as_i64) == Some(0)
                && sd.get("max_average").and_then(Value::as_i64) == Some(0);
            !unknown
        })
        .filter_map(|sd| sd.get("side_data_type").and_then(Value::as_str))
        .filter(|k| KINDS.contains(k))
        .map(str::to_owned)
        .collect();
    kinds.sort();
    kinds.dedup();
    kinds
}

fn apply_frame_colour(video: &mut VideoStream, frame: &Value) {
    video.color_transfer = video.color_transfer.take().or_else(|| s(frame, "color_transfer"));
    video.color_primaries = video.color_primaries.take().or_else(|| s(frame, "color_primaries"));
    video.color_space = video.color_space.take().or_else(|| s(frame, "color_space"));
    video.color_range = video.color_range.take().or_else(|| s(frame, "color_range"));
    if video.hdr == HdrKind::None {
        video.hdr = transfer_hdr(video.color_transfer.as_deref());
    }
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned).filter(|s| !s.is_empty() && s != "unknown")
}

fn num<T: std::str::FromStr>(v: &Value, k: &str) -> Option<T> {
    match v.get(k)? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

/// A number of seconds, bytes or bits, when it is one: Rust reads "nan" and
/// "inf" as numbers too, and those would pass every comparison silently.
fn finite(v: &Value, k: &str) -> Option<f64> {
    num::<f64>(v, k).filter(|x| x.is_finite())
}

fn rational(v: &Value, k: &str) -> Option<Rational> {
    let r = Rational::parse(v.get(k)?.as_str()?).ok()?;
    (r.num > 0).then_some(r)
}

fn tag(v: &Value, k: &str) -> Option<String> {
    v.get("tags").and_then(|t| t.get(k)).and_then(Value::as_str).map(str::to_owned)
}

fn disposition(v: &Value, k: &str) -> bool {
    v.get("disposition").and_then(|d| d.get(k)).and_then(Value::as_i64) == Some(1)
}

fn bit_depth(stream: &Value, pix_fmt: &str) -> u32 {
    if let Some(b) = num::<u32>(stream, "bits_per_raw_sample").filter(|&b| b > 0) {
        return b;
    }
    pix_fmt_depth(pix_fmt)
}

/// Bits per component from an ffmpeg pixel format name. Only a number right
/// before the endianness suffix is a depth (`yuv420p10le`, `gray12be`); the
/// digits in `yuv410p`, `nv12` or `nv16` are chroma layouts.
fn pix_fmt_depth(pix_fmt: &str) -> u32 {
    // Semi-planar high-depth formats: p010, p016, p210, p410, y210, nv20…
    match pix_fmt.trim_end_matches("le").trim_end_matches("be") {
        "p010" | "p210" | "p410" | "y210" | "nv20" | "x2rgb10" | "x2bgr10" | "v30x" | "xv30" => return 10,
        "p012" | "p212" | "p412" | "y212" | "xv36" => return 12,
        "p016" | "p216" | "p416" | "rgb48" | "bgr48" | "rgba64" | "bgra64" | "ya16" | "y216" => return 16,
        _ => {}
    }
    let Some(stem) = pix_fmt.strip_suffix("le").or_else(|| pix_fmt.strip_suffix("be")) else { return 8 };
    let digits: String =
        stem.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse::<u32>().ok().filter(|d| (9..=16).contains(d)).unwrap_or(8)
}

fn rotation(stream: &Value) -> i32 {
    let from_side = stream
        .get("side_data_list")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find_map(|sd| sd.get("rotation").and_then(Value::as_f64).map(|r| r.round() as i32)));
    let raw = from_side.or_else(|| tag(stream, "rotate").and_then(|r| r.parse().ok())).unwrap_or(0);
    raw.rem_euclid(360)
}

fn hdr(stream: &Value) -> HdrKind {
    let dovi = stream.get("side_data_list").and_then(Value::as_array).is_some_and(|l| {
        l.iter().any(|sd| sd.get("side_data_type").and_then(Value::as_str) == Some("DOVI configuration record"))
    });
    if dovi {
        return HdrKind::DolbyVision;
    }
    transfer_hdr(s(stream, "color_transfer").as_deref())
}

fn transfer_hdr(transfer: Option<&str>) -> HdrKind {
    match transfer {
        Some("smpte2084") => HdrKind::Pq,
        Some("arib-std-b67") => HdrKind::Hlg,
        _ => HdrKind::None,
    }
}

fn is_commentary_title(title: &str) -> bool {
    let t = title.to_lowercase();
    ["commentary", "kommentar", "audio description", "descriptive", "audiodeskription"].iter().any(|w| t.contains(w))
}

const TEXT_SUBS: &[&str] = &["subrip", "srt", "ass", "ssa", "mov_text", "webvtt", "text", "microdvd"];

fn parse(v: &Value) -> Option<MediaInfo> {
    let format = v.get("format")?;
    let streams = v.get("streams")?.as_array()?;
    let is = |st: &&Value, ty: &str| st.get("codec_type").and_then(Value::as_str) == Some(ty);

    // First real video stream (skip cover art / attached pictures).
    let vs = streams.iter().find(|st| is(st, "video") && !disposition(st, "attached_pic"))?;
    let pix_fmt = s(vs, "pix_fmt").unwrap_or_else(|| "unknown".into());
    let video = VideoStream {
        index: num(vs, "index")?,
        codec: s(vs, "codec_name").unwrap_or_else(|| "unknown".into()),
        profile: s(vs, "profile"),
        bit_depth: bit_depth(vs, &pix_fmt),
        pix_fmt,
        width: num(vs, "width")?,
        height: num(vs, "height")?,
        sar: rational(vs, "sample_aspect_ratio").unwrap_or(Rational::new(1, 1)),
        r_frame_rate: rational(vs, "r_frame_rate").unwrap_or(Rational::new(24, 1)),
        avg_frame_rate: rational(vs, "avg_frame_rate"),
        time_base: rational(vs, "time_base").unwrap_or(Rational::new(1, 1000)),
        color_primaries: s(vs, "color_primaries"),
        color_transfer: s(vs, "color_transfer"),
        color_space: s(vs, "color_space"),
        color_range: s(vs, "color_range"),
        field_order: s(vs, "field_order"),
        rotation: rotation(vs),
        hdr: hdr(vs),
    };

    let audio = streams
        .iter()
        .filter(|st| is(st, "audio"))
        .enumerate()
        .filter_map(|(i, st)| {
            Some(AudioStream {
                index: num(st, "index")?,
                ordinal: i as u32,
                codec: s(st, "codec_name").unwrap_or_else(|| "unknown".into()),
                profile: s(st, "profile").filter(|p| p != "LC"),
                channels: num(st, "channels").unwrap_or(2),
                channel_layout: s(st, "channel_layout"),
                sample_rate: num(st, "sample_rate").unwrap_or(48_000),
                language: tag(st, "language").filter(|l| l != "und"),
                commentary: disposition(st, "comment")
                    || disposition(st, "visual_impaired")
                    || tag(st, "title").is_some_and(|t| is_commentary_title(&t)),
                title: tag(st, "title"),
                default: disposition(st, "default"),
            })
        })
        .collect();

    let subtitles = streams
        .iter()
        .filter(|st| is(st, "subtitle"))
        .enumerate()
        .filter_map(|(i, st)| {
            let codec = s(st, "codec_name").unwrap_or_else(|| "unknown".into());
            Some(SubtitleStream {
                index: num(st, "index")?,
                ordinal: i as u32,
                text_based: TEXT_SUBS.contains(&codec.as_str()),
                codec,
                language: tag(st, "language").filter(|l| l != "und"),
                title: tag(st, "title"),
                default: disposition(st, "default"),
                forced: disposition(st, "forced"),
            })
        })
        .collect();

    Some(MediaInfo {
        container: s(format, "format_name").unwrap_or_default(),
        duration_sec: finite(format, "duration"),
        start_sec: finite(format, "start_time").unwrap_or(0.0),
        size_bytes: finite(format, "size"),
        bit_rate: finite(format, "bit_rate"),
        video,
        audio,
        subtitles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stream(transfer: Option<&str>) -> Value {
        let mut st = json!({
            "index": 0, "codec_type": "video", "codec_name": "hevc", "pix_fmt": "yuv420p10le",
            "width": 3840, "height": 2160, "r_frame_rate": "24000/1001", "time_base": "1/1000",
            "color_space": "bt2020nc"
        });
        if let Some(t) = transfer {
            st["color_transfer"] = json!(t);
        }
        json!({ "format": { "format_name": "matroska,webm" }, "streams": [st] })
    }

    fn episode(container: &str, width: u32, duration: &str, audio: &[(&str, u32)]) -> MediaInfo {
        let mut streams = vec![json!({
            "index": 0, "codec_type": "video", "codec_name": "h264", "pix_fmt": "yuv420p",
            "width": width, "height": width * 9 / 16, "r_frame_rate": "24000/1001", "time_base": "1/1000"
        })];
        for (i, (codec, channels)) in audio.iter().enumerate() {
            streams.push(json!({
                "index": i + 1, "codec_type": "audio", "codec_name": codec, "channels": channels,
                "sample_rate": "48000"
            }));
        }
        let v = json!({
            "format": { "format_name": container, "duration": duration, "start_time": "0.000000" },
            "streams": streams
        });
        parse(&v).unwrap()
    }

    #[test]
    fn a_moved_copy_is_the_same_source_another_video_is_not() {
        let old = episode("matroska,webm", 1920, "2531.400", &[("dts", 6), ("ac3", 2)]);
        assert_eq!(same_source(&old, &old), Ok(()));
        // Within half a frame (about 0.02 s at 23.976).
        assert_eq!(same_source(&old, &episode("matroska,webm", 1920, "2531.410", &[("dts", 6), ("ac3", 2)])), Ok(()));
        let differs = |new: MediaInfo| same_source(&old, &new).unwrap_err();
        assert_eq!(
            differs(episode("matroska,webm", 1920, "2400.000", &[("dts", 6), ("ac3", 2)])),
            "It's 40:00 long instead of 42:11."
        );
        assert!(differs(episode("matroska,webm", 1280, "2531.400", &[("dts", 6), ("ac3", 2)])).contains("1280×720"));
        assert!(
            differs(episode("mov,mp4,m4a,3gp,3g2,mj2", 1920, "2531.400", &[("dts", 6), ("ac3", 2)])).contains("kind")
        );
        assert!(differs(episode("matroska,webm", 1920, "2531.400", &[("dts", 6)])).contains("audio"));
    }

    #[test]
    fn a_phone_recording_is_judged_by_its_frames_not_its_ticks() {
        let phone = |duration: &str, avg: Option<&str>| {
            let mut video = json!({
                "index": 0, "codec_type": "video", "codec_name": "h264", "pix_fmt": "yuv420p",
                "width": 1920, "height": 1080, "r_frame_rate": "90000/1", "time_base": "1/90000"
            });
            if let Some(avg) = avg {
                video["avg_frame_rate"] = json!(avg);
            }
            let format = json!({ "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": duration, "start_time": "0" });
            parse(&json!({ "format": format, "streams": [video] })).unwrap()
        };
        // A remux 10 ms longer: within half a frame at 30 fps (not half a tick).
        let old = phone("61.000000", Some("30/1"));
        assert_eq!(same_source(&old, &phone("61.010000", Some("30/1"))), Ok(()));
        assert!(same_source(&old, &phone("61.040000", Some("30/1"))).is_err());
        // Without an average rate: half a frame at 240 fps at least (2.08 ms).
        let old = phone("61.000000", None);
        assert_eq!(same_source(&old, &phone("61.002000", None)), Ok(()));
        assert!(same_source(&old, &phone("61.003000", None)).is_err());
    }

    #[test]
    fn numbers_that_are_not_numbers_are_unknown() {
        let mut v = stream(None);
        v["format"] = json!({
            "format_name": "matroska,webm", "duration": "nan", "start_time": "-inf", "size": "infinity",
            "bit_rate": "NaN"
        });
        let info = parse(&v).unwrap();
        assert_eq!((info.duration_sec, info.start_sec, info.size_bytes, info.bit_rate), (None, 0.0, None, None));
        v["format"]["duration"] = json!("42.5");
        assert_eq!(parse(&v).unwrap().duration_sec, Some(42.5));
    }

    #[test]
    fn a_pixel_shape_that_is_not_positive_is_square() {
        let mut video = parse(&stream(None)).unwrap().video;
        assert_eq!(video.pixel_aspect(), 1.0);
        video.sar = Rational::new(64, 45);
        assert!((video.pixel_aspect() - 64.0 / 45.0).abs() < 1e-12);
        for (num, den) in [(0, 1), (-4, 3)] {
            video.sar = Rational::new(num, den);
            assert_eq!(video.pixel_aspect(), 1.0, "{num}/{den}");
        }
    }

    #[test]
    fn bit_depth_from_pixel_formats() {
        for (fmt, bits) in [
            ("yuv420p", 8),
            ("yuv410p", 8),
            ("nv12", 8),
            ("nv16", 8),
            ("yuv420p10le", 10),
            ("yuv422p12be", 12),
            ("gray16le", 16),
            ("p010le", 10),
            ("p016le", 16),
            ("rgb48le", 16),
            ("gbrp10le", 10),
        ] {
            assert_eq!(pix_fmt_depth(fmt), bits, "{fmt}");
        }
    }

    #[test]
    fn hdr_from_stream_tags() {
        assert_eq!(parse(&stream(Some("smpte2084"))).unwrap().video.hdr, HdrKind::Pq);
        assert_eq!(parse(&stream(Some("arib-std-b67"))).unwrap().video.hdr, HdrKind::Hlg);
        assert_eq!(parse(&stream(Some("unknown"))).unwrap().video.color_transfer, None);
    }

    fn audio(ordinal: u32, lang: Option<&str>, default: bool, title: Option<&str>) -> Value {
        let mut a = json!({ "index": ordinal + 1, "codec_type": "audio", "codec_name": "ac3", "channels": 2,
                            "disposition": { "default": i32::from(default) }, "tags": {} });
        if let Some(l) = lang {
            a["tags"]["language"] = json!(l);
        }
        if let Some(t) = title {
            a["tags"]["title"] = json!(t);
        }
        a
    }

    #[test]
    fn preferred_audio_follows_language_then_default_flag() {
        let mut v = stream(None);
        let streams = v["streams"].as_array_mut().unwrap();
        // Breaking Bad REMUX: both tracks flagged default, Hindi first.
        streams.push(audio(0, Some("hin"), true, Some("Org TV-DL Hindi DD 2.0Ch")));
        streams.push(audio(1, Some("eng"), true, Some("DTS-HD MA 5.1")));
        streams.push(audio(2, Some("eng"), false, Some("Commentary with Vince Gilligan")));
        let info = parse(&v).unwrap();
        assert!(info.audio[2].commentary);
        assert_eq!(info.preferred_audio("eng").unwrap().ordinal, 1);
        assert_eq!(info.preferred_audio("English").unwrap().ordinal, 1);
        assert_eq!(info.preferred_audio("hi").unwrap().ordinal, 0);
        // No preference or no match: the file's default.
        assert_eq!(info.preferred_audio("").unwrap().ordinal, 0);
        assert_eq!(info.preferred_audio("jpn").unwrap().ordinal, 0);
    }

    #[test]
    fn missing_container_colour_comes_from_the_first_frame() {
        let mut info = parse(&stream(None)).unwrap();
        assert_eq!(info.video.hdr, HdrKind::None);
        let frame = json!({ "color_transfer": "smpte2084", "color_primaries": "bt2020", "color_space": "bt709" });
        apply_frame_colour(&mut info.video, &frame);
        assert_eq!(info.video.hdr, HdrKind::Pq);
        assert_eq!(info.video.color_primaries.as_deref(), Some("bt2020"));
        // Container values win where present.
        assert_eq!(info.video.color_space.as_deref(), Some("bt2020nc"));
    }
}
