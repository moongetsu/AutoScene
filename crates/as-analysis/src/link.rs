//! Whose face is on which body. Sample by sample, a face goes with the body
//! it sits on top of — its centre inside the body's visible box, nearly all
//! of it covered, near the top and the middle — unless another pairing
//! fits almost as well (two people side by side, someone in front of
//! someone else): then that sample says nothing. A face track goes with the
//! body most of its samples went with. A body carries one face at a time;
//! one turning away and back again leaves two face tracks on one body,
//! which is the point.

use std::collections::HashMap;

use as_core::FrameIdx;

use crate::bodies::BodyTrack;
use crate::tracks::FaceTrack;

#[derive(Clone, Copy, Debug)]
pub struct LinkParams {
    /// Share of the face box that must be inside the body's visible box.
    pub min_cover: f32,
    /// A face is at least this share of its body box's height: a face in
    /// the far background inside a big foreground box is someone else's.
    pub min_height: f32,
    /// How far the face's centre may be from the body's centre line, in
    /// body widths.
    pub max_dx: f32,
    /// How far below the top of the body's box the face's centre may be, in
    /// face heights: the face of someone in front of a taller box is lower.
    pub max_drop: f32,
    /// A pairing whose rival scores within this of it makes the sample
    /// too close to call.
    pub rival_margin: f32,
    /// Body boxes are interpolated across gaps up to this many frames.
    pub max_interp: u32,
    /// Frames between face samples: two faces tied to one body less than a
    /// sample apart are on it at the same time.
    pub stride: u32,
    /// A tie needs this many samples (all of the face's, if it has fewer)
    /// and this share of the samples the face sat on any body (those too
    /// close to call included).
    pub min_samples: u32,
    pub min_share: f32,
}

impl Default for LinkParams {
    fn default() -> Self {
        Self {
            min_cover: 0.75,
            min_height: 0.07,
            max_dx: 0.35,
            max_drop: 2.0,
            rival_margin: 0.12,
            max_interp: 24,
            stride: 3,
            min_samples: 2,
            min_share: 0.6,
        }
    }
}

/// A face track tied to a body track (indices into what [`link_shot`] got).
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub face: usize,
    pub body: usize,
    /// The face's samples that went with this body, with another one, and
    /// that were too close to call.
    pub samples: u32,
    pub elsewhere: u32,
    pub ambiguous: u32,
    /// The frames tied, in order.
    pub frames: Vec<FrameIdx>,
}

/// How well face box `f` sits on body box `b` (x1, y1, x2, y2, fractions of
/// one frame), 0..=1; `None` if it doesn't.
pub fn fit(f: &[f32; 4], b: &[f32; 4], p: &LinkParams) -> Option<f32> {
    let (fcx, fcy) = ((f[0] + f[2]) / 2.0, (f[1] + f[3]) / 2.0);
    if !(b[0] <= fcx && fcx <= b[2] && b[1] <= fcy && fcy <= b[3]) {
        return None;
    }
    let (fw, fh, bw, bh) = (f[2] - f[0], f[3] - f[1], b[2] - b[0], b[3] - b[1]);
    if !(fw > 0.0 && fh > 0.0 && bw > 0.0 && bh > 0.0) {
        return None;
    }
    let inter = (f[2].min(b[2]) - f[0].max(b[0])).max(0.0) * (f[3].min(b[3]) - f[1].max(b[1])).max(0.0);
    let cover = inter / (fw * fh);
    let dx = (fcx - (b[0] + b[2]) / 2.0).abs() / bw;
    let drop = (fcy - b[1]) / fh;
    (cover >= p.min_cover && fh >= p.min_height * bh && dx <= p.max_dx && drop <= p.max_drop)
        .then(|| 0.75 * cover + 0.25 * (1.0 - 2.0 * dx).max(0.0))
}

/// Tie the face tracks of one shot to its body tracks.
pub fn link_shot(faces: &[&FaceTrack], bodies: &[&BodyTrack], p: &LinkParams) -> Vec<Link> {
    // Per frame, every face seen there.
    let mut at: HashMap<FrameIdx, Vec<(usize, [f32; 4])>> = HashMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for d in &f.dets {
            at.entry(d.frame).or_default().push((fi, d.bbox));
        }
    }
    // (face, body) → the frames they were tied; per face: samples too close to call.
    let mut tied: HashMap<(usize, usize), Vec<FrameIdx>> = HashMap::new();
    let mut ambiguous = vec![0u32; faces.len()];
    let mut frames: Vec<FrameIdx> = at.keys().copied().collect();
    frames.sort_unstable();
    for frame in frames {
        let here = &at[&frame];
        let boxes: Vec<(usize, [f32; 4])> =
            bodies.iter().enumerate().filter_map(|(bi, b)| b.box_at(frame, p.max_interp).map(|x| (bi, x))).collect();
        let mut cands: Vec<(f32, usize, usize)> = Vec::new();
        for &(fi, fb) in here {
            for &(bi, bb) in &boxes {
                if let Some(s) = fit(&fb, &bb, p) {
                    cands.push((s, fi, bi));
                }
            }
        }
        cands.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (mut face_used, mut body_used) = (Vec::new(), Vec::new());
        for &(s, fi, bi) in &cands {
            if face_used.contains(&fi) || body_used.contains(&bi) {
                continue;
            }
            let rival = cands.iter().any(|&(r, f2, b2)| ((f2 == fi) != (b2 == bi)) && r > s - p.rival_margin);
            if rival {
                // Neither this face nor this body is told apart here.
                ambiguous[fi] += 1;
                face_used.push(fi);
                body_used.push(bi);
                continue;
            }
            face_used.push(fi);
            body_used.push(bi);
            tied.entry((fi, bi)).or_default().push(frame);
        }
    }
    let mut total = vec![0u32; faces.len()];
    for (&(fi, _), v) in &tied {
        total[fi] += v.len() as u32;
    }
    let mut links: Vec<Link> = Vec::new();
    for (fi, f) in faces.iter().enumerate() {
        let best =
            tied.iter().filter(|((f2, _), _)| *f2 == fi).max_by_key(|((_, bi), v)| (v.len(), std::cmp::Reverse(*bi)));
        let Some((&(_, bi), v)) = best else { continue };
        let n = v.len() as u32;
        let need = p.min_samples.min(f.dets.len() as u32);
        if n >= need && n as f32 >= p.min_share * (total[fi] + ambiguous[fi]) as f32 {
            links.push(Link {
                face: fi,
                body: bi,
                samples: n,
                elsewhere: total[fi] - n,
                ambiguous: ambiguous[fi],
                frames: v.clone(),
            });
        }
    }
    // One face per body at a time: of two tied to it less than a sample
    // apart, the better-attested one stays. (Faces in turn, one turned away
    // while the other shows, both stay.)
    links.sort_by(|a, b| b.samples.cmp(&a.samples).then(a.face.cmp(&b.face)));
    let mut kept: Vec<Link> = Vec::new();
    for l in links {
        let together = |k: &Link| {
            k.body == l.body
                && k.frames.iter().any(|&a| {
                    let i = l.frames.partition_point(|&b| b + p.stride < a);
                    l.frames.get(i).is_some_and(|&b| b <= a + p.stride)
                })
        };
        if !kept.iter().any(together) {
            kept.push(l);
        }
    }
    kept.sort_by_key(|l| (l.body, l.frames.first().copied(), l.face));
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bodies::BodyDet;
    use crate::tracks::FaceDet;

    fn face(frames: &[FrameIdx], bbox: [f32; 4]) -> FaceTrack {
        FaceTrack {
            start: frames[0],
            end: frames.last().unwrap() + 3,
            shot: 0,
            dets: frames.iter().map(|&frame| FaceDet { frame, bbox, score: 0.9, norm: 20.0, quality: 0.8 }).collect(),
            embedding: vec![],
            quality: 0.8,
            thumb: vec![],
        }
    }

    fn body(frames: &[FrameIdx], visible: [f32; 4]) -> BodyTrack {
        BodyTrack {
            start: frames[0],
            end: frames.last().unwrap() + 3,
            shot: 0,
            dets: frames
                .iter()
                .map(|&frame| BodyDet {
                    frame,
                    bbox: visible,
                    visible,
                    score: 0.9,
                    sharpness: 100.0,
                    luma: 120.0,
                    rival: 0.0,
                })
                .collect(),
            reids: vec![None; frames.len()],
        }
    }

    const P: LinkParams = LinkParams {
        min_cover: 0.75,
        min_height: 0.07,
        max_dx: 0.35,
        max_drop: 2.0,
        rival_margin: 0.12,
        max_interp: 24,
        stride: 3,
        min_samples: 2,
        min_share: 0.6,
    };

    #[test]
    fn the_defaults_are_the_tested_ones() {
        let d = LinkParams::default();
        assert_eq!(
            (
                d.min_cover,
                d.min_height,
                d.max_dx,
                d.max_drop,
                d.rival_margin,
                d.max_interp,
                d.stride,
                d.min_samples,
                d.min_share
            ),
            (
                P.min_cover,
                P.min_height,
                P.max_dx,
                P.max_drop,
                P.rival_margin,
                P.max_interp,
                P.stride,
                P.min_samples,
                P.min_share
            )
        );
    }

    #[test]
    fn a_face_goes_with_the_body_it_sits_on() {
        let f = face(&[0, 3, 6], [0.45, 0.1, 0.55, 0.25]);
        let near = body(&[0, 3, 6], [0.35, 0.05, 0.65, 0.95]);
        let other = body(&[0, 3, 6], [0.7, 0.05, 0.95, 0.95]);
        let links = link_shot(&[&f], &[&other, &near], &P);
        assert_eq!(links.len(), 1);
        assert_eq!((links[0].body, links[0].samples, links[0].ambiguous), (1, 3, 0));
    }

    #[test]
    fn a_face_low_on_a_body_is_someone_in_front() {
        // The face of a short person in front of a tall box: centre 4.5 face heights down.
        let f = face(&[0, 3], [0.45, 0.5, 0.55, 0.6]);
        let tall = body(&[0, 3], [0.3, 0.1, 0.7, 0.95]);
        assert!(link_shot(&[&f], &[&tall], &P).is_empty());
        assert_eq!(fit(&[0.45, 0.5, 0.55, 0.6], &[0.3, 0.1, 0.7, 0.95], &P), None);
    }

    #[test]
    fn two_bodies_alike_around_a_face_say_nothing() {
        // Two overlapping boxes, the face between their centres.
        let f = face(&[0, 3], [0.45, 0.1, 0.55, 0.25]);
        let a = body(&[0, 3], [0.3, 0.05, 0.62, 0.95]);
        let b = body(&[0, 3], [0.38, 0.05, 0.7, 0.95]);
        let links = link_shot(&[&f], &[&a, &b], &P);
        assert!(links.is_empty(), "{links:?}");
    }

    #[test]
    fn turning_away_and_back_leaves_two_faces_on_one_body() {
        let before = face(&[0, 3, 6], [0.45, 0.1, 0.55, 0.25]);
        let after = face(&[30, 33], [0.46, 0.1, 0.56, 0.25]);
        // Sampled every 12 frames: boxes in between are interpolated.
        let b = body(&[0, 12, 24, 36], [0.35, 0.05, 0.65, 0.95]);
        let links = link_shot(&[&before, &after], &[&b], &P);
        assert_eq!(links.iter().map(|l| (l.face, l.body, l.samples)).collect::<Vec<_>>(), [(0, 0, 3), (1, 0, 2)]);
    }

    #[test]
    fn a_body_carries_one_face_at_a_time() {
        // Two face tracks on one body in alternate samples (a body box over
        // two people): the better-attested one stays.
        let a = face(&[0, 6, 12], [0.45, 0.1, 0.55, 0.25]);
        let b = face(&[3, 9], [0.46, 0.11, 0.56, 0.26]);
        let body = body(&[0, 3, 6, 9, 12], [0.35, 0.05, 0.65, 0.95]);
        let links = link_shot(&[&a, &b], &[&body], &P);
        assert_eq!(links.iter().map(|l| l.face).collect::<Vec<_>>(), [0]);
        // One face gone for a while (its track rejoined by identity) and
        // another on the same body in the gap: never on it together, both stay.
        let a = face(&[0, 3, 6, 30, 33, 36], [0.45, 0.1, 0.55, 0.25]);
        let b = face(&[12, 15, 18], [0.46, 0.11, 0.56, 0.26]);
        let body = super::tests::body(&[0, 12, 24, 36], [0.35, 0.05, 0.65, 0.95]);
        let links = link_shot(&[&a, &b], &[&body], &P);
        assert_eq!(links.iter().map(|l| (l.face, l.samples)).collect::<Vec<_>>(), [(0, 6), (1, 3)]);
    }

    #[test]
    fn a_small_face_far_behind_a_big_body_is_someone_else() {
        // A background face near the top of a foreground shoulder's box.
        let f = face(&[0, 3], [0.62, 0.06, 0.66, 0.12]);
        let shoulder = body(&[0, 3], [0.4, 0.0, 1.0, 1.0]);
        assert_eq!(fit(&[0.62, 0.06, 0.66, 0.12], &[0.4, 0.0, 1.0, 1.0], &P), None);
        assert!(link_shot(&[&f], &[&shoulder], &P).is_empty());
    }

    #[test]
    fn a_face_mostly_too_close_to_call_is_tied_to_nobody() {
        // Two clear samples on `a`, then four where `b` stands as close.
        let f = face(&[0, 3, 6, 9, 12, 15], [0.45, 0.1, 0.55, 0.25]);
        let a = body(&[0, 3, 6, 9, 12, 15], [0.3, 0.05, 0.62, 0.95]);
        let b = body(&[6, 9, 12, 15], [0.38, 0.05, 0.7, 0.95]);
        let links = link_shot(&[&f], &[&a, &b], &P);
        assert!(links.is_empty(), "{links:?}");
    }
}
