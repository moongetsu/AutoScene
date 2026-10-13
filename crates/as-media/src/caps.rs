//! What this machine's ffmpeg can actually do — verified by smoke tests, not
//! just by what the build lists (an `h264_nvenc` entry means nothing on an AMD
//! machine).

use std::collections::HashSet;
use std::thread;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::Tools;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum CodecFamily {
    H264,
    Hevc,
    Av1,
    Prores,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HwEncoder {
    pub name: String,
    pub family: CodecFamily,
    pub vendor: Vendor,
}

/// Hardware decode setup for analysis/index passes.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HwDecode {
    pub hwaccel: String,
    /// Keep frames on the GPU when nothing downstream needs them in RAM.
    pub output_format: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Caps {
    pub decode: Option<HwDecode>,
    pub hw_encoders: Vec<HwEncoder>,
    /// Software encoders present in this build (libx264, libx265, prores_ks, …).
    pub sw_encoders: Vec<String>,
}

const HW_ENCODERS: &[(&str, CodecFamily, Vendor)] = &[
    ("h264_nvenc", CodecFamily::H264, Vendor::Nvidia),
    ("hevc_nvenc", CodecFamily::Hevc, Vendor::Nvidia),
    ("av1_nvenc", CodecFamily::Av1, Vendor::Nvidia),
    ("h264_amf", CodecFamily::H264, Vendor::Amd),
    ("hevc_amf", CodecFamily::Hevc, Vendor::Amd),
    ("av1_amf", CodecFamily::Av1, Vendor::Amd),
    ("h264_qsv", CodecFamily::H264, Vendor::Intel),
    ("hevc_qsv", CodecFamily::Hevc, Vendor::Intel),
    ("av1_qsv", CodecFamily::Av1, Vendor::Intel),
    ("h264_videotoolbox", CodecFamily::H264, Vendor::Apple),
    ("hevc_videotoolbox", CodecFamily::Hevc, Vendor::Apple),
    ("prores_videotoolbox", CodecFamily::Prores, Vendor::Apple),
];

const SW_ENCODERS: &[&str] = &["libx264", "libx265", "libsvtav1", "prores_ks", "dnxhd", "aac", "pcm_s24le"];

/// Decode accelerators in preference order: (hwaccel, device type, GPU-resident format).
const DECODERS: &[(&str, &str, &str)] = if cfg!(target_os = "macos") {
    &[("videotoolbox", "videotoolbox", "videotoolbox_vld")]
} else if cfg!(windows) {
    &[("cuda", "cuda", "cuda"), ("d3d11va", "d3d11va", "d3d11")]
} else {
    &[("cuda", "cuda", "cuda"), ("vaapi", "vaapi", "vaapi")]
};

impl Caps {
    pub fn detect(tools: &Tools) -> Self {
        let listed = |flag: &str| -> String {
            tools
                .ffmpeg()
                .arg(flag)
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        };
        let enc_list = listed("-encoders");
        let present: HashSet<&str> = enc_list.lines().filter_map(|l| l.split_whitespace().nth(1)).collect();
        let hwaccels = listed("-hwaccels");

        let candidates: Vec<_> = HW_ENCODERS.iter().filter(|(n, ..)| present.contains(n)).collect();
        let hw_encoders: Vec<HwEncoder> = thread::scope(|s| {
            let handles: Vec<_> = candidates
                .iter()
                .map(|&&(name, family, vendor)| {
                    s.spawn(move || smoke_encode(tools, name).then(|| HwEncoder { name: name.into(), family, vendor }))
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect()
        });

        let decode = DECODERS
            .iter()
            .filter(|(accel, ..)| hwaccels.lines().any(|l| l.trim() == *accel))
            .find(|(_, device, _)| smoke_device(tools, device))
            .map(|(accel, _, fmt)| HwDecode { hwaccel: (*accel).into(), output_format: (*fmt).into() });

        let sw_encoders = SW_ENCODERS.iter().filter(|n| present.contains(*n)).map(|n| (*n).to_string()).collect();
        let caps = Self { decode, hw_encoders, sw_encoders };
        tracing::info!(?caps, "media capabilities");
        caps
    }

    pub fn has_sw(&self, name: &str) -> bool {
        self.sw_encoders.iter().any(|e| e == name)
    }

    /// Best working hardware encoder for a codec family (NVIDIA first: fastest, best quality).
    pub fn hw_encoder(&self, family: CodecFamily) -> Option<&HwEncoder> {
        self.hw_encoders.iter().find(|e| e.family == family)
    }
}

fn smoke_encode(tools: &Tools, encoder: &str) -> bool {
    tools
        .ffmpeg()
        .args(["-v", "error", "-f", "lavfi", "-i", "color=c=gray:s=320x240:r=24", "-frames:v", "3"])
        .args(["-pix_fmt", if encoder.contains("videotoolbox") { "nv12" } else { "yuv420p" }])
        .args(["-c:v", encoder, "-f", "null", "-"])
        .output()
        .is_ok_and(|o| o.status.success())
}

fn smoke_device(tools: &Tools, device: &str) -> bool {
    tools
        .ffmpeg()
        .args(["-v", "error", "-init_hw_device", &format!("{device}=hw"), "-f", "lavfi", "-i", "nullsrc=s=64x64"])
        .args(["-frames:v", "1", "-f", "null", "-"])
        .output()
        .is_ok_and(|o| o.status.success())
}
