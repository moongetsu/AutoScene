//! How big an export will be, roughly — "about 18 GB" before it starts.
//! Intra codecs (ProRes, DNxHR) write a near-constant amount per frame, so
//! their figures hold; long-GOP ones depend on the picture, so theirs are
//! typical for film and series, not promised.

use as_media::MediaInfo;

use crate::preset::{AudioCodec, DnxProfile, ExportPreset, Gop, ProResProfile, VideoCodec};

/// Bits per pixel and frame.
fn video_bpp(v: &VideoCodec) -> f64 {
    match v {
        // Apple's target data rates (ProRes white paper), at 1920×1080 29.97.
        VideoCodec::Prores { profile } => match profile {
            ProResProfile::Proxy => 0.72,
            ProResProfile::Lt => 1.64,
            ProResProfile::Standard => 2.37,
            ProResProfile::Hq => 3.54,
            ProResProfile::P4444 => 5.31,
        },
        // Avid's DNxHR HQ/HQX at 1080p30 run at about ProRes HQ's rate.
        VideoCodec::Dnxhr { profile: DnxProfile::Hq | DnxProfile::Hqx } => 3.5,
        VideoCodec::H264 { quality, gop, .. } => long_gop(*quality, *gop),
        // HEVC needs roughly 60 % of H.264's rate for the same look.
        VideoCodec::Hevc { quality, gop, .. } => 0.6 * long_gop(*quality, *gop),
    }
}

/// CRF-like quality: the rate about doubles every 6 steps down; 18 is about
/// 0.25 bits per pixel for film. Editing GOPs (a keyframe a second, no
/// B-frames) and all-intra cost more.
fn long_gop(quality: u8, gop: Gop) -> f64 {
    let base = 0.25 * 2f64.powf((18.0 - f64::from(quality)) / 6.0);
    base * match gop {
        Gop::Delivery => 1.0,
        Gop::Editing => 1.5,
        Gop::AllIntra => 4.0,
    }
}

/// Bytes an export of `frames` frames (`seconds` long) of `info` makes with
/// `preset`, with audio track `audio_ordinal` (the default one when `None`).
pub fn estimate_bytes(
    preset: &ExportPreset,
    info: &MediaInfo,
    audio_ordinal: Option<u32>,
    frames: u64,
    seconds: f64,
) -> f64 {
    let (w, h) = (f64::from(info.video.width), f64::from(info.video.height));
    let scale = preset.max_height.map_or(1.0, |max| (f64::from(max) / h.max(1.0)).min(1.0));
    let video = video_bpp(&preset.video) * (w * scale) * (h * scale) * frames as f64 / 8.0;
    let track = audio_ordinal.and_then(|o| info.audio.iter().find(|a| a.ordinal == o)).or_else(|| info.default_audio());
    let audio = match (preset.audio, track) {
        (AudioCodec::None, _) | (_, None) => 0.0,
        (AudioCodec::Pcm24, Some(a)) => {
            let channels = if preset.stereo { a.channels.min(2) } else { a.channels };
            f64::from(a.sample_rate) * 3.0 * f64::from(channels) * seconds
        }
        (AudioCodec::Aac { kbps }, Some(_)) => f64::from(kbps) * 1000.0 / 8.0 * seconds,
    };
    video + audio
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_presets;

    fn info(w: u32, h: u32) -> MediaInfo {
        use as_core::Rational;
        use as_media::{AudioStream, HdrKind, VideoStream};
        MediaInfo {
            container: "matroska,webm".into(),
            duration_sec: Some(3600.0),
            start_sec: 0.0,
            size_bytes: None,
            bit_rate: None,
            video: VideoStream {
                index: 0,
                codec: "h264".into(),
                profile: None,
                pix_fmt: "yuv420p".into(),
                bit_depth: 8,
                width: w,
                height: h,
                sar: Rational::new(1, 1),
                r_frame_rate: Rational::new(30000, 1001),
                avg_frame_rate: None,
                time_base: Rational::new(1, 1000),
                color_primaries: None,
                color_transfer: None,
                color_space: None,
                color_range: None,
                field_order: None,
                rotation: 0,
                hdr: HdrKind::None,
            },
            audio: vec![AudioStream {
                index: 1,
                ordinal: 0,
                codec: "aac".into(),
                profile: None,
                channels: 6,
                channel_layout: Some("5.1".into()),
                sample_rate: 48000,
                language: Some("eng".into()),
                title: None,
                default: true,
                commentary: false,
            }],
            subtitles: vec![],
        }
    }

    fn preset(id: &str) -> ExportPreset {
        builtin_presets().into_iter().find(|p| p.id == id).unwrap_or_else(|| panic!("preset {id}"))
    }

    const HOUR_FRAMES: u64 = 107_892; // 3600 s at 29.97

    #[test]
    fn prores_hq_runs_at_apples_rate() {
        // ~220 Mbit/s at 1080p29.97: ~99 GB of picture an hour.
        let video = estimate_bytes(&preset("prores-hq"), &info(1920, 1080), None, HOUR_FRAMES, 3600.0);
        let audio = 48_000.0 * 3.0 * 6.0 * 3600.0;
        assert!((video - audio - 99e9).abs() < 3e9, "{} GB", (video - audio) / 1e9);
    }

    #[test]
    fn sizes_follow_the_codec_and_picture() {
        let hd = info(1920, 1080);
        let est = |id: &str, i: &MediaInfo| estimate_bytes(&preset(id), i, None, 2400, 100.0);
        assert!(est("prores-hq", &hd) > est("h264-intra", &hd));
        assert!(est("h264-intra", &hd) > est("h264-edit", &hd));
        assert!(est("h264-edit", &hd) > est("hevc10", &hd) * 0.5);
        // Four times the pixels, about four times the picture.
        let uhd = est("prores-hq", &info(3840, 2160)) / est("prores-hq", &hd);
        assert!((3.5..4.1).contains(&uhd), "{uhd}");
        // A capped height scales it down.
        let mut small = preset("prores-hq");
        small.max_height = Some(540);
        let quarter = estimate_bytes(&small, &hd, None, 2400, 100.0) / est("prores-hq", &hd);
        assert!(quarter < 0.4, "{quarter}");
    }

    #[test]
    fn audio_follows_the_track_and_stereo() {
        let hd = info(1920, 1080);
        let mut pcm = preset("prores-hq");
        let surround = estimate_bytes(&pcm, &hd, None, 0, 100.0);
        assert_eq!(surround, 48_000.0 * 3.0 * 6.0 * 100.0);
        pcm.stereo = true;
        assert_eq!(estimate_bytes(&pcm, &hd, None, 0, 100.0), 48_000.0 * 3.0 * 2.0 * 100.0);
        // A track that isn't there: no audio.
        let mut silent = hd.clone();
        silent.audio.clear();
        assert_eq!(estimate_bytes(&pcm, &silent, None, 0, 100.0), 0.0);
    }
}
