//! Small RGB24 helpers for the analysis passes: downscaling for detection
//! and shot sprites, square face thumbnails, crops of people resized for
//! their embedder, and how sharp and bright a picture is.

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

/// Shrink a whole `w`×`h` RGB24 frame to `ow`×`oh` — every source pixel
/// under an output pixel averaged — into `out`, an RGB24 image `out_w`
/// pixels wide, at column `ox`. The frame must be at least as big as the
/// result.
pub fn shrink_into(
    rgb: &[u8],
    (w, h): (usize, usize),
    out: &mut [u8],
    out_w: usize,
    ox: usize,
    (ow, oh): (usize, usize),
) {
    assert!(ow > 0 && oh > 0 && ow <= w && oh <= h && rgb.len() == w * h * 3);
    assert!(ox + ow <= out_w && out.len() >= out_w * oh * 3);
    // Source span [i·n/on, (i+1)·n/on) under output pixel i: never empty as n ≥ on.
    let edge = |n: usize, on: usize, i: usize| i * n / on;
    let cols: Vec<(usize, usize)> = (0..ow).map(|u| (edge(w, ow, u), edge(w, ow, u + 1))).collect();
    // Rows are summed whole first (a loop the compiler vectorises), then
    // each output pixel adds up its columns of that sum.
    let mut sum = vec![0u32; w * 3];
    for v in 0..oh {
        let (ya, yb) = (edge(h, oh, v), edge(h, oh, v + 1));
        sum.fill(0);
        for y in ya..yb {
            for (s, &p) in sum.iter_mut().zip(&rgb[y * w * 3..(y + 1) * w * 3]) {
                *s += p as u32;
            }
        }
        let row = &mut out[(v * out_w + ox) * 3..(v * out_w + ox + ow) * 3];
        for (o, &(xa, xb)) in row.as_chunks_mut::<3>().0.iter_mut().zip(&cols) {
            let mut a = [0u32; 3];
            for px in sum[xa * 3..xb * 3].as_chunks::<3>().0 {
                a[0] += px[0];
                a[1] += px[1];
                a[2] += px[2];
            }
            let n = ((xb - xa) * (yb - ya)) as u32;
            for c in 0..3 {
                o[c] = ((a[c] + n / 2) / n) as u8;
            }
        }
    }
}

/// A `size`×`size` RGB24 thumbnail of the square around `bbox` (x1, y1, x2,
/// y2), enlarged `context` times so hair and chin show; black outside the
/// frame. Shrinking averages every source pixel under an output pixel (no
/// aliasing on big faces); faces smaller than the thumbnail are interpolated.
pub fn face_thumb(rgb: &[u8], w: usize, h: usize, bbox: [f32; 4], context: f32, size: usize) -> Vec<u8> {
    let (cx, cy) = ((bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0);
    let side = (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]) * context;
    // The square stays inside the frame where it fits: a face at the edge sits
    // off-centre rather than next to black. Faces sit a little above the box
    // centre in a portrait crop.
    let fit = |origin: f32, len: usize| {
        let len = len as f32;
        if side <= len { origin.clamp(0.0, len - side) } else { (len - side) / 2.0 }
    };
    let (x0, y0) = (fit(cx - side / 2.0, w), fit(cy - side * 0.45, h));
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

/// Source pixels behind each of `n` output pixels that cover `from..to` of
/// a source axis `len` pixels long (inside it): the first one and the weight
/// of each from there on. Shrinking averages everything under an output
/// pixel (partly covered pixels count by how much is covered); enlarging
/// interpolates linearly between the pixels the span touches — as if it
/// had been cut out first, its edge pixels held beyond their centres.
fn taps(len: usize, from: f32, to: f32, n: usize) -> Vec<(usize, Vec<f32>)> {
    let step = (to - from) / n as f32;
    let lo = (from.floor() as usize).min(len - 1);
    let hi = (to.ceil() as usize).clamp(lo + 1, len);
    (0..n)
        .map(|i| {
            if step > 1.0 {
                let (a, b) = (from + i as f32 * step, from + (i + 1) as f32 * step);
                let first = (a.floor() as usize).clamp(lo, hi - 1);
                let last = (b.ceil() as usize).clamp(first + 1, hi);
                let w = (first..last).map(|p| (((p + 1) as f32).min(b) - (p as f32).max(a)).max(0.0) / step).collect();
                (first, w)
            } else {
                let c = (from + (i as f32 + 0.5) * step - 0.5).clamp(lo as f32, (hi - 1) as f32);
                let p = c.floor() as usize;
                let f = c - p as f32;
                if p + 1 < hi { (p, vec![1.0 - f, f]) } else { (p, vec![1.0]) }
            }
        })
        .collect()
}

/// The part `region` (x1, y1, x2, y2 in pixels; clipped to the frame) of a
/// `w`×`h` RGB24 frame, resized to `ow`×`oh` — sides scaled independently,
/// so the region's shape isn't kept. Shrinking averages every source pixel
/// under an output pixel, enlarging interpolates. Black if nothing of the
/// region is inside the frame.
pub fn resize_region(rgb: &[u8], (w, h): (usize, usize), region: [f32; 4], (ow, oh): (usize, usize)) -> Vec<u8> {
    assert!(w > 0 && h > 0 && ow > 0 && oh > 0 && rgb.len() == w * h * 3);
    let mut out = vec![0u8; ow * oh * 3];
    let (x0, x1) = (region[0].clamp(0.0, w as f32), region[2].clamp(0.0, w as f32));
    let (y0, y1) = (region[1].clamp(0.0, h as f32), region[3].clamp(0.0, h as f32));
    if !(x1 - x0 >= 1e-3 && y1 - y0 >= 1e-3) {
        return out;
    }
    let (xt, yt) = (taps(w, x0, x1, ow), taps(h, y0, y1, oh));
    // The rows the vertical taps reach, each resampled across once.
    let r0 = yt.iter().map(|t| t.0).min().expect("oh > 0");
    let r1 = yt.iter().map(|t| t.0 + t.1.len()).max().expect("oh > 0");
    let mut rows = vec![0f32; (r1 - r0) * ow * 3];
    for (y, dst) in (r0..r1).zip(rows.chunks_exact_mut(ow * 3)) {
        let src = &rgb[y * w * 3..(y + 1) * w * 3];
        for ((first, ws), o) in xt.iter().zip(dst.as_chunks_mut::<3>().0) {
            for (k, &wt) in ws.iter().enumerate() {
                let p = &src[(first + k) * 3..(first + k) * 3 + 3];
                o[0] += wt * p[0] as f32;
                o[1] += wt * p[1] as f32;
                o[2] += wt * p[2] as f32;
            }
        }
    }
    for ((first, ws), line) in yt.iter().zip(out.chunks_exact_mut(ow * 3)) {
        for (u, o) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let mut acc = [0f32; 3];
            for (k, &wt) in ws.iter().enumerate() {
                let p = &rows[((first + k - r0) * ow + u) * 3..((first + k - r0) * ow + u) * 3 + 3];
                acc[0] += wt * p[0];
                acc[1] += wt * p[1];
                acc[2] += wt * p[2];
            }
            for c in 0..3 {
                o[c] = acc[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Brightness of an RGB24 pixel (Rec. 601 luma).
fn luma(p: &[u8]) -> f32 {
    0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
}

/// Sharpness of a `w`×`h` RGB24 image: variance of the Laplacian of its
/// brightness. Out-of-focus and motion-blurred pictures score low; compare
/// pictures of one size (it grows with detail per pixel).
pub fn sharpness(rgb: &[u8], w: usize, h: usize) -> f32 {
    assert_eq!(rgb.len(), w * h * 3);
    if w < 3 || h < 3 {
        return 0.0;
    }
    let g = |x: usize, y: usize| luma(&rgb[(y * w + x) * 3..(y * w + x) * 3 + 3]);
    let (mut sum, mut sq, mut n) = (0f64, 0f64, 0f64);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let l = (4.0 * g(x, y) - g(x - 1, y) - g(x + 1, y) - g(x, y - 1) - g(x, y + 1)) as f64;
            sum += l;
            sq += l * l;
            n += 1.0;
        }
    }
    let m = sum / n;
    (sq / n - m * m).max(0.0) as f32
}

/// Mean brightness of an RGB24 image, 0..=255.
pub fn mean_luma(rgb: &[u8]) -> f32 {
    let px = rgb.as_chunks::<3>().0;
    if px.is_empty() {
        return 0.0;
    }
    (px.iter().map(|p| luma(p) as f64).sum::<f64>() / px.len() as f64) as f32
}

/// Encode a `w`×`h` RGB24 image as a baseline JPEG (Robidoux's tables and
/// optimised Huffman codes: about a fifth smaller than the textbook ones at
/// the same look).
pub fn jpeg(rgb: &[u8], w: usize, h: usize, quality: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 << 10);
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality);
    let table = jpeg_encoder::QuantizationTableType::ImageMagick;
    enc.set_quantization_tables(table.clone(), table);
    enc.set_optimized_huffman_tables(true);
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
        // A crop bigger than the frame: centred, black beyond its edges.
        let t = face_thumb(&rgb, w, h, [0.0, 0.0, 40.0, 40.0], 1.5, 10);
        assert_eq!(&t[..3], &[0, 0, 0]);
        assert_eq!(t[(5 * 10 + 3) * 3], 255, "the frame's white half inside");
    }

    #[test]
    fn frames_shrink_into_a_strip() {
        // 6×4: left third red, the rest a gradient of blue; shrunk to 3×2 at column 1 of a 5-wide strip.
        let (w, h) = (6, 4);
        let rgb: Vec<u8> =
            (0..w * h).flat_map(|i| if i % w < 2 { [200, 0, 0] } else { [0, 0, (i % w * 40) as u8] }).collect();
        let mut out = vec![7u8; 5 * 2 * 3];
        shrink_into(&rgb, (w, h), &mut out, 5, 1, (3, 2));
        assert_eq!(&out[..3], &[7, 7, 7], "left of the cell untouched");
        assert_eq!(&out[3..6], &[200, 0, 0]);
        assert_eq!(&out[6..9], &[0, 0, 100]);
        assert_eq!(&out[9..12], &[0, 0, 180]);
        assert_eq!(&out[12..15], &[7, 7, 7], "right of the cell untouched");
        assert_eq!(&out[15 + 3..15 + 6], &[200, 0, 0], "second row");
    }

    #[test]
    fn thumbs_of_faces_at_the_edge_stay_inside_the_frame() {
        let (w, h) = (64, 48);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 4) as u8, 50 + (i / w) as u8, 90]).collect();
        // A face in the top-left corner: its crop would reach past both edges.
        let t = face_thumb(&rgb, w, h, [0.0, 0.0, 12.0, 12.0], 1.7, 16);
        assert!(t.chunks(3).all(|p| p[2] == 90), "no black from outside the frame");
        assert_eq!(&t[..3], &rgb[..3], "the crop starts at the frame's corner");
    }

    #[test]
    fn taps_cover_each_output_pixel_once() {
        for (len, from, to, n) in [(100, 0.0, 100.0, 7), (100, 12.5, 40.25, 9), (10, 2.0, 5.0, 40), (5, 0.0, 5.0, 5)] {
            for (first, ws) in taps(len, from, to, n) {
                assert!(first + ws.len() <= len, "{len} {from} {to} {n}");
                assert!((ws.iter().sum::<f32>() - 1.0).abs() < 1e-4, "{len} {from} {to} {n}: {ws:?}");
            }
        }
    }

    #[test]
    fn regions_shrink_by_averaging_and_enlarge_smoothly() {
        // 4×2: a white and a black 2×2 block side by side.
        let (w, h) = (4, 2);
        let rgb: Vec<u8> = (0..w * h).flat_map(|i| [if i % w < 2 { 255 } else { 0 }; 3]).collect();
        assert_eq!(resize_region(&rgb, (w, h), [0.0, 0.0, 4.0, 2.0], (2, 1)), [255, 255, 255, 0, 0, 0]);
        // A third of the white block and two thirds of the black one, averaged.
        assert_eq!(resize_region(&rgb, (w, h), [1.0, 0.0, 4.0, 2.0], (1, 1)), [85, 85, 85]);
        // Enlarged 4×: the edges keep their colour, the middle blends.
        let big = resize_region(&rgb, (w, h), [0.0, 0.0, 4.0, 2.0], (16, 8));
        assert_eq!((big[0], big[15 * 3]), (255, 0));
        let mid = big[7 * 3];
        assert!(mid > 0 && mid < 255, "{mid}");
        // Only the part inside the frame counts; nothing inside is black.
        assert_eq!(resize_region(&rgb, (w, h), [-4.0, -1.0, 2.0, 2.0], (1, 1)), [255, 255, 255]);
        assert_eq!(resize_region(&rgb, (w, h), [5.0, 0.0, 9.0, 2.0], (2, 2)), [0; 12]);
    }

    #[test]
    fn sharpness_and_brightness() {
        let (w, h) = (16, 16);
        let flat = vec![128u8; w * h * 3];
        let checker: Vec<u8> = (0..w * h).flat_map(|i| [if (i % w + i / w) % 2 == 0 { 255 } else { 0 }; 3]).collect();
        assert_eq!(sharpness(&flat, w, h), 0.0);
        let soft = resize_region(
            &resize_region(&checker, (w, h), [0.0, 0.0, 16.0, 16.0], (4, 4)),
            (4, 4),
            [0.0, 0.0, 4.0, 4.0],
            (w, h),
        );
        assert!(sharpness(&checker, w, h) > 10.0 * sharpness(&soft, w, h).max(1.0));
        assert_eq!(mean_luma(&flat), 128.0);
        assert!((mean_luma(&checker) - 127.5).abs() < 0.01);
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
