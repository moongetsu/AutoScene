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

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioStream {
    pub index: u32,
    /// Position among audio streams (`0:a:N`).
    pub ordinal: u32,
    pub codec: String,
    pub channels: u32,
    pub channel_layout: Option<String>,
    pub sample_rate: u32,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
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
}

pub fn probe(tools: &Tools, path: &Path) -> MediaResult<MediaInfo> {
    let err = |message: String| MediaError::Probe { path: path.display().to_string(), message };
    let out = tools.ffprobe().args(["-print_format", "json", "-show_format", "-show_streams"]).arg(path).output()?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(err(if msg.is_empty() { "unreadable file".into() } else { msg }));
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| err(e.to_string()))?;
    parse(&v).ok_or_else(|| MediaError::NoVideo(path.display().to_string()))
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
    if let Some(b) = num::<u32>(stream, "bits_per_raw_sample") {
        return b;
    }
    for (needle, bits) in [("16", 16), ("14", 14), ("12", 12), ("10", 10), ("p010", 10)] {
        if pix_fmt.contains(needle) {
            return bits;
        }
    }
    8
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
    match s(stream, "color_transfer").as_deref() {
        Some("smpte2084") => HdrKind::Pq,
        Some("arib-std-b67") => HdrKind::Hlg,
        _ => HdrKind::None,
    }
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
                channels: num(st, "channels").unwrap_or(2),
                channel_layout: s(st, "channel_layout"),
                sample_rate: num(st, "sample_rate").unwrap_or(48_000),
                language: tag(st, "language").filter(|l| l != "und"),
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
        duration_sec: num(format, "duration"),
        start_sec: num(format, "start_time").unwrap_or(0.0),
        size_bytes: num(format, "size"),
        bit_rate: num(format, "bit_rate"),
        video,
        audio,
        subtitles,
    })
}
