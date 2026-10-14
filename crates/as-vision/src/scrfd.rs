//! SCRFD-10G (Guo et al. 2021, InsightFace `det_10g`): face boxes plus five
//! landmarks (eyes, nose tip, mouth corners) at three strides.
//!
//! The graph takes any input size that is a multiple of 32, so frames are
//! fed at their own aspect ratio (padded at the right and bottom) instead of
//! letterboxed into a square — no compute is spent on padding. A session is
//! planned for one frame size and batch: with the whole shape known, the GPU
//! provider runs the graph as one unit, and batches of a few frames keep it
//! busy (on an RTX 3060 Ti: 4.7 ms per 640×360 frame one at a time with a
//! free shape, 2.2 ms in fixed batches of 4).

use std::path::Path;

use as_infer::onnx::InputShape;
use as_infer::{Device, InferError, InferResult, Runtime, Session, TensorRef};

/// Strides of the three output levels, two anchors per location.
const STRIDES: [usize; 3] = [8, 16, 32];
const ANCHORS: usize = 2;
const NMS_IOU: f32 = 0.4;

/// Input value of each byte: (v − 127.5) / 128.
const NORM: [f32; 256] = {
    let mut t = [0f32; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as f32 - 127.5) / 128.0;
        i += 1;
    }
    t
};

/// The input size for a `w`×`h` frame: both sides padded to multiples of 32.
fn padded(w: usize, h: usize) -> (usize, usize) {
    (w.div_ceil(32) * 32, h.div_ceil(32) * 32)
}

/// One detected face, in the pixel coordinates of the frame given to
/// [`Scrfd::detect_many`] (before padding).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceBox {
    /// x1, y1, x2, y2.
    pub bbox: [f32; 4],
    pub score: f32,
    /// Left eye, right eye, nose, left and right mouth corner (as seen in the image).
    pub kps: [[f32; 2]; 5],
}

impl FaceBox {
    pub fn width(&self) -> f32 {
        self.bbox[2] - self.bbox[0]
    }

    pub fn height(&self) -> f32 {
        self.bbox[3] - self.bbox[1]
    }

    /// Scale every coordinate by `k` (detection ran on a downscaled frame).
    pub fn scaled(mut self, k: f32) -> Self {
        for v in &mut self.bbox {
            *v *= k;
        }
        for p in &mut self.kps {
            p[0] *= k;
            p[1] *= k;
        }
        self
    }

    /// How frontal the face is, 0 (profile) ..= 1 (frontal), from how far the
    /// nose sits from the middle of the eyes relative to the eye distance.
    pub fn frontalness(&self) -> f32 {
        let [l, r, nose, ..] = self.kps;
        let eye = ((r[0] - l[0]).powi(2) + (r[1] - l[1]).powi(2)).sqrt().max(1.0);
        let off = (nose[0] - (l[0] + r[0]) / 2.0).abs() / eye;
        (1.0 - 2.0 * off).clamp(0.0, 1.0)
    }
}

pub fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

pub struct Scrfd {
    session: Session,
    input: String,
    device: Device,
    /// The frame size the session is planned for.
    size: (usize, usize),
    /// Frames per inference.
    batch: usize,
    /// Input buffer (NCHW float). Frames only ever overwrite their own area,
    /// so the padding stays black.
    x: Vec<f32>,
}

impl Scrfd {
    /// Load SCRFD for `w`×`h` RGB24 frames, `batch` per inference. Frames of
    /// another size need another `Scrfd`.
    pub fn load(rt: &Runtime, model: &Path, (w, h): (usize, usize), batch: usize) -> InferResult<Self> {
        assert!(w > 0 && h > 0 && batch > 0, "SCRFD shape {w}×{h}, batch {batch}");
        let (pw, ph) = padded(w, h);
        let (session, device) = rt.session_shaped(model, InputShape::Fixed(&[batch, 3, ph, pw]))?;
        let input = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| InferError::Model(format!("{}: no input", model.display())))?;
        if session.outputs().len() != 9 {
            return Err(InferError::Model(format!(
                "{}: expected 9 outputs (scores, boxes, landmarks × 3 strides), got {}",
                model.display(),
                session.outputs().len()
            )));
        }
        tracing::info!(device = ?device, w, h, batch, "SCRFD ready");
        Ok(Self { session, input, device, size: (w, h), batch, x: vec![NORM[0]; batch * 3 * pw * ph] })
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

    /// Detect the faces in RGB24 frames of [`Scrfd::frame_size`], any number
    /// (the last inference runs on a partial batch). Boxes come back per
    /// frame, in its coordinates, NMS applied, strongest first.
    pub fn detect_many(&mut self, frames: &[&[u8]], threshold: f32) -> InferResult<Vec<Vec<FaceBox>>> {
        let mut out = Vec::with_capacity(frames.len());
        for group in frames.chunks(self.batch) {
            out.extend(self.infer(group, threshold)?);
        }
        Ok(out)
    }

    /// One inference on up to `batch` frames. Slots past the given frames
    /// keep earlier pixels; their results are ignored.
    fn infer(&mut self, frames: &[&[u8]], threshold: f32) -> InferResult<Vec<Vec<FaceBox>>> {
        let (w, h) = self.size;
        let (pw, ph) = padded(w, h);
        let plane = pw * ph;
        for (slot, rgb) in self.x.chunks_exact_mut(3 * plane).zip(frames) {
            assert_eq!(rgb.len(), w * h * 3, "SCRFD frame size");
            let (r, rest) = slot.split_at_mut(plane);
            let (g, b) = rest.split_at_mut(plane);
            for (y, row) in rgb.chunks_exact(w * 3).enumerate() {
                let o = y * pw..y * pw + w;
                let dst = r[o.clone()].iter_mut().zip(&mut g[o.clone()]).zip(&mut b[o]);
                for (px, ((r, g), b)) in row.as_chunks::<3>().0.iter().zip(dst) {
                    *r = NORM[px[0] as usize];
                    *g = NORM[px[1] as usize];
                    *b = NORM[px[2] as usize];
                }
            }
        }
        let batch = self.batch;
        let t = TensorRef::from_array_view(([batch, 3, ph, pw], &self.x[..]))?;
        let outs = self.session.run(as_infer::inputs![self.input.as_str() => t])?;
        let mut faces: Vec<Vec<FaceBox>> = vec![Vec::new(); frames.len()];
        // Outputs: scores for strides 8/16/32, then boxes, then landmarks.
        // Each head is transposed to (y, x, frame, anchor) before flattening.
        for (level, &stride) in STRIDES.iter().enumerate() {
            let (gw, gh) = (pw / stride, ph / stride);
            let locs = gw * gh;
            let n = locs * batch * ANCHORS;
            let (_, scores) = outs[level].try_extract_tensor::<f32>()?;
            let (_, boxes) = outs[level + 3].try_extract_tensor::<f32>()?;
            let (_, kps) = outs[level + 6].try_extract_tensor::<f32>()?;
            if scores.len() != n || boxes.len() != n * 4 || kps.len() != n * 10 {
                return Err(InferError::Run(format!(
                    "SCRFD stride {stride}: {} / {} / {} values for {n} anchors",
                    scores.len(),
                    boxes.len(),
                    kps.len()
                )));
            }
            let s = stride as f32;
            for loc in 0..locs {
                let (cx, cy) = (((loc % gw) * stride) as f32, ((loc / gw) * stride) as f32);
                for (f, found) in faces.iter_mut().enumerate() {
                    for a in 0..ANCHORS {
                        let i = (loc * batch + f) * ANCHORS + a;
                        let score = scores[i];
                        if score < threshold {
                            continue;
                        }
                        let bx = &boxes[i * 4..i * 4 + 4];
                        let k = &kps[i * 10..i * 10 + 10];
                        let mut p = [[0f32; 2]; 5];
                        for (j, pt) in p.iter_mut().enumerate() {
                            *pt = [cx + k[2 * j] * s, cy + k[2 * j + 1] * s];
                        }
                        let bbox = [
                            (cx - bx[0] * s).max(0.0),
                            (cy - bx[1] * s).max(0.0),
                            (cx + bx[2] * s).min(w as f32),
                            (cy + bx[3] * s).min(h as f32),
                        ];
                        if bbox[2] > bbox[0] && bbox[3] > bbox[1] {
                            found.push(FaceBox { bbox, score, kps: p });
                        }
                    }
                }
            }
        }
        drop(outs);
        Ok(faces.into_iter().map(nms).collect())
    }
}

/// Greedy non-maximum suppression, strongest first.
fn nms(mut faces: Vec<FaceBox>) -> Vec<FaceBox> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
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

    fn face(x: f32, score: f32) -> FaceBox {
        FaceBox { bbox: [x, 0.0, x + 10.0, 10.0], score, kps: [[0.0; 2]; 5] }
    }

    #[test]
    fn nms_keeps_the_strongest_of_overlapping_boxes() {
        let kept = nms(vec![face(0.0, 0.6), face(1.0, 0.9), face(30.0, 0.7)]);
        assert_eq!(kept.iter().map(|f| f.score).collect::<Vec<_>>(), [0.9, 0.7]);
    }

    #[test]
    fn inputs_pad_to_multiples_of_32() {
        assert_eq!(padded(640, 360), (640, 384));
        assert_eq!(padded(1280, 720), (1280, 736));
        assert_eq!(padded(32, 1), (32, 32));
        assert_eq!((NORM[0], NORM[255]), (-127.5 / 128.0, 127.5 / 128.0));
    }

    #[test]
    fn frontalness_falls_with_yaw() {
        let mut f = face(0.0, 1.0);
        f.kps = [[10.0, 10.0], [30.0, 10.0], [20.0, 20.0], [12.0, 30.0], [28.0, 30.0]];
        assert!((f.frontalness() - 1.0).abs() < 1e-6);
        f.kps[2] = [28.0, 20.0];
        assert!(f.frontalness() < 0.3);
    }
}
