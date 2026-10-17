//! Person re-identification: an appearance embedding of a person's picture
//! — clothes, build, hair — that points the same way for the same person in
//! the same outfit (cosine similarity), whichever way they face.
//!
//! The model is OpenVINO's person-reidentification-retail-0277 (OmniScaleNet
//! with linear context transform blocks, 2 GFLOPs; Market-1501 rank-1 96 %),
//! which InsightFace ships as `reid_0265.onnx`: a 256×128 crop of RGB
//! normalised with ImageNet statistics in, a 256-d vector out. It learned
//! from pedestrians filmed upright: a film's close-ups and backs of heads are
//! outside what it saw, so its answer is a hint within a scene (one outfit),
//! not an identity across an episode.
//!
//! The export pins its batch at 1 in every Reshape; they are made to copy
//! the batch when the model is loaded ([`Runtime::session_batched`], what
//! `tools/model-prep/inspect_onnx.py free-batch` writes to a file), so it
//! runs in batches while the file stays as downloaded.

use std::path::Path;

use as_infer::onnx::InputShape;
use as_infer::{Device, InferError, InferResult, Runtime, Session, TensorRef};

use crate::arcface::Embedding;
use crate::image;
use crate::person::Norm;

/// Crop size.
pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 256;
/// Bytes of one RGB24 crop.
pub const CROP_BYTES: usize = WIDTH * HEIGHT * 3;
/// Embedding size.
pub const DIM: usize = 256;

pub struct PersonReid {
    session: Session,
    input: String,
    device: Device,
    batch: usize,
    tables: [[f32; 256]; 3],
    x: Vec<f32>,
}

impl PersonReid {
    /// `batch`: crops per inference call (the last call of a run is padded).
    pub fn load(rt: &Runtime, model: &Path, batch: usize, norm: Norm) -> InferResult<Self> {
        assert!(batch > 0);
        let (session, device) = rt.session_batched(model, InputShape::Fixed(&[batch, 3, HEIGHT, WIDTH]))?;
        let input = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| InferError::Model(format!("{}: no input", model.display())))?;
        tracing::info!(device = ?device, batch, "person ReID ready");
        Ok(Self { session, input, device, batch, tables: norm.tables(), x: vec![0.0; batch * 3 * HEIGHT * WIDTH] })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    /// The crop [`PersonReid::embed`] takes of the person in `bbox` (pixels)
    /// of a `w`×`h` RGB24 frame: exactly the box, clipped to the frame and
    /// stretched to 128×256.
    pub fn crop(rgb: &[u8], size: (usize, usize), bbox: [f32; 4]) -> Vec<u8> {
        image::resize_region(rgb, size, bbox, (WIDTH, HEIGHT))
    }

    /// Embed crops (each [`CROP_BYTES`] of RGB24), any number.
    pub fn embed(&mut self, crops: &[&[u8]]) -> InferResult<Vec<Embedding>> {
        let mut out = Vec::with_capacity(crops.len());
        let plane = WIDTH * HEIGHT;
        let t = &self.tables;
        for group in crops.chunks(self.batch) {
            self.x.fill(0.0);
            for (i, crop) in group.iter().enumerate() {
                assert_eq!(crop.len(), CROP_BYTES, "person crop size");
                let base = i * 3 * plane;
                for (p, px) in crop.as_chunks::<3>().0.iter().enumerate() {
                    self.x[base + p] = t[0][px[0] as usize];
                    self.x[base + plane + p] = t[1][px[1] as usize];
                    self.x[base + 2 * plane + p] = t[2][px[2] as usize];
                }
            }
            let x = TensorRef::from_array_view(([self.batch, 3, HEIGHT, WIDTH], &self.x[..]))?;
            let outs = self.session.run(as_infer::inputs![self.input.as_str() => x])?;
            let (shape, e) = outs[0].try_extract_tensor::<f32>()?;
            // The whole shape, not just the count: a graph that pins its batch
            // at 1 inside could otherwise mix the crops of a batch unnoticed.
            if shape.len() != 2 || shape[0] != self.batch as i64 || shape[1] != DIM as i64 {
                return Err(InferError::Run(format!(
                    "person ReID returned shape {:?} for a batch of {}",
                    &shape[..],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_are_the_box_stretched() {
        // A 20×10 frame, left half red: the crop of the left half is all red.
        let (w, h) = (20, 10);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| if i % w < 10 { [200, 0, 0] } else { [0, 0, 200] }).collect();
        let c = PersonReid::crop(&rgb, (w, h), [0.0, 0.0, 10.0, 10.0]);
        assert_eq!(c.len(), CROP_BYTES);
        assert!(c.as_chunks::<3>().0.iter().all(|p| *p == [200, 0, 0]));
        // A box reaching past the frame keeps only what's inside.
        let c = PersonReid::crop(&rgb, (w, h), [10.0, -5.0, 30.0, 10.0]);
        assert!(c.as_chunks::<3>().0.iter().all(|p| *p == [0, 0, 200]));
    }
}
