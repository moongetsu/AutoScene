//! The anime pass: decode every `stride`-th frame, find characters' heads —
//! the unit a character is tracked by here — embed them, follow them
//! through each shot and hand finished tracks out in chunks, the
//! checkpoints a later run resumes from. The same samples make each shot's
//! sprite.
//!
//! Heads are tracked where the face pass tracks faces: in drawn characters
//! the face is a small part of the head, often in profile or turned away
//! where a photo-trained face detector sees nothing, while hair and
//! silhouette carry much of who the character is. The models are deepghs'
//! (Hugging Face, pinned in `bench/anime/models.tsv`): `anime_head_detection`
//! (`head_detect_v2.0_s` and kin) finds heads through
//! [`as_vision::yolo::Yolo`], and `ccip_onnx` (`ccip-caformer-24-randaug-pruned`)
//! embeds square crops around them through [`as_vision::ccip::Ccip`]. A
//! person detector (`anime_person_detection`) may run as a second source at
//! a bigger input: the head a person box implies ([`person_head`]) is added
//! where the head detector saw none ([`union_heads`]) — someone standing there
//! means a head the head model missed more often than none at all.
//!
//! Chunks and sampling follow [`crate::faces`]: chunks are made of whole
//! shots (tracks never cross a cut), samples sit on absolute multiples of
//! the stride, and a resumed run produces the same tracks as an
//! uninterrupted one. Every tunable of the pass is a field of [`Params`]
//! (all set by [`Params::anime`]) or of [`TrackerParams::anime`], so each
//! choice the anime bench (`bench/anime`, DECISIONS.md) makes lands in one
//! place.
//!
//! Toward the engine this mirrors [`crate::faces::face_tracks`]: the same
//! inputs — `tools`, `src`, `shots`, `resume`, `chunk`, `params`, `cancel`,
//! `progress`, `on_chunk` — and the same [`FaceChunk`] handed out, except
//! the models come as one [`Anime`] bundle where the face pass takes a
//! `Faces` and an optional `BodyModels`, and there is no known-faces mode,
//! so `bodies` of a chunk is always empty. Errors are `AnalysisError` like
//! the face pass's (inference failures ride in its `Infer` variant).

use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};

use as_core::{CancelToken, FrameIdx, FrameRange};
use as_media::{MediaInfo, Source, Tools};
use as_vision::ccip::{self, Ccip};
use as_vision::image;
use as_vision::scrfd::iou;
use as_vision::yolo::{Det, Yolo};

use crate::AnalysisError;
use crate::faces::FaceChunk;
use crate::sprites::{ShotSprite, SpriteCanvas};
use crate::tracker::{Observation, TrackerParams, track_shot};
use crate::tracks::{FaceDet, FaceTrack};

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Frames between samples.
    pub stride: u32,
    /// Frames are decoded to fit this box (crops and thumbnails come from it).
    pub decode_max: (u32, u32),
    /// Heads the detectors are less sure of than this are dropped — the
    /// detectors' own operating point until the bench (rule E1) picks one
    /// threshold per variant.
    pub det_threshold: f32,
    /// The same for the person detector's second source.
    pub person_threshold: f32,
    /// Heads smaller than this (fraction of the frame height) are ignored.
    pub min_height: f32,
    /// A CCIP crop is the square of this many head-box sides on the box's
    /// centre: the whole head plus context (CCIP is trained on crops this
    /// loose; rule E2 tunes it).
    pub crop_scale: f32,
    /// The top share of a person box taken as its implied head.
    pub person_head: f32,
    /// An implied head overlapping a kept one at this IoU or more counts
    /// as already found.
    pub union_iou: f32,
    /// Thumbnail side and how much context around the head box it shows.
    pub thumb_size: usize,
    pub thumb_context: f32,
    /// CCIP embedding norms from "says nothing" to "a clear head" — see
    /// [`Params::quality`].
    pub norm_floor: f32,
    pub norm_full: f32,
    /// Head height (fraction of the frame) from which size stops limiting quality.
    pub full_height: f32,
    /// Tracks whose clearest head is below this quality say nothing about
    /// anyone and are dropped (the tracker's `min_track_quality`).
    pub min_track_quality: f32,
    /// Shot sprites: frames per shot, cell width, JPEG quality.
    pub sprite_cells: u32,
    pub sprite_width: usize,
    pub sprite_quality: u8,
}

impl Params {
    /// The anime pass's parameters as planned (`bench/anime`): every value
    /// a bench rule hasn't fixed yet is marked in place and lands here so
    /// a decision touches this constructor alone.
    pub fn anime() -> Self {
        Self {
            stride: 6,
            decode_max: (1280, 720),
            det_threshold: 0.5,
            person_threshold: 0.5,
            min_height: 0.04,
            crop_scale: 1.9,
            person_head: 0.35,
            union_iou: 0.3,
            thumb_size: 160,
            thumb_context: 1.7,
            norm_floor: 110.0,
            norm_full: 170.0,
            full_height: 0.12,
            min_track_quality: 0.05,
            sprite_cells: 8,
            sprite_width: 240,
            sprite_quality: 72,
        }
    }

    /// How much a detection says about identity, 0..=1: its CCIP norm and
    /// the head's size, as the face pass reads ArcFace norms and face size.
    /// CCIP's raw lengths run about 150-200 on clear heads (the baked
    /// parity crops of `crates/as-vision/fixtures/anime` measure 157-197)
    /// where ArcFace's run ~14-22 on clear faces — about 7.7 times as
    /// long — so the defaults scale the face pass's bars by that. The bench
    /// decides whether norm earns its place (rule Q0) and where the bars
    /// truly fall (`identity_bench norms`).
    pub fn quality(&self, norm: f32, height: f32) -> f32 {
        let n = ((norm - self.norm_floor) / (self.norm_full - self.norm_floor)).clamp(0.0, 1.0);
        let s = (height / self.full_height).clamp(0.0, 1.0).sqrt();
        n * s
    }
}

impl Default for Params {
    fn default() -> Self {
        Self::anime()
    }
}

/// The models an anime pass finds characters with, all planned for the
/// media ([`frame_size`]) before the pass starts; the detectors' own inputs
/// may differ (the person detector's is the bigger one).
pub struct Anime<'a> {
    pub heads: &'a mut Yolo,
    pub persons: Option<&'a mut Yolo>,
    pub ccip: &'a mut Ccip,
}

/// The size a media's samples are decoded at — the frame size the detectors
/// are loaded for (their `frame`; each model's `input` is the variant's own).
pub fn frame_size(info: &MediaInfo, params: &Params) -> (usize, usize) {
    let (w, h) = as_media::frames::sample_size(info, params.decode_max, 4);
    (w as usize, h as usize)
}

/// The head a person box (x1, y1, x2, y2 in pixels) implies: its top
/// `share`, where the head of a figure on screen sits.
pub fn person_head(bbox: [f32; 4], share: f32) -> [f32; 4] {
    [bbox[0], bbox[1], bbox[2], bbox[1] + (bbox[3] - bbox[1]) * share]
}

/// The heads of one sample: the head detector's own, plus the implied head
/// of each person box ([`person_head`]) none of them covers. The person
/// detector looks at a bigger picture and catches heads the head detector
/// misses, but where both find the same head only the head box counts — an
/// implied head overlapping a kept one at `max_iou` (IoU) or more counts
/// as already found, by a real detection or by a person before it. A box
/// too small to hold a head adds nothing.
pub fn union_heads(mut heads: Vec<Det>, persons: &[Det], head_share: f32, max_iou: f32) -> Vec<Det> {
    for p in persons {
        let head = person_head(p.bbox, head_share);
        if head[2] - head[0] < 1.0 || head[3] - head[1] < 1.0 {
            continue;
        }
        if heads.iter().all(|h| iou(&h.bbox, &head) < max_iou) {
            heads.push(Det { bbox: head, score: p.score });
        }
    }
    heads
}

/// A head found in a sample, waiting for its embedding.
struct Found {
    det: FaceDet,
    crop: Vec<u8>,
    thumb: Vec<u8>,
    embedding: Option<Vec<f32>>,
}

/// Samples of one shot.
struct ShotWork {
    range: FrameRange,
    samples: Vec<(FrameIdx, Vec<Found>)>,
    sprite: Option<SpriteCanvas>,
}

/// A sample in a shot, on its way through the detectors.
struct Queued {
    frame: FrameIdx,
    rgb: Vec<u8>,
}

/// A batch of samples on its way through the detectors, with the heads and
/// the people found in each (`persons` stays empty when no one was looked
/// for).
#[derive(Default)]
struct Batch {
    samples: Vec<Queued>,
    heads: Vec<Vec<Det>>,
    persons: Vec<Vec<Det>>,
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

/// Embed every head of `shots` still waiting (the last batch padded).
fn embed(shots: &mut [&mut ShotWork], ccip: &mut Ccip, params: &Params) -> Result<(), AnalysisError> {
    let mut slots: Vec<&mut Found> = shots
        .iter_mut()
        .flat_map(|s| s.samples.iter_mut().flat_map(|(_, f)| f.iter_mut()))
        .filter(|f| f.embedding.is_none())
        .collect();
    if slots.is_empty() {
        return Ok(());
    }
    let crops: Vec<&[u8]> = slots.iter().map(|f| f.crop.as_slice()).collect();
    let embs = ccip.embed(&crops)?;
    for (f, e) in slots.iter_mut().zip(embs) {
        f.det.norm = e.norm;
        f.det.quality = params.quality(e.norm, f.det.height());
        f.embedding = Some(e.v);
        f.crop = Vec::new();
    }
    Ok(())
}

/// Track the heads of an embedded shot.
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

/// One anime pass in progress: samples in shots go to the detector threads
/// in batches and come back — in frame order — into shots and chunks.
struct Pass<'a, C> {
    params: &'a Params,
    tracker: TrackerParams,
    ccip: &'a mut Ccip,
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
    on_chunk: C,
}

impl<C: FnMut(FaceChunk) -> Result<(), AnalysisError>> Pass<'_, C> {
    /// Whether frame `f` lies in a shot; only those are searched for heads.
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
        Err(AnalysisError::Internal("the head detector stopped".into()))
    }

    /// Take in the batches the detectors have finished, in order — all of
    /// them when `wait`.
    fn collect(&mut self, wait: bool) -> Result<(), AnalysisError> {
        while self.in_flight > 0 {
            let next = match self.from_detector.try_recv() {
                Ok(d) => d,
                Err(TryRecvError::Empty) if !wait => return Ok(()),
                Err(TryRecvError::Empty) => self
                    .from_detector
                    .recv()
                    .map_err(|_| AnalysisError::Internal("the head detector stopped".into()))?,
                Err(TryRecvError::Disconnected) => {
                    return Err(AnalysisError::Internal("the head detector stopped".into()));
                }
            };
            self.in_flight -= 1;
            let Batch { samples, heads, persons } = next?;
            let mut heads = heads.into_iter();
            for (i, q) in samples.iter().enumerate() {
                let extra = persons.get(i).map_or(&[][..], Vec::as_slice);
                self.take(q.frame, &q.rgb, heads.next().unwrap_or_default(), extra)?;
            }
            self.spare.extend(samples.into_iter().map(|q| q.rgb));
        }
        Ok(())
    }

    /// Take in the heads found in sample `f`: open and close shots and
    /// chunks up to it, keep its heads' crops and thumbnails, and embed and
    /// track as they pile up.
    fn take(&mut self, f: FrameIdx, rgb: &[u8], heads: Vec<Det>, persons: &[Det]) -> Result<(), AnalysisError> {
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
                let sprite = SpriteCanvas::new(s, p.stride, p.sprite_cells, self.size, p.sprite_width);
                self.current = Some(ShotWork { range: s, samples: Vec::new(), sprite });
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
        let mut found = Vec::new();
        for d in union_heads(heads, persons, p.person_head, p.union_iou) {
            let height = (d.bbox[3] - d.bbox[1]) / h as f32;
            if height < p.min_height {
                continue;
            }
            let side = (d.bbox[2] - d.bbox[0]).max(d.bbox[3] - d.bbox[1]) * p.crop_scale;
            let (cx, cy) = ((d.bbox[0] + d.bbox[2]) / 2.0, (d.bbox[1] + d.bbox[3]) / 2.0);
            let thumb = image::face_thumb(rgb, w, h, d.bbox, p.thumb_context, p.thumb_size);
            found.push(Found {
                det: FaceDet {
                    frame: f,
                    bbox: [d.bbox[0] / w as f32, d.bbox[1] / h as f32, d.bbox[2] / w as f32, d.bbox[3] / h as f32],
                    score: d.score,
                    norm: 0.0,
                    quality: 0.0,
                },
                crop: ccip::crop(rgb, w, h, cx - side / 2.0, cy - side / 2.0, side),
                thumb: image::jpeg(&thumb, p.thumb_size, p.thumb_size, 85),
                embedding: None,
            });
        }
        self.pending_crops += found.len();
        cur.samples.push((f, found));
        // Embed in full batches as heads pile up; track finished shots.
        if self.pending_crops >= self.ccip.batch() * 4 {
            self.embed_heads(true)?;
            self.finish_shots();
        }
        Ok(())
    }

    /// Embed the heads waiting in finished shots, and in the current one too
    /// with `current`.
    fn embed_heads(&mut self, current: bool) -> Result<(), AnalysisError> {
        let mut refs: Vec<&mut ShotWork> =
            self.done_shots.iter_mut().chain(self.current.iter_mut().filter(|_| current)).collect();
        embed(&mut refs, self.ccip, self.params)?;
        self.pending_crops = 0;
        Ok(())
    }

    /// Track the heads of the finished (embedded) shots and finish their
    /// sprites.
    fn finish_shots(&mut self) {
        for mut s in std::mem::take(&mut self.done_shots) {
            if let Some(sprite) = s.sprite.take() {
                self.sprites_in_chunk.push(sprite.finish(self.params.sprite_quality));
            }
            self.tracks_in_chunk.extend(track(&mut s, &self.tracker));
        }
    }

    /// Hand out the chunk that ends at `end` (a shot start, or the end):
    /// every shot before it is finished.
    fn close_chunk(&mut self, end: FrameIdx) -> Result<(), AnalysisError> {
        self.embed_heads(false)?;
        self.finish_shots();
        (self.on_chunk)(FaceChunk {
            start: self.chunk_start,
            end,
            tracks: &self.tracks_in_chunk,
            sprites: &self.sprites_in_chunk,
            bodies: &[],
        })?;
        self.tracks_in_chunk.clear();
        self.sprites_in_chunk.clear();
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

/// Find, embed and track the characters' heads of `src` in `shots` (sorted,
/// disjoint) from frame `resume` on — which must be 0 or a chunk end handed
/// out earlier. Each finished chunk goes to `on_chunk`; chunks tile
/// `resume..index.len()` and end at shot starts at least `chunk` frames
/// after their own start (or at the end). `progress` gets 0..1.
///
/// The head detector of `anime` must be planned for this media's
/// [`frame_size`]; the optional person detector for the same frames (its
/// own input may be bigger). Chunks are the face pass's [`FaceChunk`],
/// `bodies` always empty.
#[allow(clippy::too_many_arguments)]
pub fn face_tracks(
    tools: &Tools,
    src: Source,
    anime: Anime,
    shots: &[FrameRange],
    resume: FrameIdx,
    chunk: u32,
    params: &Params,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
    on_chunk: impl FnMut(FaceChunk) -> Result<(), AnalysisError>,
) -> Result<(), AnalysisError> {
    assert!(chunk > 0 && params.stride > 0);
    let n = src.index.len();
    if resume >= n {
        return Ok(());
    }
    let size = frame_size(src.info, params);
    if anime.heads.frame_size() != size {
        return Err(AnalysisError::Internal(format!(
            "head detector loaded for {:?} frames, this media needs {size:?}",
            anime.heads.frame_size()
        )));
    }
    if let Some(persons) = &anime.persons {
        if persons.frame_size() != size {
            return Err(AnalysisError::Internal(format!(
                "person detector loaded for {:?} frames, this media needs {size:?}",
                persons.frame_size()
            )));
        }
    }
    // Shots from the resume point on; samples outside them are skipped.
    let shots = &shots[shots.partition_point(|s| s.end <= resume)..];
    debug_assert!(shots.first().is_none_or(|s| s.start >= resume), "resume points are shot starts");
    let batch = anime.heads.batch();
    let Anime { heads, persons, ccip } = anime;
    std::thread::scope(|scope| {
        // One batch waits while a detector works on the next. Heads first,
        // then people, then back here.
        let (to_detector, first) = mpsc::sync_channel::<Detected>(1);
        let (detected, from_detector) = mpsc::channel::<Detected>();
        let starting = |what: &str, e: std::io::Error| AnalysisError::Internal(format!("starting the {what}: {e}"));
        // What the person detector takes its batches from, when it runs.
        let into_persons: Option<Receiver<Detected>> = {
            let work = move |b: &mut Batch| {
                let frames: Vec<&[u8]> = b.samples.iter().map(|q| q.rgb.as_slice()).collect();
                b.heads = heads.detect_many(&frames, params.det_threshold)?;
                Ok(())
            };
            let thread = std::thread::Builder::new().name("head detector".into());
            if persons.is_some() {
                let (out, rx) = mpsc::sync_channel::<Detected>(1);
                thread
                    .spawn_scoped(scope, move || stage("head detector", first, out, work))
                    .map_err(|e| starting("head detector", e))?;
                Some(rx)
            } else {
                let out = detected.clone();
                thread
                    .spawn_scoped(scope, move || stage("head detector", first, out, work))
                    .map_err(|e| starting("head detector", e))?;
                None
            }
        };
        if let (Some(d), Some(input)) = (persons, into_persons) {
            let out = detected.clone();
            std::thread::Builder::new()
                .name("person detector".into())
                .spawn_scoped(scope, move || {
                    stage("person detector", input, out, |b| {
                        let frames: Vec<&[u8]> = b.samples.iter().map(|q| q.rgb.as_slice()).collect();
                        b.persons = d.detect_many(&frames, params.person_threshold)?;
                        Ok(())
                    })
                })
                .map_err(|e| starting("person detector", e))?;
        }
        drop(detected);
        let tracker = TrackerParams {
            stride: params.stride,
            min_track_quality: params.min_track_quality,
            ..TrackerParams::anime()
        };
        let mut pass = Pass {
            params,
            tracker,
            ccip,
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
            on_chunk,
        };
        let mut failure: Option<AnalysisError> = None;
        let stride = params.stride;
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
                "anime pass decoded {delivered} samples from {first}, expected {expected}"
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

    fn info(width: u32, height: u32) -> MediaInfo {
        use as_core::Rational;
        use as_media::{HdrKind, VideoStream};
        MediaInfo {
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
        }
    }

    fn head(x: f32, y: f32, side: f32, score: f32) -> Det {
        Det { bbox: [x, y, x + side, y + side], score }
    }

    #[test]
    fn frames_decode_to_fit_the_box() {
        let p = Params::anime();
        assert_eq!(frame_size(&info(1920, 1080), &p), (1280, 720));
        assert_eq!(frame_size(&info(720, 480), &p), (720, 480));
    }

    #[test]
    fn a_person_head_is_the_top_of_the_box() {
        assert_eq!(person_head([10.0, 20.0, 50.0, 220.0], 0.35), [10.0, 20.0, 50.0, 90.0]);
    }

    #[test]
    fn a_person_adds_a_head_only_where_none_was_seen() {
        let p = Params::anime();
        let persons = [Det { bbox: [10.0, 20.0, 50.0, 220.0], score: 0.7 }];
        // The head detector silent there: the implied head joins the found ones.
        let out = union_heads(vec![head(200.0, 30.0, 60.0, 0.9)], &persons, p.person_head, p.union_iou);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].bbox, [200.0, 30.0, 260.0, 90.0]);
        assert_eq!(out[1].bbox, [10.0, 20.0, 50.0, 90.0]);
        assert_eq!(out[1].score, 0.7);
        // Found already: the person adds nothing.
        let seen = vec![Det { bbox: [12.0, 22.0, 52.0, 92.0], score: 0.8 }];
        assert_eq!(union_heads(seen, &persons, p.person_head, p.union_iou).len(), 1);
    }

    #[test]
    fn a_second_person_on_the_same_head_adds_nothing() {
        let p = Params::anime();
        // Two overlapping person boxes whose implied heads are one.
        let persons =
            [Det { bbox: [10.0, 20.0, 50.0, 220.0], score: 0.9 }, Det { bbox: [12.0, 22.0, 52.0, 225.0], score: 0.8 }];
        let out = union_heads(Vec::new(), &persons, p.person_head, p.union_iou);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].score, 0.9);
    }

    #[test]
    fn a_person_box_too_small_for_a_head_adds_none() {
        let p = Params::anime();
        let squeezed = [Det { bbox: [10.0, 20.0, 10.0, 20.0], score: 0.9 }];
        assert!(union_heads(Vec::new(), &squeezed, p.person_head, p.union_iou).is_empty());
    }

    #[test]
    fn quality_follows_norm_and_size() {
        let p = Params::anime();
        assert_eq!(p.quality(50.0, 0.3), 0.0);
        assert_eq!(p.quality(200.0, 0.3), 1.0);
        assert!((p.quality(140.0, 0.3) - 0.5).abs() < 1e-6);
        assert!((p.quality(200.0, 0.03) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_defaults_are_the_anime_preset() {
        let p = Params::default();
        assert_eq!(
            (p.stride, p.det_threshold, p.crop_scale, p.person_head, p.union_iou, p.norm_floor, p.norm_full),
            (6, 0.5, 1.9, 0.35, 0.3, 110.0, 170.0)
        );
    }
}
