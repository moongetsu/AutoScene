//! Fast Scan: the face pass of the Scener app, as its shipped worker does it
//! (constants recovered from the binary, see `models/manifest.toml`) —
//! YuNet finds faces, SFace embeds them, and only a few frames of each shot
//! are looked at instead of every third one.
//!
//! Each shot has a pool of candidate frames, one a second from its first
//! frame on plus its last one. Looked at first: its first and last frame,
//! its middle, a frame where the picture changed from the candidate before
//! ([`visual_change`]), and enough others that no two looks are more than
//! two seconds apart. A look that leaves doubt — a face less sure than
//! [`FastParams::low_confidence`], a small face, or another number of faces
//! than the look before — gets the candidates next to it looked at too,
//! once. The faces then go to the same tracker and the same grouping as
//! the full pass's.
//!
//! Like the full pass ([`crate::faces`]), chunks are whole shots and the
//! candidates sit on an absolute grid of frames ([`FastParams::grid`]): what
//! a shot looks at depends on the shot alone, so a resumed run produces the
//! same tracks as an uninterrupted one. Looking at this few frames, the pass
//! is as fast as its frames are decoded: like Scener, it has an NVIDIA GPU
//! decode them where there is one ([`FastParams::decoder`]).
//!
//! Measured on `bench/identity` (2026-10-27): about three times as fast as
//! the full pass on a 4K film (39–57 s against 133–149 s for its last
//! 22,319 frames). It groups what it finds about as well, but finds fewer
//! faces: 78% of a live-action main cast's screen time ends up on their
//! character (full pass 96%; Breaking Bad's 960×540 proxy), 59% of a CGI
//! film's (94%).

use std::collections::VecDeque;

use as_core::{CancelToken, FrameIdx, FrameRange};
use as_media::frames::Decoder;
use as_media::{MediaInfo, Source, Tools};
use as_vision::arcface;
use as_vision::{FaceBox, SFace, YuNet, image};

use crate::AnalysisError;
use crate::faces::FaceChunk;
use crate::sprites::{ShotSprite, SpriteCanvas};
use crate::tracker::{Observation, TrackerParams, track_shot};
use crate::tracks::{DIM, FaceDet, FaceTrack};

#[derive(Clone, Copy, Debug)]
pub struct FastParams {
    /// Frames are decoded to fit this box: Scener searches at most 1280
    /// pixels along the longer side (`DEFAULT_YUNET_DETECT_MAX_EDGE`).
    pub decode_max: (u32, u32),
    /// Where they are decoded (software where the GPU can't).
    pub decoder: Decoder,
    /// Candidates sit on a grid of this many frames a second ...
    pub grid_per_sec: f64,
    /// ... and a shot's pool takes one every this many seconds (Scener's
    /// `sample_fps` of 1).
    pub pool_secs: f64,
    /// Looks no more than this far apart (`REAL_SHOT_MAX_GAP_SECONDS`).
    pub max_gap_secs: f64,
    /// A candidate this much unlike the one before is looked at
    /// (`VISUAL_CHANGE_THRESHOLD`, see [`visual_change`]).
    pub visual_change: f32,
    /// A look with a face less sure than this (`LOW_CONFIDENCE_THRESHOLD`),
    /// ...
    pub low_confidence: f32,
    /// ... one covering less of the frame than this (`SMALL_FACE_AREA_THRESHOLD`),
    pub small_area: f32,
    /// ... or one narrower or shorter than this share of the frame's
    /// (`SMALL_FACE_SIDE_THRESHOLD`) has its neighbours looked at too.
    pub small_side: f32,
    /// YuNet keeps faces from this score. Scener keeps them from 0.85, which
    /// loses a quarter of the faces a live-action episode shows at the
    /// looks (BB S01E01 from its 960×540 proxy: 76% held, 90% at 0.7) and
    /// half those of a CGI film (160 hand-labelled frames: 48% found, 67%
    /// at 0.7, still 98% of them faces). The faces it is less sure of count
    /// for less ([`FastParams::score_floor`]).
    pub det_threshold: f32,
    /// Faces smaller than this (fraction of the frame height) are ignored,
    /// as in the full pass.
    pub min_height: f32,
    /// Thumbnail side and how much context around the face box it shows.
    pub thumb_size: usize,
    pub thumb_context: f32,
    /// SFace embedding norms from "says nothing" to "clear frontal face":
    /// where the full pass's bars for ArcFace (14 and 22) fall among the
    /// same faces' SFace norms (1072 faces of a live-action episode; the
    /// two norms correlate at 0.75).
    pub norm_floor: f32,
    pub norm_full: f32,
    /// Face height (fraction of the frame) from which size stops limiting quality.
    pub full_height: f32,
    /// YuNet scores from "says nothing" to "sure" (equal: score doesn't
    /// count). Without it, faces from 0.7 split Walter in most runs of the
    /// identity bench's stress test; with it, none.
    pub score_floor: f32,
    pub score_full: f32,
    /// Tracks whose clearest face is below this quality are dropped.
    pub min_track_quality: f32,
    /// Shot sprites: frames per shot, cell width, JPEG quality.
    pub sprite_cells: u32,
    pub sprite_width: usize,
    pub sprite_quality: u8,
}

impl Default for FastParams {
    fn default() -> Self {
        Self {
            decode_max: (1280, 1280),
            decoder: Decoder::Nvdec,
            grid_per_sec: 4.0,
            pool_secs: 1.0,
            max_gap_secs: 2.0,
            visual_change: 0.18,
            low_confidence: 0.92,
            small_area: 0.012,
            small_side: 0.08,
            det_threshold: 0.7,
            min_height: 0.04,
            thumb_size: 160,
            thumb_context: 1.7,
            norm_floor: 5.5,
            norm_full: 8.0,
            full_height: 0.12,
            score_floor: 0.7,
            score_full: 0.95,
            min_track_quality: 0.05,
            sprite_cells: 8,
            sprite_width: 240,
            sprite_quality: 72,
        }
    }
}

impl FastParams {
    /// Frames between candidates on the grid at `fps`.
    pub fn grid(&self, fps: f64) -> u32 {
        ((fps / self.grid_per_sec).round() as u32).max(1)
    }

    /// Grid steps between a pool's candidates.
    fn pool_steps(&self, fps: f64) -> u32 {
        ((self.pool_secs * fps / f64::from(self.grid(fps))).round() as u32).max(1)
    }

    /// Pool candidates two looks may lie apart at most.
    fn max_steps(&self) -> usize {
        ((self.max_gap_secs / self.pool_secs) + 1e-6).floor().max(1.0) as usize
    }

    /// How the tracker follows faces looked at this sparsely: the grid is
    /// its stride, and a face may be missed for as long as two looks may lie
    /// apart and still be on screen. A face moving fast is followed over as
    /// many grid steps, at a quarter of the full pass's reach per step.
    pub fn tracker(&self, fps: f64) -> TrackerParams {
        let gap = self.max_steps() as u32 * self.pool_steps(fps);
        TrackerParams {
            max_gap: gap,
            reach: 0.4,
            reach_gap: gap,
            min_track_quality: self.min_track_quality,
            ..TrackerParams::new(self.grid(fps))
        }
    }

    /// How much a detection says about identity and how good a cover it
    /// makes, 0..=1: the SFace norm and the face size, as in the full pass,
    /// and how sure YuNet is of it.
    pub fn quality(&self, norm: f32, height: f32, score: f32) -> f32 {
        let n = ((norm - self.norm_floor) / (self.norm_full - self.norm_floor)).clamp(0.0, 1.0);
        let s = (height / self.full_height).clamp(0.0, 1.0).sqrt();
        let c = if self.score_full > self.score_floor {
            ((score - self.score_floor) / (self.score_full - self.score_floor)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        n * s * c
    }
}

/// The size Fast Scan decodes `info`'s frames at (YuNet's input, padded).
pub fn frame_size(info: &MediaInfo, params: &FastParams) -> (usize, usize) {
    let (w, h) = as_media::frames::sample_size(info, params.decode_max, 4);
    (w as usize, h as usize)
}

/// A shot's candidates: every `steps`-th frame of the `grid` from the
/// shot's first grid frame on, and its last grid frame. Empty when no grid
/// frame lies in the shot.
fn pool(shot: FrameRange, grid: u32, steps: u32) -> Vec<FrameIdx> {
    let first = shot.start.div_ceil(grid) * grid;
    if first >= shot.end {
        return Vec::new();
    }
    let last = (shot.end - 1) / grid * grid;
    let mut out: Vec<FrameIdx> = (first..=last).step_by((grid * steps) as usize).collect();
    if out.last() != Some(&last) {
        out.push(last);
    }
    out
}

/// What a look found: the inexpensive facts that say whether the
/// candidates next to it need a look too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub faces: usize,
    /// Of its faces: the least sure, the smallest area and the smallest
    /// side (shares of the frame's). 1 when there are none.
    pub min_confidence: f32,
    pub min_area: f32,
    pub min_side: f32,
}

impl Summary {
    /// The faces of a `w`×`h` frame (boxes in its pixels).
    fn of(faces: &[FaceBox], (w, h): (usize, usize)) -> Self {
        let mut s = Self { faces: faces.len(), min_confidence: 1.0, min_area: 1.0, min_side: 1.0 };
        for f in faces {
            let (fw, fh) = (f.width() / w as f32, f.height() / h as f32);
            s.min_confidence = s.min_confidence.min(f.score);
            s.min_area = s.min_area.min(fw * fh);
            s.min_side = s.min_side.min(fw.min(fh));
        }
        s
    }
}

/// Why a candidate is looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The shot's first or last candidate.
    Boundary,
    Midpoint,
    /// The picture changed from the candidate before.
    Change,
    /// Two looks would otherwise lie too far apart.
    Gap,
    /// A look next to it left doubt.
    Refine,
}

/// Which of a shot's candidates are looked at, decided as they come in
/// order (see the module docs). Only the candidates a few places back can
/// still be asked for ([`Selector::WINDOW`]).
#[derive(Debug)]
struct Selector {
    /// Candidates in the pool, and the middle one.
    count: usize,
    mid: usize,
    /// Pool steps two looks may lie apart at most.
    max_steps: usize,
    why: Vec<Option<Why>>,
    /// The last look of the first round (not a refinement), and its faces.
    last: Option<(usize, usize)>,
    /// The next candidate is looked at to refine one before it.
    next: bool,
    low_confidence: f32,
    small_area: f32,
    small_side: f32,
}

impl Selector {
    /// Candidates kept back for a later refinement: a look's neighbours go
    /// back at most to the one before the look before it.
    const WINDOW: usize = 4;

    fn new(count: usize, max_steps: usize, p: &FastParams) -> Self {
        Self {
            count,
            mid: count / 2,
            max_steps,
            why: vec![None; count],
            last: None,
            next: false,
            low_confidence: p.low_confidence,
            small_area: p.small_area,
            small_side: p.small_side,
        }
    }

    /// Whether candidate `i` (the next in order) is looked at, and why;
    /// `changed`: its picture changed from the one before.
    fn look(&mut self, i: usize, changed: bool) -> Option<Why> {
        let since = self.last.map_or(i + 1, |(l, _)| i - l);
        let why = if i == 0 || i + 1 == self.count {
            Some(Why::Boundary)
        } else if i == self.mid {
            Some(Why::Midpoint)
        } else if changed {
            Some(Why::Change)
        } else if since + 1 > self.max_steps {
            // Skipping it would leave the next look too far from the last.
            Some(Why::Gap)
        } else if self.next {
            Some(Why::Refine)
        } else {
            None
        };
        self.next = false;
        self.why[i] = why;
        why
    }

    /// What look `i` of the first round found: the candidates before it
    /// that need a look now (in order), and — through [`Selector::look`] —
    /// whether the next one does.
    fn found(&mut self, i: usize, s: Summary) -> Vec<usize> {
        let unsure = s.faces > 0
            && (s.min_confidence < self.low_confidence || s.min_area < self.small_area || s.min_side < self.small_side);
        let mut doubts: Vec<usize> = Vec::new();
        if unsure {
            doubts.push(i);
        }
        if let Some((l, faces)) = self.last
            && faces != s.faces
        {
            doubts.extend([l, i]);
        }
        self.last = Some((i, s.faces));
        let mut now: Vec<usize> = Vec::new();
        for d in doubts {
            if d > 0 {
                now.push(d - 1);
            }
            if d + 1 < i {
                now.push(d + 1);
            } else if d == i && i + 1 < self.count {
                self.next = true;
            }
        }
        now.sort_unstable();
        now.dedup();
        now.retain(|&j| self.why[j].is_none() && j + Self::WINDOW > i);
        for &j in &now {
            self.why[j] = Some(Why::Refine);
        }
        now
    }
}

/// Side of the small grey picture candidates are compared by, and its
/// blocks (Scener compares 64×36 luma pictures by the 90th percentile of
/// their blocks' differences, each relative to its picture's mean).
const COMPACT: (usize, usize) = (64, 36);
const BLOCK: (usize, usize) = (8, 6);

/// A candidate's picture for [`visual_change`]: its luma (0..1) in blocks
/// over a 64×36 shrink, each relative to the mean.
fn compact(rgb: &[u8], (w, h): (usize, usize)) -> Vec<f32> {
    let (cw, ch) = COMPACT;
    let mut luma = vec![0f32; cw * ch];
    for (y, row) in luma.chunks_exact_mut(cw).enumerate() {
        let (y0, y1) = (y * h / ch, ((y + 1) * h / ch).max(y * h / ch + 1));
        for (x, v) in row.iter_mut().enumerate() {
            let (x0, x1) = (x * w / cw, ((x + 1) * w / cw).max(x * w / cw + 1));
            let mut sum = 0f32;
            for yy in y0..y1 {
                for px in rgb[(yy * w + x0) * 3..(yy * w + x1) * 3].as_chunks::<3>().0 {
                    sum += 0.299 * f32::from(px[0]) + 0.587 * f32::from(px[1]) + 0.114 * f32::from(px[2]);
                }
            }
            *v = sum / ((y1 - y0) * (x1 - x0)) as f32 / 255.0;
        }
    }
    let mean = luma.iter().sum::<f32>() / luma.len() as f32;
    let (bw, bh) = BLOCK;
    let mut blocks = Vec::with_capacity((cw / bw) * (ch / bh));
    for by in 0..ch / bh {
        for bx in 0..cw / bw {
            let mut sum = 0f32;
            for y in by * bh..(by + 1) * bh {
                sum += luma[y * cw + bx * bw..y * cw + (bx + 1) * bw].iter().sum::<f32>();
            }
            blocks.push(sum / (bw * bh) as f32 - mean);
        }
    }
    blocks
}

/// How much two candidates' pictures ([`compact`]) differ, 0..1: the 90th
/// percentile of their blocks' differences.
pub fn visual_change(a: &[f32], b: &[f32]) -> f32 {
    let mut d: Vec<f32> = a.iter().zip(b).map(|(x, y)| (x - y).abs()).collect();
    d.sort_by(f32::total_cmp);
    d.get(((d.len() as f32 * 0.9).ceil() as usize).saturating_sub(1)).copied().unwrap_or(0.0)
}

/// A candidate kept back for a possible refinement.
struct Kept {
    i: usize,
    frame: FrameIdx,
    rgb: Vec<u8>,
}

/// A face found in a look, waiting for its embedding.
struct Found {
    det: FaceDet,
    crop: Vec<u8>,
    thumb: Vec<u8>,
}

/// The shot being looked at.
struct Shot {
    range: FrameRange,
    pool: Vec<FrameIdx>,
    /// Next candidate expected.
    next: usize,
    selector: Selector,
    kept: VecDeque<Kept>,
    previous: Option<Vec<f32>>,
    looks: Vec<(FrameIdx, Vec<Found>)>,
    sprite: Option<SpriteCanvas>,
}

/// The models Fast Scan finds faces with: the detector loaded for the
/// media's [`frame_size`].
pub struct FastModels<'a> {
    pub detector: &'a mut YuNet,
    pub embedder: &'a mut SFace,
}

struct Pass<'a, C> {
    params: &'a FastParams,
    models: FastModels<'a>,
    tracker: TrackerParams,
    size: (usize, usize),
    grid: u32,
    steps: u32,
    shots: &'a [FrameRange],
    next_shot: usize,
    current: Option<Shot>,
    chunk: u32,
    chunk_start: FrameIdx,
    chunk_goal: FrameIdx,
    tracks: Vec<FaceTrack>,
    sprites: Vec<ShotSprite>,
    on_chunk: C,
    /// Looks taken, for the log.
    looked: usize,
}

impl<C: FnMut(FaceChunk) -> Result<(), AnalysisError>> Pass<'_, C> {
    /// Take in grid frame `f`: open and close shots and chunks up to it,
    /// paint it into its shot's sprite, and look at it when its shot wants.
    fn take(&mut self, f: FrameIdx, rgb: &[u8]) -> Result<(), AnalysisError> {
        if self.current.as_ref().is_some_and(|c| f >= c.range.end) {
            self.finish_shot()?;
        }
        while self.next_shot < self.shots.len() && self.shots[self.next_shot].start <= f {
            let s = self.shots[self.next_shot];
            self.finish_shot()?;
            if s.start >= self.chunk_goal {
                self.close_chunk(s.start)?;
            }
            if s.end > f {
                let pool = pool(s, self.grid, self.steps);
                let p = self.params;
                self.current = Some(Shot {
                    range: s,
                    selector: Selector::new(pool.len(), p.max_steps(), p),
                    pool,
                    next: 0,
                    kept: VecDeque::new(),
                    previous: None,
                    looks: Vec::new(),
                    sprite: SpriteCanvas::new(s, self.grid, p.sprite_cells, self.size, p.sprite_width),
                });
            }
            self.next_shot += 1;
        }
        let size = self.size;
        let Some(cur) = self.current.as_mut().filter(|c| c.range.contains(f)) else { return Ok(()) };
        if let Some(sprite) = cur.sprite.as_mut() {
            sprite.paint(f, rgb, size);
        }
        // (Candidates before the first frame delivered can't come any more.)
        while cur.pool.get(cur.next).is_some_and(|&p| p < f) {
            cur.next += 1;
        }
        if cur.pool.get(cur.next) != Some(&f) {
            return Ok(());
        }
        let i = cur.next;
        cur.next += 1;
        let picture = compact(rgb, size);
        let changed =
            cur.previous.as_ref().is_some_and(|before| visual_change(before, &picture) > self.params.visual_change);
        cur.previous = Some(picture);
        let look = cur.selector.look(i, changed);
        cur.kept.push_back(Kept { i, frame: f, rgb: rgb.to_vec() });
        while cur.kept.len() > Selector::WINDOW {
            cur.kept.pop_front();
        }
        let Some(why) = look else { return Ok(()) };
        let summary = self.look_at(f, rgb)?;
        if why == Why::Refine {
            return Ok(());
        }
        let cur = self.current.as_mut().expect("a shot is open");
        let again = cur.selector.found(i, summary);
        let back: Vec<(FrameIdx, Vec<u8>)> =
            cur.kept.iter().filter(|k| again.contains(&k.i)).map(|k| (k.frame, k.rgb.clone())).collect();
        for (frame, rgb) in back {
            self.look_at(frame, &rgb)?;
        }
        Ok(())
    }

    /// Find the faces of frame `f` of the current shot; keep their crops and
    /// thumbnails for embedding at the shot's end.
    fn look_at(&mut self, f: FrameIdx, rgb: &[u8]) -> Result<Summary, AnalysisError> {
        let (w, h) = self.size;
        let p = self.params;
        let faces: Vec<FaceBox> = self
            .models
            .detector
            .detect_many(&[rgb], p.det_threshold)?
            .pop()
            .unwrap_or_default()
            .into_iter()
            .filter(|b| b.height() >= p.min_height * h as f32)
            .collect();
        self.looked += 1;
        let summary = Summary::of(&faces, (w, h));
        let found = faces
            .iter()
            .map(|b| Found {
                det: FaceDet {
                    frame: f,
                    bbox: [b.bbox[0] / w as f32, b.bbox[1] / h as f32, b.bbox[2] / w as f32, b.bbox[3] / h as f32],
                    score: b.score,
                    norm: 0.0,
                    quality: 0.0,
                },
                crop: arcface::align(rgb, w, h, &b.kps),
                thumb: image::jpeg(
                    &image::face_thumb(rgb, w, h, b.bbox, p.thumb_context, p.thumb_size),
                    p.thumb_size,
                    p.thumb_size,
                    85,
                ),
            })
            .collect();
        self.current.as_mut().expect("a shot is open").looks.push((f, found));
        Ok(summary)
    }

    /// Embed and track the faces of the current shot, finish its sprite.
    fn finish_shot(&mut self) -> Result<(), AnalysisError> {
        let Some(mut shot) = self.current.take() else { return Ok(()) };
        if let Some(sprite) = shot.sprite.take() {
            self.sprites.push(sprite.finish(self.params.sprite_quality));
        }
        shot.looks.sort_by_key(|(f, _)| *f);
        let crops: Vec<&[u8]> =
            shot.looks.iter().flat_map(|(_, found)| found.iter().map(|x| x.crop.as_slice())).collect();
        let embeddings = if crops.is_empty() { Vec::new() } else { self.models.embedder.embed(&crops)? };
        let mut embeddings = embeddings.into_iter();
        let p = self.params;
        let samples: Vec<(FrameIdx, Vec<Observation>)> = shot
            .looks
            .into_iter()
            .map(|(f, found)| {
                let obs = found
                    .into_iter()
                    .map(|x| {
                        let e = embeddings.next().expect("one embedding per face");
                        let mut det = x.det;
                        det.norm = e.norm;
                        det.quality = p.quality(e.norm, det.height(), det.score);
                        // Tracks keep the full pass's size: the rest stays zero
                        // and leaves every similarity as SFace's own.
                        let mut embedding = e.v;
                        embedding.resize(DIM, 0.0);
                        Observation { det, embedding, thumb: x.thumb }
                    })
                    .collect();
                (f, obs)
            })
            .collect();
        self.tracks.extend(track_shot(shot.range, samples, &self.tracker));
        Ok(())
    }

    fn close_chunk(&mut self, end: FrameIdx) -> Result<(), AnalysisError> {
        (self.on_chunk)(FaceChunk {
            start: self.chunk_start,
            end,
            tracks: &self.tracks,
            sprites: &self.sprites,
            bodies: &[],
        })?;
        self.tracks.clear();
        self.sprites.clear();
        self.chunk_start = end;
        self.chunk_goal = end + self.chunk;
        Ok(())
    }
}

/// [`crate::face_tracks`] the way Fast Scan finds faces: of `src` in
/// `shots` (sorted, disjoint) from frame `resume` on — 0 or a chunk end
/// handed out earlier — each finished chunk to `on_chunk`, chunks tiling
/// `resume..index.len()` and ending at shot starts at least `chunk` frames
/// after their own start (or at the end). Nobody is looked for by their
/// body. `progress` gets 0..1.
#[allow(clippy::too_many_arguments)]
pub fn fast_face_tracks(
    tools: &Tools,
    src: Source,
    models: FastModels,
    shots: &[FrameRange],
    resume: FrameIdx,
    chunk: u32,
    params: &FastParams,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
    on_chunk: impl FnMut(FaceChunk) -> Result<(), AnalysisError>,
) -> Result<(), AnalysisError> {
    assert!(chunk > 0);
    let n = src.index.len();
    if resume >= n {
        return Ok(());
    }
    let size = frame_size(src.info, params);
    if models.detector.frame_size() != size {
        return Err(AnalysisError::Internal(format!(
            "face detector loaded for {:?} frames, this media needs {size:?}",
            models.detector.frame_size()
        )));
    }
    let fps = src.index.nominal_fps.to_f64();
    let grid = params.grid(fps);
    let shots = &shots[shots.partition_point(|s| s.end <= resume)..];
    debug_assert!(shots.first().is_none_or(|s| s.start >= resume), "resume points are shot starts");
    let started = std::time::Instant::now();
    let mut pass = Pass {
        params,
        models,
        tracker: params.tracker(fps),
        size,
        grid,
        steps: params.pool_steps(fps),
        shots,
        next_shot: 0,
        current: None,
        chunk,
        chunk_start: resume,
        chunk_goal: resume + chunk,
        tracks: Vec::new(),
        sprites: Vec::new(),
        on_chunk,
        looked: 0,
    };
    let mut failure: Option<AnalysisError> = None;
    let first = resume - resume % grid;
    let decoded = (size.0 as u32, size.1 as u32);
    let delivered =
        as_media::frames::decode_samples_with(tools, src, first, grid, decoded, params.decoder, cancel, |f, rgb| {
            if f < resume {
                return Ok(true);
            }
            if f % (grid * 80) == 0 {
                progress(f as f64 / n as f64);
            }
            match pass.take(f, rgb) {
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
    let expected = (n - first).div_ceil(grid);
    if delivered != expected {
        return Err(AnalysisError::Media(as_media::MediaError::Verify(format!(
            "fast face pass decoded {delivered} samples from {first}, expected {expected}"
        ))));
    }
    pass.finish_shot()?;
    pass.close_chunk(n)?;
    tracing::debug!(looks = pass.looked, secs = started.elapsed().as_secs_f64(), "fast face pass done");
    progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pool_takes_a_candidate_a_second_and_the_last() {
        // 24 fps: a grid of 6 frames, a candidate every 4 steps.
        let p = FastParams::default();
        assert_eq!((p.grid(24.0), p.pool_steps(24.0)), (6, 4));
        assert_eq!((p.grid(23.976), p.pool_steps(23.976)), (6, 4));
        assert_eq!((p.grid(60.0), p.pool_steps(60.0)), (15, 4));
        assert_eq!(pool(FrameRange::new(10, 100), 6, 4), [12, 36, 60, 84, 96]);
        assert_eq!(pool(FrameRange::new(12, 37), 6, 4), [12, 36]);
        // The last grid frame is never added twice; no grid frame, no pool.
        assert_eq!(pool(FrameRange::new(0, 25), 6, 4), [0, 24]);
        assert_eq!(pool(FrameRange::new(7, 12), 6, 4), Vec::<FrameIdx>::new());
        assert_eq!(pool(FrameRange::new(6, 7), 6, 4), [6]);
    }

    fn calm(faces: usize) -> Summary {
        Summary { faces, min_confidence: 0.98, min_area: 0.05, min_side: 0.2 }
    }

    /// The looks of a shot of `n` candidates, given what each would find
    /// and whether its picture changed.
    fn select(n: usize, found: impl Fn(usize) -> Summary, changed: impl Fn(usize) -> bool) -> Vec<Option<Why>> {
        let mut s = Selector::new(n, FastParams::default().max_steps(), &FastParams::default());
        for i in 0..n {
            if let Some(why) = s.look(i, changed(i))
                && why != Why::Refine
            {
                for j in s.found(i, found(i)) {
                    assert!(j < i && j + Selector::WINDOW > i, "{j} is still kept at {i}");
                }
            }
        }
        s.why
    }

    #[test]
    fn a_calm_shot_is_looked_at_every_two_seconds() {
        use Why::*;
        let why = select(9, |_| calm(1), |_| false);
        assert_eq!(why, [Some(Boundary), None, Some(Gap), None, Some(Midpoint), None, Some(Gap), None, Some(Boundary)]);
        // A short shot: its ends and middle.
        assert_eq!(select(3, |_| calm(1), |_| false), [Some(Boundary), Some(Midpoint), Some(Boundary)]);
        assert_eq!(select(1, |_| calm(0), |_| false), [Some(Boundary)]);
    }

    #[test]
    fn a_change_of_picture_is_looked_at() {
        use Why::*;
        let why = select(9, |_| calm(1), |i| i == 1);
        assert_eq!(why[..4], [Some(Boundary), Some(Change), None, Some(Gap)]);
    }

    #[test]
    fn doubt_has_the_neighbours_looked_at() {
        use Why::*;
        // An unsure face at the middle: the candidates either side too.
        let unsure = |i| if i == 4 { Summary { min_confidence: 0.9, ..calm(1) } } else { calm(1) };
        let why = select(9, unsure, |_| false);
        assert_eq!(why[2..7], [Some(Gap), Some(Refine), Some(Midpoint), Some(Refine), Some(Gap)]);
        // A small face counts as doubt too, by area or by side.
        for small in [Summary { min_area: 0.01, ..calm(1) }, Summary { min_side: 0.07, ..calm(1) }] {
            let why = select(9, |i| if i == 4 { small } else { calm(1) }, |_| false);
            assert_eq!((why[3], why[5]), (Some(Refine), Some(Refine)));
        }
        // No face is no doubt.
        let why = select(9, |i| if i == 4 { Summary { min_confidence: 1.0, ..calm(0) } } else { calm(0) }, |_| false);
        assert_eq!(why.iter().filter(|w| **w == Some(Refine)).count(), 0);
    }

    #[test]
    fn a_new_face_has_both_looks_refined() {
        use Why::*;
        // One face up to candidate 2, two from 4 on: around both looks.
        let why = select(9, |i| calm(if i < 4 { 1 } else { 2 }), |_| false);
        assert_eq!(why[..6], [Some(Boundary), Some(Refine), Some(Gap), Some(Refine), Some(Midpoint), Some(Refine)]);
    }

    #[test]
    fn pictures_are_compared_by_their_blocks() {
        let (w, h) = (128, 72);
        let grey = |v: u8| vec![v; w * h * 3];
        // Brighter all over: no change.
        assert!(visual_change(&compact(&grey(60), (w, h)), &compact(&grey(160), (w, h))) < 1e-4);
        // Half the picture brighter: a change.
        let mut half = grey(60);
        for px in half.as_chunks_mut::<3>().0.iter_mut().take(w * h / 2) {
            *px = [220, 220, 220];
        }
        let d = visual_change(&compact(&grey(60), (w, h)), &compact(&half, (w, h)));
        assert!(d > FastParams::default().visual_change, "{d}");
    }

    #[test]
    fn faces_are_summed_up_by_the_least_of_them() {
        let face = |side: f32, score| FaceBox { bbox: [0.0, 0.0, side, side / 2.0], score, kps: [[0.0; 2]; 5] };
        let s = Summary::of(&[face(100.0, 0.95), face(20.0, 0.88)], (200, 100));
        assert_eq!(s.faces, 2);
        assert_eq!((s.min_confidence, s.min_area, s.min_side), (0.88, 0.1 * 0.1, 0.1));
    }

    #[test]
    fn faces_yunet_is_less_sure_of_count_for_less() {
        let p = FastParams::default();
        assert_eq!((p.det_threshold, p.score_floor, p.score_full), (0.7, 0.7, 0.95));
        let (clear, tall) = (p.norm_full, p.full_height);
        assert_eq!(p.quality(clear, tall, 0.95), 1.0);
        assert!((p.quality(clear, tall, 0.825) - 0.5).abs() < 1e-6);
        assert_eq!(p.quality(clear, tall, 0.7), 0.0);
        // Off when the two scores are equal.
        let off = FastParams { score_full: 0.7, ..p };
        assert_eq!(off.quality(clear, tall, 0.75), 1.0);
    }

    #[test]
    fn the_tracker_follows_faces_over_the_gaps_between_looks() {
        let p = FastParams::default();
        let t = p.tracker(24.0);
        assert_eq!((t.stride, t.max_gap, t.reach_gap), (6, 8, 8));
        assert_eq!(t.min_track_quality, p.min_track_quality);
        // At any frame rate, as far as two looks may lie apart.
        for fps in [5.0, 9.0, 23.976, 25.0, 29.97, 50.0, 59.94] {
            let t = p.tracker(fps);
            assert_eq!(t.max_gap, p.max_steps() as u32 * p.pool_steps(fps), "{fps} fps");
        }
    }
}
