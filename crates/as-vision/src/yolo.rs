//! Ultralytics-style YOLO detectors as deepghs exports them for anime
//! (`anime_head_detect_*`, `anime_person_detect_*`, `anime_face_detect_*`,
//! `anime_halfbody_detect_*`): one class — head, person, face, half-body —
//! and one output tensor of detection rows `cx, cy, w, h, score` in padded
//! input pixels, with no NMS inside the graph.
//!
//! A frame is letterboxed into the input's static size: scaled to fit and
//! centred on a grey canvas (114, the fill colour the graphs were trained
//! with), and boxes map back by the same transform. Exports differ in how
//! the rows lie: the newer graphs give `[B,5,N]`, the older face/person
//! ones `[B,N,5]` — [`rows`] tells them apart the way
//! `tools/anime-bench/yolo.py` does.

use std::path::Path;

use as_infer::onnx::InputShape;
use as_infer::{Device, InferError, InferResult, Runtime, Session, TensorRef};

use crate::image;
use crate::scrfd::iou;

/// A detection starts at this confidence (the bench's operating point).
pub const SCORE_THRESHOLD: f32 = 0.5;
/// Overlap at which the weaker of two detections is dropped.
pub const NMS_IOU: f32 = 0.5;
/// The letterbox canvas colour, scaled like the pixels (114 of 255).
const GREY: f32 = 114.0 / 255.0;

/// How a `w`×`h` frame sits inside the `iw`×`ih` input: scaled `s` times,
/// its `nw`×`nh` image pasted at (`px`, `py`) of the canvas, and output
/// coordinates mapped back by `x' = (x − ox)/s` with the exact centred
/// offset.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Letterbox {
    s: f32,
    ox: f32,
    oy: f32,
    nw: usize,
    nh: usize,
    px: usize,
    py: usize,
}

impl Letterbox {
    fn new((w, h): (usize, usize), (iw, ih): (usize, usize)) -> Self {
        let s = (iw as f32 / w as f32).min(ih as f32 / h as f32);
        let nw = ((w as f32 * s).round() as usize).clamp(1, iw);
        let nh = ((h as f32 * s).round() as usize).clamp(1, ih);
        // The pixels go to whole positions, within half a canvas pixel of
        // the exact offset (`tools/anime-bench/yolo.py` pastes the same).
        Self {
            s,
            ox: (iw as f32 - w as f32 * s) / 2.0,
            oy: (ih as f32 - h as f32 * s) / 2.0,
            nw,
            nh,
            px: (iw - nw) / 2,
            py: (ih - nh) / 2,
        }
    }
}

/// One detection, in the pixel coordinates of the frame given to
/// [`Yolo::detect_many`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Det {
    /// x1, y1, x2, y2, clipped to the frame.
    pub bbox: [f32; 4],
    pub score: f32,
}

/// A deepghs anime detector, planned for one frame size, one input size and
/// one batch.
pub struct Yolo {
    session: Session,
    input: String,
    device: Device,
    /// The frame size the session is planned for.
    frame: (usize, usize),
    /// The input's static size (width × height).
    input_size: (usize, usize),
    batch: usize,
    /// Input buffer (NCHW float). The letterbox canvas stays [`GREY`];
    /// frames only ever overwrite their pasted area.
    x: Vec<f32>,
}

impl Yolo {
    /// Load `model` for `frame`-sized RGB24 frames letterboxed into `input`
    /// size (both width × height), `batch` per inference.
    pub fn load(
        rt: &Runtime,
        model: &Path,
        frame: (usize, usize),
        input: (usize, usize),
        batch: usize,
    ) -> InferResult<Self> {
        let ((w, h), (iw, ih)) = (frame, input);
        assert!(w > 0 && h > 0 && iw > 0 && ih > 0 && batch > 0, "YOLO shape {w}×{h} → {iw}×{ih}, batch {batch}");
        let (session, device) = rt.session_shaped(model, InputShape::Fixed(&[batch, 3, ih, iw]))?;
        let input = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| InferError::Model(format!("{}: no input", model.display())))?;
        if session.outputs().len() != 1 {
            return Err(InferError::Model(format!(
                "{}: expected one output (the detections), got {}",
                model.display(),
                session.outputs().len()
            )));
        }
        tracing::info!(device = ?device, w, h, iw, ih, batch, "YOLO ready");
        Ok(Self { session, input, device, frame, input_size: (iw, ih), batch, x: vec![GREY; batch * 3 * iw * ih] })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    /// The frame size this detector takes.
    pub fn frame_size(&self) -> (usize, usize) {
        self.frame
    }

    /// The static input size (width × height).
    pub fn input_size(&self) -> (usize, usize) {
        self.input_size
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    /// Detect in RGB24 frames of [`Yolo::frame_size`], any number (the last
    /// inference runs on a partial batch). Boxes come back per frame, in
    /// its coordinates, strongest first, NMS applied.
    pub fn detect_many(&mut self, frames: &[&[u8]], conf: f32) -> InferResult<Vec<Vec<Det>>> {
        let mut out = Vec::with_capacity(frames.len());
        for group in frames.chunks(self.batch) {
            out.extend(self.infer(group, conf)?);
        }
        Ok(out)
    }

    /// One inference on up to `batch` frames. Slots past the given frames
    /// keep earlier pixels; their results are ignored.
    fn infer(&mut self, frames: &[&[u8]], conf: f32) -> InferResult<Vec<Vec<Det>>> {
        let ((w, h), (iw, ih)) = (self.frame, self.input_size);
        let lb = Letterbox::new(self.frame, self.input_size);
        let plane = iw * ih;
        for (slot, rgb) in self.x.chunks_exact_mut(3 * plane).zip(frames) {
            assert_eq!(rgb.len(), w * h * 3, "YOLO frame size");
            let img = image::resize_region(rgb, (w, h), [0.0, 0.0, w as f32, h as f32], (lb.nw, lb.nh));
            let (r, rest) = slot.split_at_mut(plane);
            let (g, b) = rest.split_at_mut(plane);
            for (y, line) in img.chunks_exact(lb.nw * 3).enumerate() {
                let o = (lb.py + y) * iw + lb.px;
                let dst = r[o..o + lb.nw].iter_mut().zip(&mut g[o..o + lb.nw]).zip(&mut b[o..o + lb.nw]);
                for (p, ((r, g), b)) in line.as_chunks::<3>().0.iter().zip(dst) {
                    (*r, *g, *b) = (f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0);
                }
            }
        }
        let batch = self.batch;
        let t = TensorRef::from_array_view(([batch, 3, ih, iw], &self.x[..]))?;
        let outs = self.session.run(as_infer::inputs![self.input.as_str() => t])?;
        let (shape, e) = outs[0].try_extract_tensor::<f32>()?;
        let (n, transposed) = rows(&shape[..], batch)
            .ok_or_else(|| InferError::Run(format!("YOLO returned shape {:?} for a batch of {batch}", &shape[..])))?;
        if e.len() != n * 5 * batch {
            return Err(InferError::Run(format!(
                "YOLO returned {} values for shape {:?} × {batch}",
                e.len(),
                &shape[..]
            )));
        }
        let found: Vec<Vec<Det>> =
            (0..frames.len()).map(|f| nms(detections(e, f, n, transposed, self.frame, lb, conf))).collect();
        drop(outs);
        Ok(found)
    }
}

/// How the detection rows of a `[B,·,·]` output lie: `n` rows of 5 values,
/// `[B,5,N]` (`transposed` — the newer exports) or `[B,N,5]` (the older
/// face/person ones). `None` for a shape that is neither.
fn rows(shape: &[i64], batch: usize) -> Option<(usize, bool)> {
    if shape.len() != 3 || shape[0] != batch as i64 {
        return None;
    }
    let (a, b) = (shape[1], shape[2]);
    if a == 5 && b != 5 {
        Some((b as usize, true))
    } else if b == 5 {
        Some((a as usize, false))
    } else {
        None
    }
}

/// One detection row of the flat output `e` for frame `f`: cx, cy, w, h,
/// score in padded input pixels. `n` rows lie `[B,5,N]` when `transposed`,
/// `[B,N,5]` else.
fn row(e: &[f32], f: usize, n: usize, i: usize, transposed: bool) -> [f32; 5] {
    if transposed {
        let b = f * 5 * n;
        [e[b + i], e[b + n + i], e[b + 2 * n + i], e[b + 3 * n + i], e[b + 4 * n + i]]
    } else {
        let b = (f * n + i) * 5;
        e[b..b + 5].try_into().expect("a row is 5 values")
    }
}

/// The detections of frame `f` in the flat output `e`, de-letterboxed into
/// frame pixels and clipped to the `w`×`h` frame, at confidence `conf` or
/// better.
fn detections(
    e: &[f32],
    f: usize,
    n: usize,
    transposed: bool,
    (w, h): (usize, usize),
    lb: Letterbox,
    conf: f32,
) -> Vec<Det> {
    let mut found = Vec::new();
    for i in 0..n {
        let [cx, cy, bw, bh, score] = row(e, f, n, i, transposed);
        // A score that is no number (a faulty execution provider) passes no
        // threshold.
        if !score.is_finite() || score < conf {
            continue;
        }
        // Nor does a box of no numbers: clipped to the frame it would be a
        // detection as big as the frame.
        if ![cx, cy, bw, bh].iter().all(|v| v.is_finite()) {
            continue;
        }
        let bbox = [
            ((cx - bw / 2.0 - lb.ox) / lb.s).clamp(0.0, w as f32),
            ((cy - bh / 2.0 - lb.oy) / lb.s).clamp(0.0, h as f32),
            ((cx + bw / 2.0 - lb.ox) / lb.s).clamp(0.0, w as f32),
            ((cy + bh / 2.0 - lb.oy) / lb.s).clamp(0.0, h as f32),
        ];
        if bbox[2] > bbox[0] && bbox[3] > bbox[1] {
            found.push(Det { bbox, score });
        }
    }
    found
}

/// Greedy non-maximum suppression, strongest first: a box overlapping a
/// stronger one kept before by more than [`NMS_IOU`] goes.
fn nms(mut dets: Vec<Det>) -> Vec<Det> {
    dets.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut keep: Vec<Det> = Vec::with_capacity(dets.len());
    for d in dets {
        if keep.iter().all(|k| iou(&k.bbox, &d.bbox) <= NMS_IOU) {
            keep.push(d);
        }
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_letterbox_scales_to_fit_and_centres() {
        // A 1280×720 frame in a 960×544 input: width binds, height floats.
        let lb = Letterbox::new((1280, 720), (960, 544));
        assert_eq!((lb.s, lb.nw, lb.nh, lb.px, lb.py, lb.ox, lb.oy), (0.75, 960, 540, 0, 2, 0.0, 2.0));
        // A wide frame into a square input, unscaled.
        let lb = Letterbox::new((640, 360), (640, 640));
        assert_eq!((lb.s, lb.nw, lb.nh, lb.px, lb.py, lb.ox, lb.oy), (1.0, 640, 360, 0, 140, 0.0, 140.0));
        // Upscaled small frame, centred on the canvas.
        let lb = Letterbox::new((100, 50), (640, 640));
        assert_eq!((lb.s, lb.nw, lb.nh, lb.px, lb.py), (6.4, 640, 320, 0, 160));
        assert_eq!((lb.ox, lb.oy), (0.0, 160.0));
    }

    #[test]
    fn detections_come_back_in_frame_pixels() {
        let (w, h, iw, ih) = (1280, 720, 960, 544);
        let lb = Letterbox::new((w, h), (iw, ih));
        // A 200×150 source box at (100,50), forward-mapped into the canvas:
        // de-letterboxing must undo scale and offset exactly.
        let n = 3usize;
        let mut e = vec![0.0f32; 5 * n];
        e[0] = 200.0 * lb.s + lb.ox;
        e[1 * n] = 125.0 * lb.s + lb.oy;
        e[2 * n] = 200.0 * lb.s;
        e[3 * n] = 150.0 * lb.s;
        e[4 * n] = 0.9;
        // A too-quiet and a no-number row find nothing.
        e[4 * n + 1] = 0.1;
        e[4 * n + 2] = f32::NAN;
        let found = detections(&e, 0, n, true, (w, h), lb, SCORE_THRESHOLD);
        assert_eq!(found.len(), 1);
        assert!((found[0].bbox[0] - 100.0).abs() < 1e-3 && (found[0].bbox[1] - 50.0).abs() < 1e-3, "{:?}", found[0]);
        assert!((found[0].bbox[2] - 300.0).abs() < 1e-3 && (found[0].bbox[3] - 200.0).abs() < 1e-3, "{:?}", found[0]);
        // Boxes outside the frame are clipped to it.
        e[0] = lb.ox;
        e[1 * n] = lb.oy;
        e[2 * n] = 4.0 * iw as f32;
        e[3 * n] = 4.0 * ih as f32;
        e[4 * n] = 0.9;
        let found = detections(&e, 0, n, true, (w, h), lb, SCORE_THRESHOLD);
        assert_eq!(found[0].bbox, [0.0, 0.0, w as f32, h as f32]);
    }

    #[test]
    fn rows_of_either_layout_read_the_same() {
        // [B,5,3] and [B,3,5] holding the same three rows.
        let want = [[1.0, 2.0, 3.0, 4.0, 0.9], [5.0, 6.0, 7.0, 8.0, 0.8], [9.0, 10.0, 11.0, 12.0, 0.7]];
        let flat_t: Vec<f32> = (0..5).flat_map(|j| (0..3).map(move |i| want[i][j])).collect();
        let flat_n: Vec<f32> = want.into_iter().flatten().collect();
        assert_eq!(rows(&[2, 5, 3], 2), Some((3, true)));
        assert_eq!(rows(&[2, 3, 5], 2), Some((3, false)));
        for i in 0..3 {
            assert_eq!(row(&flat_t, 0, 3, i, true), want[i]);
            assert_eq!(row(&flat_n, 0, 3, i, false), want[i]);
        }
        // Ambiguous only when N = 5 — where both readings are the same — and
        // shapes that are no detections at all are refused.
        assert_eq!(rows(&[1, 5, 5], 1), Some((5, false)));
        assert_eq!(rows(&[1, 6, 9], 1), None);
        assert_eq!(rows(&[4, 5, 9], 1), None);
        assert_eq!(rows(&[5, 9], 1), None);
    }

    #[test]
    fn nms_keeps_the_strongest_of_overlapping_boxes() {
        let det = |x: f32, score| Det { bbox: [x, 0.0, x + 10.0, 10.0], score };
        // Boxes 3 apart overlap by IoU 7/13 ≈ 0.54 (goes); 6 apart 4/16 = 0.25 (stays).
        let kept = nms(vec![det(0.0, 0.9), det(3.0, 0.95), det(9.0, 0.86)]);
        assert_eq!(kept, [det(3.0, 0.95), det(9.0, 0.86)]);
        assert_eq!(nms(vec![det(0.0, 0.6), det(6.0, 0.9)]), [det(6.0, 0.9), det(0.0, 0.6)]);
    }
}
