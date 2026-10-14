//! TransNetV2 (Souček & Lokoč 2020): per-frame probabilities that a frame is
//! a shot boundary (`single`, hard cuts) or inside a transition (`many`,
//! gradual ones), from 48×27 RGB frames.
//!
//! The model sees windows of 100 frames and is trusted only for the middle
//! 50; windows advance by 50 and the video is padded with copies of its
//! first and last frame — exactly the reference inference, so results match
//! the published model. [`TransNetStream`] does this incrementally and can
//! start at any multiple of 50 given the 25 frames before it, which makes a
//! resumed analysis produce the same numbers as an uninterrupted one.

use std::collections::VecDeque;
use std::path::Path;

use as_infer::{Device, InferError, InferResult, Runtime, Session, Tensor};

pub const WIDTH: usize = 48;
pub const HEIGHT: usize = 27;
/// Bytes of one RGB24 input frame.
pub const FRAME_BYTES: usize = WIDTH * HEIGHT * 3;
pub const WINDOW: usize = 100;
/// Window advance; also the granularity at which a stream can (re)start.
pub const STEP: usize = 50;
/// Frames of context before (and after) the predicted middle of a window.
pub const CONTEXT: usize = 25;

pub struct TransNet {
    session: Session,
    input: String,
    device: Device,
}

impl TransNet {
    pub fn load(rt: &Runtime, model: &Path) -> InferResult<Self> {
        let (session, device) = rt.session(model)?;
        let input = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| InferError::Model(format!("{}: no input", model.display())))?;
        if session.outputs().len() < 2 {
            return Err(InferError::Model(format!("{}: expected two outputs", model.display())));
        }
        tracing::info!(device = ?device, "TransNetV2 ready");
        Ok(Self { session, input, device })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    /// One window of exactly [`WINDOW`] frames (concatenated RGB24) →
    /// (`single`, `many`) probabilities for each of them.
    pub fn window(&mut self, frames: &[u8]) -> InferResult<(Vec<f32>, Vec<f32>)> {
        assert_eq!(frames.len(), WINDOW * FRAME_BYTES, "TransNet window size");
        // The graph takes 0..255 floats and normalises itself.
        let x: Vec<f32> = frames.iter().map(|&v| v as f32).collect();
        let t = Tensor::from_array(([1usize, WINDOW, HEIGHT, WIDTH, 3], x))?;
        let outs = self.session.run(as_infer::inputs![self.input.as_str() => t])?;
        let (_, single) = outs[0].try_extract_tensor::<f32>()?;
        let (_, many) = outs[1].try_extract_tensor::<f32>()?;
        if single.len() != WINDOW || many.len() != WINDOW {
            return Err(InferError::Run(format!("TransNet returned {} / {} values", single.len(), many.len())));
        }
        Ok((single.to_vec(), many.to_vec()))
    }
}

/// Per-frame TransNet output.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Prediction {
    pub frame: u32,
    pub single: f32,
    pub many: f32,
}

/// Feeds frames through [`TransNet`] window by window.
pub struct TransNetStream {
    /// Frames from `buf_start` on.
    buf: VecDeque<Vec<u8>>,
    buf_start: i64,
    /// First frame of the next window's predicted middle.
    next: i64,
    /// Frame index of the first frame pushed (a multiple of STEP, minus CONTEXT unless it is 0).
    first_pushed: i64,
    pushed: i64,
}

impl TransNetStream {
    /// Predictions will start at `start`, which must be a multiple of
    /// [`STEP`]. For `start > 0` the caller pushes frames from
    /// `start - CONTEXT` on; at 0 the start is padded with the first frame.
    pub fn new(start: u32) -> Self {
        assert!((start as usize).is_multiple_of(STEP), "TransNet streams start at multiples of {STEP}");
        let first = if start == 0 { 0 } else { start as i64 - CONTEXT as i64 };
        Self { buf: VecDeque::new(), buf_start: first, next: start as i64, first_pushed: first, pushed: first }
    }

    /// Frame the caller has to push first.
    pub fn first_frame(&self) -> u32 {
        self.first_pushed as u32
    }

    /// Push the next frame (RGB24, 48×27); returns predictions that became final.
    pub fn push(&mut self, net: &mut TransNet, frame: &[u8]) -> InferResult<Vec<Prediction>> {
        assert_eq!(frame.len(), FRAME_BYTES);
        if self.pushed == 0 {
            // Reference padding: the first frame repeated before the video.
            for _ in 0..CONTEXT {
                self.buf.push_back(frame.to_vec());
            }
            self.buf_start = -(CONTEXT as i64);
        }
        self.buf.push_back(frame.to_vec());
        self.pushed += 1;
        let mut out = Vec::new();
        while self.buf_end() >= self.next + (WINDOW - CONTEXT) as i64 {
            out.extend(self.run_window(net, None)?);
        }
        Ok(out)
    }

    /// The video ended after `total` frames: pad with the last frame and
    /// return the remaining predictions (up to `total`).
    pub fn finish(&mut self, net: &mut TransNet, total: u32) -> InferResult<Vec<Prediction>> {
        let mut out = Vec::new();
        while self.next < total as i64 {
            out.extend(self.run_window(net, Some(total as i64))?);
        }
        Ok(out)
    }

    fn buf_end(&self) -> i64 {
        self.buf_start + self.buf.len() as i64
    }

    fn run_window(&mut self, net: &mut TransNet, total: Option<i64>) -> InferResult<Vec<Prediction>> {
        let lo = self.next - CONTEXT as i64;
        let mut x = Vec::with_capacity(WINDOW * FRAME_BYTES);
        for f in lo..lo + WINDOW as i64 {
            let i = (f - self.buf_start).clamp(0, self.buf.len() as i64 - 1) as usize;
            debug_assert!(f >= self.buf_start, "window starts before the buffer");
            debug_assert!(total.is_some() || f < self.buf_end(), "window runs past the buffer");
            x.extend_from_slice(&self.buf[i]);
        }
        let (single, many) = net.window(&x)?;
        let end = total.map_or(self.next + STEP as i64, |t| (self.next + STEP as i64).min(t));
        let preds = (self.next..end)
            .map(|f| {
                let k = (f - lo) as usize;
                Prediction { frame: f as u32, single: single[k], many: many[k] }
            })
            .collect();
        self.next += STEP as i64;
        // Keep only what the next window needs.
        while self.buf_start < self.next - CONTEXT as i64 && self.buf.len() > 1 {
            self.buf.pop_front();
            self.buf_start += 1;
        }
        Ok(preds)
    }
}
