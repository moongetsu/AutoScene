//! YuNet (Wu et al. 2023; OpenCV Zoo `face_detection_yunet_2023mar`): a
//! small face detector — boxes plus five points — and Fast Scan's (the
//! pipeline of the Scener app) choice for speed over SCRFD-10G.
//!
//! The graph takes any input size that is a multiple of 32 (padded at the
//! right and bottom, as OpenCV's `FaceDetectorYN` does) and pins its batch
//! at 1 in its output Reshapes; both are fixed in memory when a session is
//! planned for a frame size and batch (see [`Runtime::session_batched`]).
//! It reads BGR bytes as they are (0..255), heads at strides 8, 16 and 32
//! with one anchor per location: a class and an objectness score (the face
//! score is the geometric mean of the two), a box (centre offset in cells,
//! log size) and five points (offsets in cells).

use std::path::Path;

use as_infer::onnx::InputShape;
use as_infer::{Device, InferError, InferResult, Runtime, Session, TensorRef};

use crate::scrfd::{FaceBox, iou};

/// Input sides are padded to multiples of the largest stride.
const MULTIPLE: usize = 32;
const STRIDES: [usize; 3] = [8, 16, 32];
/// The graph's outputs per stride: class score, objectness, box, points.
const HEADS: [&str; 4] = ["cls", "obj", "bbox", "kps"];

/// Scener's settings for the detector (`YUNET_SCORE_THRESHOLD`,
/// `YUNET_NMS_THRESHOLD`, `YUNET_TOP_K` of its worker, recovered from the
/// shipped binary): faces from this score ...
pub const SCORE_THRESHOLD: f32 = 0.85;
/// ... the weaker of two dropped from this overlap ...
pub const NMS_IOU: f32 = 0.3;
/// ... and at most this many candidates looked at per frame.
pub const TOP_K: usize = 5000;

/// The input size for a `w`×`h` frame.
fn padded(w: usize, h: usize) -> (usize, usize) {
    (w.div_ceil(MULTIPLE) * MULTIPLE, h.div_ceil(MULTIPLE) * MULTIPLE)
}

pub struct YuNet {
    session: Session,
    input: String,
    /// Output positions of [`HEADS`] × [`STRIDES`], in that order.
    outputs: [[usize; 3]; 4],
    device: Device,
    /// The frame size the session is planned for.
    size: (usize, usize),
    batch: usize,
    /// Input buffer (NCHW float, BGR). Frames only ever overwrite their own
    /// area, so the padding stays black.
    x: Vec<f32>,
}

impl YuNet {
    /// Load the detector for `w`×`h` RGB24 frames, `batch` per inference.
    pub fn load(rt: &Runtime, model: &Path, (w, h): (usize, usize), batch: usize) -> InferResult<Self> {
        assert!(w > 0 && h > 0 && batch > 0, "YuNet shape {w}×{h}, batch {batch}");
        let (pw, ph) = padded(w, h);
        let (session, device) = rt.session_batched(model, InputShape::Fixed(&[batch, 3, ph, pw]))?;
        let input = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| InferError::Model(format!("{}: no input", model.display())))?;
        let names: Vec<String> = session.outputs().iter().map(|o| o.name().to_string()).collect();
        let mut outputs = [[0usize; 3]; 4];
        for (head, slots) in HEADS.iter().zip(&mut outputs) {
            for (stride, slot) in STRIDES.iter().zip(slots.iter_mut()) {
                let name = format!("{head}_{stride}");
                *slot = names.iter().position(|n| *n == name).ok_or_else(|| {
                    InferError::Model(format!("{}: no output {name} (has {names:?})", model.display()))
                })?;
            }
        }
        tracing::info!(device = ?device, w, h, batch, "YuNet ready");
        Ok(Self { session, input, outputs, device, size: (w, h), batch, x: vec![0.0; batch * 3 * pw * ph] })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    /// The frame size this detector takes.
    pub fn frame_size(&self) -> (usize, usize) {
        self.size
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    /// Detect the faces in RGB24 frames of [`YuNet::frame_size`], any number
    /// (the last inference runs on a partial batch). Boxes come back per
    /// frame, in its coordinates, strongest first, NMS applied.
    pub fn detect_many(&mut self, frames: &[&[u8]], threshold: f32) -> InferResult<Vec<Vec<FaceBox>>> {
        let mut out = Vec::with_capacity(frames.len());
        for group in frames.chunks(self.batch) {
            out.extend(self.infer(group, threshold)?);
        }
        Ok(out)
    }

    fn infer(&mut self, frames: &[&[u8]], threshold: f32) -> InferResult<Vec<Vec<FaceBox>>> {
        let (w, h) = self.size;
        let (pw, ph) = padded(w, h);
        let plane = pw * ph;
        for (slot, rgb) in self.x.chunks_exact_mut(3 * plane).zip(frames) {
            assert_eq!(rgb.len(), w * h * 3, "YuNet frame size");
            let (b, rest) = slot.split_at_mut(plane);
            let (g, r) = rest.split_at_mut(plane);
            for (y, row) in rgb.chunks_exact(w * 3).enumerate() {
                let o = y * pw..y * pw + w;
                let dst = b[o.clone()].iter_mut().zip(&mut g[o.clone()]).zip(&mut r[o]);
                for (px, ((b, g), r)) in row.as_chunks::<3>().0.iter().zip(dst) {
                    (*b, *g, *r) = (f32::from(px[2]), f32::from(px[1]), f32::from(px[0]));
                }
            }
        }
        let batch = self.batch;
        let t = TensorRef::from_array_view(([batch, 3, ph, pw], &self.x[..]))?;
        let outs = self.session.run(as_infer::inputs![self.input.as_str() => t])?;
        let mut faces: Vec<Vec<FaceBox>> = vec![Vec::new(); frames.len()];
        for (level, &stride) in STRIDES.iter().enumerate() {
            let (cols, rows) = (pw / stride, ph / stride);
            let n = cols * rows;
            let [cls, obj, bbox, kps] = self.outputs.map(|o| o[level]);
            let (_, cls) = outs[cls].try_extract_tensor::<f32>()?;
            let (_, obj) = outs[obj].try_extract_tensor::<f32>()?;
            let (_, bbox) = outs[bbox].try_extract_tensor::<f32>()?;
            let (_, kps) = outs[kps].try_extract_tensor::<f32>()?;
            if cls.len() != n * batch
                || obj.len() != n * batch
                || bbox.len() != n * batch * 4
                || kps.len() != n * batch * 10
            {
                return Err(InferError::Run(format!(
                    "YuNet stride {stride}: {} / {} / {} / {} values for {n} cells × {batch}",
                    cls.len(),
                    obj.len(),
                    bbox.len(),
                    kps.len()
                )));
            }
            let s = stride as f32;
            for (f, found) in faces.iter_mut().enumerate() {
                for cell in 0..n {
                    let i = f * n + cell;
                    let score = (cls[i].clamp(0.0, 1.0) * obj[i].clamp(0.0, 1.0)).sqrt();
                    // A score that is no number (a faulty execution provider)
                    // passes no threshold.
                    if score.is_nan() || score < threshold {
                        continue;
                    }
                    let (c, r) = ((cell % cols) as f32, (cell / cols) as f32);
                    let b = &bbox[i * 4..i * 4 + 4];
                    let (cx, cy) = ((c + b[0]) * s, (r + b[1]) * s);
                    let (bw, bh) = (b[2].exp() * s, b[3].exp() * s);
                    let k = &kps[i * 10..i * 10 + 10];
                    let mut p = [[0f32; 2]; 5];
                    for (j, pt) in p.iter_mut().enumerate() {
                        *pt = [(k[2 * j] + c) * s, (k[2 * j + 1] + r) * s];
                    }
                    // Nor does a box or landmark that is no number: clipped
                    // to the frame it would be a face as big as the frame.
                    if ![cx, cy, bw, bh].iter().chain(p.iter().flatten()).all(|v| v.is_finite()) {
                        continue;
                    }
                    let bbox = [
                        (cx - bw / 2.0).max(0.0),
                        (cy - bh / 2.0).max(0.0),
                        (cx + bw / 2.0).min(w as f32),
                        (cy + bh / 2.0).min(h as f32),
                    ];
                    if bbox[2] > bbox[0] && bbox[3] > bbox[1] {
                        found.push(FaceBox { bbox, score, kps: p });
                    }
                }
            }
        }
        drop(outs);
        Ok(faces.into_iter().map(nms).collect())
    }
}

/// OpenCV's `NMSBoxes` as YuNet's demo runs it: the strongest [`TOP_K`]
/// candidates, a box overlapping a stronger one kept by more than
/// [`NMS_IOU`] dropped.
fn nms(mut faces: Vec<FaceBox>) -> Vec<FaceBox> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    faces.truncate(TOP_K);
    let mut keep: Vec<FaceBox> = Vec::with_capacity(faces.len());
    for f in faces {
        if keep.iter().all(|k| iou(&k.bbox, &f.bbox) <= NMS_IOU) {
            keep.push(f);
        }
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_are_padded_to_multiples_of_32() {
        assert_eq!(padded(1280, 720), (1280, 736));
        assert_eq!(padded(640, 268), (640, 288));
        assert_eq!(padded(32, 32), (32, 32));
    }

    #[test]
    fn nms_drops_overlaps_beyond_its_bar() {
        let face = |x: f32, score| FaceBox { bbox: [x, 0.0, x + 10.0, 10.0], score, kps: [[0.0; 2]; 5] };
        // IoU of boxes 3 apart: 7/13 = 0.54 (goes); 6 apart: 4/16 = 0.25 (stays).
        let kept = nms(vec![face(0.0, 0.9), face(3.0, 0.95), face(9.0, 0.86)]);
        assert_eq!(kept, [face(3.0, 0.95), face(9.0, 0.86)]);
    }
}
