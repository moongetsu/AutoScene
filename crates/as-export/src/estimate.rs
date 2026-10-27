//! How big an export will be, roughly — "about 18 GB" before it starts.
//!
//! Measured on real exports (October 2026): the app's own encodings of a
//! film-grain series (Breaking Bad, from the BluRay) and of a clean digital
//! one (The Blacklist, a web release), 1080p at 23.976 fps.
//! - Intra codecs (ProRes, DNxHR) write a near-constant amount per frame:
//!   their figures hold to a few percent.
//! - The software encoders' CRF spends what the picture needs (grain up to
//!   four times what a clean picture costs): theirs are the middle of the two.
//! - NVENC's constant quality does the same, with less between grain and a
//!   clean picture (all-intra a little, with a GOP up to 2.6 times): its
//!   figures are the middle too (see [`nvenc_bpp`]), under the cap the
//!   export gives it.
//! - Other hardware encoders are counted like the software ones.
//!
//! Every clip is counted as the export makes it: upright, in square pixels,
//! under the preset's height cap — in a file joined from the clips, at the
//! size of that file.

use as_media::{Caps, CodecFamily, MediaInfo, Vendor};

use crate::clip::{AUDIO_RATE, encoder_fps, nvenc_cq, nvenc_maxrate};
use crate::merge::{audio_stream, file_size, natural_size};
use crate::preset::{AudioCodec, DnxProfile, EncoderPref, ExportPreset, Gop, ProResProfile, VideoCodec};

/// A clip of an export, as its size is worked out.
#[derive(Clone, Copy, Debug)]
pub struct EstimateClip<'a> {
    pub info: &'a MediaInfo,
    /// Its sound track (`None`: the file's default).
    pub audio_ordinal: Option<u32>,
    pub frames: u64,
    pub seconds: f64,
    /// Its episode's frame rate.
    pub fps: f64,
}

/// Bits per pixel and frame of the intra codecs.
fn intra_bpp(v: &VideoCodec) -> Option<f64> {
    Some(match v {
        // Apple's target data rates (ProRes white paper), at 1920×1080 29.97;
        // measured 3.53 (grain) and 3.30 (clean) for HQ.
        VideoCodec::Prores { profile } => match profile {
            ProResProfile::Proxy => 0.72,
            ProResProfile::Lt => 1.64,
            ProResProfile::Standard => 2.37,
            ProResProfile::Hq => 3.54,
            ProResProfile::P4444 => 5.31,
        },
        // Avid's DNxHR HQ/HQX at 1080p30 run at about ProRes HQ's rate (3.54 measured).
        VideoCodec::Dnxhr { profile: DnxProfile::Hq | DnxProfile::Hqx } => 3.5,
        VideoCodec::H264 { .. } | VideoCodec::Hevc { .. } => return None,
    })
}

/// Bits per pixel and frame from libx264/libx265 at CRF `quality`: 0.24 at
/// 18 with a keyframe a second, about doubling every 6 steps down. Measured
/// (grain/clean, x264): editing 18 0.47/0.12, 16 0.60/0.16; all-intra 14
/// 0.87/0.64; encoder defaults 18 0.42/0.09. x265 at the same CRF: 0.47/0.09
/// (editing), 0.31/0.06 (defaults).
fn crf_bpp(quality: u8, gop: Gop, hevc: bool) -> f64 {
    let base = 0.24 * 2f64.powf((18.0 - f64::from(quality)) / 6.0);
    let gop = match gop {
        Gop::Delivery => 0.8,
        Gop::Editing => 1.0,
        Gop::AllIntra => 2.0,
    };
    base * gop * if hevc { 0.85 } else { 1.0 }
}

/// Bits per pixel and frame NVENC writes at constant quality `cq`: the
/// middle of what real scenepacks took, halving every 6 steps up all-intra
/// and every 5.3 with a GOP (as 1080p pieces of both series did from CQ 10
/// to 30). Three whole packs at 1080p23.976 (Breaking Bad, two of The
/// Blacklist), bits a pixel: all-intra CQ 21 1.21, 1.04, 1.34; editing CQ 17
/// 0.62, 0.40, 0.39; CQ 19 0.49, 0.29, 0.27; HEVC CQ 19 0.46, 0.20, 0.18.
/// Encoder defaults take 0.85 of an editing GOP. Very short clips differ:
/// 3 frames take about half all-intra and up to twice with a GOP.
fn nvenc_bpp(cq: u8, gop: Gop, hevc: bool) -> f64 {
    let steps = 18.0 - f64::from(cq);
    let bpp = match gop {
        Gop::AllIntra => 1.67 * 2f64.powf(steps / 6.0),
        Gop::Editing => 0.43 * 2f64.powf(steps / 5.3),
        Gop::Delivery => 0.85 * 0.43 * 2f64.powf(steps / 5.3),
    };
    bpp * if hevc { 0.755 } else { 1.0 }
}

/// Bits NVENC writes of a clip of `w`×`h` pictures, capped for `fastest`
/// frames a second: what its quality takes, but no more than the cap lets
/// through. A stream at the cap starts with a burst of about 0.8 seconds'
/// worth (measured: 0.57 with a GOP, 0.76 all-intra), then keeps to it.
fn nvenc_bits(quality: u8, gop: Gop, hevc: bool, (w, h): (u32, u32), fastest: f64, clip: &EstimateClip<'_>) -> f64 {
    let rate = nvenc_maxrate(gop, hevc, (w, h), fastest);
    let pixels = f64::from(w) * f64::from(h) * clip.frames as f64;
    (nvenc_bpp(nvenc_cq(quality, gop), gop, hevc) * pixels).min(rate * (0.8 + clip.seconds))
}

/// Bytes of picture `preset` makes of a clip shown at `size`, in a file
/// whose fastest frame rate is `fastest` (`clip::encoder_fps`).
fn video_bytes(preset: &ExportPreset, caps: &Caps, size: (u32, u32), fastest: f64, clip: &EstimateClip<'_>) -> f64 {
    let pixels = f64::from(size.0) * f64::from(size.1) * clip.frames as f64;
    match &preset.video {
        VideoCodec::H264 { quality, gop, encoder } | VideoCodec::Hevc { quality, gop, encoder, .. } => {
            let hevc = matches!(preset.video, VideoCodec::Hevc { .. });
            let family = if hevc { CodecFamily::Hevc } else { CodecFamily::H264 };
            // The encoder the export will use (`clip::resolve_encoding`).
            let nvenc =
                *encoder == EncoderPref::Auto && caps.hw_encoder(family).is_some_and(|e| e.vendor == Vendor::Nvidia);
            if nvenc {
                nvenc_bits(*quality, *gop, hevc, size, fastest, clip) / 8.0
            } else {
                crf_bpp(*quality, *gop, hevc) * pixels / 8.0
            }
        }
        v => intra_bpp(v).unwrap_or_default() * pixels / 8.0,
    }
}

/// Bytes of `channels` of sound, `seconds` long, as `preset` stores it
/// (resampled to 48 kHz).
fn audio_bytes(preset: &ExportPreset, channels: u32, seconds: f64) -> f64 {
    match preset.audio {
        AudioCodec::None => 0.0,
        AudioCodec::Pcm24 => f64::from(AUDIO_RATE) * 3.0 * f64::from(channels) * seconds,
        AudioCodec::Aac { kbps } => f64::from(kbps) * 1000.0 / 8.0 * seconds,
    }
}

/// Bytes `clips` make with `preset` on a machine with `caps`: a file each,
/// or `merged` into one file (as `merge::plan_merge` shapes it).
pub fn estimate_bytes(preset: &ExportPreset, caps: &Caps, clips: &[EstimateClip<'_>], merged: bool) -> f64 {
    let size = |c: &EstimateClip<'_>| natural_size(c.info, preset.max_height);
    let rate = |c: &EstimateClip<'_>| encoder_fps(c.info, c.fps);
    let channels = |c: &EstimateClip<'_>| {
        // A stereo preset makes two channels of any sound, mono too.
        audio_stream(c.info, c.audio_ordinal).map(|a| if preset.stereo { 2 } else { a.channels })
    };
    if !merged {
        return clips
            .iter()
            .map(|c| {
                video_bytes(preset, caps, size(c), rate(c), c)
                    + channels(c).map_or(0.0, |n| audio_bytes(preset, n, c.seconds))
            })
            .sum();
    }
    // One picture size for every part; one sound for the whole file, silent
    // where a clip has none (`merge::sound_plan`): its channels when every
    // clip's sound has as many, else stereo.
    let Some(picture) = file_size(clips.iter().map(|c| (size(c), c.frames))) else { return 0.0 };
    let fastest = clips.iter().map(rate).fold(0.0, f64::max);
    let tracks: Vec<u32> = clips.iter().filter_map(channels).collect();
    let sound = match tracks.first() {
        None => None,
        Some(&n) if tracks.iter().all(|&m| m == n) => Some(n),
        Some(_) => Some(2),
    };
    clips
        .iter()
        .map(|c| {
            video_bytes(preset, caps, picture, fastest, c) + sound.map_or(0.0, |n| audio_bytes(preset, n, c.seconds))
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin_presets;
    use as_core::Rational;
    use as_media::HwEncoder;

    fn info(w: u32, h: u32) -> MediaInfo {
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

    /// `p` without its sound.
    fn silent(id: &str) -> ExportPreset {
        ExportPreset { audio: AudioCodec::None, ..preset(id) }
    }

    /// A machine with an NVIDIA card (the encoders a smoke test finds there).
    fn nvidia() -> Caps {
        let hw = |name: &str, family| HwEncoder { name: name.into(), family, vendor: Vendor::Nvidia };
        Caps {
            hw_encoders: vec![hw("h264_nvenc", CodecFamily::H264), hw("hevc_nvenc", CodecFamily::Hevc)],
            ..Caps::default()
        }
    }

    const FILM: f64 = 24000.0 / 1001.0;

    /// `n` clips of `info`, `frames` each at `fps`.
    fn clips(info: &MediaInfo, n: usize, frames: u64, fps: f64) -> Vec<EstimateClip<'_>> {
        let clip = EstimateClip { info, audio_ordinal: None, frames, seconds: frames as f64 / fps, fps };
        vec![clip; n]
    }

    #[test]
    fn prores_hq_runs_at_apples_rate() {
        // ~220 Mbit/s at 1080p29.97: ~99 GB of picture an hour.
        let fps = 30000.0 / 1001.0;
        let video =
            estimate_bytes(&silent("prores-hq"), &Caps::default(), &clips(&info(1920, 1080), 1, 107_892, fps), false);
        assert!((video - 99e9).abs() < 3e9, "{} GB", video / 1e9);
    }

    #[test]
    fn real_scenepacks_came_out_as_estimated() {
        let hd = info(1920, 1080);
        // Jesse's 130 shots of Breaking Bad as ProRes HQ in one file, with
        // the 5.1 sound: 13,953 frames, 13,230,246,959 bytes.
        let prores = estimate_bytes(&preset("prores-hq"), &nvidia(), &clips(&hd, 1, 13_953, FILM), true);
        assert!((prores / 13_230_246_959.0 - 1.0).abs() < 0.03, "{prores}");
        // Three packs made by NVENC, each in one file (`tests/calibrate.rs`):
        // the same 130 shots, and 124 and 167 shots of The Blacklist; bytes
        // with each preset. All-intra takes about the same of every picture;
        // with a GOP, Breaking Bad's grain takes up to 2.6 times what The
        // Blacklist's clean picture does, and the figure is their middle.
        let packs: [(u64, u64, [f64; 4]); 3] = [
            (130, 13_953, [4_410_629_335.0, 2_250_807_948.0, 1_788_467_809.0, 1_683_101_757.0]),
            (124, 9_060, [2_455_099_470.0, 959_774_575.0, 686_833_539.0, 488_905_509.0]),
            (167, 14_888, [5_191_584_783.0, 1_543_727_950.0, 1_041_304_319.0, 703_099_331.0]),
        ];
        // The figure lies between the packs, at most this far from any.
        let within = [("h264-intra", 1.15), ("h264-edit", 1.3), ("h264-mobile", 1.45), ("hevc10", 1.65)];
        for (i, (id, most)) in within.into_iter().enumerate() {
            let ratios: Vec<f64> = packs
                .iter()
                .map(|&(shots, frames, bytes)| {
                    let mut pack = clips(&hd, shots as usize, frames / shots, FILM);
                    pack.extend(clips(&hd, 1, frames % shots, FILM));
                    estimate_bytes(&preset(id), &nvidia(), &pack, true) / bytes[i]
                })
                .collect();
            assert!(ratios.iter().any(|&r| r < 1.0) && ratios.iter().any(|&r| r > 1.0), "{id}: {ratios:.3?}");
            assert!(ratios.iter().all(|&r| (1.0 / most..most).contains(&r)), "{id}: {ratios:.3?}");
        }
        // Software x264 takes about two thirds of NVENC's all-intra.
        let shots = clips(&hd, 130, 107, FILM);
        let ratio = estimate_bytes(&preset("h264-intra"), &Caps::default(), &shots, true)
            / estimate_bytes(&preset("h264-intra"), &nvidia(), &shots, true);
        assert!((0.55..0.8).contains(&ratio), "{ratio}");
    }

    #[test]
    fn each_figure_lies_within_what_real_pictures_took() {
        // Bits per pixel and frame the app's encodings took of the grainy and
        // the clean series (1080p23.976; software: 16 pieces of 96 frames,
        // NVENC: the three packs above, without their sound): the figure lies
        // between the two, or within a tenth of them.
        let hd = info(1920, 1080);
        let pieces = clips(&hd, 16, 96, FILM);
        let bpp = |id: &str, caps: &Caps| {
            estimate_bytes(&silent(id), caps, &pieces, false) * 8.0 / (1920.0 * 1080.0 * 16.0 * 96.0)
        };
        let measured: [(&str, bool, f64, f64); 10] = [
            ("h264-intra", false, 0.635, 0.872),
            ("h264-intra", true, 1.039, 1.339),
            ("h264-edit", false, 0.157, 0.598),
            ("h264-edit", true, 0.394, 0.616),
            ("h264-mobile", false, 0.118, 0.472),
            ("h264-mobile", true, 0.265, 0.489),
            ("hevc10", false, 0.092, 0.468),
            ("hevc10", true, 0.176, 0.459),
            ("prores-hq", false, 3.304, 3.528),
            ("dnxhr-hq", false, 3.540, 3.540),
        ];
        for (id, hw, low, high) in measured {
            let got = bpp(id, &if hw { nvidia() } else { Caps::default() });
            assert!((low * 0.9..=high * 1.1).contains(&got), "{id} (nvenc {hw}): {got:.3} bpp, measured {low}–{high}");
        }
    }

    #[test]
    fn nvenc_follows_its_quality_up_to_its_cap() {
        let hd = info(1920, 1080);
        let with = |quality: u8, info: &MediaInfo, fps: f64, seconds: f64| {
            let mut p = silent("h264-intra");
            p.video = VideoCodec::H264 { quality, gop: Gop::AllIntra, encoder: EncoderPref::Auto };
            let frames = (fps * seconds).round() as u64;
            estimate_bytes(&p, &nvidia(), &clips(info, 1, frames, fps), false)
        };
        // Six steps of quality, twice the bytes; four times the pixels, four times.
        let ratio = with(8, &hd, FILM, 60.0) / with(14, &hd, FILM, 60.0);
        assert!((ratio - 2.0).abs() < 1e-9, "{ratio}");
        let ratio = with(14, &info(3840, 2160), FILM, 60.0) / with(14, &hd, FILM, 60.0);
        assert!((ratio - 4.0).abs() < 1e-9, "{ratio}");
        // At the best quality a UHD picture at 60 fps meets the cap: level
        // 5.2's 300 Mbit/s, after a burst of 0.8 seconds' worth.
        assert_eq!(with(0, &info(3840, 2160), 60.0, 60.0), 300e6 * 60.8 / 8.0);
        // Asking for the software encoder gets its figures.
        let mut sw = preset("h264-intra");
        if let VideoCodec::H264 { encoder, .. } = &mut sw.video {
            *encoder = EncoderPref::Software;
        }
        let pieces = clips(&hd, 3, 240, FILM);
        assert_eq!(
            estimate_bytes(&sw, &nvidia(), &pieces, false),
            estimate_bytes(&sw, &Caps::default(), &pieces, false)
        );
    }

    #[test]
    fn very_short_nvenc_clips_are_counted_like_longer_ones() {
        // Clips of 3 to 12 frames, 1080p23.976 (16 of each, grainy and
        // clean series): bytes a clip. NVENC writes a stream's first frames
        // small all-intra and large with a GOP; the figure counts every frame
        // alike, so it is above them all-intra (by less than twice) and at
        // most a tenth below them with a GOP.
        let hd = info(1920, 1080);
        let measured: [(&str, u64, f64, f64); 7] = [
            ("h264-intra", 3, 365_985.0, 491_181.0),
            ("h264-intra", 6, 741_855.0, 997_367.0),
            ("h264-intra", 12, 2_103_556.0, 2_693_657.0),
            ("h264-edit", 3, 402_016.0, 657_900.0),
            ("h264-edit", 12, 1_383_763.0, 2_471_940.0),
            ("hevc10", 3, 238_000.0, 508_675.0),
            ("hevc10", 12, 749_804.0, 1_591_109.0),
        ];
        for (id, frames, low, high) in measured {
            let got = estimate_bytes(&silent(id), &nvidia(), &clips(&hd, 1, frames, FILM), false);
            let (from, to) = if id == "h264-intra" { (high, 2.0 * high) } else { (0.9 * low, high) };
            assert!((from..=to).contains(&got), "{id}, {frames} frames: {got:.0} B, measured {low}–{high}");
        }
    }

    #[test]
    fn the_nvenc_figures_hold_only_for_its_rate_control() {
        // `nvenc_bpp` was measured with these settings. Other ones change
        // what NVENC writes: measure again (`tests/calibrate.rs`) and change
        // the figures with them.
        for id in ["h264-intra", "h264-edit", "h264-mobile", "hevc10"] {
            let v = preset(id).video;
            let (VideoCodec::H264 { quality, gop, .. } | VideoCodec::Hevc { quality, gop, .. }) = v else { continue };
            let args = crate::clip::resolve_encoding(&v, &nvidia(), FILM, (1920, 1080), FILM).unwrap().args.join(" ");
            let max = nvenc_maxrate(gop, matches!(v, VideoCodec::Hevc { .. }), (1920, 1080), FILM) / 1000.0;
            let want = format!(
                "-preset p6 -tune hq -rc vbr -cq {} -b:v 0 -maxrate {max:.0}k -bufsize {:.0}k -spatial-aq 1",
                nvenc_cq(quality, gop),
                2.0 * max
            );
            assert!(args.contains(&want), "{id}: {args}");
        }
    }

    #[test]
    fn clips_are_counted_as_the_export_shows_them() {
        let est =
            |p: &ExportPreset, i: &MediaInfo, caps: &Caps| estimate_bytes(p, caps, &clips(i, 1, 240, 25.0), false);
        // A PAL DVD in 16:9: 720×576 stored, 1024×576 square pixels exported.
        let mut pal = info(720, 576);
        pal.video.sar = Rational::new(64, 45);
        let square = info(1024, 576);
        for caps in [Caps::default(), nvidia()] {
            for id in ["prores-hq", "h264-intra", "h264-edit"] {
                assert_eq!(est(&silent(id), &pal, &caps), est(&silent(id), &square, &caps), "{id}");
            }
        }
        // A phone clip on its side under a 1080 cap: 608×1080 upright.
        let mut phone = info(1920, 1080);
        phone.video.rotation = 90;
        let upright = info(608, 1080);
        assert_eq!(
            est(&silent("h264-mobile"), &phone, &Caps::default()),
            est(&silent("h264-mobile"), &upright, &Caps::default())
        );
    }

    #[test]
    fn a_joined_file_counts_every_clip_in_its_shape() {
        let (sd, hd) = (info(640, 480), info(1920, 1080));
        let mut mute = info(1920, 1080);
        mute.audio.clear();
        let clip =
            |info, frames| EstimateClip { info, audio_ordinal: None, frames, seconds: frames as f64 / 25.0, fps: 25.0 };
        let pack = [clip(&sd, 100), clip(&hd, 150), clip(&mute, 150)];
        let p = silent("prores-hq");
        let caps = Caps::default();
        // Most frames are HD: the SD shot is made HD-sized in the file.
        let joined = estimate_bytes(&p, &caps, &pack, true);
        let all_hd = estimate_bytes(&p, &caps, &[clip(&hd, 400)], false);
        assert!((joined / all_hd - 1.0).abs() < 1e-9, "{joined} vs {all_hd}");
        assert!(estimate_bytes(&p, &caps, &pack, false) < joined);
        // The file's sound runs through the clip without any, at 48 kHz:
        // 5.1 where every sound is 5.1, stereo where they differ.
        let sound = |pack: &[EstimateClip<'_>], joined: bool| {
            estimate_bytes(&preset("prores-hq"), &caps, pack, joined) - estimate_bytes(&p, &caps, pack, joined)
        };
        let near = |got: f64, want: f64| assert!((got - want).abs() < 1.0, "{got} vs {want}");
        near(sound(&pack, true), 48_000.0 * 3.0 * 6.0 * 16.0);
        let mut stereo = info(1920, 1080);
        stereo.audio[0].channels = 2;
        near(sound(&[clip(&hd, 150), clip(&stereo, 150)], true), 48_000.0 * 3.0 * 2.0 * 12.0);
        // Apart, a clip without sound has none.
        near(sound(&pack, false), 48_000.0 * 3.0 * 6.0 * 10.0);
    }

    #[test]
    fn sizes_follow_the_codec_and_picture() {
        let hd = info(1920, 1080);
        let sw = Caps::default();
        let est = |id: &str, i: &MediaInfo| estimate_bytes(&preset(id), &sw, &clips(i, 1, 2400, 24.0), false);
        assert!(est("prores-hq", &hd) > est("h264-intra", &hd));
        assert!(est("h264-intra", &hd) > est("h264-edit", &hd));
        assert!(est("h264-edit", &hd) > est("hevc10", &hd));
        // Four times the pixels, about four times the picture.
        let uhd = est("prores-hq", &info(3840, 2160)) / est("prores-hq", &hd);
        assert!((3.5..4.1).contains(&uhd), "{uhd}");
        // A capped height scales it down.
        let mut small = preset("prores-hq");
        small.max_height = Some(540);
        let quarter = estimate_bytes(&small, &sw, &clips(&hd, 1, 2400, 24.0), false) / est("prores-hq", &hd);
        assert!(quarter < 0.4, "{quarter}");
    }

    #[test]
    fn audio_follows_the_track_and_stereo() {
        let mut hd = info(1920, 1080);
        // Sound at 44.1 kHz is exported at 48.
        hd.audio[0].sample_rate = 44_100;
        let sw = Caps::default();
        let mut pcm = preset("prores-hq");
        let hundred = |p: &ExportPreset, i: &MediaInfo| {
            let c = EstimateClip { info: i, audio_ordinal: None, frames: 0, seconds: 100.0, fps: 24.0 };
            estimate_bytes(p, &sw, &[c], false)
        };
        assert_eq!(hundred(&pcm, &hd), 48_000.0 * 3.0 * 6.0 * 100.0);
        pcm.stereo = true;
        assert_eq!(hundred(&pcm, &hd), 48_000.0 * 3.0 * 2.0 * 100.0);
        assert_eq!(hundred(&preset("h264-edit"), &hd), 320_000.0 / 8.0 * 100.0);
        // A track that isn't there: no audio.
        let mut silent = hd.clone();
        silent.audio.clear();
        assert_eq!(hundred(&pcm, &silent), 0.0);
    }
}
