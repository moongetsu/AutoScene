//! The face pass: decode every `stride`-th frame, find faces (a few samples
//! per inference, on a thread of its own so the GPU works while this one
//! crops, embeds and tracks), embed them, follow them through each shot, and
//! hand finished tracks out in chunks — the checkpoints a later run resumes
//! from. The same samples make each shot's sprite.
//!
//! It can look for people too ([`BodyModels`]): every second sample goes on
//! from the face detector to a person detector on a thread of its own, which
//! also cuts out everyone big enough to say how they look. Here they are
//! followed through their shot by place, the clearest few sightings of each
//! embedded (person ReID), and tied to the shot's face tracks once the shot
//! is done ([`crate::link`]). A pass over faces found before
//! ([`Faces::Known`]) looks for people alone.
//!
//! Chunks are made of whole shots (tracks never cross a cut), so a chunk
//! boundary is the start of a shot and a resumed run produces the same
//! tracks as an uninterrupted one. Samples sit on absolute multiples of the
//! stride for the same reason.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};

use as_core::{CancelToken, FrameIdx, FrameRange};
use as_media::{MediaInfo, Source, Tools};
use as_vision::arcface::{self, ArcFace};
use as_vision::{FaceBox, PersonBox, PersonReid, PpDet, Scrfd, image, reid};

use crate::AnalysisError;
use crate::bodies::{self, BodyDet, BodyObs, BodyTrack, FoundBody};
use crate::body_tracker::{BodyTracker, BodyTrackerParams};
use crate::link::{LinkParams, link_shot};
use crate::sprites::{ShotSprite, SpriteCanvas};
use crate::tracker::{Observation, TrackerParams, track_shot};
use crate::tracks::{self, FaceDet, FaceTrack, KnownFace};

#[derive(Clone, Copy, Debug)]
pub struct FaceParams {
    /// Frames between samples.
    pub stride: u32,
    /// Frames are decoded to fit this box (aligned crops and thumbnails come
    /// from it; detection runs at half size for frames wider than 960, see
    /// [`frame_sizes`]).
    pub decode_max: (u32, u32),
    pub det_threshold: f32,
    /// Faces smaller than this (fraction of the frame height) are ignored.
    pub min_height: f32,
    /// Thumbnail side and how much context around the face box it shows.
    pub thumb_size: usize,
    pub thumb_context: f32,
    /// Embedding norms from "says nothing" to "clear frontal face".
    pub norm_floor: f32,
    pub norm_full: f32,
    /// Face height (fraction of the frame) from which size stops limiting quality.
    pub full_height: f32,
    /// Tracks whose clearest face is below this quality say nothing about
    /// anyone and are dropped (the tracker's `min_track_quality`).
    pub min_track_quality: f32,
    /// Shot sprites: frames per shot, cell width, JPEG quality.
    pub sprite_cells: u32,
    pub sprite_width: usize,
    pub sprite_quality: u8,
}

impl Default for FaceParams {
    fn default() -> Self {
        Self {
            stride: 3,
            decode_max: (1280, 720),
            det_threshold: 0.5,
            min_height: 0.04,
            thumb_size: 160,
            thumb_context: 1.7,
            norm_floor: 14.0,
            norm_full: 22.0,
            full_height: 0.12,
            min_track_quality: 0.05,
            sprite_cells: 8,
            sprite_width: 240,
            sprite_quality: 72,
        }
    }
}

/// Samples per face-detector inference: enough to keep the GPU busy, few
/// enough that a cut never waits long (RTX 3060 Ti, 640×360: 2.2 ms per
/// frame at 4, 2.6 at 8, 3.2 at 1).
pub const DETECT_BATCH: usize = 4;

/// The person detector's input, width × height: the frame's shape kept and
/// padded (bench/bodies: as many people found as stretched to 640×640, in
/// two thirds of the time).
pub const PERSON_INPUT: (usize, usize) = (640, 384);

/// How the face pass looks for people (set on bench/bodies).
#[derive(Clone, Copy, Debug)]
pub struct PeopleParams {
    /// Frames between the samples looked at, a multiple of the face pass's
    /// stride: every second sample finds as much as every one.
    pub stride: u32,
    /// Sightings down to this detector score are kept; the unsure ones only
    /// ever continue a track.
    pub min_score: f32,
    /// People at least this tall (share of the frame height) are cut out
    /// for their appearance ...
    pub embed_height: f32,
    /// ... and the clearest this many sightings of each track embedded
    /// (8 finds a tenth fewer people again, 32 as many as all of them).
    pub embed_per_track: usize,
}

impl Default for PeopleParams {
    fn default() -> Self {
        Self { stride: 6, min_score: 0.3, embed_height: 0.08, embed_per_track: 32 }
    }
}

impl PeopleParams {
    /// Samples a body may go unseen and still count as on screen (as far
    /// as its tracker links by place: 24 frames).
    pub fn span_gap(&self) -> u32 {
        (24 / self.stride.max(1)).max(1)
    }
}

/// The models a pass finds people with.
pub struct BodyModels<'a> {
    /// Planned for the decoded frame size of [`frame_sizes`], with input
    /// [`PERSON_INPUT`].
    pub detector: &'a mut PpDet,
    pub reid: &'a mut PersonReid,
    pub params: PeopleParams,
}

/// Where a pass's faces come from.
pub enum Faces<'a> {
    /// Found by the pass: the detector takes frames of the detection size
    /// of [`frame_sizes`].
    Find { detector: &'a mut Scrfd, embedder: &'a mut ArcFace },
    /// Found before: the pass only looks for people (with [`BodyModels`])
    /// and ties them to these.
    Known(&'a [KnownFace]),
}

/// The size a media's samples are decoded at, and the size faces are
/// detected at (half of it for frames wider than 960) — the frame size the
/// detector given to [`face_tracks`] must be loaded for.
pub fn frame_sizes(info: &MediaInfo, params: &FaceParams) -> ((usize, usize), (usize, usize)) {
    let (w, h) = as_media::frames::sample_size(info, params.decode_max, 4);
    let (w, h) = (w as usize, h as usize);
    ((w, h), if w > 960 { (w / 2, h / 2) } else { (w, h) })
}

impl FaceParams {
    /// How much a detection says about identity, 0..=1.
    pub fn quality(&self, norm: f32, height: f32) -> f32 {
        let n = ((norm - self.norm_floor) / (self.norm_full - self.norm_floor)).clamp(0.0, 1.0);
        let s = (height / self.full_height).clamp(0.0, 1.0).sqrt();
        n * s
    }
}

/// A face found in a sample, waiting for its embedding.
struct Found {
    det: FaceDet,
    crop: Vec<u8>,
    thumb: Vec<u8>,
    embedding: Option<Vec<f32>>,
}

/// A person found in a sample: where, and — when they are big enough to
/// say how they look — their crop, how sharp and bright it is.
struct Person {
    found: PersonBox,
    crop: Option<Vec<u8>>,
    sharpness: f32,
    luma: f32,
    /// The sharpness of the sharpest other person there.
    rival: f32,
}

/// The people of one shot, followed as they come.
struct ShotPeople {
    tracker: BodyTracker,
    /// Per track (as the tracker numbers them), the sightings picked for its
    /// appearance with their weight: the clearest few so far.
    picks: Vec<Vec<(usize, f32)>>,
    /// The appearance of each picked sighting embedded so far.
    reids: HashMap<(usize, usize), Vec<f32>>,
}

/// Samples of one shot.
struct ShotWork {
    range: FrameRange,
    samples: Vec<(FrameIdx, Vec<Found>)>,
    sprite: Option<SpriteCanvas>,
    people: Option<ShotPeople>,
}

/// A finished checkpoint chunk of the face pass: frames `start..end` (whole
/// shots), their tracks, the sprites of their shots and the people found.
#[derive(Clone, Copy, Debug)]
pub struct FaceChunk<'a> {
    pub start: FrameIdx,
    pub end: FrameIdx,
    /// Empty when the faces were known.
    pub tracks: &'a [FaceTrack],
    pub sprites: &'a [ShotSprite],
    /// People found, tied to faces: [`FoundBody::faces`] names a face by
    /// its place in `tracks` — or, when the faces were known, by its id.
    pub bodies: &'a [FoundBody],
}

/// A sample in a shot, on its way through the detectors.
struct Queued {
    frame: FrameIdx,
    rgb: Vec<u8>,
}

/// A batch of samples on its way through the detectors, with the faces and
/// the people found in each (`None`: no one was looked for there).
#[derive(Default)]
struct Batch {
    samples: Vec<Queued>,
    faces: Vec<Vec<FaceBox>>,
    people: Vec<Option<Vec<Person>>>,
}

type Detected = Result<Batch, AnalysisError>;

/// Where a detector thread hands its batches on: the next detector (which
/// takes one at a time, so none runs ahead of another) or this thread.
trait Next: Clone + Send {
    fn put(&self, d: Detected) -> bool;
}

impl Next for Sender<Detected> {
    fn put(&self, d: Detected) -> bool {
        self.send(d).is_ok()
    }
}

impl Next for SyncSender<Detected> {
    fn put(&self, d: Detected) -> bool {
        self.send(d).is_ok()
    }
}

/// A detector thread: batches in, batches out, in order, `work` done to
/// each. Stops after the first error, which goes on; a panic becomes one,
/// so the pass fails instead of the engine.
fn stage(
    what: &str,
    input: Receiver<Detected>,
    out: impl Next,
    mut work: impl FnMut(&mut Batch) -> Result<(), AnalysisError>,
) {
    let errors = out.clone();
    let run = std::panic::AssertUnwindSafe(|| {
        for batch in input {
            let next = batch.and_then(|mut b| work(&mut b).map(|()| b));
            let failed = next.is_err();
            if !out.put(next) || failed {
                return;
            }
        }
    });
    if let Err(panic) = std::panic::catch_unwind(run) {
        let why = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        errors.put(Err(AnalysisError::Internal(format!("the {what} failed: {why}"))));
    }
}

/// The faces of a batch's samples. `halve`: detect on half-size copies
/// (boxes are scaled back).
fn find_faces(
    detector: &mut Scrfd,
    (w, h): (usize, usize),
    halve: bool,
    threshold: f32,
    batch: &mut Batch,
) -> Result<(), AnalysisError> {
    batch.faces = if halve {
        let small: Vec<Vec<u8>> = batch.samples.iter().map(|q| image::half(&q.rgb, w, h)).collect();
        let frames: Vec<&[u8]> = small.iter().map(Vec::as_slice).collect();
        let found = detector.detect_many(&frames, threshold)?;
        found.into_iter().map(|faces| faces.into_iter().map(|b| b.scaled(2.0)).collect()).collect()
    } else {
        let frames: Vec<&[u8]> = batch.samples.iter().map(|q| q.rgb.as_slice()).collect();
        detector.detect_many(&frames, threshold)?
    };
    Ok(())
}

/// The people of a batch's samples on the people grid.
fn find_people(
    detector: &mut PpDet,
    (w, h): (usize, usize),
    p: &PeopleParams,
    batch: &mut Batch,
) -> Result<(), AnalysisError> {
    batch.people = Vec::with_capacity(batch.samples.len());
    for q in &batch.samples {
        if !q.frame.is_multiple_of(p.stride) {
            batch.people.push(None);
            continue;
        }
        let mut people: Vec<Person> = detector
            .detect(&q.rgb, p.min_score)?
            .into_iter()
            .map(|found| {
                let crop = (found.height() >= p.embed_height * h as f32)
                    .then(|| PersonReid::crop(&q.rgb, (w, h), found.visible));
                let (sharpness, luma) = crop
                    .as_deref()
                    .map_or((0.0, 0.0), |c| (image::sharpness(c, reid::WIDTH, reid::HEIGHT), image::mean_luma(c)));
                Person { found, crop, sharpness, luma, rival: 0.0 }
            })
            .collect();
        for i in 0..people.len() {
            people[i].rival =
                people.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, o)| o.sharpness).fold(0.0, f32::max);
        }
        batch.people.push(Some(people));
    }
    Ok(())
}

/// Embed every face of `shots` still waiting (the last batch padded).
fn embed(shots: &mut [&mut ShotWork], embedder: &mut ArcFace, params: &FaceParams) -> Result<(), AnalysisError> {
    let mut slots: Vec<&mut Found> = shots
        .iter_mut()
        .flat_map(|s| s.samples.iter_mut().flat_map(|(_, f)| f.iter_mut()))
        .filter(|f| f.embedding.is_none())
        .collect();
    if slots.is_empty() {
        return Ok(());
    }
    let crops: Vec<&[u8]> = slots.iter().map(|f| f.crop.as_slice()).collect();
    let embs = embedder.embed(&crops)?;
    for (f, e) in slots.iter_mut().zip(embs) {
        f.det.norm = e.norm;
        f.det.quality = params.quality(e.norm, f.det.height());
        f.embedding = Some(e.v);
        f.crop = Vec::new();
    }
    Ok(())
}

/// Track the faces of an embedded shot.
fn track(shot: &mut ShotWork, tracker: &TrackerParams) -> Vec<FaceTrack> {
    let samples = std::mem::take(&mut shot.samples)
        .into_iter()
        .map(|(f, found)| {
            let obs = found
                .into_iter()
                .map(|x| Observation {
                    det: x.det,
                    embedding: x.embedding.expect("embedded before tracking"),
                    thumb: x.thumb,
                })
                .collect();
            (f, obs)
        })
        .collect();
    track_shot(shot.range, samples, tracker)
}

/// A crop waiting for the person ReID, and whose sighting it is.
struct Pending {
    shot: FrameIdx,
    track: usize,
    det: usize,
    crop: Vec<u8>,
}

/// The people part of a pass.
struct PeoplePass<'a> {
    reid: &'a mut PersonReid,
    params: PeopleParams,
    tracker: BodyTrackerParams,
    link: LinkParams,
    /// The face samples' stride and the gap of samples a face may be
    /// missed and still be on screen (when a body's face shows).
    face_stride: u32,
    face_gap: u32,
    /// Width ÷ height of the frame, in pixels.
    aspect: f32,
    pending: Vec<Pending>,
}

impl PeoplePass<'_> {
    /// Follow the people of sample `f` of shot `cur` (boxes in pixels of a
    /// `w`×`h` frame), and queue each track's clearest sightings for their
    /// appearance — a clearer one taking the place of the weakest so far.
    fn take(&mut self, cur: &mut ShotWork, f: FrameIdx, people: Vec<Person>, (w, h): (usize, usize)) {
        let (shot, tp) = (cur.range, self.tracker);
        let mine = cur.people.get_or_insert_with(|| ShotPeople {
            tracker: BodyTracker::new(shot, tp),
            picks: Vec::new(),
            reids: HashMap::new(),
        });
        let frac = |b: [f32; 4]| [b[0] / w as f32, b[1] / h as f32, b[2] / w as f32, b[3] / h as f32];
        let (dets, crops): (Vec<BodyDet>, Vec<Option<Vec<u8>>>) = people
            .into_iter()
            .map(|p| {
                let det = BodyDet {
                    frame: f,
                    bbox: frac(p.found.bbox),
                    visible: frac(p.found.visible),
                    score: p.found.score,
                    sharpness: p.sharpness,
                    luma: p.luma,
                    rival: p.rival,
                };
                (det, p.crop)
            })
            .unzip();
        let placed = mine.tracker.push(f, dets.iter().map(|&det| BodyObs { det, reid: None }).collect());
        for ((place, crop), d) in placed.into_iter().zip(crops).zip(&dets) {
            let (Some((track, det)), Some(crop)) = (place, crop) else { continue };
            if d.luma < bodies::MIN_LUMA {
                continue;
            }
            if mine.picks.len() <= track {
                mine.picks.resize_with(track + 1, Vec::new);
            }
            let (picks, weight) = (&mut mine.picks[track], d.weight());
            if picks.len() >= self.params.embed_per_track {
                let (i, &(weakest, least)) =
                    picks.iter().enumerate().min_by(|a, b| a.1.1.total_cmp(&b.1.1)).expect("a full list");
                if weight <= least {
                    continue;
                }
                picks.swap_remove(i);
                mine.reids.remove(&(track, weakest));
                self.pending.retain(|p| (p.shot, p.track, p.det) != (shot.start, track, weakest));
            }
            picks.push((det, weight));
            self.pending.push(Pending { shot: shot.start, track, det, crop });
        }
    }

    /// The bodies of a finished shot (every crop embedded), tied to its
    /// face tracks; `names` says what each face is called in the chunk. A
    /// body tied to no face that has no appearance either says nothing
    /// about anyone and stays out.
    fn finish(&self, people: ShotPeople, faces: &[FaceTrack], names: &[u32]) -> Vec<FoundBody> {
        let ShotPeople { tracker, mut reids, .. } = people;
        let tracks: Vec<BodyTrack> = tracker
            .finish()
            .into_iter()
            .map(|(n, mut t)| {
                for (i, e) in t.reids.iter_mut().enumerate() {
                    *e = reids.remove(&(n, i));
                }
                t
            })
            .collect();
        let links = {
            let fs: Vec<&FaceTrack> = faces.iter().collect();
            let bs: Vec<&BodyTrack> = tracks.iter().collect();
            link_shot(&fs, &bs, &self.link)
        };
        let mut tied: Vec<Vec<(u32, u32)>> = vec![Vec::new(); tracks.len()];
        // When each body's faces show.
        let mut shown: Vec<Vec<(FrameIdx, FrameIdx)>> = vec![Vec::new(); tracks.len()];
        for l in &links {
            let face = &faces[l.face];
            tied[l.body].push((names[l.face], l.samples));
            shown[l.body].extend(tracks::presence_spans(&face.dets, face.end, self.face_stride, self.face_gap));
        }
        tracks
            .into_iter()
            .zip(tied)
            .zip(shown)
            .filter_map(|((t, faces), shown)| {
                let reid = t.appearance(|_| true);
                if faces.is_empty() && reid.is_none() {
                    return None;
                }
                let reid_away = t.appearance(|d| !shown.iter().any(|&(a, e)| a <= d.frame && d.frame < e));
                Some(FoundBody {
                    start: t.start,
                    end: t.end,
                    shot: t.shot,
                    showing: t.showing(self.aspect),
                    dets: t.dets,
                    faces,
                    reid,
                    reid_away,
                })
            })
            .collect()
    }
}

/// One face pass in progress: samples in shots go to the detector threads
/// in batches and come back — in frame order — into shots and chunks.
struct Pass<'a, C> {
    params: &'a FaceParams,
    tracker: TrackerParams,
    /// `None` when the faces are known.
    embedder: Option<&'a mut ArcFace>,
    /// Faces found before, by the shot they are in.
    known: HashMap<FrameIdx, Vec<&'a KnownFace>>,
    people: Option<PeoplePass<'a>>,
    /// Shots from the resume point on.
    shots: &'a [FrameRange],
    /// Decoded frame size.
    size: (usize, usize),
    /// Samples per detector batch.
    batch: usize,
    queue: Vec<Queued>,
    /// `None` once every batch is sent.
    to_detector: Option<SyncSender<Detected>>,
    from_detector: Receiver<Detected>,
    /// Batches sent and not yet taken back.
    in_flight: usize,
    /// Frame buffers of detected samples, for reuse.
    spare: Vec<Vec<u8>>,
    /// First shot not yet started.
    next_shot: usize,
    current: Option<ShotWork>,
    done_shots: Vec<ShotWork>,
    pending_crops: usize,
    chunk: u32,
    chunk_start: FrameIdx,
    /// The chunk closes at the first shot starting at or after this frame.
    chunk_goal: FrameIdx,
    tracks_in_chunk: Vec<FaceTrack>,
    sprites_in_chunk: Vec<ShotSprite>,
    bodies_in_chunk: Vec<FoundBody>,
    on_chunk: C,
}

impl<C: FnMut(FaceChunk) -> Result<(), AnalysisError>> Pass<'_, C> {
    /// Whether frame `f` lies in a shot; only those are searched for faces.
    fn in_shot(&self, f: FrameIdx) -> bool {
        let i = self.shots.partition_point(|s| s.end <= f);
        self.shots.get(i).is_some_and(|s| s.contains(f))
    }

    /// Queue a sample in a shot; a full batch goes to the detector. Takes in
    /// whatever the detector has finished meanwhile.
    fn push(&mut self, f: FrameIdx, rgb: &[u8]) -> Result<(), AnalysisError> {
        let mut frame = self.spare.pop().unwrap_or_default();
        frame.clear();
        frame.extend_from_slice(rgb);
        self.queue.push(Queued { frame: f, rgb: frame });
        if self.queue.len() >= self.batch {
            self.send()?;
        }
        self.collect(false)
    }

    /// Hand the queued samples to the detector (waits while it is a batch
    /// behind).
    fn send(&mut self) -> Result<(), AnalysisError> {
        if self.queue.is_empty() {
            return Ok(());
        }
        let samples = std::mem::replace(&mut self.queue, Vec::with_capacity(self.batch));
        let batch = Batch { samples, ..Batch::default() };
        if self.to_detector.as_ref().is_some_and(|tx| tx.send(Ok(batch)).is_ok()) {
            self.in_flight += 1;
            return Ok(());
        }
        // The detector stopped: its error is waiting.
        self.collect(true)?;
        Err(AnalysisError::Internal("the face detector stopped".into()))
    }

    /// Take in the batches the detector has finished, in order — all of
    /// them when `wait`.
    fn collect(&mut self, wait: bool) -> Result<(), AnalysisError> {
        while self.in_flight > 0 {
            let next = match self.from_detector.try_recv() {
                Ok(d) => d,
                Err(TryRecvError::Empty) if !wait => return Ok(()),
                Err(TryRecvError::Empty) => self
                    .from_detector
                    .recv()
                    .map_err(|_| AnalysisError::Internal("the face detector stopped".into()))?,
                Err(TryRecvError::Disconnected) => {
                    return Err(AnalysisError::Internal("the face detector stopped".into()));
                }
            };
            self.in_flight -= 1;
            let Batch { samples, faces, mut people } = next?;
            let mut faces = faces.into_iter();
            for (i, q) in samples.iter().enumerate() {
                let found = people.get_mut(i).and_then(Option::take);
                self.take(q.frame, &q.rgb, faces.next().unwrap_or_default(), found)?;
            }
            self.spare.extend(samples.into_iter().map(|q| q.rgb));
        }
        Ok(())
    }

    /// Take in the faces and people found in sample `f`: open and close
    /// shots and chunks up to it, keep its faces' crops and thumbnails and
    /// follow its people, and embed and track as they pile up.
    fn take(
        &mut self,
        f: FrameIdx,
        rgb: &[u8],
        faces: Vec<FaceBox>,
        people: Option<Vec<Person>>,
    ) -> Result<(), AnalysisError> {
        // Shots that started up to this frame: close the current one when it ended.
        if self.current.as_ref().is_some_and(|c| f >= c.range.end) {
            self.done_shots.push(self.current.take().expect("checked"));
        }
        // A chunk closes at the first shot start at or past its goal.
        while self.next_shot < self.shots.len() && self.shots[self.next_shot].start <= f {
            let s = self.shots[self.next_shot];
            if let Some(c) = self.current.take() {
                self.done_shots.push(c);
            }
            if s.start >= self.chunk_goal {
                self.close_chunk(s.start)?;
            }
            if s.end > f {
                let p = self.params;
                // Sprites come with faces found now (known ones have theirs).
                let sprite = self
                    .embedder
                    .is_some()
                    .then(|| SpriteCanvas::new(s, p.stride, p.sprite_cells, self.size, p.sprite_width))
                    .flatten();
                self.current = Some(ShotWork { range: s, samples: Vec::new(), sprite, people: None });
            }
            self.next_shot += 1;
        }
        let Some(cur) = self.current.as_mut().filter(|c| c.range.contains(f)) else {
            return Ok(());
        };
        let (w, h) = self.size;
        let p = self.params;
        if let Some(sprite) = cur.sprite.as_mut() {
            sprite.paint(f, rgb, self.size);
        }
        if self.embedder.is_some() {
            let mut found = Vec::new();
            for b in faces {
                let height = b.height() / h as f32;
                if height < p.min_height {
                    continue;
                }
                let thumb = image::face_thumb(rgb, w, h, b.bbox, p.thumb_context, p.thumb_size);
                found.push(Found {
                    det: FaceDet {
                        frame: f,
                        bbox: [b.bbox[0] / w as f32, b.bbox[1] / h as f32, b.bbox[2] / w as f32, b.bbox[3] / h as f32],
                        score: b.score,
                        norm: 0.0,
                        quality: 0.0,
                    },
                    crop: arcface::align(rgb, w, h, &b.kps),
                    thumb: image::jpeg(&thumb, p.thumb_size, p.thumb_size, 85),
                    embedding: None,
                });
            }
            self.pending_crops += found.len();
            cur.samples.push((f, found));
        }
        if let (Some(pp), Some(people)) = (self.people.as_mut(), people) {
            pp.take(cur, f, people, self.size);
        }
        // Embed in full batches as faces pile up; track finished shots.
        let faces_due = self.embedder.as_ref().is_some_and(|e| self.pending_crops >= e.batch() * 4);
        let people_due = self.people.as_ref().is_some_and(|pp| pp.pending.len() >= pp.reid.batch() * 4);
        if faces_due {
            self.embed_faces(true)?;
            self.embed_people()?;
            self.finish_shots();
        } else if people_due {
            self.embed_people()?;
            // Finished shots without faces to wait for can go.
            if self.embedder.is_none() {
                self.finish_shots();
            }
        }
        Ok(())
    }

    /// Embed the faces waiting in finished shots, and in the current one too
    /// with `current`.
    fn embed_faces(&mut self, current: bool) -> Result<(), AnalysisError> {
        let Some(embedder) = self.embedder.as_deref_mut() else { return Ok(()) };
        let mut refs: Vec<&mut ShotWork> =
            self.done_shots.iter_mut().chain(self.current.iter_mut().filter(|_| current)).collect();
        embed(&mut refs, embedder, self.params)?;
        self.pending_crops = 0;
        Ok(())
    }

    /// Embed every person crop waiting (the last batch padded).
    fn embed_people(&mut self) -> Result<(), AnalysisError> {
        let Some(pp) = self.people.as_mut() else { return Ok(()) };
        if pp.pending.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut pp.pending);
        let crops: Vec<&[u8]> = pending.iter().map(|x| x.crop.as_slice()).collect();
        let embs = pp.reid.embed(&crops)?;
        let mut shots: Vec<&mut ShotWork> = self.done_shots.iter_mut().chain(self.current.iter_mut()).collect();
        for (x, e) in pending.iter().zip(embs) {
            let at = shots.iter_mut().find(|s| s.range.start == x.shot).and_then(|s| s.people.as_mut());
            if let Some(people) = at {
                people.reids.insert((x.track, x.det), e.v);
            }
        }
        Ok(())
    }

    /// Track the faces of the finished (embedded) shots, finish their
    /// sprites, and tie their people to their faces.
    fn finish_shots(&mut self) {
        for mut s in std::mem::take(&mut self.done_shots) {
            if let Some(sprite) = s.sprite.take() {
                self.sprites_in_chunk.push(sprite.finish(self.params.sprite_quality));
            }
            let people = s.people.take();
            let (faces, names): (Vec<FaceTrack>, Vec<u32>) = if self.embedder.is_some() {
                let found = track(&mut s, &self.tracker);
                let first = self.tracks_in_chunk.len();
                let names = (first..first + found.len()).map(|i| i as u32).collect();
                (found, names)
            } else {
                let known = self.known.get(&s.range.start).map_or(&[][..], Vec::as_slice);
                (known.iter().map(|k| k.track()).collect(), known.iter().map(|k| k.id).collect())
            };
            if let (Some(pp), Some(people)) = (self.people.as_ref(), people) {
                self.bodies_in_chunk.extend(pp.finish(people, &faces, &names));
            }
            if self.embedder.is_some() {
                self.tracks_in_chunk.extend(faces);
            }
        }
    }

    /// Hand out the chunk that ends at `end` (a shot start, or the end):
    /// every shot before it is finished.
    fn close_chunk(&mut self, end: FrameIdx) -> Result<(), AnalysisError> {
        self.embed_faces(false)?;
        self.embed_people()?;
        self.finish_shots();
        (self.on_chunk)(FaceChunk {
            start: self.chunk_start,
            end,
            tracks: &self.tracks_in_chunk,
            sprites: &self.sprites_in_chunk,
            bodies: &self.bodies_in_chunk,
        })?;
        self.tracks_in_chunk.clear();
        self.sprites_in_chunk.clear();
        self.bodies_in_chunk.clear();
        self.chunk_start = end;
        self.chunk_goal = end + self.chunk;
        Ok(())
    }

    /// Every sample is decoded: detect the rest and hand out the last chunk.
    fn end(mut self, n: FrameIdx) -> Result<(), AnalysisError> {
        self.send()?;
        // The detector ends after the last batch.
        self.to_detector = None;
        self.collect(true)?;
        if let Some(c) = self.current.take() {
            self.done_shots.push(c);
        }
        self.close_chunk(n)
    }
}

/// Find, embed and track the faces of `src` in `shots` (sorted, disjoint)
/// from frame `resume` on — which must be 0 or a chunk end handed out
/// earlier — and with `bodies` the people tied to them; or, with known
/// faces, the people alone. Each finished chunk goes to `on_chunk`; chunks
/// tile `resume..index.len()` and end at shot starts at least `chunk`
/// frames after their own start (or at the end). `progress` gets 0..1.
#[allow(clippy::too_many_arguments)]
pub fn face_tracks(
    tools: &Tools,
    src: Source,
    faces: Faces,
    bodies: Option<BodyModels>,
    shots: &[FrameRange],
    resume: FrameIdx,
    chunk: u32,
    params: &FaceParams,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
    on_chunk: impl FnMut(FaceChunk) -> Result<(), AnalysisError>,
) -> Result<(), AnalysisError> {
    assert!(chunk > 0 && params.stride > 0);
    let n = src.index.len();
    if resume >= n {
        return Ok(());
    }
    let (size, det_size) = frame_sizes(src.info, params);
    if let Faces::Find { detector, .. } = &faces
        && detector.frame_size() != det_size
    {
        return Err(AnalysisError::Internal(format!(
            "face detector loaded for {:?} frames, this media needs {det_size:?}",
            detector.frame_size()
        )));
    }
    if let Some(b) = &bodies {
        if b.detector.frame_size() != size {
            return Err(AnalysisError::Internal(format!(
                "person detector loaded for {:?} frames, this media needs {size:?}",
                b.detector.frame_size()
            )));
        }
        if b.params.stride == 0 || !b.params.stride.is_multiple_of(params.stride) {
            return Err(AnalysisError::Internal(format!(
                "people every {} frames aren't on the face samples' grid of {}",
                b.params.stride, params.stride
            )));
        }
    }
    let (embedder, known, face_detector) = match faces {
        Faces::Find { detector, embedder } => (Some(embedder), &[][..], Some(detector)),
        Faces::Known(k) if bodies.is_some() => (None, k, None),
        // Nothing to look for.
        Faces::Known(_) => return Ok(()),
    };
    // Faces found before only need the samples people are looked for in.
    let stride = match (&face_detector, &bodies) {
        (None, Some(b)) => b.params.stride,
        _ => params.stride,
    };

    // Shots from the resume point on; samples outside them are skipped.
    let shots = &shots[shots.partition_point(|s| s.end <= resume)..];
    debug_assert!(shots.first().is_none_or(|s| s.start >= resume), "resume points are shot starts");
    let batch = face_detector.as_ref().map_or(DETECT_BATCH, |d| d.batch());
    std::thread::scope(|scope| {
        // One batch waits while a detector works on the next. Faces first,
        // then people, then back here.
        let (to_detector, first) = mpsc::sync_channel::<Detected>(1);
        let (detected, from_detector) = mpsc::channel::<Detected>();
        let (halve, threshold) = (det_size != size, params.det_threshold);
        let starting = |what: &str, e: std::io::Error| AnalysisError::Internal(format!("starting the {what}: {e}"));
        let (reid, people_detector, people_params) = match bodies {
            Some(BodyModels { detector, reid, params }) => (Some(reid), Some(detector), params),
            None => (None, None, PeopleParams::default()),
        };
        // What the person detector takes its batches from, when it runs.
        let into_people: Option<Receiver<Detected>> = match face_detector {
            Some(d) => {
                let work = move |b: &mut Batch| find_faces(d, size, halve, threshold, b);
                let thread = std::thread::Builder::new().name("face detector".into());
                if people_detector.is_some() {
                    let (out, rx) = mpsc::sync_channel::<Detected>(1);
                    thread
                        .spawn_scoped(scope, move || stage("face detector", first, out, work))
                        .map_err(|e| starting("face detector", e))?;
                    Some(rx)
                } else {
                    let out = detected.clone();
                    thread
                        .spawn_scoped(scope, move || stage("face detector", first, out, work))
                        .map_err(|e| starting("face detector", e))?;
                    None
                }
            }
            None => Some(first),
        };
        if let (Some(d), Some(input)) = (people_detector, into_people) {
            let out = detected.clone();
            std::thread::Builder::new()
                .name("person detector".into())
                .spawn_scoped(scope, move || {
                    stage("person detector", input, out, |b| find_people(d, size, &people_params, b))
                })
                .map_err(|e| starting("person detector", e))?;
        }
        drop(detected);
        let mut by_shot: HashMap<FrameIdx, Vec<&KnownFace>> = HashMap::new();
        for k in known {
            by_shot.entry(k.shot).or_default().push(k);
        }
        let tracker =
            TrackerParams { min_track_quality: params.min_track_quality, ..TrackerParams::new(params.stride) };
        let people = reid.map(|reid| PeoplePass {
            reid,
            params: people_params,
            tracker: BodyTrackerParams::new(people_params.stride),
            link: LinkParams { stride: params.stride, ..LinkParams::default() },
            face_stride: params.stride,
            face_gap: tracker.max_gap,
            aspect: size.0 as f32 / size.1 as f32,
            pending: Vec::new(),
        });
        let mut pass = Pass {
            params,
            tracker,
            embedder,
            known: by_shot,
            people,
            shots,
            size,
            batch,
            queue: Vec::with_capacity(batch),
            to_detector: Some(to_detector),
            from_detector,
            in_flight: 0,
            spare: Vec::new(),
            next_shot: 0,
            current: None,
            done_shots: Vec::new(),
            pending_crops: 0,
            chunk,
            chunk_start: resume,
            chunk_goal: resume + chunk,
            tracks_in_chunk: Vec::new(),
            sprites_in_chunk: Vec::new(),
            bodies_in_chunk: Vec::new(),
            on_chunk,
        };
        let mut failure: Option<AnalysisError> = None;
        let first = resume - resume % stride;
        let (w, h) = size;
        let delivered =
            as_media::frames::decode_samples(tools, src, first, stride, (w as u32, h as u32), cancel, |f, rgb| {
                if f < resume {
                    return Ok(true);
                }
                if f % (stride * 80) == 0 {
                    progress(f as f64 / n as f64);
                }
                if !pass.in_shot(f) {
                    return Ok(true);
                }
                match pass.push(f, rgb) {
                    Ok(()) => Ok(true),
                    Err(e) => {
                        failure = Some(e);
                        Ok(false)
                    }
                }
            });
        if let Some(e) = failure {
            return Err(e);
        }
        let delivered = delivered?;
        let expected = (n - first).div_ceil(stride);
        if delivered != expected {
            return Err(AnalysisError::Media(as_media::MediaError::Verify(format!(
                "face pass decoded {delivered} samples from {first}, expected {expected}"
            ))));
        }
        pass.end(n)?;
        progress(1.0);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_frames_are_searched_at_half_size() {
        use as_core::Rational;
        use as_media::{HdrKind, VideoStream};
        let info = |width, height| MediaInfo {
            container: "matroska,webm".into(),
            duration_sec: Some(10.0),
            start_sec: 0.0,
            size_bytes: None,
            bit_rate: None,
            video: VideoStream {
                index: 0,
                codec: "h264".into(),
                profile: None,
                pix_fmt: "yuv420p".into(),
                bit_depth: 8,
                width,
                height,
                sar: Rational::new(1, 1),
                r_frame_rate: Rational::new(24, 1),
                avg_frame_rate: None,
                time_base: Rational::new(1, 1000),
                color_primaries: None,
                color_transfer: None,
                color_space: None,
                color_range: None,
                field_order: None,
                rotation: 0,
                hdr: HdrKind::None,
            },
            audio: vec![],
            subtitles: vec![],
        };
        let p = FaceParams::default();
        assert_eq!(frame_sizes(&info(1920, 1080), &p), ((1280, 720), (640, 360)));
        assert_eq!(frame_sizes(&info(720, 480), &p), ((720, 480), (720, 480)));
    }

    #[test]
    fn quality_follows_norm_and_size() {
        let p = FaceParams::default();
        assert_eq!(p.quality(10.0, 0.3), 0.0);
        assert_eq!(p.quality(30.0, 0.3), 1.0);
        assert!((p.quality(18.0, 0.3) - 0.5).abs() < 1e-6);
        assert!((p.quality(30.0, 0.03) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn people_are_looked_for_on_every_second_face_sample() {
        let p = PeopleParams::default();
        assert!(p.stride.is_multiple_of(FaceParams::default().stride));
        assert_eq!(p.span_gap(), 4);
    }
}
