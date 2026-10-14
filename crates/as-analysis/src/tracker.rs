//! Follows faces through one shot. Samples are a few frames apart, so a face
//! usually barely moves between them: detections link to the track whose box
//! — moved on as the face was moving — overlaps theirs, unless their
//! identities clearly disagree (two people crossing). A face moving faster
//! than its own size per sample (a small face walking past, a whip pan)
//! links to a track it is within reach of and the size of, if their
//! identities agree more than they need to for an overlap. A track that
//! loses its face for a moment (a turn, an occlusion) picks it up again
//! later in the shot when the identity matches well. Tracks never cross a
//! cut.

use as_core::{FrameIdx, FrameRange};
use as_vision::arcface::cosine;
use as_vision::scrfd::iou;

use crate::tracks::{DIM, FaceDet, FaceTrack};

/// A face detected and embedded in one sample, before tracking.
#[derive(Clone, Debug)]
pub struct Observation {
    pub det: FaceDet,
    /// Unit-length embedding.
    pub embedding: Vec<f32>,
    /// JPEG thumbnail.
    pub thumb: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct TrackerParams {
    /// Frames between samples.
    pub stride: u32,
    /// A track keeps linking by overlap for this many samples without a face.
    pub max_gap: u32,
    /// Minimum box overlap to link by position.
    pub min_iou: f32,
    /// A face this many of its own sizes (per sample) from where its track
    /// was heading still links, for this many samples after the last one
    /// (one, while the track's face was seen only once and its way is unknown).
    pub reach: f32,
    pub reach_gap: u32,
    /// ... if its box is about the track's size (ratio of heights at most this)
    pub max_size_ratio: f32,
    /// ... and their identities agree this much: a neighbour is about as
    /// near as a face that moved.
    pub min_cos_reach: f32,
    /// Identity agreement still required when linking by position.
    pub min_cos_near: f32,
    /// Identity agreement that links on its own (anywhere in the shot).
    pub min_cos_far: f32,
    /// Both detections need this quality before identity alone links them.
    pub min_quality_far: f32,
    /// Tracks below this quality say nothing about anyone and are dropped.
    pub min_track_quality: f32,
}

impl TrackerParams {
    pub fn new(stride: u32) -> Self {
        Self {
            stride,
            max_gap: 8,
            min_iou: 0.3,
            reach: 1.5,
            reach_gap: 2,
            max_size_ratio: 1.5,
            min_cos_reach: 0.3,
            min_cos_near: 0.15,
            min_cos_far: 0.5,
            min_quality_far: 0.3,
            min_track_quality: 0.05,
        }
    }
}

struct Open {
    obs: Vec<Observation>,
    sum: Vec<f32>,
    mean: Vec<f32>,
    last_frame: FrameIdx,
    last_bbox: [f32; 4],
    /// How far the box centre moved per frame lately (`None`: one sighting).
    velocity: Option<(f32, f32)>,
    best_quality: f32,
}

fn center(b: &[f32; 4]) -> (f32, f32) {
    ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)
}

fn shifted(b: &[f32; 4], (dx, dy): (f32, f32)) -> [f32; 4] {
    [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy]
}

impl Open {
    fn new(o: Observation) -> Self {
        let mut t = Self {
            obs: Vec::new(),
            sum: vec![0.0; DIM],
            mean: vec![0.0; DIM],
            last_frame: 0,
            last_bbox: [0.0; 4],
            velocity: None,
            best_quality: 0.0,
        };
        t.push(o);
        t
    }

    fn push(&mut self, o: Observation) {
        // Weight by quality so a blurry profile barely moves the identity.
        let w = o.det.quality.max(0.02);
        for (s, x) in self.sum.iter_mut().zip(&o.embedding) {
            *s += w * x;
        }
        let n = self.sum.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        self.mean = self.sum.iter().map(|x| x / n).collect();
        if !self.obs.is_empty() {
            let frames = o.det.frame.saturating_sub(self.last_frame).max(1) as f32;
            let ((x0, y0), (x1, y1)) = (center(&self.last_bbox), center(&o.det.bbox));
            self.velocity = Some(((x1 - x0) / frames, (y1 - y0) / frames));
        }
        self.last_frame = o.det.frame;
        self.last_bbox = o.det.bbox;
        self.best_quality = self.best_quality.max(o.det.quality);
        self.obs.push(o);
    }

    /// Where the box would be at `frame` if the face kept moving as it was.
    fn predicted(&self, frame: FrameIdx) -> [f32; 4] {
        let frames = frame.saturating_sub(self.last_frame) as f32;
        self.velocity.map_or(self.last_bbox, |(vx, vy)| shifted(&self.last_bbox, (vx * frames, vy * frames)))
    }

    /// Whether `b` at `frame` could be this face moving fast: about its size,
    /// within `reach` of its sizes per sample of where it was heading.
    fn within_reach(&self, b: &[f32; 4], frame: FrameIdx, p: &TrackerParams) -> bool {
        let gap = frame.saturating_sub(self.last_frame) / p.stride.max(1);
        let max_gap = if self.velocity.is_some() { p.reach_gap } else { 1 };
        if gap == 0 || gap > max_gap {
            return false;
        }
        let (w, h) = (self.last_bbox[2] - self.last_bbox[0], self.last_bbox[3] - self.last_bbox[1]);
        let ratio = (b[3] - b[1]) / h.max(1e-6);
        if !(1.0 / p.max_size_ratio..=p.max_size_ratio).contains(&ratio) {
            return false;
        }
        let ((px, py), (x, y)) = (center(&self.predicted(frame)), center(b));
        let reach = p.reach * gap as f32;
        (x - px).abs() <= reach * w && (y - py).abs() <= reach * h
    }

    fn finish(self, shot: FrameRange, stride: u32) -> FaceTrack {
        let mut qs: Vec<f32> = self.obs.iter().map(|o| o.det.quality).collect();
        qs.sort_by(|a, b| b.total_cmp(a));
        let top = &qs[..qs.len().min(3)];
        let quality = top.iter().sum::<f32>() / top.len() as f32;
        // Embedding: quality² weights, so the clearest views dominate.
        let mut sum = vec![0f32; DIM];
        let mut total = 0.0;
        for o in &self.obs {
            let w = o.det.quality * o.det.quality;
            total += w;
            for (s, x) in sum.iter_mut().zip(&o.embedding) {
                *s += w * x;
            }
        }
        if total < 1e-4 {
            sum = self.sum;
        }
        let n = sum.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        let embedding = sum.iter().map(|x| x / n).collect();
        let best = self
            .obs
            .iter()
            .max_by(|a, b| a.det.quality.total_cmp(&b.det.quality).then(a.det.height().total_cmp(&b.det.height())))
            .expect("tracks have observations");
        let start = self.obs[0].det.frame;
        let end = (self.last_frame + stride).min(shot.end);
        FaceTrack {
            start,
            end,
            shot: shot.start,
            dets: self.obs.iter().map(|o| o.det).collect(),
            embedding,
            quality,
            thumb: best.thumb.clone(),
        }
    }
}

/// Link the observations of one shot (`samples` in frame order) into tracks,
/// ordered by start.
pub fn track_shot(shot: FrameRange, samples: Vec<(FrameIdx, Vec<Observation>)>, p: &TrackerParams) -> Vec<FaceTrack> {
    let mut open: Vec<Open> = Vec::new();
    for (frame, obs) in samples {
        debug_assert!(shot.contains(frame));
        // Candidate links, best first: overlap plus identity.
        let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
        for (oi, o) in obs.iter().enumerate() {
            for (ti, t) in open.iter().enumerate() {
                let gap = (frame - t.last_frame) / p.stride.max(1);
                let cos = cosine(&o.embedding, &t.mean);
                let overlap = iou(&o.det.bbox, &t.predicted(frame)).max(iou(&o.det.bbox, &t.last_bbox));
                let near = gap <= p.max_gap && overlap >= p.min_iou && cos >= p.min_cos_near;
                let moving = cos >= p.min_cos_reach && t.within_reach(&o.det.bbox, frame, p);
                let far =
                    cos >= p.min_cos_far && o.det.quality >= p.min_quality_far && t.best_quality >= p.min_quality_far;
                if near || moving || far {
                    // Overlap counts most; a face within reach a little.
                    let place = if gap > p.max_gap { 0.0 } else { overlap.max(if moving { 0.2 } else { 0.0 }) };
                    pairs.push((cos + place, oi, ti));
                }
            }
        }
        pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut target: Vec<Option<usize>> = vec![None; obs.len()];
        let mut taken = vec![false; open.len()];
        for (_, oi, ti) in pairs {
            if target[oi].is_none() && !taken[ti] {
                target[oi] = Some(ti);
                taken[ti] = true;
            }
        }
        for (o, t) in obs.into_iter().zip(target) {
            match t {
                Some(ti) => open[ti].push(o),
                None => open.push(Open::new(o)),
            }
        }
    }
    let mut tracks: Vec<FaceTrack> =
        open.into_iter().filter(|t| t.best_quality >= p.min_track_quality).map(|t| t.finish(shot, p.stride)).collect();
    tracks.sort_by_key(|t| (t.start, t.end));
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emb(axis: usize) -> Vec<f32> {
        let mut v = vec![0f32; DIM];
        v[axis] = 1.0;
        v
    }

    fn obs(frame: FrameIdx, x: f32, who: usize, quality: f32) -> Observation {
        Observation {
            det: FaceDet { frame, bbox: [x, 0.2, x + 0.1, 0.4], score: 0.9, norm: 20.0, quality },
            embedding: emb(who),
            thumb: vec![who as u8],
        }
    }

    #[test]
    fn two_people_crossing_stay_apart() {
        // A walks right, B walks left; they overlap at the middle sample.
        let shot = FrameRange::new(0, 30);
        let samples = (0..5u32)
            .map(|i| {
                let f = i * 3;
                (f, vec![obs(f, 0.1 + 0.05 * i as f32, 1, 0.8), obs(f, 0.3 - 0.05 * i as f32, 2, 0.8)])
            })
            .collect();
        let tracks = track_shot(shot, samples, &TrackerParams::new(3));
        assert_eq!(tracks.len(), 2);
        for t in &tracks {
            assert_eq!(t.dets.len(), 5);
            let who = t.thumb[0] as usize;
            assert!((cosine(&t.embedding, &emb(who)) - 1.0).abs() < 1e-5);
        }
        assert_eq!((tracks[0].start, tracks[0].end), (0, 15));
    }

    #[test]
    fn a_small_face_moving_fast_stays_one_track() {
        // 0.03 wide, 0.03 to the left per sample: no two boxes overlap.
        let shot = FrameRange::new(0, 60);
        let at = |i: u32| {
            let mut o = obs(i * 3, 0.9 - 0.03 * i as f32, 4, 0.1);
            o.det.bbox[2] = o.det.bbox[0] + 0.03;
            o.det.bbox[3] = o.det.bbox[1] + 0.05;
            (i * 3, vec![o])
        };
        let tracks = track_shot(shot, (0..15).map(at).collect(), &TrackerParams::new(3));
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].dets.len(), 15);
        // Someone else crossing the same way doesn't join it.
        let mut samples: Vec<(FrameIdx, Vec<Observation>)> = (0..6).map(at).collect();
        let mut other = obs(18, 0.9 - 0.03 * 6.0, 7, 0.1);
        other.det.bbox[2] = other.det.bbox[0] + 0.03;
        other.det.bbox[3] = other.det.bbox[1] + 0.05;
        samples.push((18, vec![other]));
        assert_eq!(track_shot(shot, samples, &TrackerParams::new(3)).len(), 2);
    }

    /// A unit vector with cosine `cos` to axis `a`, in the plane of axes `a` and `b`.
    fn blend(a: usize, b: usize, cos: f32) -> Vec<f32> {
        let mut v = vec![0f32; DIM];
        v[a] = cos;
        v[b] = (1.0 - cos * cos).sqrt();
        v
    }

    #[test]
    fn a_neighbour_seen_first_when_a_face_is_missed_is_someone_else() {
        // A wide two-shot: A (small) is found once, then missed, while B —
        // a face width to the right, about as alike as strangers' blurry
        // faces get — is found for the first time.
        let shot = FrameRange::new(0, 30);
        let small = |frame, x: f32, embedding| Observation {
            det: FaceDet { frame, bbox: [x, 0.2, x + 0.04, 0.27], score: 0.9, norm: 16.0, quality: 0.1 },
            embedding,
            thumb: vec![],
        };
        let samples = vec![(0, vec![small(0, 0.40, emb(1))]), (3, vec![small(3, 0.45, blend(1, 2, 0.25))])];
        assert_eq!(track_shot(shot, samples, &TrackerParams::new(3)).len(), 2);
        // The same face, moved as far, links.
        let samples = vec![(0, vec![small(0, 0.40, emb(1))]), (3, vec![small(3, 0.45, blend(1, 2, 0.6))])];
        assert_eq!(track_shot(shot, samples, &TrackerParams::new(3)).len(), 1);
        // Seen once, then gone for a sample: too unsure where it went.
        let samples = vec![(0, vec![small(0, 0.40, emb(1))]), (6, vec![small(6, 0.45, blend(1, 2, 0.6))])];
        assert_eq!(track_shot(shot, samples, &TrackerParams::new(3)).len(), 2);
    }

    #[test]
    fn a_far_jump_or_a_different_size_starts_a_new_track() {
        let shot = FrameRange::new(0, 30);
        let samples = vec![
            (0, vec![obs(0, 0.1, 3, 0.1)]),
            // Four box widths away in one sample.
            (3, vec![obs(3, 0.5, 3, 0.1)]),
        ];
        assert_eq!(track_shot(shot, samples, &TrackerParams::new(3)).len(), 2);
        let mut big = obs(3, 0.15, 3, 0.1);
        big.det.bbox = [0.15, 0.2, 0.45, 0.8];
        let samples = vec![(0, vec![obs(0, 0.1, 3, 0.1)]), (3, vec![big])];
        assert_eq!(track_shot(shot, samples, &TrackerParams::new(3)).len(), 2);
    }

    #[test]
    fn a_face_that_returns_rejoins_its_track() {
        let shot = FrameRange::new(100, 400);
        // Seen, gone for 40 samples, back on the other side of the frame.
        let samples = vec![
            (102, vec![obs(102, 0.1, 5, 0.9)]),
            (105, vec![obs(105, 0.1, 5, 0.9)]),
            (225, vec![obs(225, 0.7, 5, 0.9)]),
        ];
        let tracks = track_shot(shot, samples, &TrackerParams::new(3));
        assert_eq!(tracks.len(), 1);
        assert_eq!((tracks[0].start, tracks[0].end, tracks[0].shot), (102, 228, 100));
    }

    #[test]
    fn useless_tracks_are_dropped() {
        let shot = FrameRange::new(0, 9);
        let samples = vec![(0, vec![obs(0, 0.1, 1, 0.01)]), (3, vec![obs(3, 0.6, 2, 0.5)])];
        let tracks = track_shot(shot, samples, &TrackerParams::new(3));
        assert_eq!(tracks.len(), 1);
        // The last sample reaches one stride further, clipped to the shot.
        assert_eq!((tracks[0].start, tracks[0].end), (3, 6));
        assert!((tracks[0].quality - 0.5).abs() < 1e-6);
    }
}
