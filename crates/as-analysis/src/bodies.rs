//! People followed through a shot by their bodies — for when a face can't
//! say who someone is (turned away, too small, behind a gas mask) while the
//! body it was on still can. A body track is one person through one shot
//! (never across a cut), with the appearance of their clothes and build
//! (person ReID). [`crate::link`] ties face tracks to body tracks;
//! `as_identity::bodies` passes characters along those ties.

use as_core::FrameIdx;
use half::f16;

/// Bodies darker than this (mean brightness of the crop, 0..=255) are
/// silhouettes: their appearance says nothing.
pub const MIN_LUMA: f32 = 30.0;

/// Appearance embedding size (person ReID).
pub const REID_DIM: usize = as_vision::reid::DIM;

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
    /// Sharpness of the sharpest other person in the sample (0: alone).
    pub rival: f32,
}

impl BodyDet {
    /// Height of the visible part (fraction of the frame).
    pub fn height(&self) -> f32 {
        self.visible[3] - self.visible[1]
    }

    /// A blurred part of someone in the foreground — big, at a bottom
    /// corner of the frame, blurrier than someone else there: in an
    /// over-the-shoulder shot the shot is about the other person.
    pub fn blurred_foreground(&self) -> bool {
        let v = self.visible;
        let corner = v[3] >= 0.98 && (v[0] <= 0.02 || v[2] >= 0.98);
        corner && self.height() >= 0.45 && self.sharpness < 0.5 * self.rival
    }

    /// How much this sighting's appearance counts: sure and big ones most.
    pub fn weight(&self) -> f32 {
        self.score * (self.height() / 0.3).min(1.0)
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

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

impl BodyTrack {
    /// Median height of the visible part (fraction of the frame).
    pub fn height(&self) -> f32 {
        median(self.dets.iter().map(BodyDet::height).collect())
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
        spans_of(&self.dets, self.end, stride, max_gap)
    }

    /// Unit-length mean appearance of the sightings `pick` keeps, weighted
    /// by how sure and how big each is; silhouettes left out. `None`
    /// without any.
    pub fn appearance(&self, pick: impl Fn(&BodyDet) -> bool) -> Option<Vec<f32>> {
        let mut sum: Option<Vec<f32>> = None;
        for (d, e) in self.dets.iter().zip(&self.reids) {
            let Some(e) = e.as_ref().filter(|_| d.luma >= MIN_LUMA && pick(d)) else { continue };
            let w = d.weight();
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

    /// How the person shows in the track, in a frame `aspect` (width ÷
    /// height) wide.
    pub fn showing(&self, aspect: f32) -> Showing {
        showing(&self.dets, aspect)
    }
}

/// [`BodyTrack::spans`] of stored sightings.
pub fn spans_of(dets: &[BodyDet], end: FrameIdx, stride: u32, max_gap: u32) -> Vec<(FrameIdx, FrameIdx)> {
    crate::tracks::spans(dets.iter().map(|d| d.frame), end, stride, max_gap)
}

/// How a person shows in their track — for whether a shot of them alone,
/// without their face, makes a clip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Showing {
    /// Median height of the visible part (fraction of the frame).
    pub height: f32,
    /// Median width ÷ height of the visible part, in pixels.
    pub aspect: f32,
    /// Median top edge of the visible part (fraction of the frame).
    pub top: f32,
    /// Share of the sightings that are a blurred part in the foreground.
    pub foreground: f32,
    /// The surest sighting's detector score.
    pub score: f32,
}

/// What a clip of someone's body needs (set on bench/bodies: the hand, the
/// arm and the dark object the clothes matched stay out, every person the
/// audit saw from behind stays in): a body this tall ...
pub const CLIP_MIN_HEIGHT: f32 = 0.15;
/// ... at most this wide for its height (pixels): wider is a hand, an arm,
/// or something lying across the frame ...
pub const CLIP_MAX_ASPECT: f32 = 1.6;
/// ... with the head in the box: one hanging from the frame's top edge
/// (top within this) is the lower part of someone, unless it is this tall
/// (a close view) ...
pub const CLIP_TOP_EDGE: f32 = 0.01;
pub const CLIP_CLOSE: f32 = 0.6;
/// ... and mostly not a blurred part in the foreground.
pub const CLIP_MAX_FOREGROUND: f32 = 0.5;

/// [`Showing`] of sightings in a frame `aspect` (width ÷ height) wide.
pub fn showing(dets: &[BodyDet], aspect: f32) -> Showing {
    if dets.is_empty() {
        return Showing::default();
    }
    let ratio = |d: &BodyDet| (d.visible[2] - d.visible[0]) * aspect / d.height().max(1e-6);
    Showing {
        height: median(dets.iter().map(BodyDet::height).collect()),
        aspect: median(dets.iter().map(ratio).collect()),
        top: median(dets.iter().map(|d| d.visible[1]).collect()),
        foreground: dets.iter().filter(|d| d.blurred_foreground()).count() as f32 / dets.len() as f32,
        score: dets.iter().map(|d| d.score).fold(0.0, f32::max),
    }
}

/// About how tall the face of a body's person is (share of the frame), for
/// the size of a shot it is alone in: a head is an eighth of a whole
/// figure; a figure the frame's bottom cuts off is seen closer, the part
/// that shows about three heads tall.
pub fn face_height_of(dets: &[BodyDet]) -> f32 {
    let height = median(dets.iter().map(BodyDet::height).collect());
    let bottom = median(dets.iter().map(|d| d.visible[3]).collect());
    if bottom >= 0.97 { height / 3.0 } else { height / 8.0 }
}

impl Showing {
    /// Whether the person shows enough to be a clip of their own.
    pub fn prominent(&self) -> bool {
        let headless = self.top <= CLIP_TOP_EDGE && self.height < CLIP_CLOSE;
        self.height >= CLIP_MIN_HEIGHT
            && self.aspect <= CLIP_MAX_ASPECT
            && !headless
            && self.foreground <= CLIP_MAX_FOREGROUND
    }
}

/// A body track as the face pass hands it out: the face tracks tied to it,
/// and how its person looks — all of them, and while no face of theirs
/// shows.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundBody {
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub shot: FrameIdx,
    pub dets: Vec<BodyDet>,
    /// `(face, samples that tied them)`: the face as the pass names it (see
    /// `as_analysis::faces::FaceChunk::bodies`).
    pub faces: Vec<(u32, u32)>,
    pub reid: Option<Vec<f32>>,
    pub reid_away: Option<Vec<f32>>,
    pub showing: Showing,
}

const FORMAT: u8 = 1;
/// Bytes of one stored sighting.
const DET_BYTES: usize = 52;

fn put_f32s(out: &mut Vec<u8>, v: &[f32]) {
    v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
}

/// Pack sightings (what the database keeps): 52 bytes each.
pub fn dets_to_bytes(dets: &[BodyDet]) -> Vec<u8> {
    let mut out = Vec::with_capacity(dets.len() * DET_BYTES);
    for d in dets {
        out.extend_from_slice(&d.frame.to_le_bytes());
        put_f32s(&mut out, &d.bbox);
        put_f32s(&mut out, &d.visible);
        put_f32s(&mut out, &[d.score, d.sharpness, d.luma, d.rival]);
    }
    out
}

pub fn dets_from_bytes(b: &[u8]) -> Option<Vec<BodyDet>> {
    if !b.len().is_multiple_of(DET_BYTES) {
        return None;
    }
    let f = |c: &[u8], i: usize| f32::from_le_bytes(c[4 + 4 * i..8 + 4 * i].try_into().expect("4 bytes"));
    Some(
        b.as_chunks::<DET_BYTES>()
            .0
            .iter()
            .map(|c| BodyDet {
                frame: u32::from_le_bytes(c[..4].try_into().expect("4 bytes")),
                bbox: [f(c, 0), f(c, 1), f(c, 2), f(c, 3)],
                visible: [f(c, 4), f(c, 5), f(c, 6), f(c, 7)],
                score: f(c, 8),
                sharpness: f(c, 9),
                luma: f(c, 10),
                rival: f(c, 11),
            })
            .collect(),
    )
}

/// Pack an appearance as little-endian f16 (what the database keeps).
pub fn reid_to_bytes(e: &[f32]) -> Vec<u8> {
    e.iter().flat_map(|&v| f16::from_f32(v).to_le_bytes()).collect()
}

/// Unpack [`reid_to_bytes`], renormalised to unit length.
pub fn reid_from_bytes(b: &[u8]) -> Option<Vec<f32>> {
    if b.len() != REID_DIM * 2 {
        return None;
    }
    let mut v: Vec<f32> = b.as_chunks::<2>().0.iter().map(|c| f16::from_le_bytes(*c).to_f32()).collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter_mut().for_each(|x| *x /= n);
    Some(v)
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        (self.0.len() >= n).then(|| {
            let (a, b) = self.0.split_at(n);
            self.0 = b;
            a
        })
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn reid(&mut self) -> Option<Option<Vec<f32>>> {
        match self.take(1)?[0] {
            0 => Some(None),
            _ => Some(Some(reid_from_bytes(self.take(REID_DIM * 2)?)?)),
        }
    }
}

/// Serialise found bodies (one checkpoint chunk) for the engine protocol.
pub fn encode(bodies: &[FoundBody]) -> Vec<u8> {
    let mut out = vec![FORMAT];
    out.extend_from_slice(&(bodies.len() as u32).to_le_bytes());
    for b in bodies {
        for v in [b.start, b.end, b.shot, b.dets.len() as u32, b.faces.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&dets_to_bytes(&b.dets));
        for &(face, samples) in &b.faces {
            out.extend_from_slice(&face.to_le_bytes());
            out.extend_from_slice(&samples.to_le_bytes());
        }
        for e in [&b.reid, &b.reid_away] {
            match e {
                Some(e) => {
                    out.push(1);
                    out.extend_from_slice(&reid_to_bytes(e));
                }
                None => out.push(0),
            }
        }
        let s = b.showing;
        put_f32s(&mut out, &[s.height, s.aspect, s.top, s.foreground, s.score]);
    }
    out
}

pub fn decode(blob: &[u8]) -> Option<Vec<FoundBody>> {
    let (&format, body) = blob.split_first()?;
    if format != FORMAT {
        return None;
    }
    let mut r = Reader(body);
    let n = r.u32()? as usize;
    let mut out = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let (start, end, shot, dets, faces) = (r.u32()?, r.u32()?, r.u32()?, r.u32()? as usize, r.u32()? as usize);
        let dets = dets_from_bytes(r.take(dets.checked_mul(DET_BYTES)?)?)?;
        let faces = (0..faces).map(|_| Some((r.u32()?, r.u32()?))).collect::<Option<Vec<_>>>()?;
        let (reid, reid_away) = (r.reid()?, r.reid()?);
        let showing =
            Showing { height: r.f32()?, aspect: r.f32()?, top: r.f32()?, foreground: r.f32()?, score: r.f32()? };
        out.push(FoundBody { start, end, shot, dets, faces, reid, reid_away, showing });
    }
    r.0.is_empty().then_some(out)
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
            rival: 0.0,
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

    fn at(visible: [f32; 4], score: f32) -> BodyDet {
        BodyDet { visible, bbox: visible, score, ..det(0, 0.0) }
    }

    #[test]
    fn a_clip_needs_someone_big_upright_and_in_focus() {
        // 16:9 frames. Someone from behind filling half the frame: a clip.
        let walking = showing(&[at([0.4, 0.3, 0.55, 0.9], 0.9)], 16.0 / 9.0);
        assert!(walking.prominent(), "{walking:?}");
        assert!((walking.aspect - 0.15 * 16.0 / 9.0 / 0.6).abs() < 1e-5);
        // A tiny figure in the back; a hand across the frame; legs hanging from the top edge.
        assert!(!showing(&[at([0.4, 0.4, 0.45, 0.5], 0.9)], 16.0 / 9.0).prominent());
        assert!(!showing(&[at([0.0, 0.0, 0.89, 0.97], 0.9)], 16.0 / 9.0).prominent(), "1.6+ wide: a hand");
        assert!(!showing(&[at([0.4, 0.0, 0.5, 0.3], 0.9)], 16.0 / 9.0).prominent());
        // A close view hanging from the top edge has its head in it.
        assert!(showing(&[at([0.36, 0.0, 1.0, 0.99], 0.96)], 16.0 / 9.0).prominent(), "bent over, filling the frame");
        // A blurred shoulder in a bottom corner while someone else is sharp: not theirs.
        let mut shoulder = at([0.6, 0.2, 1.0, 1.0], 0.95);
        shoulder.sharpness = 40.0;
        shoulder.rival = 300.0;
        assert!(shoulder.blurred_foreground());
        assert!(!showing(&[shoulder], 16.0 / 9.0).prominent());
        // Alone in the frame, nothing is blurrier than anyone.
        assert!(!BodyDet { rival: 0.0, ..shoulder }.blurred_foreground());
        assert_eq!(showing(&[at([0.4, 0.3, 0.55, 0.9], 0.5), at([0.4, 0.3, 0.55, 0.9], 0.8)], 1.0).score, 0.8);
        // A back filling the frame is about a close-up; a whole figure, a medium or wide shot.
        assert!((face_height_of(&[at([0.3, 0.0, 0.9, 1.0], 0.9)]) - 1.0 / 3.0).abs() < 1e-6);
        assert!((face_height_of(&[at([0.4, 0.1, 0.5, 0.9], 0.9)]) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn found_bodies_round_trip() {
        let mut e = vec![0f32; REID_DIM];
        e[1] = 0.6;
        e[9] = 0.8;
        let b = FoundBody {
            start: 30,
            end: 99,
            shot: 12,
            dets: vec![det(30, 0.1), BodyDet { rival: 55.5, ..det(96, 0.2) }],
            faces: vec![(3, 7), (9, 2)],
            reid: Some(e.clone()),
            reid_away: None,
            showing: Showing { height: 0.8, aspect: 0.5, top: 0.1, foreground: 0.0, score: 0.9 },
        };
        let back = decode(&encode(std::slice::from_ref(&b))).unwrap();
        assert_eq!(back.len(), 1);
        let x = &back[0];
        assert_eq!((x.start, x.end, x.shot, &x.dets, &x.faces, x.showing), (30, 99, 12, &b.dets, &b.faces, b.showing));
        assert!(x.reid.as_ref().unwrap().iter().zip(&e).all(|(a, b)| (a - b).abs() < 1e-3));
        assert_eq!(x.reid_away, None);
        assert!(decode(&encode(&[])).unwrap().is_empty());
        assert!(decode(b"\x01\x05").is_none());
        assert_eq!(dets_from_bytes(&dets_to_bytes(&b.dets)).unwrap(), b.dets);
    }
}
