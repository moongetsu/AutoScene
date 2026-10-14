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

use crate::filters::deinterlace;
use crate::{MediaError, MediaResult, Source, Tools};

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
    mut on_frame: impl FnMut(FrameIdx, &[u8]) -> MediaResult<bool>,
) -> MediaResult<u32> {
    let Source { path, info, index } = src;
    let n = index.len();
    if from >= n {
        return Ok(0);
    }
    let (trim_start, _) = index.trim_us(FrameRange::new(from, n));
    let mut chain = vec![format!("trim=start={trim_start}us")];
    chain.extend(deinterlace(info));
    chain.push(format!("scale={w}:{h}:flags=bicubic"));
    chain.push("format=rgb24".into());

    let mut cmd = tools.ffmpeg();
    cmd.args(["-v", "error", "-nostdin", "-copyts"]);
    if from > 0 {
        let seek = (index.t(from) - index.t0() - PREROLL).max(0.0);
        cmd.args(["-noaccurate_seek", "-ss", &format!("{seek:.6}")]);
    }
    cmd.arg("-i").arg(path);
    cmd.args(["-map", &format!("0:{}", info.video.index), "-an", "-sn", "-dn"]);
    cmd.args(["-filter:v", &chain.join(","), "-fps_mode", "passthrough", "-f", "rawvideo", "-"]);
    tracing::debug!(cmd = ?cmd, "decode for analysis");
    let mut child = crate::proc::spawn(cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))?;
    let mut stdout = BufReader::with_capacity(1 << 20, child.stdout.take().expect("piped stdout"));
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stderr_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
    });

    let size = (w * h * 3) as usize;
    let mut buf = vec![0u8; size];
    let mut f = from;
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
        match on_frame(f, &buf) {
            Ok(true) => f += 1,
            Ok(false) => {
                f += 1;
                break Some(MediaError::Cancelled);
            }
            Err(e) => break Some(e),
        }
    };
    let delivered = f - from;
    if let Some(e) = stopped {
        let _ = child.kill();
        let _ = child.wait();
        let _ = stderr_thread.join();
        // Stopping on purpose (on_frame said so) isn't an error for the caller.
        return match e {
            MediaError::Cancelled if !cancel.is_cancelled() => Ok(delivered),
            e => Err(e),
        };
    }
    let status = child.wait()?;
    let tail = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(MediaError::Ffmpeg(if tail.is_empty() { format!("exit status {status}") } else { tail }));
    }
    Ok(delivered)
}
