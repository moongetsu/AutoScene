//! People in a frame: where they stand and which part of them shows.
//!
//! Two detector families run here:
//! - PaddleDetection's pedestrian detectors as InsightFace ships them in its
//!   `cheetah` packages — `pp_det_med.onnx` (PP-YOLOE, 640×640) and
//!   `pp_det_small_320.onnx` (PicoDet, 320×320) — through [`PpDet`]. They
//!   take an RGB image normalised with ImageNet statistics ([`Norm`]) plus a
//!   `scale_factor`, and return their candidates before non-maximum
//!   suppression, in input pixels when the scale factor is one. The exports
//!   handle one frame per run: given a batch, they return the boxes of every
//!   frame as one list.
//! - SCRFD's person model (`scrfd_person_2.5g.onnx`) through [`crate::Scrfd`]
//!   with [`crate::scrfd::Heads::PERSONS`]; [`from_scrfd`] reads its points.

use std::path::Path;

use as_infer::onnx::InputShape;
use as_infer::{Device, InferError, InferResult, Runtime, Session, TensorRef};

use crate::image;
use crate::scrfd::{FaceBox, iou};

/// Overlap at which the weaker of two people is dropped (InsightFace's value:
/// people in a two-shot overlap more than faces do).
const NMS_IOU: f32 = 0.6;
/// People kept per frame at most.
const MAX_PEOPLE: usize = 100;

/// A person in a frame, in the frame's pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PersonBox {
    /// x1, y1, x2, y2 of the whole person, clipped to the frame.
    pub bbox: [f32; 4],
    /// The part of them that shows (the same box for detectors that only
    /// find that part).
    pub visible: [f32; 4],
    pub score: f32,
}

impl PersonBox {
    /// Height of the visible part, in pixels.
    pub fn height(&self) -> f32 {
        self.visible[3] - self.visible[1]
    }
}

/// A person found by SCRFD's person model: its box is the whole body, its
/// first and last points the corners of the part that shows. Points on top
/// of each other leave the whole box as the visible part.
pub fn from_scrfd(b: &FaceBox, (w, h): (usize, usize)) -> PersonBox {
    let ([px, py], [qx, qy]) = (b.kps[0], b.kps[4]);
    let (x0, x1) = (px.min(qx).clamp(0.0, w as f32), px.max(qx).clamp(0.0, w as f32));
    let (y0, y1) = (py.min(qy).clamp(0.0, h as f32), py.max(qy).clamp(0.0, h as f32));
    let visible = if x1 - x0 >= 1.0 && y1 - y0 >= 1.0 { [x0, y0, x1, y1] } else { b.bbox };
    PersonBox { bbox: b.bbox, visible, score: b.score }
}

/// Input normalisation: (v − mean) / std per RGB channel, v in 0..=255.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Norm {
    pub mean: [f32; 3],
    pub std: [f32; 3],
}

impl Norm {
    /// ImageNet statistics in 0..=255: what PaddleDetection's detectors and
    /// OpenVINO's person ReID take (InsightFace's `cheetah` manifests).
    pub const IMAGENET: Self = Self { mean: [123.675, 116.28, 103.53], std: [58.395, 57.12, 57.375] };

    /// The input value of each byte, per channel.
    pub(crate) fn tables(&self) -> [[f32; 256]; 3] {
        std::array::from_fn(|c| std::array::from_fn(|v| (v as f32 - self.mean[c]) / self.std[c]))
    }
}

/// How a frame fits into a detector's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Stretched over the whole input, its shape not kept — how PP-Human
    /// trains these detectors and how InsightFace runs them.
    Squeeze,
    /// Scaled to fit with its shape kept; the rest of the input (right and
    /// bottom) stays neutral.
    Pad,
}

/// The size the frame takes inside the input.
fn content_size((w, h): (usize, usize), (iw, ih): (usize, usize), fit: Fit) -> (usize, usize) {
    match fit {
        Fit::Squeeze => (iw, ih),
        Fit::Pad => {
            let k = (iw as f64 / w as f64).min(ih as f64 / h as f64);
            (((w as f64 * k).round() as usize).clamp(1, iw), ((h as f64 * k).round() as usize).clamp(1, ih))
        }
    }
}

/// The people among a detector's candidates (`boxes`: x1, y1, x2, y2 in input
/// pixels; one score each) scoring at least `threshold`, in the pixels of a
/// `frame` that filled `content` of the input; strongest first, overlaps
/// suppressed.
fn people(
    boxes: &[f32],
    scores: &[f32],
    threshold: f32,
    (w, h): (usize, usize),
    (cw, ch): (usize, usize),
) -> Vec<PersonBox> {
    let (sx, sy) = (w as f32 / cw as f32, h as f32 / ch as f32);
    let mut found = Vec::new();
    for (b, &score) in boxes.as_chunks::<4>().0.iter().zip(scores) {
        // A NaN score is no person.
        if score.is_nan() || score < threshold {
            continue;
        }
        let bbox = [
            (b[0] * sx).clamp(0.0, w as f32),
            (b[1] * sy).clamp(0.0, h as f32),
            (b[2] * sx).clamp(0.0, w as f32),
            (b[3] * sy).clamp(0.0, h as f32),
        ];
        if bbox[2] > bbox[0] && bbox[3] > bbox[1] {
            found.push(PersonBox { bbox, visible: bbox, score });
        }
    }
    nms(found, NMS_IOU, MAX_PEOPLE)
}

/// Greedy non-maximum suppression, strongest first (equal scores in
/// candidate order), at most `max` kept.
fn nms(mut people: Vec<PersonBox>, max_iou: f32, max: usize) -> Vec<PersonBox> {
    people.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut keep: Vec<PersonBox> = Vec::new();
    for p in people {
        if keep.len() == max {
            break;
        }
        if keep.iter().all(|k| iou(&k.bbox, &p.bbox) <= max_iou) {
            keep.push(p);
        }
    }
    keep
}

/// A PaddleDetection pedestrian detector, planned for one frame size and
/// one input size.
pub struct PpDet {
    session: Session,
    image: String,
    scale: Option<String>,
    boxes: usize,
    scores: usize,
    device: Device,
    frame: (usize, usize),
    input: (usize, usize),
    content: (usize, usize),
    tables: [[f32; 256]; 3],
    /// The frame resized to `content` (RGB24).
    canvas: Vec<u8>,
    /// Input buffer (NCHW float); what `content` doesn't cover stays 0, the
    /// normalised mean colour.
    x: Vec<f32>,
}

impl PpDet {
    /// Load `model` for `frame`-sized RGB24 frames fed at `input` size (both
    /// width × height; the input's sides multiples of 32).
    pub fn load(
        rt: &Runtime,
        model: &Path,
        frame: (usize, usize),
        input: (usize, usize),
        fit: Fit,
        norm: Norm,
    ) -> InferResult<Self> {
        let ((w, h), (iw, ih)) = (frame, input);
        assert!(w > 0 && h > 0 && iw > 0 && ih > 0, "detector shape {w}×{h} → {iw}×{ih}");
        let bad = |what: String| InferError::Model(format!("{}: {what}", model.display()));
        // `Fixed` shapes the graph's first input: it must be the image.
        let (session, device) = rt.session_shaped(model, InputShape::Fixed(&[1, 3, ih, iw]))?;
        let inputs: Vec<String> = session.inputs().iter().map(|i| i.name().to_string()).collect();
        let image = inputs
            .first()
            .filter(|n| *n != "scale_factor")
            .cloned()
            .ok_or_else(|| bad(format!("inputs {inputs:?}")))?;
        let scale = inputs.iter().find(|n| *n == "scale_factor").cloned();
        let outputs: Vec<String> = session.outputs().iter().map(|o| o.name().to_string()).collect();
        let at = |name: &str, or: usize| outputs.iter().position(|o| o == name).unwrap_or(or);
        let (boxes, scores) = (at("nms_pre_boxes", 0), at("nms_pre_scores", 1));
        if outputs.len() < 2 || boxes == scores {
            return Err(bad(format!("outputs {outputs:?}")));
        }
        let content = content_size(frame, input, fit);
        tracing::info!(device = ?device, w, h, iw, ih, ?fit, "person detector ready");
        Ok(Self {
            session,
            image,
            scale,
            boxes,
            scores,
            device,
            frame,
            input,
            content,
            tables: norm.tables(),
            canvas: vec![0; content.0 * content.1 * 3],
            x: vec![0.0; 3 * iw * ih],
        })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    /// The frame size this detector takes.
    pub fn frame_size(&self) -> (usize, usize) {
        self.frame
    }

    /// The people in an RGB24 frame of [`PpDet::frame_size`] scoring at
    /// least `threshold`, strongest first.
    pub fn detect(&mut self, rgb: &[u8], threshold: f32) -> InferResult<Vec<PersonBox>> {
        let ((w, h), (iw, ih), (cw, ch)) = (self.frame, self.input, self.content);
        assert_eq!(rgb.len(), w * h * 3, "person detector frame size");
        if cw <= w && ch <= h {
            image::shrink_into(rgb, (w, h), &mut self.canvas, cw, 0, (cw, ch));
        } else {
            self.canvas = image::resize_region(rgb, (w, h), [0.0, 0.0, w as f32, h as f32], (cw, ch));
        }
        let plane = iw * ih;
        let (r, rest) = self.x.split_at_mut(plane);
        let (g, b) = rest.split_at_mut(plane);
        let t = &self.tables;
        for (y, row) in self.canvas.chunks_exact(cw * 3).enumerate() {
            let o = y * iw..y * iw + cw;
            let dst = r[o.clone()].iter_mut().zip(&mut g[o.clone()]).zip(&mut b[o]);
            for (px, ((r, g), b)) in row.as_chunks::<3>().0.iter().zip(dst) {
                *r = t[0][px[0] as usize];
                *g = t[1][px[1] as usize];
                *b = t[2][px[2] as usize];
            }
        }
        let image = TensorRef::from_array_view(([1, 3, ih, iw], &self.x[..]))?;
        let one = [1f32; 2];
        let outs = match &self.scale {
            Some(s) => {
                let scale = TensorRef::from_array_view(([1usize, 2], &one[..]))?;
                self.session.run(as_infer::inputs![self.image.as_str() => image, s.as_str() => scale])?
            }
            None => self.session.run(as_infer::inputs![self.image.as_str() => image])?,
        };
        let (_, boxes) = outs[self.boxes].try_extract_tensor::<f32>()?;
        let (_, scores) = outs[self.scores].try_extract_tensor::<f32>()?;
        if boxes.len() != scores.len() * 4 {
            return Err(InferError::Run(format!(
                "person detector: {} box values for {} scores",
                boxes.len(),
                scores.len()
            )));
        }
        Ok(people(boxes, scores, threshold, self.frame, self.content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(x: f32, score: f32) -> PersonBox {
        let bbox = [x, 0.0, x + 10.0, 20.0];
        PersonBox { bbox, visible: bbox, score }
    }

    #[test]
    fn overlapping_people_are_suppressed_and_capped() {
        // 4 apart: IoU 0.43, two people; 2 apart: IoU 0.67, one.
        let kept = nms(vec![person(0.0, 0.7), person(4.0, 0.9), person(6.0, 0.8)], NMS_IOU, 10);
        assert_eq!(kept.iter().map(|p| p.score).collect::<Vec<_>>(), [0.9, 0.7]);
        let many: Vec<PersonBox> = (0..5).map(|i| person(100.0 * i as f32, 0.5)).collect();
        assert_eq!(nms(many, NMS_IOU, 3).iter().map(|p| p.bbox[0]).collect::<Vec<_>>(), [0.0, 100.0, 200.0]);
    }

    #[test]
    fn candidates_come_back_in_frame_pixels() {
        // A 1280×720 frame squeezed into 640×640, and padded into 640×384.
        let frame = (1280, 720);
        assert_eq!(content_size(frame, (640, 640), Fit::Squeeze), (640, 640));
        assert_eq!(content_size(frame, (640, 384), Fit::Pad), (640, 360));
        assert_eq!(content_size((720, 480), (640, 384), Fit::Pad), (576, 384));
        let boxes = [320.0, 320.0, 640.0, 640.0, 0.0, 0.0, 10.0, 10.0, 5.0, 5.0, 4.0, 9.0, 0.0, 0.0, 1.0, 1.0];
        let scores = [0.9, 0.2, 0.8, f32::NAN];
        let found = people(&boxes, &scores, 0.5, frame, (640, 640));
        // Low, NaN and empty candidates go.
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bbox, [640.0, 360.0, 1280.0, 720.0]);
        // Padded: input pixels scale by 2 both ways; past the frame is clipped.
        let found = people(&[0.0, 0.0, 640.0, 400.0], &[0.6], 0.5, frame, (640, 360));
        assert_eq!(found[0].bbox, [0.0, 0.0, 1280.0, 720.0]);
        assert_eq!(found[0].visible, found[0].bbox);
    }

    #[test]
    fn scrfd_points_are_the_visible_part() {
        let mut b = FaceBox { bbox: [100.0, -50.0, 300.0, 700.0], score: 0.8, kps: [[0.0; 2]; 5] };
        b.kps[0] = [290.0, 20.0];
        b.kps[4] = [110.0, 500.0];
        let p = from_scrfd(&b, (640, 360));
        assert_eq!(p.visible, [110.0, 20.0, 290.0, 360.0], "ordered and clipped to the frame");
        assert_eq!(p.height(), 340.0);
        b.kps[4] = b.kps[0];
        assert_eq!(from_scrfd(&b, (640, 360)).visible, b.bbox);
    }

    #[test]
    fn imagenet_tables() {
        let t = Norm::IMAGENET.tables();
        assert!((t[0][0] + 123.675 / 58.395).abs() < 1e-6);
        assert!((t[2][255] - (255.0 - 103.53) / 57.375).abs() < 1e-6);
    }
}
