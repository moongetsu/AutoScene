//! The shot pass: decode every frame at 48×27, measure pixel signals, run
//! TransNetV2, and hand finished signals out in chunks — the checkpoints a
//! later run resumes from. Fusion then turns the signals into segments.
//!
//! Resuming at a chunk start `K` decodes from `K - 25` (TransNet's context;
//! the pixel lags need 4) and drops everything before `K`, so the signals of
//! a resumed run are identical to those of an uninterrupted one on the same
//! device and runtime (one resumed on another differs in the last digits).

use std::collections::VecDeque;

use as_core::{CancelToken, FrameIdx};
use as_media::{Source, Tools};
use as_vision::TransNet;
use as_vision::transnet::{HEIGHT, STEP, TransNetStream, WIDTH};

use crate::AnalysisError;
use crate::signals::{FrameSignals, PixelMeter, PixelSignals};

/// Frames per checkpoint chunk (100 s at 24 fps). A multiple of TransNet's step.
pub const CHUNK: u32 = 2400;

/// Measure frames `resume..` of `src` (`resume` a multiple of `chunk`, which
/// is a multiple of 50) and pass each finished chunk to `on_chunk(start,
/// signals)` in order; the last one may be shorter. `progress` gets 0..1.
#[allow(clippy::too_many_arguments)]
pub fn shot_signals(
    tools: &Tools,
    src: Source,
    net: &mut TransNet,
    resume: FrameIdx,
    chunk: u32,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
    mut on_chunk: impl FnMut(FrameIdx, &[FrameSignals]) -> Result<(), AnalysisError>,
) -> Result<(), AnalysisError> {
    assert!(chunk > 0 && (chunk as usize).is_multiple_of(STEP), "chunks are whole TransNet steps");
    assert!(resume.is_multiple_of(chunk), "resume at a chunk start");
    let n = src.index.len();
    if resume >= n {
        return Ok(());
    }
    let mut stream = TransNetStream::new(resume);
    let first = stream.first_frame();
    let mut meter = PixelMeter::default();
    let mut pending: VecDeque<PixelSignals> = VecDeque::new();
    let mut done: Vec<FrameSignals> = Vec::with_capacity(chunk as usize);
    let mut chunk_start = resume;
    let mut next = resume;
    let mut failure: Option<AnalysisError> = None;

    let mut take = |preds: Vec<as_vision::transnet::Prediction>,
                    pending: &mut VecDeque<PixelSignals>,
                    done: &mut Vec<FrameSignals>,
                    chunk_start: &mut FrameIdx,
                    next: &mut FrameIdx|
     -> Result<(), AnalysisError> {
        for p in preds {
            debug_assert_eq!(p.frame, *next, "TransNet predictions arrive in order");
            let px = pending.pop_front().ok_or_else(|| AnalysisError::Internal("prediction without a frame".into()))?;
            done.push(FrameSignals {
                single: p.single,
                many: p.many,
                mu: px.mu,
                sd: px.sd,
                diff: px.diff,
                hist: px.hist,
                dark: px.dark,
                light: px.light,
                colors: px.colors,
            });
            *next += 1;
            if done.len() == chunk as usize {
                on_chunk(*chunk_start, done)?;
                *chunk_start += chunk;
                done.clear();
            }
        }
        Ok(())
    };

    let delivered = as_media::frames::decode_rgb(tools, src, first, (WIDTH as u32, HEIGHT as u32), cancel, |f, rgb| {
        let px = meter.measure(rgb);
        if f >= resume {
            pending.push_back(px);
        }
        let step = stream
            .push(net, rgb)
            .map_err(AnalysisError::from)
            .and_then(|preds| take(preds, &mut pending, &mut done, &mut chunk_start, &mut next));
        if let Err(e) = step {
            failure = Some(e);
            return Ok(false);
        }
        if f % 240 == 0 {
            progress(f as f64 / n as f64);
        }
        Ok(true)
    });
    if let Some(e) = failure {
        return Err(e);
    }
    let delivered = delivered?;
    if first + delivered != n {
        return Err(AnalysisError::Media(as_media::MediaError::Verify(format!(
            "analysis decoded {} frames from {first}, the index has {n}",
            delivered
        ))));
    }
    let preds = stream.finish(net, n)?;
    take(preds, &mut pending, &mut done, &mut chunk_start, &mut next)?;
    if !done.is_empty() {
        on_chunk(chunk_start, &done)?;
    }
    if next != n || !pending.is_empty() {
        return Err(AnalysisError::Internal(format!("shot pass ended at {next} of {n}")));
    }
    progress(1.0);
    Ok(())
}
