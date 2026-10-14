//! Raw frame streams for analysis.
//!
//! Frames are decoded in software on purpose: it is fast on any modern CPU
//! (≈ 800 fps for 1080p H.264 at 48×27 on 16 threads, faster than the GPU
//! decoder plus download), it is exactly what exports decode, and it is
//! bit-for-bit repeatable — a run resumed at frame K sees the same pixels a
//! full run saw from K on.

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
    decode_raw(tools, src, from, 1, &chain, (w * h * 3) as usize, cancel, on_frame).map(|(n, _)| n)
}

/// Size (even, square pixels, upright) at which [`decode_samples`] delivers
/// `info`'s frames when they must fit `max_w`×`max_h`; never upscaled.
pub fn sample_size(info: &MediaInfo, (max_w, max_h): (u32, u32), multiple: u32) -> (u32, u32) {
    let v = &info.video;
    let sar = if v.sar.num > 0 && v.sar.den > 0 { v.sar.to_f64() } else { 1.0 };
    let (mut dw, mut dh) = (v.width as f64 * sar, v.height as f64);
    if v.rotation.rem_euclid(180) == 90 {
        std::mem::swap(&mut dw, &mut dh);
    }
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
    decode_raw(tools, src, from, step, &chain, (w * h * 3) as usize, cancel, on_frame).map(|(n, _)| n)
}

/// Shared decoder: `-ss` pre-roll, deinterlacing (when the source needs it),
/// `trim` to frame `from`, then `chain`; the k-th raw frame out is frame
/// `from + k·step`. Deinterlacing runs before the trim so it sees the frames
/// before `from` too: a run that starts mid-way produces exactly the pixels
/// of a run from the start. Returns (frames delivered, last frame).
#[allow(clippy::too_many_arguments)]
fn decode_raw(
    tools: &Tools,
    src: Source,
    from: FrameIdx,
    step: u32,
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
