//! Shot sprites: a few frames spread over a shot, side by side in one JPEG,
//! so a shot can be scrubbed by hovering without touching the video. They
//! are made from the face pass's samples — no extra decoding.

use as_core::{FrameIdx, FrameRange};
use as_vision::image;

/// A shot's sprite: `cells` frames, left to right in time, each the same
/// size, in one JPEG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShotSprite {
    /// The shot: its start and end frame.
    pub shot: FrameIdx,
    pub end: FrameIdx,
    pub cells: u32,
    pub jpeg: Vec<u8>,
}

/// The size of a cell for frames of `w`×`h`: `width` wide (or the frame's
/// width, if smaller), the frame's shape.
pub fn cell_size((w, h): (usize, usize), width: usize) -> (usize, usize) {
    let cw = width.min(w).max(1);
    (cw, ((cw * h + w / 2) / w).clamp(1, h))
}

/// The sample (a multiple of `stride`) each of `cells` cells shows: the one
/// nearest the middle of the cell's stretch of the shot. `None` when no
/// sample lies in the shot.
pub fn cell_frames(range: FrameRange, stride: u32, cells: u32) -> Option<Vec<FrameIdx>> {
    let first = range.start.next_multiple_of(stride);
    if first >= range.end || cells == 0 {
        return None;
    }
    let last = (range.end - 1) / stride * stride;
    let samples = (last - first) / stride;
    let len = (range.end - range.start) as f64;
    Some(
        (0..cells)
            .map(|k| {
                let middle = range.start as f64 + (2 * k + 1) as f64 * len / (2 * cells) as f64;
                let i = ((middle - first as f64) / stride as f64).round().clamp(0.0, samples as f64) as u32;
                first + i * stride
            })
            .collect(),
    )
}

/// A sprite being filled while its shot's samples come in (in order).
pub(crate) struct SpriteCanvas {
    shot: FrameRange,
    frames: Vec<FrameIdx>,
    cell: (usize, usize),
    rgb: Vec<u8>,
}

impl SpriteCanvas {
    /// A canvas for shot `range` of frames `frame` big; `None` when no
    /// sample lies in the shot.
    pub fn new(range: FrameRange, stride: u32, cells: u32, frame: (usize, usize), width: usize) -> Option<Self> {
        let frames = cell_frames(range, stride, cells)?;
        let cell = cell_size(frame, width);
        let rgb = vec![0u8; cell.0 * frames.len() * cell.1 * 3];
        Some(Self { shot: range, frames, cell, rgb })
    }

    /// Put sample `f` (a `w`×`h` frame) in the cells that show it.
    pub fn paint(&mut self, f: FrameIdx, rgb: &[u8], size: (usize, usize)) {
        let Some(first) = self.frames.iter().position(|&x| x == f) else { return };
        let (cw, ch) = self.cell;
        let strip = cw * self.frames.len();
        image::shrink_into(rgb, size, &mut self.rgb, strip, first * cw, self.cell);
        // Short shots show one sample in several cells.
        for k in first + 1..self.frames.len() {
            if self.frames[k] != f {
                continue;
            }
            for y in 0..ch {
                let row = y * strip * 3;
                self.rgb.copy_within(row + first * cw * 3..row + (first + 1) * cw * 3, row + k * cw * 3);
            }
        }
    }

    pub fn finish(self, quality: u8) -> ShotSprite {
        let (cw, ch) = self.cell;
        let cells = self.frames.len();
        ShotSprite {
            shot: self.shot.start,
            end: self.shot.end,
            cells: cells as u32,
            jpeg: image::jpeg(&self.rgb, cw * cells, ch, quality),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_spread_over_the_shot() {
        // 96 frames, stride 3, 8 cells: middles at 6, 18, …, 90.
        assert_eq!(cell_frames(FrameRange::new(0, 96), 3, 8).unwrap(), [6, 18, 30, 42, 54, 66, 78, 90]);
        // Starting off the stride grid: samples are 102, 105, …, 120.
        let f = cell_frames(FrameRange::new(100, 122), 3, 8).unwrap();
        assert!(f.iter().all(|x| x % 3 == 0 && (100..122).contains(x)), "{f:?}");
        assert!(f.windows(2).all(|w| w[0] <= w[1]));
        // A shot with one sample shows it everywhere; one with none has no sprite.
        assert_eq!(cell_frames(FrameRange::new(10, 13), 3, 4).unwrap(), [12; 4]);
        assert_eq!(cell_frames(FrameRange::new(10, 12), 3, 4), None);
    }

    #[test]
    fn cells_keep_the_frame_shape() {
        assert_eq!(cell_size((1280, 720), 240), (240, 135));
        assert_eq!(cell_size((1280, 534), 240), (240, 100));
        assert_eq!(cell_size((160, 120), 240), (160, 120));
    }

    #[test]
    fn a_short_shot_repeats_its_sample() {
        // 2 samples (12 and 15) over 4 cells.
        let mut c = SpriteCanvas::new(FrameRange::new(10, 17), 3, 4, (8, 4), 4).unwrap();
        assert_eq!(c.frames, [12, 12, 15, 15]);
        let frame = |v: u8| vec![v; 8 * 4 * 3];
        c.paint(9, &frame(1), (8, 4));
        c.paint(12, &frame(100), (8, 4));
        c.paint(15, &frame(200), (8, 4));
        // Strip 16×2: cells 0 and 1 are 100, cells 2 and 3 are 200, both rows.
        for y in 0..2 {
            let row = &c.rgb[y * 16 * 3..(y + 1) * 16 * 3];
            assert!(row[..8 * 3].iter().all(|&v| v == 100) && row[8 * 3..].iter().all(|&v| v == 200));
        }
        let s = c.finish(80);
        assert_eq!((s.shot, s.end, s.cells, &s.jpeg[..2]), (10, 17, 4, &[0xFF, 0xD8][..]));
    }
}
