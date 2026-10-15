//! ArcFace (Deng et al. 2019; InsightFace `glintr100`, ResNet-100 trained
//! on Glint360K — any InsightFace-style recognition graph runs here): a
//! 512-d identity embedding of a face aligned to 112×112 by its five
//! landmarks. Two faces of one person point the same way, so cosine
//! similarity is the identity score.
//!
//! The graph's batch dimension is free, so faces are embedded in fixed-size
//! batches (padded at the end) and the session is planned for exactly that
//! shape.

use std::path::Path;

use as_infer::onnx::InputShape;
use as_infer::{Device, InferError, InferResult, Runtime, Session, TensorRef};

/// Side of the aligned face crop.
pub const CROP: usize = 112;
/// Bytes of one aligned RGB24 crop.
pub const CROP_BYTES: usize = CROP * CROP * 3;
/// Embedding size.
pub const DIM: usize = 512;

/// Landmark positions of the 112×112 ArcFace template (eyes, nose, mouth corners).
const TEMPLATE: [[f32; 2]; 5] =
    [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];

/// One embedding: unit length, plus the length the network gave it (low for
/// blurry, dark or odd crops — a quality hint).
#[derive(Clone, Debug, PartialEq)]
pub struct Embedding {
    pub v: Vec<f32>,
    pub norm: f32,
}

pub struct ArcFace {
    session: Session,
    input: String,
    device: Device,
    batch: usize,
    x: Vec<f32>,
}

impl ArcFace {
    /// `batch`: faces per inference call (the last call of a run is padded).
    pub fn load(rt: &Runtime, model: &Path, batch: usize) -> InferResult<Self> {
        assert!(batch > 0);
        let (session, device) = rt.session_shaped(model, InputShape::Fixed(&[batch, 3, CROP, CROP]))?;
        let input = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| InferError::Model(format!("{}: no input", model.display())))?;
        tracing::info!(device = ?device, batch, "ArcFace ready");
        Ok(Self { session, input, device, batch, x: vec![0.0; batch * 3 * CROP * CROP] })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    /// Embed aligned crops (each [`CROP_BYTES`] of RGB24), any number.
    pub fn embed(&mut self, crops: &[&[u8]]) -> InferResult<Vec<Embedding>> {
        let mut out = Vec::with_capacity(crops.len());
        for group in crops.chunks(self.batch) {
            let plane = CROP * CROP;
            self.x.fill(0.0);
            for (i, crop) in group.iter().enumerate() {
                assert_eq!(crop.len(), CROP_BYTES, "aligned crop size");
                let base = i * 3 * plane;
                for (p, px) in crop.as_chunks::<3>().0.iter().enumerate() {
                    // (v - 127.5) / 127.5, RGB, NCHW.
                    self.x[base + p] = (px[0] as f32 - 127.5) / 127.5;
                    self.x[base + plane + p] = (px[1] as f32 - 127.5) / 127.5;
                    self.x[base + 2 * plane + p] = (px[2] as f32 - 127.5) / 127.5;
                }
            }
            let t = TensorRef::from_array_view(([self.batch, 3, CROP, CROP], &self.x[..]))?;
            let outs = self.session.run(as_infer::inputs![self.input.as_str() => t])?;
            let (_, e) = outs[0].try_extract_tensor::<f32>()?;
            if e.len() != self.batch * DIM {
                return Err(InferError::Run(format!(
                    "ArcFace returned {} values for a batch of {}",
                    e.len(),
                    self.batch
                )));
            }
            for i in 0..group.len() {
                let raw = &e[i * DIM..(i + 1) * DIM];
                let norm = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
                let k = 1.0 / norm.max(1e-6);
                out.push(Embedding { v: raw.iter().map(|x| x * k).collect(), norm });
            }
        }
        Ok(out)
    }
}

/// The similarity transform (rotation, uniform scale, translation) that maps
/// `kps` onto the ArcFace template, least squares (Umeyama without
/// reflection). Returns `(a, b, tx, ty)` for `x' = a·x − b·y + tx`,
/// `y' = b·x + a·y + ty`.
fn similarity(kps: &[[f32; 2]; 5]) -> (f32, f32, f32, f32) {
    let mean = |p: &[[f32; 2]; 5]| {
        let (sx, sy) = p.iter().fold((0.0, 0.0), |(x, y), v| (x + v[0], y + v[1]));
        [sx / 5.0, sy / 5.0]
    };
    let (sm, dm) = (mean(kps), mean(&TEMPLATE));
    let (mut num_a, mut num_b, mut den) = (0f32, 0f32, 0f32);
    for (s, d) in kps.iter().zip(TEMPLATE.iter()) {
        let (sx, sy, dx, dy) = (s[0] - sm[0], s[1] - sm[1], d[0] - dm[0], d[1] - dm[1]);
        num_a += sx * dx + sy * dy;
        num_b += sx * dy - sy * dx;
        den += sx * sx + sy * sy;
    }
    let den = den.max(1e-6);
    let (a, b) = (num_a / den, num_b / den);
    (a, b, dm[0] - (a * sm[0] - b * sm[1]), dm[1] - (b * sm[0] + a * sm[1]))
}

/// Cut the face with landmarks `kps` out of a `w`×`h` RGB24 frame, aligned to
/// the ArcFace template: [`CROP_BYTES`] of RGB24 (bilinear, black outside).
pub fn align(rgb: &[u8], w: usize, h: usize, kps: &[[f32; 2]; 5]) -> Vec<u8> {
    let (a, b, tx, ty) = similarity(kps);
    let det = (a * a + b * b).max(1e-12);
    let mut out = vec![0u8; CROP_BYTES];
    for v in 0..CROP {
        for u in 0..CROP {
            // Inverse transform: template pixel centre → source position.
            let (px, py) = (u as f32 - tx, v as f32 - ty);
            let sx = (a * px + b * py) / det;
            let sy = (-b * px + a * py) / det;
            let o = (v * CROP + u) * 3;
            out[o..o + 3].copy_from_slice(&bilinear(rgb, w, h, sx, sy));
        }
    }
    out
}

/// RGB at a fractional position (black outside the frame).
pub(crate) fn bilinear(rgb: &[u8], w: usize, h: usize, x: f32, y: f32) -> [u8; 3] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xx: f32, yy: f32, c: usize| -> f32 {
        if xx < 0.0 || yy < 0.0 || xx >= w as f32 || yy >= h as f32 {
            0.0
        } else {
            rgb[(yy as usize * w + xx as usize) * 3 + c] as f32
        }
    };
    let mut px = [0u8; 3];
    for (c, p) in px.iter_mut().enumerate() {
        let v = at(x0, y0, c) * (1.0 - fx) * (1.0 - fy)
            + at(x0 + 1.0, y0, c) * fx * (1.0 - fy)
            + at(x0, y0 + 1.0, c) * (1.0 - fx) * fy
            + at(x0 + 1.0, y0 + 1.0, c) * fx * fy;
        *p = v.round().clamp(0.0, 255.0) as u8;
    }
    px
}

/// Sharpness of an aligned crop: variance of the Laplacian over the inner
/// face (eyes to mouth). Blurred or heavily compressed faces score low.
pub fn sharpness(crop: &[u8]) -> f32 {
    assert_eq!(crop.len(), CROP_BYTES);
    let g = |x: usize, y: usize| {
        let p = (y * CROP + x) * 3;
        0.299 * crop[p] as f32 + 0.587 * crop[p + 1] as f32 + 0.114 * crop[p + 2] as f32
    };
    let (mut sum, mut sq, mut n) = (0f64, 0f64, 0f64);
    for y in 30..96 {
        for x in 24..88 {
            let l = (4.0 * g(x, y) - g(x - 1, y) - g(x + 1, y) - g(x, y - 1) - g(x, y + 1)) as f64;
            sum += l;
            sq += l * l;
            n += 1.0;
        }
    }
    let m = sum / n;
    (sq / n - m * m).max(0.0) as f32
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_landmarks_align_to_themselves() {
        let (a, b, tx, ty) = similarity(&TEMPLATE);
        assert!((a - 1.0).abs() < 1e-4 && b.abs() < 1e-4 && tx.abs() < 1e-3 && ty.abs() < 1e-3);
    }

    #[test]
    fn alignment_undoes_scale_rotation_and_shift() {
        // The template, rotated 30°, scaled ×2 and moved: aligning maps it back.
        let (s, c) = (30f32.to_radians().sin(), 30f32.to_radians().cos());
        let kps = TEMPLATE.map(|[x, y]| [2.0 * (c * x - s * y) + 300.0, 2.0 * (s * x + c * y) + 100.0]);
        let (a, b, tx, ty) = similarity(&kps);
        for (p, t) in kps.iter().zip(TEMPLATE.iter()) {
            let (x, y) = (a * p[0] - b * p[1] + tx, b * p[0] + a * p[1] + ty);
            assert!((x - t[0]).abs() < 1e-2 && (y - t[1]).abs() < 1e-2, "{x},{y} vs {t:?}");
        }
    }

    #[test]
    fn flat_crops_are_not_sharp() {
        assert_eq!(sharpness(&vec![128; CROP_BYTES]), 0.0);
        let checker: Vec<u8> = (0..CROP * CROP).flat_map(|i| [((i % CROP + i / CROP) % 2 * 255) as u8; 3]).collect();
        assert!(sharpness(&checker) > 1000.0);
    }
}
