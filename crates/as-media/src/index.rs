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

use crate::{MediaError, MediaInfo, MediaResult, Tools, run_ffmpeg_counting};

/// What libavcodec logs (lowercased) when the GPU rejects a picture that the
/// software decoder would still have produced (concealed). For H.264/HEVC,
/// which frames exist and their timestamps are decided by the same native
/// decoder either way (checked on damaged, truncated and gappy streams with
/// CUDA and D3D11VA), so this is the one case where the passes could differ.
const HW_PICTURE_FAILED: &[&str] = &["hardware accelerator failed", "hw accel"];

/// Codecs whose hardware index pass is known to match software decoding.
/// Others (AV1, VP9 — whose hwaccel can drop pictures silently) index in
/// software, which is what exports decode with anyway.
fn hw_index_ok(codec: &str) -> bool {
    matches!(codec, "h264" | "hevc")
}

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

    // Returns how many pictures the hardware decoder rejected.
    let attempt = |hw: bool, progress: &mut dyn FnMut(f64)| -> MediaResult<usize> {
        let mut args: Vec<String> =
            vec!["-v".into(), "error".into(), "-y".into(), "-copyts".into(), "-threads".into(), "0".into()];
        if hw
            && hw_index_ok(&info.video.codec)
            && let Some(d) = &tools.caps().decode
        {
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
        let patterns = if hw { HW_PICTURE_FAILED } else { &[] };
        run_ffmpeg_counting(tools, &args, cancel, |p| progress((p.out_time / duration).clamp(0.0, 1.0)), patterns)
    };

    // Hardware decoders can reject unusual codecs/profiles or single damaged
    // pictures; software always works and is what exports decode with.
    match attempt(true, &mut progress) {
        Ok(0) => {}
        Ok(rejected) => {
            tracing::warn!(rejected, "hardware decoder rejected pictures; indexing again in software");
            attempt(false, &mut progress)?;
        }
        Err(e) if e.is_cancelled() => {
            let _ = std::fs::remove_file(&stats);
            return Err(e);
        }
        Err(e) if tools.caps().decode.is_some() && hw_index_ok(&info.video.codec) => {
            tracing::warn!(error = %e.detail(), "hardware index pass failed; retrying in software");
            attempt(false, &mut progress)?;
        }
        Err(e) => return Err(e),
    }

    let text = std::fs::read_to_string(&stats)?;
    let _ = std::fs::remove_file(&stats);
    let index = parse_stats(&text, info.video.r_frame_rate)?;
    let nominal = nominal_rate(info.video.r_frame_rate, info.video.avg_frame_rate, median_interval(&index));
    Ok(if nominal == index.nominal_fps { index } else { index.with_nominal_fps(nominal) })
}

/// Common production rates, for snapping a measured average.
const STANDARD_RATES: &[(i64, i64)] = &[
    (24000, 1001),
    (24, 1),
    (25, 1),
    (30000, 1001),
    (30, 1),
    (48, 1),
    (50, 1),
    (60000, 1001),
    (60, 1),
    (100, 1),
    (120000, 1001),
    (120, 1),
];

/// Typical frame duration in seconds: the median step between frames, so
/// timestamp gaps (broadcast recordings) and dropped frames don't skew it.
pub fn median_interval(index: &FrameIndex) -> Option<f64> {
    let n = index.len();
    if n < 2 {
        return None;
    }
    let mut steps: Vec<i64> = (1..n).map(|f| index.pts(f) - index.pts(f - 1)).collect();
    let mid = steps.len() / 2;
    let step = *steps.select_nth_unstable(mid).1;
    Some(step as f64 * index.tb.num as f64 / index.tb.den as f64)
}

/// The frame rate timecodes and GOP lengths should use. `r_frame_rate` is
/// right for constant-rate media, but phone and screen recordings report the
/// time base's tick rate there (`240/1`, `90000/1`) while running at 30 fps:
/// when it disagrees with the measured rate (from the median frame step),
/// use the measured rate, snapped to a standard rate when one is within
/// 1.5 % (else the nearest whole number).
pub fn nominal_rate(
    r_frame_rate: Rational,
    avg_frame_rate: Option<Rational>,
    median_interval: Option<f64>,
) -> Rational {
    let Some(step) = median_interval.filter(|s| *s > 0.0) else {
        return r_frame_rate;
    };
    let measured = 1.0 / step;
    let close = |r: Rational, tol: f64| (r.to_f64() - measured).abs() / measured <= tol;
    if close(r_frame_rate, 0.02) {
        return r_frame_rate;
    }
    let off = |&&(n, d): &&(i64, i64)| (Rational::new(n, d).to_f64() - measured).abs();
    if let Some(&(n, d)) = STANDARD_RATES.iter().min_by(|a, b| off(a).total_cmp(&off(b)))
        && close(Rational::new(n, d), 0.015)
    {
        return Rational::new(n, d);
    }
    match avg_frame_rate {
        Some(avg) if close(avg, 0.02) && avg.den <= 1001 => avg,
        _ => Rational::new((measured.round() as i64).max(1), 1),
    }
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
    fn nominal_rate_ignores_tick_rates() {
        let r = |n, d| Rational::new(n, d);
        // Film and TV: trust r_frame_rate.
        assert_eq!(nominal_rate(r(24000, 1001), Some(r(24000, 1001)), Some(1001.0 / 24000.0)), r(24000, 1001));
        // Phone recording: r_frame_rate is the tick rate, frames are ~30 fps.
        assert_eq!(nominal_rate(r(240, 1), Some(r(30, 1)), Some(1.0 / 30.0)), r(30, 1));
        assert_eq!(nominal_rate(r(90_000, 1), None, Some(3003.0 / 90_000.0)), r(30000, 1001));
        // No standard rate nearby: a whole number.
        assert_eq!(nominal_rate(r(120, 1), None, Some(1.0 / 33.0)), r(33, 1));
    }

    #[test]
    fn median_interval_ignores_gaps_and_drops() {
        // 25 fps broadcast with a 2-minute hole and a few dropped frames: the
        // overall average says 23.4 fps, the median step still says 25.
        let mut pts: Vec<i64> = (0..45_000).map(|i| i * 3600).collect();
        for p in &mut pts[20_000..] {
            *p += 120 * 90_000;
        }
        pts.retain(|p| p % 97 != 0);
        let idx = FrameIndex::from_pts(Rational::new(1, 90_000), Rational::new(25, 1), pts).unwrap();
        let step = median_interval(&idx);
        assert_eq!(step, Some(0.04));
        assert_eq!(nominal_rate(Rational::new(25, 1), Some(Rational::new(25, 1)), step), Rational::new(25, 1));
    }

    #[test]
    fn parses_and_repairs_stats() {
        let idx =
            parse_stats("0 1/1000\n42 1/1000\n42 1/1000\nN/A 1/1000\n167 1/1000\n", Rational::new(24, 1)).unwrap();
        assert_eq!(idx.len(), 5);
        assert_eq!((idx.pts(2), idx.pts(3), idx.pts(4)), (43, 44, 167));
    }
}
