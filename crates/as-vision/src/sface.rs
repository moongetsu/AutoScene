//! SFace (Zhong et al. 2021; OpenCV Zoo `face_recognition_sface_2021dec`):
//! a 128-d identity embedding of a face aligned to 112×112 by its five
//! landmarks, on the same template as ArcFace ([`crate::arcface::align`]) —
//! Fast Scan's embedder (the pipeline of the Scener app). Much smaller and
//! faster than ArcFace glintr100, and less sure of who is who.
//!
//! The graph reads RGB bytes as they are (it scales them itself) and its
//! batch dimension is free apart from what it declares, so the session is
//! planned for fixed batches like ArcFace's.

use std::path::Path;

use as_infer::onnx::InputShape;
use as_infer::{Device, InferError, InferResult, Runtime, Session, TensorRef};

use crate::arcface::{CROP, CROP_BYTES, Embedding};

/// Embedding size.
pub const DIM: usize = 128;

pub struct SFace {
    session: Session,
    input: String,
    device: Device,
    batch: usize,
    x: Vec<f32>,
}

impl SFace {
    /// `batch`: faces per inference call (the last call of a run is padded).
    pub fn load(rt: &Runtime, model: &Path, batch: usize) -> InferResult<Self> {
        assert!(batch > 0);
        let (session, device) = rt.session_shaped(model, InputShape::Fixed(&[batch, 3, CROP, CROP]))?;
        let input = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| InferError::Model(format!("{}: no input", model.display())))?;
        tracing::info!(device = ?device, batch, "SFace ready");
        Ok(Self { session, input, device, batch, x: vec![0.0; batch * 3 * CROP * CROP] })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    /// Embed aligned crops (each [`CROP_BYTES`] of RGB24), any number. The
    /// norm is the raw output's length.
    pub fn embed(&mut self, crops: &[&[u8]]) -> InferResult<Vec<Embedding>> {
        let mut out = Vec::with_capacity(crops.len());
        for group in crops.chunks(self.batch) {
            let plane = CROP * CROP;
            self.x.fill(0.0);
            for (i, crop) in group.iter().enumerate() {
                assert_eq!(crop.len(), CROP_BYTES, "aligned crop size");
                let base = i * 3 * plane;
                for (p, px) in crop.as_chunks::<3>().0.iter().enumerate() {
                    self.x[base + p] = f32::from(px[0]);
                    self.x[base + plane + p] = f32::from(px[1]);
                    self.x[base + 2 * plane + p] = f32::from(px[2]);
                }
            }
            let t = TensorRef::from_array_view(([self.batch, 3, CROP, CROP], &self.x[..]))?;
            let outs = self.session.run(as_infer::inputs![self.input.as_str() => t])?;
            let (_, e) = outs[0].try_extract_tensor::<f32>()?;
            if e.len() != self.batch * DIM {
                return Err(InferError::Run(format!(
                    "SFace returned {} values for a batch of {}",
                    e.len(),
                    self.batch
                )));
            }
            for i in 0..group.len() {
                // One with no direction (NaN, infinite) matches no one.
                out.push(crate::arcface::unit(&e[i * DIM..(i + 1) * DIM]));
            }
        }
        Ok(out)
    }
}
