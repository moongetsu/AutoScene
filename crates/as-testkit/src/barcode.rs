//! Barcode layout (in master pixels, 640×360) and reader.
//!
//! A black strip (8 px margin) at the top-left holds 24 cells. Cell `i` is white when bit
//! `i` is set: bits 0–15 are the frame index (LSB first), bits 16–23 the
//! element id. Cells are drawn 12×12 inside a 16 px grid so compression
//! artefacts never bleed between neighbours.

pub const MASTER_W: u32 = 640;
pub const MASTER_H: u32 = 360;

pub const CELL: u32 = 16;
pub const INSET: u32 = 2;
pub const X0: u32 = 12;
pub const Y0: u32 = 12;
pub const FRAME_BITS: u32 = 16;
pub const ID_BITS: u32 = 8;
pub const CELLS: u32 = FRAME_BITS + ID_BITS;

/// Background strip `(x, y, w, h)`.
pub const STRIP: (u32, u32, u32, u32) = (X0 - 8, Y0 - 8, CELLS * CELL + 16, CELL + 16);

/// Drawn rectangle of cell `i`: `(x, y, w, h)`.
pub fn cell_rect(i: u32) -> (u32, u32, u32, u32) {
    (X0 + i * CELL + INSET, Y0 + INSET, CELL - 2 * INSET, CELL - 2 * INSET)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    pub frame: u16,
    pub id: u8,
}

/// Read the barcode from an 8-bit luma (or grey) plane of any size whose
/// aspect matches the master. Returns `None` if the strip isn't present.
pub fn read_luma(plane: &[u8], width: u32, height: u32, stride: usize) -> Option<Code> {
    let sx = width as f32 / MASTER_W as f32;
    let sy = height as f32 / MASTER_H as f32;
    let sample = |cx: u32, cy: u32| -> u32 {
        // Mean of a 4×4 (master px) patch around the cell centre.
        let mut sum = 0u32;
        let mut n = 0u32;
        for dy in 0..4u32 {
            for dx in 0..4u32 {
                let x = (((cx + dx) as f32 - 2.0) * sx) as usize;
                let y = (((cy + dy) as f32 - 2.0) * sy) as usize;
                if let Some(&v) = plane.get(y * stride + x) {
                    sum += v as u32;
                    n += 1;
                }
            }
        }
        if n == 0 { 0 } else { sum / n }
    };

    // Sanity: the strip margin left of the first cell must be dark.
    if sample(X0 - 4, Y0 + CELL / 2) > 80 {
        return None;
    }
    let mut bits = 0u32;
    for i in 0..CELLS {
        let (x, y, w, h) = cell_rect(i);
        if sample(x + w / 2, y + h / 2) > 128 {
            bits |= 1 << i;
        }
    }
    Some(Code { frame: (bits & 0xFFFF) as u16, id: (bits >> FRAME_BITS) as u8 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draw a code exactly as the ffmpeg drawbox chain does, then read it back.
    fn render(code: Code, w: u32, h: u32) -> Vec<u8> {
        let mut img = vec![90u8; (w * h) as usize];
        let sx = w as f32 / MASTER_W as f32;
        let sy = h as f32 / MASTER_H as f32;
        let mut fill = |(x, y, rw, rh): (u32, u32, u32, u32), v: u8| {
            let x0 = (x as f32 * sx) as u32;
            let y0 = (y as f32 * sy) as u32;
            let x1 = ((x + rw) as f32 * sx) as u32;
            let y1 = ((y + rh) as f32 * sy) as u32;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    img[(yy * w + xx) as usize] = v;
                }
            }
        };
        fill(STRIP, 0);
        let bits = code.frame as u32 | (code.id as u32) << FRAME_BITS;
        for i in 0..CELLS {
            if bits & (1 << i) != 0 {
                fill(cell_rect(i), 255);
            }
        }
        img
    }

    #[test]
    fn round_trip_at_master_and_scaled_sizes() {
        for (w, h) in [(640, 360), (1280, 720), (1920, 1080)] {
            for code in [Code { frame: 0, id: 0 }, Code { frame: 12345, id: 7 }, Code { frame: 65535, id: 255 }] {
                let img = render(code, w, h);
                assert_eq!(read_luma(&img, w, h, w as usize), Some(code), "{w}x{h}");
            }
        }
    }

    #[test]
    fn rejects_frames_without_strip() {
        let img = vec![200u8; 640 * 360];
        assert_eq!(read_luma(&img, 640, 360, 640), None);
    }
}
