//! The face pass: decode every `stride`-th frame, find faces (a few samples
//! per inference, on a thread of its own so the GPU works while this one
//! crops, embeds and tracks), embed them, follow them through each shot, and
//! hand finished tracks out in chunks — the checkpoints a later run resumes
//! from. The same samples make each shot's sprite.
//!
//! Chunks are made of whole shots (tracks never cross a cut), so a chunk
//! boundary is the start of a shot and a resumed run produces the same
//! tracks as an uninterrupted one. Samples sit on absolute multiples of the
//! stride for the same reason.

use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};

use as_core::{CancelToken, FrameIdx, FrameRange};
use as_media::{MediaInfo, Source, Tools};
use as_vision::arcface::{self, ArcFace};
use as_vision::{FaceBox, Scrfd, image};

use crate::AnalysisError;
use crate::sprites::{ShotSprite, SpriteCanvas};
use crate::tracker::{Observation, TrackerParams, track_shot};
use crate::tracks::{FaceDet, FaceTrack};

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

/// Samples of one shot.
struct ShotWork {
    range: FrameRange,
    samples: Vec<(FrameIdx, Vec<Found>)>,
    sprite: Option<SpriteCanvas>,
}

/// A finished checkpoint chunk of the face pass: frames `start..end` (whole
/// shots), their tracks and the sprites of their shots.
#[derive(Clone, Copy, Debug)]
pub struct FaceChunk<'a> {
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub tracks: &'a [FaceTrack],
    pub sprites: &'a [ShotSprite],
}

/// A sample in a shot, on its way through the detector.
struct Queued {
    frame: FrameIdx,
    rgb: Vec<u8>,
}

/// A batch back from the detector: its samples and the faces of each.
type Detected = Result<(Vec<Queued>, Vec<Vec<FaceBox>>), AnalysisError>;

/// The detector thread: batches in, faces out, in order. `halve`: detect on
/// half-size copies (boxes are scaled back). Stops after the first error; a
/// panic becomes one, so the pass fails instead of the engine.
fn detect_batches(
    detector: &mut Scrfd,
    size: (usize, usize),
    halve: bool,
    threshold: f32,
    batches: Receiver<Vec<Queued>>,
    out: Sender<Detected>,
) {
    let errors = out.clone();
    let run = std::panic::AssertUnwindSafe(|| detect_each(detector, size, halve, threshold, batches, out));
    if let Err(panic) = std::panic::catch_unwind(run) {
        let why = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let _ = errors.send(Err(AnalysisError::Internal(format!("the face detector failed: {why}"))));
    }
}

fn detect_each(
    detector: &mut Scrfd,
    (w, h): (usize, usize),
    halve: bool,
    threshold: f32,
    batches: Receiver<Vec<Queued>>,
    out: Sender<Detected>,
) {
    for batch in batches {
        let found = if halve {
            let small: Vec<Vec<u8>> = batch.iter().map(|q| image::half(&q.rgb, w, h)).collect();
            let frames: Vec<&[u8]> = small.iter().map(Vec::as_slice).collect();
            detector.detect_many(&frames, threshold).map(|found| {
                found.into_iter().map(|faces| faces.into_iter().map(|b| b.scaled(2.0)).collect()).collect()
            })
        } else {
            let frames: Vec<&[u8]> = batch.iter().map(|q| q.rgb.as_slice()).collect();
            detector.detect_many(&frames, threshold)
        };
        let failed = found.is_err();
        if out.send(found.map(|f| (batch, f)).map_err(AnalysisError::from)).is_err() || failed {
            return;
        }
    }
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

/// Track the faces of embedded shots.
fn track(shots: Vec<ShotWork>, tracker: &TrackerParams) -> Vec<FaceTrack> {
    shots
        .into_iter()
        .flat_map(|s| {
            let samples = s
                .samples
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
            track_shot(s.range, samples, tracker)
        })
        .collect()
}

/// One face pass in progress: samples in shots go to the detector thread in
/// batches and come back — in frame order — into shots and chunks.
struct Pass<'a, C> {
    params: &'a FaceParams,
    tracker: TrackerParams,
    embedder: &'a mut ArcFace,
    /// Shots from the resume point on.
    shots: &'a [FrameRange],
    /// Decoded frame size.
    size: (usize, usize),
    /// Samples per detector batch.
    batch: usize,
    queue: Vec<Queued>,
    /// `None` once every batch is sent.
    to_detector: Option<SyncSender<Vec<Queued>>>,
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
        let batch = std::mem::replace(&mut self.queue, Vec::with_capacity(self.batch));
        if self.to_detector.as_ref().is_some_and(|tx| tx.send(batch).is_ok()) {
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
            let (batch, found) = next?;
            for (q, faces) in batch.iter().zip(found) {
                self.take(q.frame, &q.rgb, faces)?;
            }
            self.spare.extend(batch.into_iter().map(|q| q.rgb));
        }
        Ok(())
    }

    /// Take in the faces found in sample `f`: open and close shots and
    /// chunks up to it, keep its faces' crops and thumbnails, and embed and
    /// track as faces pile up.
    fn take(&mut self, f: FrameIdx, rgb: &[u8], faces: Vec<FaceBox>) -> Result<(), AnalysisError> {
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
        // Embed in full batches as faces pile up; track finished shots.
        if self.pending_crops >= self.embedder.batch() * 4 {
            let mut refs: Vec<&mut ShotWork> = self.done_shots.iter_mut().chain(self.current.iter_mut()).collect();
            embed(&mut refs, self.embedder, p)?;
            self.pending_crops = 0;
            self.finish_shots();
        }
        Ok(())
    }

    /// Track the faces of the finished (embedded) shots; finish their sprites.
    fn finish_shots(&mut self) {
        let mut shots = std::mem::take(&mut self.done_shots);
        for s in &mut shots {
            if let Some(sprite) = s.sprite.take() {
                self.sprites_in_chunk.push(sprite.finish(self.params.sprite_quality));
            }
        }
        self.tracks_in_chunk.extend(track(shots, &self.tracker));
    }

    /// Hand out the chunk that ends at `end` (a shot start, or the end):
    /// every shot before it is finished.
    fn close_chunk(&mut self, end: FrameIdx) -> Result<(), AnalysisError> {
        let mut refs: Vec<&mut ShotWork> = self.done_shots.iter_mut().collect();
        embed(&mut refs, self.embedder, self.params)?;
        self.pending_crops = 0;
        self.finish_shots();
        (self.on_chunk)(FaceChunk {
            start: self.chunk_start,
            end,
            tracks: &self.tracks_in_chunk,
            sprites: &self.sprites_in_chunk,
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

/// Find, embed and track the faces of `src` in `shots` (sorted, disjoint)
/// from frame `resume` on — which must be 0 or a chunk end handed out
/// earlier. `detector` must take frames of the detection size of
/// [`frame_sizes`]. Each finished chunk goes to `on_chunk`; chunks tile
/// `resume..index.len()` and end at shot starts at least `chunk` frames
/// after their own start (or at the end). `progress` gets 0..1.
#[allow(clippy::too_many_arguments)]
pub fn face_tracks(
    tools: &Tools,
    src: Source,
    detector: &mut Scrfd,
    embedder: &mut ArcFace,
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
    let stride = params.stride;
    let (size, det_size) = frame_sizes(src.info, params);
    if detector.frame_size() != det_size {
        return Err(AnalysisError::Internal(format!(
            "face detector loaded for {:?} frames, this media needs {det_size:?}",
            detector.frame_size()
        )));
    }

    // Shots from the resume point on; samples outside them are skipped.
    let shots = &shots[shots.partition_point(|s| s.end <= resume)..];
    debug_assert!(shots.first().is_none_or(|s| s.start >= resume), "resume points are shot starts");
    let batch = detector.batch();
    std::thread::scope(|scope| {
        // One batch waits while the detector works on the next.
        let (to_detector, batches) = mpsc::sync_channel(1);
        let (detected, from_detector) = mpsc::channel();
        let (halve, threshold) = (det_size != size, params.det_threshold);
        std::thread::Builder::new()
            .name("face detector".into())
            .spawn_scoped(scope, move || detect_batches(detector, size, halve, threshold, batches, detected))
            .map_err(|e| AnalysisError::Internal(format!("starting the face detector: {e}")))?;
        let mut pass = Pass {
            params,
            tracker: TrackerParams { min_track_quality: params.min_track_quality, ..TrackerParams::new(stride) },
            embedder,
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
}
