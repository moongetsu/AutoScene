//! Follows faces through one shot. Samples are a few frames apart, so a face
//! barely moves between them: detections link to the track whose last box
//! overlaps theirs — unless their identities clearly disagree (two people
//! crossing). A track that loses its face for a moment (a turn, an
//! occlusion) picks it up again later in the shot when the identity matches
//! well. Tracks never cross a cut.

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
    best_quality: f32,
}

impl Open {
    fn new(o: Observation) -> Self {
        let mut t = Self {
            obs: Vec::new(),
            sum: vec![0.0; DIM],
            mean: vec![0.0; DIM],
            last_frame: 0,
            last_bbox: [0.0; 4],
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
        self.last_frame = o.det.frame;
        self.last_bbox = o.det.bbox;
        self.best_quality = self.best_quality.max(o.det.quality);
        self.obs.push(o);
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
                let overlap = iou(&o.det.bbox, &t.last_bbox);
                let near = gap <= p.max_gap && overlap >= p.min_iou && cos >= p.min_cos_near;
                let far =
                    cos >= p.min_cos_far && o.det.quality >= p.min_quality_far && t.best_quality >= p.min_quality_far;
                if near || far {
                    pairs.push((cos + if gap <= p.max_gap { overlap } else { 0.0 }, oi, ti));
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
