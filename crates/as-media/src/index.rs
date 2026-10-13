//! Frame index: the timestamp of every frame the decoder actually outputs.
//!
//! We decode the whole video stream once (hardware-accelerated when possible,
//! frames stay on the GPU) into ffmpeg's null sink with `-stats_enc_pre`,
//! which logs each decoded frame's original timestamp (`{ptsi}`) in the input
//! time base (`{tbi}`). `-copyts` keeps container start offsets, and
//! `-fps_mode passthrough` stops ffmpeg from duplicating or dropping frames,
//! so the result is exactly the frame grid that exports will cut on.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use as_core::{CancelToken, FrameIndex, Rational};

use crate::{MediaError, MediaInfo, MediaResult, Tools, run_ffmpeg};

pub fn build_frame_index(
    tools: &Tools,
    path: &Path,
    info: &MediaInfo,
    work_dir: &Path,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
) -> MediaResult<FrameIndex> {
    std::fs::create_dir_all(work_dir)?;
    // Unique per call: several media are indexed in parallel from one process.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let stats = work_dir.join(format!("index-{}-{n}.txt", std::process::id()));
    let duration = info.duration_sec.unwrap_or(0.0).max(0.001);

    let attempt = |hw: bool, progress: &mut dyn FnMut(f64)| -> MediaResult<()> {
        let mut args: Vec<String> =
            vec!["-v".into(), "error".into(), "-y".into(), "-copyts".into(), "-threads".into(), "0".into()];
        if hw && let Some(d) = &tools.caps().decode {
            args.extend([
                "-hwaccel".into(),
                d.hwaccel.clone(),
                "-hwaccel_output_format".into(),
                d.output_format.clone(),
            ]);
        }
        args.extend(["-i".into(), path.display().to_string()]);
        args.extend(
            [
                "-map",
                &format!("0:{}", info.video.index),
                "-fps_mode",
                "passthrough",
                "-enc_time_base:v",
                "demux",
                "-c:v",
                "wrapped_avframe",
                "-stats_enc_pre:v",
                &stats.display().to_string(),
                "-stats_enc_pre_fmt:v",
                "{ptsi} {tbi}",
                "-f",
                "null",
                "-",
            ]
            .map(String::from),
        );
        run_ffmpeg(tools, &args, cancel, |p| progress((p.out_time / duration).clamp(0.0, 1.0)))
    };

    // Hardware decoders can reject unusual codecs/profiles; software always works.
    if let Err(e) = attempt(true, &mut progress) {
        if e.is_cancelled() {
            let _ = std::fs::remove_file(&stats);
            return Err(e);
        }
        if tools.caps().decode.is_some() {
            tracing::warn!(error = %e, "hardware index pass failed; retrying in software");
            attempt(false, &mut progress)?;
        } else {
            return Err(e);
        }
    }

    let text = std::fs::read_to_string(&stats)?;
    let _ = std::fs::remove_file(&stats);
    parse_stats(&text, info.video.r_frame_rate)
}

/// Parse `{ptsi} {tbi}` lines. Frames without a timestamp, or with one that
/// doesn't advance (broken muxes), are nudged forward by one tick so every
/// real frame keeps its own slot on the grid.
pub fn parse_stats(text: &str, nominal_fps: Rational) -> MediaResult<FrameIndex> {
    let mut tb: Option<Rational> = None;
    let mut pts: Vec<i64> = Vec::new();
    let mut repaired = 0usize;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(p), Some(t)) = (parts.next(), parts.next()) else { continue };
        if tb.is_none() {
            tb = Rational::parse(t).ok();
        }
        let prev = pts.last().copied();
        let v = match (p.parse::<i64>().ok(), prev) {
            (Some(v), Some(prev)) if v > prev => v,
            (Some(v), None) => v,
            (_, Some(prev)) => {
                repaired += 1;
                prev + 1
            }
            (None, None) => 0,
        };
        pts.push(v);
    }
    if repaired > 0 {
        tracing::warn!(repaired, "frame index had missing/non-increasing timestamps");
    }
    let tb = tb.ok_or_else(|| MediaError::Ffmpeg("decoder produced no frames".into()))?;
    Ok(FrameIndex::from_pts(tb, nominal_fps, pts)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_repairs_stats() {
        let idx =
            parse_stats("0 1/1000\n42 1/1000\n42 1/1000\nN/A 1/1000\n167 1/1000\n", Rational::new(24, 1)).unwrap();
        assert_eq!(idx.len(), 5);
        assert_eq!((idx.pts(2), idx.pts(3), idx.pts(4)), (43, 44, 167));
    }
}
