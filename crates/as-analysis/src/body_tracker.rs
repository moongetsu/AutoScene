//! Follows people through one shot. Like the face tracker, a sighting links
//! to the track whose box — moved on as the person was moving — overlaps
//! its own; two people crossing stay apart because their clothes don't
//! match. Sightings the detector is unsure of only ever continue a track
//! (ByteTrack's second round: a person half hidden for a moment stays the
//! same person), and someone back later in the shot links by appearance
//! alone when no one else there looks like them. Tracks never cross a cut.
//!
//! Appearance is optional: sightings without it link by place and size
//! alone. That is how the face pass tracks (bench/bodies: as clean, with
//! fewer tracks broken where someone turns), so it embeds only a track's
//! clearest sightings, once it knows whose they are.

use as_core::{FrameIdx, FrameRange};
use as_vision::arcface::cosine;
use as_vision::scrfd::iou;

use crate::bodies::{BodyObs, BodyTrack};

#[derive(Clone, Copy, Debug)]
pub struct BodyTrackerParams {
    /// Frames between samples.
    pub stride: u32,
    /// A track keeps linking by overlap for this many frames without a sighting.
    pub max_gap: u32,
    /// Sightings this sure start and continue tracks; those down to `low`
    /// only continue them.
    pub high: f32,
    pub low: f32,
    /// Overlap with where a track was heading that links a sure sighting,
    /// and the larger one an unsure sighting needs.
    pub min_iou: f32,
    pub min_iou_low: f32,
    /// Weight of appearance in a link's score, and the appearance below
    /// which a sighting never links by overlap (two people crossing).
    pub w_reid: f32,
    pub veto_cos: f32,
    /// Someone back later in the shot (gone longer than `max_gap`) links by
    /// appearance alone: this alike, and this much more alike than any other
    /// pairing of the returning sightings and the tracks gone.
    pub far_cos: f32,
    pub far_margin: f32,
    /// A link's sighting is at most this many times taller or shorter.
    pub max_size_ratio: f32,
    /// Tracks need this many sightings, or one at least this sure.
    pub min_hits: usize,
    pub sure: f32,
}

impl BodyTrackerParams {
    pub fn new(stride: u32) -> Self {
        Self {
            stride,
            max_gap: 24,
            high: 0.5,
            low: 0.3,
            min_iou: 0.3,
            min_iou_low: 0.5,
            w_reid: 0.5,
            veto_cos: 0.4,
            far_cos: 0.8,
            far_margin: 0.1,
            max_size_ratio: 1.6,
            min_hits: 2,
            sure: 0.8,
        }
    }
}

struct Open {
    obs: Vec<BodyObs>,
    /// Running sum of the weighted appearances, and its unit mean.
    sum: Vec<f32>,
    mean: Option<Vec<f32>>,
    /// How far the visible box's centre moved per frame lately (`None`: one sighting).
    velocity: Option<(f32, f32)>,
    best: f32,
}

fn center(b: &[f32; 4]) -> (f32, f32) {
    ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)
}

impl Open {
    fn new(o: BodyObs) -> Self {
        let mut t = Self { obs: Vec::new(), sum: Vec::new(), mean: None, velocity: None, best: 0.0 };
        t.push(o);
        t
    }

    fn last(&self) -> &BodyObs {
        self.obs.last().expect("tracks have sightings")
    }

    fn push(&mut self, o: BodyObs) {
        if let Some(prev) = self.obs.last() {
            let frames = o.det.frame.saturating_sub(prev.det.frame).max(1) as f32;
            let ((x0, y0), (x1, y1)) = (center(&prev.det.visible), center(&o.det.visible));
            let v = ((x1 - x0) / frames, (y1 - y0) / frames);
            // Smoothed: boxes of bodies jitter more than faces (arms, turns).
            self.velocity = Some(self.velocity.map_or(v, |(vx, vy)| (0.5 * (vx + v.0), 0.5 * (vy + v.1))));
        }
        if let Some(e) = o.reid.as_ref().filter(|_| o.det.luma >= crate::bodies::MIN_LUMA) {
            if self.sum.is_empty() {
                self.sum = vec![0.0; e.len()];
            }
            let w = o.det.score * (o.det.height() / 0.3).min(1.0);
            for (s, x) in self.sum.iter_mut().zip(e) {
                *s += w * x;
            }
            let n = self.sum.iter().map(|x| x * x).sum::<f32>().sqrt();
            if n > 1e-6 {
                self.mean = Some(self.sum.iter().map(|x| x / n).collect());
            }
        }
        self.best = self.best.max(o.det.score);
        self.obs.push(o);
    }

    /// Where the visible box would be at `frame` if the person kept moving.
    fn predicted(&self, frame: FrameIdx) -> [f32; 4] {
        let last = &self.last().det;
        let frames = frame.saturating_sub(last.frame) as f32;
        let b = last.visible;
        self.velocity
            .map_or(b, |(vx, vy)| [b[0] + vx * frames, b[1] + vy * frames, b[2] + vx * frames, b[3] + vy * frames])
    }

    fn finish(self, shot: FrameRange, stride: u32) -> BodyTrack {
        let start = self.obs[0].det.frame;
        let end = (self.last().det.frame + stride).min(shot.end);
        let (dets, reids) = self.obs.into_iter().map(|o| (o.det, o.reid)).unzip();
        BodyTrack { start, end, shot: shot.start, dets, reids }
    }
}

/// Link the people of one shot (`samples` in frame order) into tracks,
/// ordered by start.
pub fn track_bodies(shot: FrameRange, samples: Vec<(FrameIdx, Vec<BodyObs>)>, p: &BodyTrackerParams) -> Vec<BodyTrack> {
    let mut tracker = BodyTracker::new(shot, *p);
    for (frame, obs) in samples {
        tracker.push(frame, obs);
    }
    tracker.finish().into_iter().map(|(_, t)| t).collect()
}

/// [`track_bodies`] a sample at a time: each sighting's track is known as
/// soon as it is seen.
pub struct BodyTracker {
    shot: FrameRange,
    p: BodyTrackerParams,
    open: Vec<Open>,
}

impl BodyTracker {
    pub fn new(shot: FrameRange, p: BodyTrackerParams) -> Self {
        Self { shot, p, open: Vec::new() }
    }

    /// Link the people of the next sample (frames in order, within the
    /// shot). Returns, per sighting, the track it joined or began — `(track,
    /// its sighting there)`, numbered as [`BodyTracker::finish`] gives them
    /// back — or `None` for an unsure sighting that begins nothing.
    pub fn push(&mut self, frame: FrameIdx, obs: Vec<BodyObs>) -> Vec<Option<(usize, usize)>> {
        let (p, open) = (&self.p, &mut self.open);
        debug_assert!(self.shot.contains(frame));
        debug_assert!(open.iter().all(|t| t.last().det.frame <= frame), "frames in order");
        let mut target: Vec<Option<usize>> = vec![None; obs.len()];
        let mut taken = vec![false; open.len()];
        // Sure sightings first, then the unsure ones for the tracks left.
        for (lo, hi, min_iou) in [(p.high, f32::INFINITY, p.min_iou), (p.low, p.high, p.min_iou_low)] {
            let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
            for (oi, o) in obs.iter().enumerate() {
                if target[oi].is_some() || !(lo..hi).contains(&o.det.score) {
                    continue;
                }
                for (ti, t) in open.iter().enumerate() {
                    let last = &t.last().det;
                    if taken[ti] || frame.saturating_sub(last.frame) > p.max_gap {
                        continue;
                    }
                    let overlap = iou(&o.det.visible, &t.predicted(frame)).max(iou(&o.det.visible, &last.visible));
                    let ratio = o.det.height() / last.height().max(1e-6);
                    if overlap < min_iou || !(1.0 / p.max_size_ratio..=p.max_size_ratio).contains(&ratio) {
                        continue;
                    }
                    let cos = o.reid.as_ref().zip(t.mean.as_ref()).map(|(a, b)| cosine(a, b));
                    if cos.is_some_and(|c| c < p.veto_cos) {
                        continue;
                    }
                    pairs.push((overlap + p.w_reid * cos.unwrap_or(0.5), oi, ti));
                }
            }
            pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (_, oi, ti) in pairs {
                if target[oi].is_none() && !taken[ti] {
                    target[oi] = Some(ti);
                    taken[ti] = true;
                }
            }
        }
        // Someone back after a while (gone longer than overlap reaches):
        // appearance alone, most alike first, and clearly more alike than
        // any other sighting or track in play.
        let back: Vec<(usize, &Vec<f32>)> = obs
            .iter()
            .enumerate()
            .filter(|&(oi, o)| target[oi].is_none() && o.det.score >= p.high)
            .filter_map(|(oi, o)| o.reid.as_ref().map(|e| (oi, e)))
            .collect();
        let gone: Vec<(usize, &Vec<f32>)> = open
            .iter()
            .enumerate()
            .filter(|&(ti, t)| !taken[ti] && frame.saturating_sub(t.last().det.frame) > p.max_gap)
            .filter_map(|(ti, t)| t.mean.as_ref().map(|m| (ti, m)))
            .collect();
        let mut far: Vec<(f32, usize, usize)> =
            back.iter().flat_map(|&(oi, e)| gone.iter().map(move |&(ti, m)| (cosine(e, m), oi, ti))).collect();
        far.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for &(c, oi, ti) in &far {
            if c < p.far_cos {
                break;
            }
            if target[oi].is_some() || taken[ti] {
                continue;
            }
            let rival = far
                .iter()
                .filter(|&&(_, o2, t2)| (o2 == oi) != (t2 == ti))
                .map(|r| r.0)
                .fold(f32::NEG_INFINITY, f32::max);
            if c - rival >= p.far_margin {
                target[oi] = Some(ti);
                taken[ti] = true;
            }
        }
        obs.into_iter()
            .zip(target)
            .map(|(o, t)| match t {
                Some(ti) => {
                    open[ti].push(o);
                    Some((ti, open[ti].obs.len() - 1))
                }
                None if o.det.score >= p.high => {
                    open.push(Open::new(o));
                    Some((open.len() - 1, 0))
                }
                None => None,
            })
            .collect()
    }

    /// The shot's tracks, ordered by start, each with the number
    /// [`BodyTracker::push`] gave it. Tracks of a single unsure glimpse go.
    pub fn finish(self) -> Vec<(usize, BodyTrack)> {
        let (p, shot) = (self.p, self.shot);
        let mut tracks: Vec<(usize, BodyTrack)> = self
            .open
            .into_iter()
            .enumerate()
            .filter(|(_, t)| t.obs.len() >= p.min_hits || t.best >= p.sure)
            .map(|(i, t)| (i, t.finish(shot, p.stride)))
            .collect();
        tracks.sort_by_key(|(_, t)| (t.start, t.end));
        tracks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bodies::BodyDet;

    fn emb(axis: usize) -> Vec<f32> {
        let mut v = vec![0f32; 8];
        v[axis] = 1.0;
        v
    }

    fn obs(frame: FrameIdx, x: f32, who: usize, score: f32) -> BodyObs {
        let b = [x, 0.1, x + 0.2, 0.9];
        BodyObs {
            det: BodyDet { frame, bbox: b, visible: b, score, sharpness: 100.0, luma: 120.0, rival: 0.0 },
            reid: Some(emb(who)),
        }
    }

    fn who(t: &BodyTrack) -> Vec<usize> {
        t.reids.iter().map(|e| e.as_ref().unwrap().iter().position(|&v| v == 1.0).unwrap()).collect()
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "frames in order")]
    fn frames_out_of_order_are_caught() {
        let shot = FrameRange::new(0, 30);
        let samples = vec![(6, vec![obs(6, 0.1, 1, 0.9)]), (3, vec![obs(3, 0.1, 1, 0.9)])];
        track_bodies(shot, samples, &BodyTrackerParams::new(3));
    }

    #[test]
    fn people_crossing_stay_apart() {
        // A walks right, B left; their boxes overlap most at the middle sample.
        let shot = FrameRange::new(0, 30);
        let samples = (0..5u32)
            .map(|i| {
                let f = i * 3;
                (f, vec![obs(f, 0.1 + 0.05 * i as f32, 1, 0.9), obs(f, 0.3 - 0.05 * i as f32, 2, 0.9)])
            })
            .collect();
        let tracks = track_bodies(shot, samples, &BodyTrackerParams::new(3));
        assert_eq!(tracks.len(), 2);
        for t in &tracks {
            let w = who(t);
            assert!(w.iter().all(|&x| x == w[0]), "{w:?}");
            assert_eq!(t.dets.len(), 5);
        }
    }

    #[test]
    fn a_short_gap_is_bridged_and_unsure_sightings_only_continue() {
        let shot = FrameRange::new(0, 100);
        let samples = vec![
            (0, vec![obs(0, 0.2, 1, 0.9)]),
            // Half hidden: the detector is unsure, the track goes on.
            (3, vec![obs(3, 0.21, 1, 0.35)]),
            // Gone for 18 frames, then back where it was.
            (21, vec![obs(21, 0.22, 1, 0.9)]),
            // Someone unsure elsewhere: never a track of their own.
            (24, vec![obs(24, 0.22, 1, 0.9), obs(24, 0.7, 2, 0.4)]),
        ];
        let tracks = track_bodies(shot, samples, &BodyTrackerParams::new(3));
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].dets.iter().map(|d| d.frame).collect::<Vec<_>>(), [0, 3, 21, 24]);
        assert_eq!((tracks[0].start, tracks[0].end), (0, 27));
    }

    #[test]
    fn someone_back_later_rejoins_by_appearance_alone() {
        let shot = FrameRange::new(0, 300);
        let samples = vec![
            (0, vec![obs(0, 0.1, 1, 0.9), obs(0, 0.6, 2, 0.9)]),
            (3, vec![obs(3, 0.1, 1, 0.9), obs(3, 0.6, 2, 0.9)]),
            // A is back on the other side long after: no overlap, but their clothes.
            (150, vec![obs(150, 0.7, 1, 0.9)]),
        ];
        let tracks = track_bodies(shot, samples, &BodyTrackerParams::new(3));
        assert_eq!(tracks.len(), 2);
        let a = tracks.iter().find(|t| t.dets.len() == 3).expect("A's track goes on");
        assert_eq!(who(a), [1, 1, 1]);
        // Not alike enough to anyone: a new person.
        let samples =
            vec![(0, vec![obs(0, 0.1, 1, 0.9)]), (3, vec![obs(3, 0.1, 1, 0.9)]), (150, vec![obs(150, 0.7, 3, 0.9)])];
        assert_eq!(track_bodies(shot, samples, &BodyTrackerParams::new(3)).len(), 2);
    }

    /// A unit vector with cosine `cos` to axis `a`, in the plane of axes `a` and `b`.
    fn blend(a: usize, b: usize, cos: f32) -> Vec<f32> {
        let mut v = vec![0f32; 8];
        v[a] = cos;
        v[b] = (1.0 - cos * cos).sqrt();
        v
    }

    #[test]
    fn the_most_alike_of_two_returning_takes_the_track() {
        // A was seen at the start; much later C (a look-alike, cos 0.85,
        // found first by the detector) and A come back.
        let shot = FrameRange::new(0, 300);
        let mut c = obs(150, 0.1, 0, 0.95);
        c.reid = Some(blend(1, 2, 0.85));
        let samples =
            vec![(0, vec![obs(0, 0.5, 1, 0.9)]), (3, vec![obs(3, 0.5, 1, 0.9)]), (150, vec![c, obs(150, 0.7, 1, 0.9)])];
        let tracks = track_bodies(shot, samples, &BodyTrackerParams::new(3));
        let a = tracks.iter().find(|t| t.dets.len() == 3).expect("A's track goes on");
        assert_eq!(a.dets[2].visible[0], 0.7);
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn a_look_alike_elsewhere_right_after_a_miss_is_someone_else() {
        // A is missed for one sample while a look-alike enters on the other side.
        let shot = FrameRange::new(0, 60);
        let mut b = obs(9, 0.75, 0, 0.9);
        b.reid = Some(blend(1, 2, 0.9));
        let samples = vec![(0, vec![obs(0, 0.05, 1, 0.9)]), (3, vec![obs(3, 0.05, 1, 0.9)]), (9, vec![b])];
        assert_eq!(track_bodies(shot, samples, &BodyTrackerParams::new(3)).len(), 2);
    }

    #[test]
    fn one_unsure_glimpse_is_no_track_and_tracks_end_in_their_shot() {
        let shot = FrameRange::new(100, 110);
        let samples = vec![
            (102, vec![obs(102, 0.1, 1, 0.6)]),
            (105, vec![obs(105, 0.6, 2, 0.85)]),
            (108, vec![obs(108, 0.6, 2, 0.85)]),
        ];
        let tracks = track_bodies(shot, samples, &BodyTrackerParams::new(3));
        assert_eq!(tracks.len(), 1);
        assert_eq!((tracks[0].start, tracks[0].end, tracks[0].shot), (105, 110, 100));
    }
}
