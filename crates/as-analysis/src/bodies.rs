//! People followed through a shot by their bodies — for when a face can't
//! say who someone is (turned away, too small, behind a gas mask) while the
//! body it was on still can. A body track is one person through one shot
//! (never across a cut), with the appearance of their clothes and build
//! (person ReID). [`crate::link`] ties face tracks to body tracks;
//! `as_identity::bodies` passes characters along those ties.

use as_core::FrameIdx;

/// Bodies darker than this (mean brightness of the crop, 0..=255) are
/// silhouettes: their appearance says nothing.
pub const MIN_LUMA: f32 = 30.0;

/// One sighting of a person. Boxes are x1, y1, x2, y2 as fractions of the
/// (upright, square-pixel) frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyDet {
    pub frame: FrameIdx,
    /// The whole person (cut off where the frame ends).
    pub bbox: [f32; 4],
    /// The part of them that shows.
    pub visible: [f32; 4],
    pub score: f32,
    /// Sharpness and brightness of the person's crop: a blurred shoulder in
    /// the foreground or a figure against the light says little.
    pub sharpness: f32,
    pub luma: f32,
}

impl BodyDet {
    /// Height of the visible part (fraction of the frame).
    pub fn height(&self) -> f32 {
        self.visible[3] - self.visible[1]
    }
}

/// A person found in one sample, before tracking.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyObs {
    pub det: BodyDet,
    /// Unit-length appearance embedding; `None` for people too small to embed.
    pub reid: Option<Vec<f32>>,
}

/// One person followed through one shot.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyTrack {
    /// Frames `start..end` (from the first sighting to one stride past the last).
    pub start: FrameIdx,
    pub end: FrameIdx,
    /// Start of the shot the track belongs to.
    pub shot: FrameIdx,
    pub dets: Vec<BodyDet>,
    /// Each sighting's appearance, if it had one (in the order of `dets`).
    pub reids: Vec<Option<Vec<f32>>>,
}

impl BodyTrack {
    /// Median height of the visible part (fraction of the frame).
    pub fn height(&self) -> f32 {
        let mut h: Vec<f32> = self.dets.iter().map(BodyDet::height).collect();
        h.sort_by(f32::total_cmp);
        h.get(h.len() / 2).copied().unwrap_or(0.0)
    }

    /// The visible box at `frame`: a sighting's, or between the two
    /// sightings around it when they are at most `max_gap` frames apart.
    pub fn box_at(&self, frame: FrameIdx, max_gap: u32) -> Option<[f32; 4]> {
        let i = self.dets.partition_point(|d| d.frame < frame);
        let after = self.dets.get(i)?;
        if after.frame == frame {
            return Some(after.visible);
        }
        let before = self.dets.get(i.checked_sub(1)?)?;
        if after.frame - before.frame > max_gap {
            return None;
        }
        let t = (frame - before.frame) as f32 / (after.frame - before.frame) as f32;
        Some(std::array::from_fn(|k| before.visible[k] + t * (after.visible[k] - before.visible[k])))
    }

    /// The frames the person is on screen, as [`crate::tracks::presence_spans`].
    pub fn spans(&self, stride: u32, max_gap: u32) -> Vec<(FrameIdx, FrameIdx)> {
        crate::tracks::spans(self.dets.iter().map(|d| d.frame), self.end, stride, max_gap)
    }

    /// Unit-length mean appearance of the sightings `pick` keeps, weighted
    /// by how sure and how big each is; silhouettes left out. `None`
    /// without any.
    pub fn appearance(&self, pick: impl Fn(&BodyDet) -> bool) -> Option<Vec<f32>> {
        let mut sum: Option<Vec<f32>> = None;
        for (d, e) in self.dets.iter().zip(&self.reids) {
            let Some(e) = e.as_ref().filter(|_| d.luma >= MIN_LUMA && pick(d)) else { continue };
            let w = d.score * (d.height() / 0.3).min(1.0);
            let s = sum.get_or_insert_with(|| vec![0.0; e.len()]);
            for (s, x) in s.iter_mut().zip(e) {
                *s += w * x;
            }
        }
        let mut v = sum?;
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n.is_nan() || n <= 1e-6 {
            return None;
        }
        v.iter_mut().for_each(|x| *x /= n);
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(frame: FrameIdx, x: f32) -> BodyDet {
        BodyDet {
            frame,
            bbox: [x, 0.1, x + 0.2, 0.9],
            visible: [x, 0.1, x + 0.2, 0.9],
            score: 0.9,
            sharpness: 100.0,
            luma: 120.0,
        }
    }

    fn track(dets: Vec<BodyDet>, reids: Vec<Option<Vec<f32>>>) -> BodyTrack {
        BodyTrack { start: dets[0].frame, end: dets.last().unwrap().frame + 3, shot: 0, dets, reids }
    }

    #[test]
    fn boxes_between_sightings_are_interpolated() {
        let t = track(vec![det(0, 0.0), det(12, 0.4), det(60, 0.4)], vec![None; 3]);
        assert_eq!(t.box_at(12, 24), Some([0.4, 0.1, 0.6, 0.9]));
        let mid = t.box_at(6, 24).unwrap();
        assert!((mid[0] - 0.2).abs() < 1e-6 && (mid[2] - 0.4).abs() < 1e-6);
        assert_eq!(t.box_at(30, 24), None, "48 frames between sightings: not interpolated");
        assert_eq!(t.box_at(61, 24), None, "after the last sighting");
        assert!((t.height() - 0.8).abs() < 1e-6);
        assert_eq!(t.spans(3, 8), [(0, 15), (60, 63)]);
    }

    #[test]
    fn appearance_is_a_weighted_mean_without_silhouettes() {
        let mut dark = det(3, 0.0);
        dark.luma = 10.0;
        let mut small = det(6, 0.0);
        small.visible[3] = 0.16;
        let t = track(
            vec![det(0, 0.0), dark, small],
            vec![Some(vec![1.0, 0.0]), Some(vec![0.0, 1.0]), Some(vec![0.0, 1.0])],
        );
        let a = t.appearance(|_| true).unwrap();
        // The big sighting weighs 0.9, the small one 0.9 × 0.06 / 0.3; the dark one nothing.
        assert!(a[0] > 0.97 && a[1] > 0.1, "{a:?}");
        assert_eq!(t.appearance(|d| d.frame == 3), None);
    }
}
