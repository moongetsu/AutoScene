//! Raw frame streams for analysis.
//!
//! Frames are decoded in software on purpose: it is fast on any modern CPU
//! (≈ 800 fps for 1080p H.264 at 48×27 on 16 threads, faster than the GPU
//! decoder plus download), it is exactly what exports decode, and it is
//! bit-for-bit repeatable — a run resumed at frame K sees the same pixels a
//! full run saw from K on. A pass that looks at few frames of heavy sources
//! (4K HEVC) can have them decoded and scaled on an NVIDIA GPU instead
//! ([`Decoder::Nvdec`]): the same decoder for a run and its resumption.

use std::io::{BufReader, Read};
use std::process::Stdio;

use as_core::{CancelToken, FrameIdx, FrameRange};

use crate::filters::{TONEMAP_SDR, deinterlace};
use crate::{HdrKind, MediaError, MediaInfo, MediaResult, Source, Tools};

/// Seconds of input pre-roll before the first wanted frame: the input seek
/// lands on the keyframe before it, `trim` then picks frames by timestamp.
const PREROLL: f64 = 3.0;

/// Decode frames `from..` of `src` scaled to `w`×`h` RGB24 (bicubic, the
/// scaling TransNetV2 was trained with) and hand each to `on_frame` in order.
/// `on_frame` returns `false` to stop early. Returns the number of frames
/// delivered; a full run delivers exactly `index.len() - from`.
pub fn decode_rgb(
    tools: &Tools,
    src: Source,
    from: FrameIdx,
    (w, h): (u32, u32),
    cancel: &CancelToken,
    on_frame: impl FnMut(FrameIdx, &[u8]) -> MediaResult<bool>,
) -> MediaResult<u32> {
    let chain = [format!("scale={w}:{h}:flags=bicubic"), "format=rgb24".into()];
    decode_raw(tools, src, from, 1, &[], &chain, (w * h * 3) as usize, cancel, on_frame).map(|(n, _)| n)
}

/// The longest side a picture is worked out at (16K). Only a damaged or
/// crafted pixel shape (a sample aspect ratio of 65535:1) gets there: such a
/// picture is scaled down to fit, instead of being given a size that
/// overflows or that ffmpeg refuses.
pub const MAX_SIDE: u32 = 16_384;

/// Size (even, square pixels, upright) at which [`decode_samples`] delivers
/// `info`'s frames when they must fit `max_w`×`max_h` (and [`MAX_SIDE`]);
/// never upscaled.
pub fn sample_size(info: &MediaInfo, (max_w, max_h): (u32, u32), multiple: u32) -> (u32, u32) {
    let v = &info.video;
    let (mut dw, mut dh) = (v.width as f64 * v.pixel_aspect(), v.height as f64);
    if v.rotation.rem_euclid(180) == 90 {
        std::mem::swap(&mut dw, &mut dh);
    }
    let (max_w, max_h) = (max_w.min(MAX_SIDE), max_h.min(MAX_SIDE));
    let k = (max_w as f64 / dw).min(max_h as f64 / dh).min(1.0);
    let round = |x: f64| ((x / multiple as f64).round() as u32).max(1) * multiple;
    (round(dw * k), round(dh * k))
}

/// Decode every `step`-th frame of `src` from `from` on (`from`, `from +
/// step`, …) as upright, square-pixel, SDR RGB24 at `w`×`h` (see
/// [`sample_size`]) and hand each to `on_frame` in order. Returns the number
/// of frames delivered; a full run delivers `(index.len() - from).div_ceil(step)`.
pub fn decode_samples(
    tools: &Tools,
    src: Source,
    from: FrameIdx,
    step: u32,
    (w, h): (u32, u32),
    cancel: &CancelToken,
    on_frame: impl FnMut(FrameIdx, &[u8]) -> MediaResult<bool>,
) -> MediaResult<u32> {
    assert!(step > 0);
    let mut chain: Vec<String> = Vec::new();
    // Right after the trim, so frame `from` is sample 0.
    if step > 1 {
        chain.push(format!("select='not(mod(n,{step}))'"));
    }
    if src.info.video.hdr != HdrKind::None {
        chain.push(TONEMAP_SDR.to_string());
    }
    chain.push(format!("scale={w}:{h}:flags=bilinear,setsar=1"));
    chain.push("format=rgb24".into());
    decode_raw(tools, src, from, step, &[], &chain, (w * h * 3) as usize, cancel, on_frame).map(|(n, _)| n)
}

/// Where [`decode_samples_with`] decodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Decoder {
    /// In software, as [`decode_samples`] does.
    #[default]
    Cpu,
    /// On an NVIDIA GPU (NVDEC), scaled there; only the samples come down.
    /// Every 6th frame of a 4K HEVC film: 500 fps on an RTX 3060 Ti, 230 in
    /// software on 16 threads (which a busy CPU slows down further). Sources
    /// it doesn't suit (interlaced, HDR, rotated, neither H.264 nor HEVC)
    /// are decoded in software.
    Nvdec,
}

/// Whether NVDEC can decode `info` the way [`decode_samples`] would: its
/// deinterlacing, tone mapping and rotation run in software. Only H.264 and
/// HEVC: on others (AV1, VP9) it can drop a picture without an error, and
/// every sample after it would then be taken for the frame before it.
fn nvdec_suits(info: &MediaInfo) -> bool {
    matches!(info.video.codec.as_str(), "h264" | "hevc")
        && !info.is_interlaced()
        && info.video.hdr == HdrKind::None
        && info.video.rotation == 0
}

/// [`decode_samples`] with `decoder`. When NVDEC can't be used or fails
/// part way (no NVIDIA GPU, a codec it doesn't take, a lost device), the
/// samples from the first one not delivered on are decoded in software
/// (trying costs a moment: the decoder fails as it starts). A run and its
/// resumption see the same pixels as long as they decode the same way; one
/// that fell back, or a resumption on a machine where the GPU decoder
/// changed, may differ slightly in its later chunks.
#[allow(clippy::too_many_arguments)]
pub fn decode_samples_with(
    tools: &Tools,
    src: Source,
    from: FrameIdx,
    step: u32,
    (w, h): (u32, u32),
    decoder: Decoder,
    cancel: &CancelToken,
    mut on_frame: impl FnMut(FrameIdx, &[u8]) -> MediaResult<bool>,
) -> MediaResult<u32> {
    assert!(step > 0);
    if decoder == Decoder::Cpu || !nvdec_suits(src.info) {
        return decode_samples(tools, src, from, step, (w, h), cancel, on_frame);
    }
    let mut chain: Vec<String> = Vec::new();
    if step > 1 {
        chain.push(format!("select='not(mod(n,{step}))'"));
    }
    chain.push(format!("scale_cuda={w}:{h}:interp_algo=bilinear:format=yuv420p"));
    chain.push("hwdownload,format=yuv420p,setsar=1,format=rgb24".into());
    // A frame the GPU can't decode stops it (rather than being dropped,
    // which would shift every later frame): software takes over from there.
    let input = ["-xerror", "-hwaccel", "cuda", "-hwaccel_output_format", "cuda"];
    let (mut delivered, mut refused) = (0u32, false);
    let gpu = decode_raw(tools, src, from, step, &input, &chain, (w * h * 3) as usize, cancel, |f, rgb| {
        let r = on_frame(f, rgb);
        refused |= r.is_err();
        delivered += 1;
        r
    });
    match gpu {
        Ok((n, _)) => Ok(n),
        // A frame count that doesn't add up is no reason to try again.
        Err(e) if refused || e.is_cancelled() || matches!(e, MediaError::Verify(_)) => Err(e),
        Err(e) => {
            tracing::warn!(error = %e.detail(), delivered, "decoding on the GPU failed; decoding in software from here");
            let next = from + delivered * step;
            Ok(delivered + decode_samples(tools, src, next, step, (w, h), cancel, on_frame)?)
        }
    }
}

/// Shared decoder: `-ss` pre-roll, deinterlacing (when the source needs it),
/// `trim` to frame `from`, then `chain`; the k-th raw frame out is frame
/// `from + k·step`. `input`: options for decoding the input (`-hwaccel`). Deinterlacing runs before the trim so it sees the frames
/// before `from` too: a run that starts mid-way produces exactly the pixels
/// of a run from the start. Returns (frames delivered, last frame).
#[allow(clippy::too_many_arguments)]
fn decode_raw(
    tools: &Tools,
    src: Source,
    from: FrameIdx,
    step: u32,
    input: &[&str],
    chain: &[String],
    size: usize,
    cancel: &CancelToken,
    mut on_frame: impl FnMut(FrameIdx, &[u8]) -> MediaResult<bool>,
) -> MediaResult<(u32, FrameIdx)> {
    let Source { path, info, index } = src;
    let n = index.len();
    if from >= n {
        return Ok((0, from));
    }
    let (trim_start, _) = index.trim_us(FrameRange::new(from, n));
    // One frame out per frame in (send_frame), timestamps kept: trim still
    // picks exactly the indexed frames.
    let mut filters: Vec<String> = deinterlace(info).into_iter().collect();
    filters.push(format!("trim=start={trim_start}us"));
    filters.extend(chain.iter().cloned());

    let mut cmd = tools.ffmpeg();
    cmd.args(["-v", "error", "-nostdin", "-copyts"]);
    if from > 0 {
        let seek = (index.t(from) - index.t0() - PREROLL).max(0.0);
        cmd.args(["-noaccurate_seek", "-ss", &format!("{seek:.6}")]);
    }
    cmd.args(input);
    cmd.arg("-i").arg(path);
    cmd.args(["-map", &format!("0:{}", info.video.index), "-an", "-sn", "-dn"]);
    cmd.args(["-filter:v", &filters.join(","), "-fps_mode", "passthrough", "-f", "rawvideo", "-"]);
    tracing::debug!(cmd = ?cmd, "decode for analysis");
    let mut child = crate::proc::spawn(cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))?;
    let mut stdout = BufReader::with_capacity(1 << 20, child.stdout.take().expect("piped stdout"));
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stderr_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
    });

    let mut buf = vec![0u8; size];
    let mut f = from;
    let mut delivered = 0u32;
    let stopped = loop {
        if cancel.is_cancelled() {
            break Some(MediaError::Cancelled);
        }
        match stdout.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break None,
            Err(e) => break Some(e.into()),
        }
        if f >= n {
            break Some(MediaError::Verify(format!("decoder produced more than the {n} indexed frames")));
        }
        delivered += 1;
        match on_frame(f, &buf) {
            Ok(true) => f += step,
            Ok(false) => {
                f += step;
                break Some(MediaError::Cancelled);
            }
            Err(e) => break Some(e),
        }
    };
    if let Some(e) = stopped {
        let _ = child.kill();
        let _ = child.wait();
        let _ = stderr_thread.join();
        // Stopping on purpose (on_frame said so) isn't an error for the caller.
        return match e {
            MediaError::Cancelled if !cancel.is_cancelled() => Ok((delivered, f)),
            e => Err(e),
        };
    }
    let status = child.wait()?;
    let tail = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(MediaError::Ffmpeg(if tail.is_empty() { format!("exit status {status}") } else { tail }));
    }
    Ok((delivered, f))
}

#[cfg(test)]
mod tests {
    use as_core::Rational;

    use super::*;

    fn info(width: u32, height: u32, sar: Rational, rotation: i32) -> MediaInfo {
        serde_json::from_value(serde_json::json!({
            "container": "mov", "durationSec": null, "startSec": 0.0, "sizeBytes": null, "bitRate": null,
            "video": {
                "index": 0, "codec": "h264", "profile": null, "pixFmt": "yuv420p", "bitDepth": 8,
                "width": width, "height": height, "sar": sar, "rFrameRate": Rational::new(24, 1),
                "avgFrameRate": null, "timeBase": Rational::new(1, 1000), "colorPrimaries": null,
                "colorTransfer": null, "colorSpace": null, "colorRange": null, "fieldOrder": null,
                "rotation": rotation, "hdr": "none"
            },
            "audio": [], "subtitles": []
        }))
        .unwrap()
    }

    #[test]
    fn samples_are_upright_square_and_never_too_big() {
        let all = (u32::MAX, u32::MAX);
        let r = Rational::new;
        assert_eq!(sample_size(&info(1920, 1080, r(1, 1), 0), (1280, 720), 4), (1280, 720));
        assert_eq!(sample_size(&info(720, 576, r(64, 45), 0), all, 2), (1024, 576));
        assert_eq!(sample_size(&info(1920, 1080, r(1, 1), 90), all, 2), (1080, 1920));
        // Pixels 20 and 2147483647 times as wide as tall: scaled down to 16K.
        assert_eq!(sample_size(&info(1920, 1080, r(20, 1), 0), all, 2), (MAX_SIDE, 460));
        let crafted = info(1920, 1080, r(i64::from(i32::MAX), 1), 0);
        assert_eq!(sample_size(&crafted, all, 2), (MAX_SIDE, 2));
        assert_eq!(sample_size(&crafted, (1280, 720), 4), (1280, 4));
        // A stored shape that isn't positive: square pixels.
        assert_eq!(sample_size(&info(1920, 1080, r(0, 1), 0), all, 2), (1920, 1080));
    }

    #[test]
    fn the_gpu_decodes_only_h264_and_hevc() {
        let coded = |codec: &str| {
            let mut i = info(1920, 1080, Rational::new(1, 1), 0);
            i.video.codec = codec.into();
            nvdec_suits(&i)
        };
        assert!(coded("h264") && coded("hevc"));
        for codec in ["av1", "vp9", "vp8", "mpeg2video", "vc1"] {
            assert!(!coded(codec), "{codec} is decoded in software");
        }
        assert!(!nvdec_suits(&info(1920, 1080, Rational::new(1, 1), 90)), "rotated");
    }
}
