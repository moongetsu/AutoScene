//! CCIP (`ccip-caformer-24-randaug-pruned` from deepghs, a CLIP-style
//! caformer): a 768-d character embedding of a square crop — the look of a
//! drawn character points the same way in every frame and still, so cosine
//! similarity is the character score.
//!
//! A crop is the square around a detection resized to 384×384 the way PIL
//! does it ([`crop`]: bilinear, its filter widened by the downscale so
//! shrinking stays antialiased, sampled at pixel centres), normalised with
//! CLIP mean/std, NCHW. The graph's batch dimension is free, so crops are
//! embedded in fixed-size batches like ArcFace's; its output is not
//! normalised — [`unit`] gives it direction and keeps its length.
//!
//! On a GPU provider the session first proves itself against the CPU
//! goldens of `tools/anime-bench/goldens.py` (`fixtures/anime/ccip_*`):
//! low-precision maths would bend every embedding a little, so a session
//! that misses them is rebuilt on the CPU.

use std::path::Path;

use as_infer::onnx::InputShape;
use as_infer::{Device, InferError, InferResult, Runtime, Session, TensorRef};

use crate::arcface::{Embedding, unit};

/// Side of the square crop the embedder reads.
pub const CROP: usize = 384;
/// Bytes of one RGB24 crop.
pub const CROP_BYTES: usize = CROP * CROP * 3;
/// Embedding size.
pub const DIM: usize = 768;
/// CLIP input normalisation, channel-wise: (v/255 − mean)/std.
const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821072];
const STD: [f32; 3] = [0.26862954, 0.2613026, 0.2757771];
/// How close a session's embeddings must sit to the CPU goldens (cosine).
const GOLDEN_COSINE: f32 = 0.999;

/// The parity check a non-CPU session runs first: the crops of
/// `tools/anime-bench/goldens.py` and the vectors its CPU run stored
/// (`fixtures/anime/ccip_*`; raw little-endian f32, unnormalised).
const GOLDEN_RGB: [&[u8; CROP_BYTES]; 5] = [
    include_bytes!("../fixtures/anime/ccip_0.rgb"),
    include_bytes!("../fixtures/anime/ccip_1.rgb"),
    include_bytes!("../fixtures/anime/ccip_2.rgb"),
    include_bytes!("../fixtures/anime/ccip_3.rgb"),
    include_bytes!("../fixtures/anime/ccip_4.rgb"),
];
const GOLDEN_VEC: [&[u8; DIM * 4]; 5] = [
    include_bytes!("../fixtures/anime/ccip_0.vec"),
    include_bytes!("../fixtures/anime/ccip_1.vec"),
    include_bytes!("../fixtures/anime/ccip_2.vec"),
    include_bytes!("../fixtures/anime/ccip_3.vec"),
    include_bytes!("../fixtures/anime/ccip_4.vec"),
];

/// The input value of each byte, per channel ((v/255 − mean)/std).
fn tables() -> [[f32; 256]; 3] {
    std::array::from_fn(|c| std::array::from_fn(|v| (v as f32 / 255.0 - MEAN[c]) / STD[c]))
}

/// Cosine similarity of a (unit) embedding to a stored raw vector. Cosine
/// is scale-invariant: the golden vectors stay unnormalised, and an
/// embedding that is all zeros — one the session spoilt — matches nothing.
fn cosine_to_raw(e: &Embedding, raw: &[f32]) -> f32 {
    let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !(norm.is_finite() && norm > 0.0) {
        return 0.0;
    }
    e.v.iter().zip(raw).map(|(a, b)| a * b).sum::<f32>() / norm
}

pub struct Ccip {
    session: Session,
    input: String,
    device: Device,
    batch: usize,
    /// Input value of each byte, per channel.
    tables: [[f32; 256]; 3],
    x: Vec<f32>,
    /// Embeddings with no direction since loading (see [`unit`]); the first
    /// is logged.
    spoilt: u64,
}

impl Ccip {
    /// `batch`: crops per inference call (the last call of a run is padded).
    /// On anything but the CPU the session first proves itself against the
    /// golden crops: one that computes them wrong is rebuilt on the CPU,
    /// and if even that misses, loading fails.
    pub fn load(rt: &Runtime, model: &Path, batch: usize) -> InferResult<Self> {
        assert!(batch > 0);
        let mut ccip = Self::open(rt, model, batch)?;
        if ccip.device != Device::Cpu && !ccip.matches_goldens() {
            tracing::warn!(
                device = ?ccip.device,
                "CCIP embeddings don't match the CPU goldens; rebuilding on the CPU"
            );
            ccip = Self::open(&rt.on_cpu(), model, batch)?;
            if !ccip.matches_goldens() {
                return Err(InferError::Model(format!("{}: embeddings don't match the CPU goldens", model.display())));
            }
        }
        Ok(ccip)
    }

    /// Load the session on the runtime's preferred device, unchecked.
    fn open(rt: &Runtime, model: &Path, batch: usize) -> InferResult<Self> {
        let (session, device) = rt.fp32_sessions().session_shaped(model, InputShape::Fixed(&[batch, 3, CROP, CROP]))?;
        let input = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| InferError::Model(format!("{}: no input", model.display())))?;
        tracing::info!(device = ?device, batch, "CCIP ready");
        Ok(Self { session, input, device, batch, tables: tables(), x: vec![0.0; batch * 3 * CROP * CROP], spoilt: 0 })
    }

    /// Every golden crop embedded within [`GOLDEN_COSINE`] of its stored
    /// CPU vector.
    fn matches_goldens(&mut self) -> bool {
        let crops: Vec<&[u8]> = GOLDEN_RGB.iter().map(|r| &r[..]).collect();
        let Ok(embeddings) = self.embed(&crops) else {
            return false;
        };
        embeddings.iter().zip(&GOLDEN_VEC).all(|(e, bytes)| {
            let golden: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
            cosine_to_raw(e, &golden) >= GOLDEN_COSINE
        })
    }

    pub fn device(&self) -> Device {
        self.device
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    /// Embed crops (each [`CROP_BYTES`] of RGB24), any number.
    pub fn embed(&mut self, crops: &[&[u8]]) -> InferResult<Vec<Embedding>> {
        let mut out = Vec::with_capacity(crops.len());
        let t = &self.tables;
        for group in crops.chunks(self.batch) {
            let plane = CROP * CROP;
            self.x.fill(0.0);
            for (i, crop) in group.iter().enumerate() {
                assert_eq!(crop.len(), CROP_BYTES, "CCIP crop size");
                let base = i * 3 * plane;
                for (p, px) in crop.as_chunks::<3>().0.iter().enumerate() {
                    self.x[base + p] = t[0][px[0] as usize];
                    self.x[base + plane + p] = t[1][px[1] as usize];
                    self.x[base + 2 * plane + p] = t[2][px[2] as usize];
                }
            }
            let x = TensorRef::from_array_view(([self.batch, 3, CROP, CROP], &self.x[..]))?;
            let outs = self.session.run(as_infer::inputs![self.input.as_str() => x])?;
            let (_, e) = outs[0].try_extract_tensor::<f32>()?;
            if e.len() != self.batch * DIM {
                return Err(InferError::Run(format!("CCIP returned {} values for a batch of {}", e.len(), self.batch)));
            }
            for i in 0..group.len() {
                let embedding = unit(&e[i * DIM..(i + 1) * DIM]);
                if embedding.norm == 0.0 {
                    // A broken run would otherwise only match no one: say so, once.
                    if self.spoilt == 0 {
                        tracing::warn!(
                            device = ?self.device,
                            "CCIP gave an embedding with no direction (NaN, infinite or zero); it matches no one"
                        );
                    }
                    self.spoilt += 1;
                }
                out.push(embedding);
            }
        }
        Ok(out)
    }
}

/// The taps of one output axis of [`crop`]: `n` output pixels resampling
/// the `side`-pixel square at `origin` (source pixels; taps may fall
/// outside the frame — they count as black). Weights are the triangle
/// `max(0, 1 − |j + 0.5 − c|/fs)` with the filter's support 1 widened to
/// `fs = max(1, side/n)` — PIL's bilinear with antialias, evaluated at
/// pixel centres — normalised over the window where it overlaps the
/// square. Each entry: source index of the window's first pixel, weights.
fn taps(origin: f32, side: f32, n: usize) -> Vec<(i64, Vec<f32>)> {
    let step = side / n as f32;
    let fs = step.max(1.0);
    let base = origin.floor() as i64;
    let bound = side.ceil() as i64;
    (0..n)
        .map(|u| {
            // Output u centres at crop-space coordinate c; the window is
            // the support's reach around it, clamped to the square.
            let c = (u as f32 + 0.5) * step;
            let lo = ((c - fs + 0.5) as i64).clamp(0, bound);
            let hi = ((c + fs + 0.5) as i64).clamp(0, bound);
            let ws: Vec<f32> = (lo..hi).map(|j| (1.0 - (j as f32 + 0.5 - c).abs() / fs).max(0.0)).collect();
            let ww = ws.iter().sum::<f32>();
            let k = if ww > 0.0 { 1.0 / ww } else { 0.0 };
            (base + lo, ws.iter().map(|w| w * k).collect())
        })
        .collect()
}

/// The 384×384 RGB24 crop of the `side`-pixel square at (`x`, `y`) in a
/// `w`×`h` RGB24 frame — the rect `tools/anime-bench/goldens.py` cuts
/// around a head — resized PIL-style: bilinear with the filter widened
/// when downscaling, black outside the frame.
pub fn crop(rgb: &[u8], w: usize, h: usize, x: f32, y: f32, side: f32) -> Vec<u8> {
    assert!(w > 0 && h > 0 && side > 0.0 && rgb.len() == w * h * 3, "CCIP crop of {w}×{h}, side {side}");
    let mut out = vec![0u8; CROP_BYTES];
    let (xt, yt) = (taps(x, side, CROP), taps(y, side, CROP));
    // The source rows the vertical taps reach inside the frame, resampled
    // across once; taps outside the frame count as black (their weights
    // still normalise).
    let r0 = yt.iter().map(|t| t.0).min().expect("n > 0").max(0);
    let r1 = yt.iter().map(|t| t.0 + t.1.len() as i64).max().expect("n > 0").min(h as i64);
    if r0 >= r1 {
        return out;
    }
    let mut rows = vec![0f32; (r1 - r0) as usize * CROP * 3];
    for (sy, dst) in (r0..r1).zip(rows.as_chunks_mut::<{ CROP * 3 }>().0) {
        let src = &rgb[sy as usize * w * 3..(sy as usize + 1) * w * 3];
        for ((first, ws), o) in xt.iter().zip(dst.as_chunks_mut::<3>().0) {
            for (k, &wt) in ws.iter().enumerate() {
                let sx = first + k as i64;
                if sx >= 0 && sx < w as i64 {
                    let p = &src[sx as usize * 3..sx as usize * 3 + 3];
                    o[0] += wt * p[0] as f32;
                    o[1] += wt * p[1] as f32;
                    o[2] += wt * p[2] as f32;
                }
            }
        }
    }
    for ((first, ws), line) in yt.iter().zip(out.as_chunks_mut::<{ CROP * 3 }>().0) {
        for (u, o) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let mut acc = [0f32; 3];
            for (k, &wt) in ws.iter().enumerate() {
                let sy = first + k as i64;
                if sy >= r0 && sy < r1 {
                    let p = ((sy - r0) as usize * CROP + u) * 3;
                    acc[0] += wt * rows[p];
                    acc[1] += wt * rows[p + 1];
                    acc[2] += wt * rows[p + 2];
                }
            }
            for c in 0..3 {
                o[c] = acc[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_tables_apply_clip_normalisation() {
        let t = tables();
        assert!((t[0][0] + MEAN[0] / STD[0]).abs() < 1e-6);
        assert!((t[2][255] - (1.0 - MEAN[2]) / STD[2]).abs() < 1e-6);
        // The constants are the ones golden.json stores for the model.
        assert_eq!(MEAN[0], 0.481_454_66);
        assert_eq!(STD[2], 0.275_777_1);
    }

    #[test]
    fn one_to_one_crops_are_exact_copies() {
        // 384×384, left half white, right half black.
        let (w, h) = (384, 384);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [if i % w < 192 { 255 } else { 0 }; 3]).collect();
        assert_eq!(crop(&rgb, w, h, 0.0, 0.0, 384.0), rgb);
        // A square half off the frame's left edge keeps black there; the
        // seam lands exactly on source column 0.
        let c = crop(&rgb, w, h, -192.0, 0.0, 384.0);
        let px = |u: usize| &c[u * 3..u * 3 + 3];
        assert_eq!(px(191), &[0, 0, 0]);
        assert_eq!(px(192), &[255, 255, 255]);
        // And a square wholly outside is all black.
        assert_eq!(crop(&rgb, w, h, 400.0, 0.0, 200.0), vec![0u8; CROP_BYTES]);
    }

    #[test]
    fn shrinking_taps_form_a_tent() {
        // side 768 → 384 (step 2): the filter widened to fs 2 gives a
        // 1:3:3:1 tent over 4 source pixels — antialiasing.
        let t = taps(0.0, 768.0, CROP);
        let (first, ws) = &t[10];
        assert_eq!(*first, 19);
        for (a, b) in ws.iter().zip([0.125, 0.375, 0.375, 0.125]) {
            assert!((a - b).abs() < 1e-6, "{ws:?}");
        }
        // Enlarging (step < 1, fs 1) interpolates between the two nearest
        // pixel centres: output 2 centres at 0.625 of a 96-side square.
        let t = taps(0.0, 96.0, CROP);
        let (first, ws) = &t[2];
        assert_eq!(*first, 0);
        assert!((ws[0] - 0.875).abs() < 1e-6 && (ws[1] - 0.125).abs() < 1e-6, "{ws:?}");
    }

    #[test]
    fn downscaling_averages_under_each_output_pixel() {
        // 768×768, left half white: cropped whole, the middle columns blend.
        let (w, h) = (768, 768);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [if i % w < 384 { 255 } else { 0 }; 3]).collect();
        let c = crop(&rgb, w, h, 0.0, 0.0, 768.0);
        let px = |u: usize| &c[u * 3..u * 3 + 3];
        assert_eq!(px(0), &[255, 255, 255]);
        assert_eq!(px(383), &[0, 0, 0]);
        let edge = px(191)[0];
        assert!(edge > 0 && edge < 255, "{edge}");
    }

    #[test]
    fn cosine_to_raw_is_scale_invariant_and_refuses_no_direction() {
        let e = unit(&[3.0, 0.0, -4.0]);
        assert!((cosine_to_raw(&e, &[6.0, 0.0, -8.0]) - 1.0).abs() < 1e-6);
        assert!((cosine_to_raw(&e, &[30.0, 0.0, -40.0]) - 1.0).abs() < 1e-6);
        assert!(cosine_to_raw(&e, &[0.0, 1.0, 0.0]).abs() < 1e-6);
        // Neither a zero stored vector nor a spoilt embedding matches.
        assert_eq!(cosine_to_raw(&e, &[0.0; 3]), 0.0);
        assert_eq!(cosine_to_raw(&Embedding { v: vec![0.0; 3], norm: 0.0 }, &[1.0, 2.0, 3.0]), 0.0);
    }

    #[test]
    fn goldens_have_the_crop_and_vector_sizes() {
        for (r, v) in GOLDEN_RGB.iter().zip(&GOLDEN_VEC) {
            assert_eq!(r.len(), CROP_BYTES);
            assert_eq!(v.len(), DIM * 4);
        }
    }
}
