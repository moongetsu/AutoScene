//! Export presets: what editors actually ask for.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Mov,
    Mp4,
}

impl Container {
    pub fn ext(self) -> &'static str {
        match self {
            Self::Mov => "mov",
            Self::Mp4 => "mp4",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProResProfile {
    Proxy,
    Lt,
    Standard,
    Hq,
    P4444,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DnxProfile {
    /// 8-bit 4:2:2.
    Hq,
    /// 10-bit 4:2:2.
    Hqx,
}

/// Keyframe spacing for long-GOP codecs. Editors scrub short GOPs far better.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Gop {
    /// About one keyframe per second, no B-frames — smooth scrubbing in NLEs.
    Editing,
    /// Every frame a keyframe (largest files, perfect for Twixtor/AE).
    AllIntra,
    /// Encoder defaults (smallest files).
    Delivery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EncoderPref {
    /// Hardware encoder when available, else software.
    Auto,
    Software,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "codec", rename_all = "snake_case")]
pub enum VideoCodec {
    Prores {
        profile: ProResProfile,
    },
    Dnxhr {
        profile: DnxProfile,
    },
    /// `quality`: 0 (lossless-ish) … 51 (worst), CRF-like. 16 is visually lossless.
    H264 {
        quality: u8,
        gop: Gop,
        encoder: EncoderPref,
    },
    Hevc {
        quality: u8,
        gop: Gop,
        encoder: EncoderPref,
        ten_bit: bool,
    },
    /// Bit-Perfect: every frame exactly as the source decodes to, without
    /// loss (see `as_export::lossless`).
    Lossless,
}

impl VideoCodec {
    pub fn is_intra(&self) -> bool {
        match self {
            Self::Prores { .. } | Self::Dnxhr { .. } => true,
            Self::H264 { gop, .. } | Self::Hevc { gop, .. } => *gop == Gop::AllIntra,
            Self::Lossless => false,
        }
    }

    /// Codecs whose output stays 8-bit SDR need HDR sources tone mapped.
    pub fn supports_hdr(&self) -> bool {
        match self {
            Self::Prores { .. } => true,
            Self::Dnxhr { profile } => *profile == DnxProfile::Hqx,
            Self::Hevc { ten_bit, .. } => *ten_bit,
            Self::H264 { .. } => false,
            Self::Lossless => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "codec", rename_all = "snake_case")]
pub enum AudioCodec {
    None,
    /// 24-bit PCM (MOV only) — what NLEs prefer.
    Pcm24,
    Aac {
        kbps: u32,
    },
    /// The source's samples as they are: PCM at its rate, in its channels
    /// and its sample format.
    Lossless,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HdrMode {
    /// Keep HDR when the codec can carry it, otherwise tone map to SDR.
    Auto,
    /// Always tone map to SDR BT.709 (for SDR-only edits).
    ToneMap,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub container: Container,
    pub video: VideoCodec,
    pub audio: AudioCodec,
    /// Cap output height (keeps aspect). `None` = source resolution.
    pub max_height: Option<u32>,
    pub hdr: HdrMode,
    /// Mix surround sound down to stereo (phone editors don't handle 5.1).
    #[serde(default)]
    pub stereo: bool,
    pub builtin: bool,
}

pub fn builtin_presets() -> Vec<ExportPreset> {
    vec![
        ExportPreset {
            id: "prores-hq".into(),
            name: "ProRes 422 HQ".into(),
            description: "Edit-grade intra codec for After Effects, Premiere, Final Cut and Resolve. Large files."
                .into(),
            container: Container::Mov,
            video: VideoCodec::Prores { profile: ProResProfile::Hq },
            audio: AudioCodec::Pcm24,
            max_height: None,
            hdr: HdrMode::Auto,
            stereo: false,
            builtin: true,
        },
        ExportPreset {
            id: "dnxhr-hq".into(),
            name: "DNxHR HQ".into(),
            description: "Edit-grade intra codec, popular with Premiere and Resolve on Windows.".into(),
            container: Container::Mov,
            video: VideoCodec::Dnxhr { profile: DnxProfile::Hq },
            audio: AudioCodec::Pcm24,
            max_height: None,
            hdr: HdrMode::ToneMap,
            stereo: false,
            builtin: true,
        },
        ExportPreset {
            id: "h264-edit".into(),
            name: "H.264 · Editing".into(),
            description: "Visually lossless H.264 with a short GOP for smooth scrubbing. Works everywhere.".into(),
            container: Container::Mp4,
            video: VideoCodec::H264 { quality: 16, gop: Gop::Editing, encoder: EncoderPref::Auto },
            audio: AudioCodec::Aac { kbps: 320 },
            max_height: None,
            hdr: HdrMode::ToneMap,
            stereo: false,
            builtin: true,
        },
        ExportPreset {
            id: "h264-mobile".into(),
            name: "H.264 · Mobile (CapCut, Alight Motion)".into(),
            description: "1080p H.264 with stereo sound that imports cleanly into phone editors.".into(),
            container: Container::Mp4,
            video: VideoCodec::H264 { quality: 18, gop: Gop::Editing, encoder: EncoderPref::Auto },
            audio: AudioCodec::Aac { kbps: 256 },
            max_height: Some(1080),
            hdr: HdrMode::ToneMap,
            stereo: true,
            builtin: true,
        },
        ExportPreset {
            id: "h264-intra".into(),
            name: "H.264 · All-Intra (Twixtor)".into(),
            description: "Every frame a keyframe — ideal for Twixtor and frame-by-frame work.".into(),
            container: Container::Mp4,
            video: VideoCodec::H264 { quality: 14, gop: Gop::AllIntra, encoder: EncoderPref::Auto },
            audio: AudioCodec::Aac { kbps: 320 },
            max_height: None,
            hdr: HdrMode::ToneMap,
            stereo: false,
            builtin: true,
        },
        ExportPreset {
            id: "hevc10".into(),
            name: "HEVC 10-bit (keeps HDR)".into(),
            description: "Smaller files that preserve 10-bit colour and HDR.".into(),
            container: Container::Mp4,
            video: VideoCodec::Hevc { quality: 18, gop: Gop::Editing, encoder: EncoderPref::Auto, ten_bit: true },
            audio: AudioCodec::Aac { kbps: 320 },
            max_height: None,
            hdr: HdrMode::Auto,
            stereo: false,
            builtin: true,
        },
        ExportPreset {
            id: "bit-perfect".into(),
            name: "Bit-Perfect (lossless)".into(),
            description: "Every frame exactly as in the episode: the same pixels, colour and HDR, and its own sound. Stored without loss as HEVC, which After Effects and other editors open (FFV1 for the rare pictures HEVC can't hold). Very large files."
                .into(),
            container: Container::Mov,
            video: VideoCodec::Lossless,
            audio: AudioCodec::Lossless,
            max_height: None,
            hdr: HdrMode::Auto,
            stereo: false,
            builtin: true,
        },
    ]
}
