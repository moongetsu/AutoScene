//! Small RGB24 helpers for the face pass: downscaling for detection and
//! square face thumbnails.

use crate::arcface::bilinear;

/// Halve a `w`×`h` RGB24 frame (2×2 box filter); `w` and `h` must be even.
pub fn half(rgb: &[u8], w: usize, h: usize) -> Vec<u8> {
    assert!(w.is_multiple_of(2) && h.is_multiple_of(2) && rgb.len() == w * h * 3);
    let (hw, hh) = (w / 2, h / 2);
    let mut out = vec![0u8; hw * hh * 3];
    for y in 0..hh {
        let (r0, r1) = (&rgb[2 * y * w * 3..(2 * y + 1) * w * 3], &rgb[(2 * y + 1) * w * 3..(2 * y + 2) * w * 3]);
        let o = &mut out[y * hw * 3..(y + 1) * hw * 3];
        for x in 0..hw {
            for c in 0..3 {
                let i = 6 * x + c;
                let s = r0[i] as u16 + r0[i + 3] as u16 + r1[i] as u16 + r1[i + 3] as u16;
                o[3 * x + c] = ((s + 2) / 4) as u8;
            }
        }
    }
    out
}

/// A `size`×`size` RGB24 thumbnail of the square around `bbox` (x1, y1, x2,
/// y2), enlarged `context` times so hair and chin show; black outside the
/// frame. Shrinking averages every source pixel under an output pixel (no
/// aliasing on big faces); faces smaller than the thumbnail are interpolated.
pub fn face_thumb(rgb: &[u8], w: usize, h: usize, bbox: [f32; 4], context: f32, size: usize) -> Vec<u8> {
    let (cx, cy) = ((bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0);
    let side = (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]) * context;
    // Faces sit a little above the box centre in a portrait crop.
    let (x0, y0) = (cx - side / 2.0, cy - side * 0.45);
    let step = side / size as f32;
    let mut out = vec![0u8; size * size * 3];
    if step <= 1.0 {
        for v in 0..size {
            for u in 0..size {
                let p = bilinear(rgb, w, h, x0 + (u as f32 + 0.5) * step - 0.5, y0 + (v as f32 + 0.5) * step - 0.5);
                out[(v * size + u) * 3..(v * size + u) * 3 + 3].copy_from_slice(&p);
            }
        }
        return out;
    }
    // Source pixel span [from, to) under output pixel i along one axis
    // (unclamped, so pixels outside the frame count as black).
    let span = |origin: f32, i: usize| -> (i64, i64) {
        let a = (origin + i as f32 * step).round() as i64;
        let b = ((origin + (i + 1) as f32 * step).round() as i64).max(a + 1);
        (a, b)
    };
    let cols: Vec<(i64, i64)> = (0..size).map(|u| span(x0, u)).collect();
    let mut row_acc = vec![0u32; size * 3];
    for v in 0..size {
        let (ya, yb) = span(y0, v);
        row_acc.fill(0);
        for y in ya.max(0)..yb.min(h as i64) {
            let line = &rgb[y as usize * w * 3..(y as usize + 1) * w * 3];
            for (u, &(xa, xb)) in cols.iter().enumerate() {
                let (a, b) = (xa.clamp(0, w as i64) as usize, xb.clamp(0, w as i64) as usize);
                let acc = &mut row_acc[u * 3..u * 3 + 3];
                for px in line[a * 3..b * 3].as_chunks::<3>().0 {
                    acc[0] += px[0] as u32;
                    acc[1] += px[1] as u32;
                    acc[2] += px[2] as u32;
                }
            }
        }
        for (u, &(xa, xb)) in cols.iter().enumerate() {
            let n = ((xb - xa) * (yb - ya)) as u32;
            for c in 0..3 {
                out[(v * size + u) * 3 + c] = ((row_acc[u * 3 + c] + n / 2) / n) as u8;
            }
        }
    }
    out
}

/// Encode a `w`×`h` RGB24 image as a baseline JPEG.
pub fn jpeg(rgb: &[u8], w: usize, h: usize, quality: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 << 10);
    let enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.encode(rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).expect("in-memory JPEG encoding");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halving_averages_blocks() {
        let rgb = [0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255];
        assert_eq!(half(&rgb, 2, 2), [191, 191, 191]);
    }

    #[test]
    fn shrinking_averages_and_pads_black() {
        // A 40×40 frame: left half white, right half black. The thumbnail of
        // its centre square (context 1) halves it: the left column stays white,
        // the right black, the middle column is the average.
        let (w, h) = (40, 40);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [if i % w < 20 { 255 } else { 0 }; 3]).collect();
        let t = face_thumb(&rgb, w, h, [10.0, 11.0, 30.0, 31.0], 1.0, 10);
        assert_eq!(t[0], 255);
        assert_eq!(t[9 * 3], 0);
        // A box reaching past the top edge: black above the frame.
        let t = face_thumb(&rgb, w, h, [0.0, -20.0, 20.0, 0.0], 1.0, 10);
        assert_eq!(&t[..3], &[0, 0, 0]);
    }

    #[test]
    fn thumbs_are_square_jpegs() {
        let (w, h) = (64, 48);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 4) as u8, (i / w * 5) as u8, 90]).collect();
        let t = face_thumb(&rgb, w, h, [20.0, 10.0, 40.0, 34.0], 1.6, 32);
        assert_eq!(t.len(), 32 * 32 * 3);
        let j = jpeg(&t, 32, 32, 85);
        assert_eq!(&j[..2], [0xFF, 0xD8]);
    }
}
