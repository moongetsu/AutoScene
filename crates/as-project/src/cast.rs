//! Faces and characters: the face pass's checkpoints (tracks plus thumbnail
//! files), clustering tracks into the project's cast, and every edit the
//! user makes to it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use as_analysis::FaceParams;
use as_analysis::tracker::TrackerParams;
use as_analysis::tracks::{self, FaceTrack};
use as_analysis::{FaceDet, ShotSprite, portrait_region};
use as_api::{
    Cast, CastEdit, Character, ContentKind, FaceTrackItem, MediaInfo, MediaStatus, Presence, SegmentKind, StageState,
    TrackAssign, TrackFilter,
};
use as_core::{CancelToken, CharacterId, FrameIdx, FrameRange, MediaId};
use as_db::{EditKind, IdentityTrack, NewTrack, TrackRow, TrackUpdate};
use as_identity::{Assign, CharKey, Outcome, Params, Space, TrackIn, identify};
use as_media::poster::{StillSize, extract_frame_jpegs};
use as_media::{Source, Tools};
use parking_lot::Mutex;

use crate::{Project, ProjectError, ProjectResult};

/// Frames per face checkpoint chunk (closed at the next shot start).
pub const FACE_CHUNK: u32 = 2400;
/// Character portraits, 3:4: sharp in the biggest places a face is shown at
/// 150 % display scaling — the review's 300 px square and a cast card (up to
/// about 240×320) — and in the 176×235 cards of the Overview. Thumbnails
/// (160 px squares from the face pass) serve lists and avatars.
pub const PORTRAIT: (u32, u32) = (450, 600);
/// Tracks pinned when a character is named, so it keeps its identity.
const NAME_ANCHORS: usize = 5;
/// Clusterings in a row that may find the cast changed by the user before
/// one is stored.
const CLUSTER_ATTEMPTS: usize = 8;
/// A track of a new face pass as decisions are re-anchored to it: id,
/// range, detections and (when readable) its embedding.
type NewFace = (i64, FrameRange, Vec<FaceDet>, Option<Vec<f32>>);
/// After the media's file was replaced by one not shown to be the same
/// picture, a decision only goes back onto a new track that looks this much
/// like the face it was about (same embedder): frames can shift between two
/// files, and someone else may sit where the face was. A new pass over the
/// same file goes by place alone (a blurry face the tracker cut differently
/// keeps its decision).
const REANCHOR_MIN_COS: f32 = 0.4;

/// The `meta` key of the [`as_identity::VERSION`] the cast was last grouped
/// with.
pub(crate) const GROUPED_WITH: &str = "identity_version";
/// The `meta` key of the face space the cast was last grouped in
/// ([`as_identity::Space`], hex): decisions place faces in the same one.
const GROUPED_IN: &str = "identity_space";

/// Set (in `meta`) while a media's decisions wait for faces from another
/// file than the one they were made on.
pub(crate) fn source_changed_key(id: MediaId) -> String {
    format!("faces_must_match:{id}")
}

/// Where a track's portrait goes and what it shows.
struct PortraitPlan {
    track: i64,
    media: MediaId,
    det: FaceDet,
    path: PathBuf,
}

/// What [`Project::portrait`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Portrait {
    /// Made just now.
    Made(PathBuf),
    /// Made before.
    Ready(PathBuf),
    /// No such track (any more), or it can't be made now: its media is
    /// missing or being indexed, or making it failed this session.
    Unavailable,
}

impl Portrait {
    pub fn path(self) -> Option<PathBuf> {
        match self {
            Self::Made(p) | Self::Ready(p) => Some(p),
            Self::Unavailable => None,
        }
    }
}

/// Portraits being made (one decode per file at a time; others go on
/// meanwhile) and ones that failed this session (not tried again until the
/// next launch).
#[derive(Default)]
pub(crate) struct PortraitWork {
    making: HashMap<PathBuf, Arc<Mutex<()>>>,
    failed: HashSet<PathBuf>,
}

/// A short tag of a detection box: two faces in the same frame get
/// different names (FNV-1a of its coordinates).
fn box_tag(bbox: [f32; 4]) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in bbox.iter().flat_map(|v| v.to_le_bytes()) {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    format!("{:06x}", h & 0xff_ffff)
}

/// What the engine needs to find a media's faces.
#[derive(Clone, Debug)]
pub struct FacesPlan {
    pub path: PathBuf,
    pub info: MediaInfo,
    /// `FrameIndex::to_bytes`.
    pub index: Vec<u8>,
    pub shots: Vec<(FrameIdx, FrameIdx)>,
    pub frames: u32,
    /// First frame without a checkpoint (`frames` = done).
    pub resume: FrameIdx,
    pub chunk: u32,
}

fn mean_box(dets: &[FaceDet]) -> [f32; 4] {
    let n = dets.len().max(1) as f32;
    let mut b = [0f32; 4];
    for d in dets {
        for (x, v) in b.iter_mut().zip(d.bbox) {
            *x += v / n;
        }
    }
    b
}

/// The mean box of `dets` in `range`, or of the one nearest to it when none
/// is (a track bridging a gap).
fn box_in(dets: &[FaceDet], range: FrameRange) -> Option<[f32; 4]> {
    let inside: Vec<FaceDet> = dets.iter().filter(|d| range.contains(d.frame)).copied().collect();
    if !inside.is_empty() {
        return Some(mean_box(&inside));
    }
    let mid = range.start + range.len() / 2;
    dets.iter().min_by_key(|d| d.frame.abs_diff(mid)).map(|d| d.bbox)
}

/// Whether an old track (a decision's) and a new one are the same face: they
/// share most of the shorter of the two in time, and the face is at the same
/// place on screen during the frames they share — so a face the tracker now
/// follows in one piece, or in two, is found again even while it moves.
/// Returns the frames they share.
fn same_face(old: (FrameRange, &[FaceDet]), new: (FrameRange, &[FaceDet])) -> Option<u32> {
    let both = old.0.intersect(&new.0)?;
    if 2 * both.len() < old.0.len().min(new.0.len()) {
        return None;
    }
    let (a, b) = (box_in(old.1, both)?, box_in(new.1, both)?);
    (box_iou(&a, &b) >= 0.5).then_some(both.len())
}

/// The decisions found again on new tracks (`(frames shared, track,
/// decision)`) made consistent, one track at a time: of "this is X" and
/// "not a face" the one sharing most frames holds — "this is X" only if no
/// "not X" shares more; the other "not Y"s hold; with neither left, the
/// track starts out on the character clustering said it was (sharing most,
/// and not one it is "not").
fn resolve(mut found: Vec<(u32, i64, EditKind)>) -> Vec<(i64, EditKind)> {
    found.sort_by_key(|&(shared, track, _)| (track, std::cmp::Reverse(shared)));
    let mut out = Vec::new();
    for group in found.chunk_by(|a, b| a.1 == b.1) {
        let track = group[0].1;
        let forbids: Vec<(u32, CharacterId)> = group
            .iter()
            .filter_map(|&(s, _, k)| if let EditKind::Forbid(c) = k { Some((s, c)) } else { None })
            .collect();
        let forbidden = |c: CharacterId, shared: u32| forbids.iter().any(|&(s, f)| f == c && s > shared);
        // Most frames first (sorted above).
        let decided = group.iter().find(|(_, _, k)| matches!(k, EditKind::Pin(_) | EditKind::NotFace));
        let decided = decided.filter(|&&(s, _, k)| !matches!(k, EditKind::Pin(c) if forbidden(c, s)));
        out.extend(
            forbids
                .iter()
                .filter(|&&(_, c)| !matches!(decided, Some(&(_, _, EditKind::Pin(p))) if p == c))
                .map(|&(_, c)| (track, EditKind::Forbid(c))),
        );
        out.extend(group.iter().filter(|e| matches!(e.2, EditKind::Cover(_))).map(|&(_, _, k)| (track, k)));
        match decided {
            Some(&(_, _, k)) => out.push((track, k)),
            None => {
                let was = group
                    .iter()
                    .find(|&&(_, _, k)| matches!(k, EditKind::Was(c) if !forbids.iter().any(|&(_, f)| f == c)));
                if let Some(&(_, _, k)) = was {
                    out.push((track, k));
                }
            }
        }
    }
    out
}

/// How much other faces take of each track's portrait (see
/// [`as_analysis::crowding`]), among `tracks` — every face of some shots:
/// the faces of a frame are all in its shot.
fn rivals(tracks: &[&[FaceDet]], frame: (u32, u32)) -> Vec<f32> {
    let mut at: HashMap<FrameIdx, Vec<(usize, [f32; 4])>> = HashMap::new();
    for (i, dets) in tracks.iter().enumerate() {
        for d in dets.iter() {
            at.entry(d.frame).or_default().push((i, d.bbox));
        }
    }
    let (w, h) = PORTRAIT;
    tracks
        .iter()
        .enumerate()
        .map(|(i, dets)| {
            let Some(d) = tracks::clearest(dets) else { return 0.0 };
            let others = at.get(&d.frame).into_iter().flatten().filter(|(j, _)| *j != i).map(|(_, b)| *b);
            as_analysis::crowding(d.bbox, others, frame, w as f32 / h as f32)
        })
        .collect()
}

pub(crate) fn box_iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// The project's tracks as the identity step takes them.
pub(crate) struct IdentityInput {
    pub(crate) tracks: Vec<TrackIn>,
    /// Per input track, its row (rows without a readable embedding are left out).
    pub(crate) rows: Vec<usize>,
    /// Existing characters; a character's key is its index here.
    existing: Vec<CharacterId>,
    pub(crate) key: HashMap<CharacterId, CharKey>,
    /// Each media's number in `tracks`.
    pub(crate) media: HashMap<MediaId, u32>,
}

impl IdentityInput {
    fn new(rows: &[IdentityTrack], existing: Vec<CharacterId>) -> Self {
        let face = FaceParams::default();
        let max_gap = TrackerParams::new(face.stride).max_gap;
        let key: HashMap<CharacterId, CharKey> = existing.iter().enumerate().map(|(i, c)| (*c, i as CharKey)).collect();
        let mut media: HashMap<MediaId, u32> = HashMap::new();
        let mut tracks = Vec::with_capacity(rows.len());
        let mut kept = Vec::with_capacity(rows.len());
        for (row, r) in rows.iter().enumerate() {
            // An unreadable face (or one with no number in it) groups with no one.
            let Some(embedding) = tracks::embedding_from_bytes(&r.emb).filter(|e| e.iter().all(|x| x.is_finite()))
            else {
                continue;
            };
            let m = media.len() as u32;
            let m = *media.entry(r.media_id).or_insert(m);
            let assigned = r.character_id.and_then(|c| key.get(&c).copied());
            let dets = tracks::dets_from_bytes(&r.det_bytes).unwrap_or_default();
            let spans =
                if dets.is_empty() { Vec::new() } else { tracks::presence_spans(&dets, r.end, face.stride, max_gap) };
            let mut seen: Vec<u32> = dets.iter().map(|d| d.frame).collect();
            seen.sort_unstable();
            seen.dedup();
            tracks.push(TrackIn {
                media: m,
                start: r.start,
                end: r.end,
                spans,
                seen,
                embedding,
                quality: r.quality,
                dets: r.dets,
                pin: (r.assign == TrackAssign::Pinned).then_some(assigned).flatten(),
                forbid: r.forbids.iter().filter_map(|c| key.get(c).copied()).collect(),
                // A face named by its body isn't who clustering saw in it.
                previous: (matches!(r.assign, TrackAssign::Auto | TrackAssign::Pinned) && !r.via_body)
                    .then_some(assigned)
                    .flatten(),
            });
            kept.push(row);
        }
        Self { tracks, rows: kept, existing, key, media }
    }
}

fn hex(b: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    b.iter().flat_map(|x| [DIGITS[(x >> 4) as usize] as char, DIGITS[(x & 15) as usize] as char]).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

pub(crate) fn track_id(id: i64) -> u32 {
    u32::try_from(id).unwrap_or(u32::MAX)
}

impl Project {
    /// What the face pass made, one folder per media: face thumbnails, and
    /// shot sprites in `shots/`.
    pub fn faces_dir(&self) -> PathBuf {
        self.dir.join("faces")
    }

    fn media_faces_dir(&self, media: MediaId) -> PathBuf {
        self.faces_dir().join(media.to_string())
    }

    pub(crate) fn thumb_path(&self, media: MediaId, track: i64) -> PathBuf {
        self.media_faces_dir(media).join(format!("{track}.jpg"))
    }

    /// Named by the shot's start and end: a shot found again differently is
    /// a new file, never a stale one from the webview's cache.
    pub(crate) fn sprite_path(&self, media: MediaId, shot: FrameIdx, end: FrameIdx) -> PathBuf {
        self.media_faces_dir(media).join("shots").join(format!("{shot}-{end}.jpg"))
    }

    /// Named by the frame and the box it shows and by its size: a track
    /// number handed out again by a new face pass, or portraits of a new
    /// size, are new files, never stale ones from the webview's cache.
    fn portrait_path(&self, media: MediaId, track: i64, det: &FaceDet) -> PathBuf {
        let (w, h) = PORTRAIT;
        let frame = det.frame;
        self.media_faces_dir(media).join(format!("{track}-{frame}-{}-{w}x{h}.jpg", box_tag(det.bbox)))
    }

    /// The portraits of `tracks` (those that exist): each shows the track's
    /// clearest face, the moment its thumbnail shows.
    fn portrait_plans(&self, tracks: &[i64]) -> ProjectResult<Vec<PortraitPlan>> {
        Ok(self
            .db()
            .track_dets(tracks)?
            .into_iter()
            .filter_map(|(track, media, dets)| {
                let det = *tracks::clearest(&tracks::dets_from_bytes(&dets)?)?;
                Some(PortraitPlan { track, media, det, path: self.portrait_path(media, track, &det) })
            })
            .collect())
    }

    /// Character covers that have no portrait yet and can get one now (their
    /// media is indexed and its file is there; it didn't fail this session),
    /// in the order characters were found.
    pub fn missing_portraits(&self) -> ProjectResult<Vec<i64>> {
        let covers = self.db().cover_tracks()?;
        let ready: HashSet<MediaId> =
            self.media_list_uncounted()?.into_iter().filter(|m| m.status == MediaStatus::Ready).map(|m| m.id).collect();
        let plans: HashMap<i64, PortraitPlan> =
            self.portrait_plans(&covers)?.into_iter().map(|p| (p.track, p)).collect();
        let failed = self.portraits.lock().failed.clone();
        Ok(covers
            .into_iter()
            .filter(|t| {
                plans.get(t).is_some_and(|p| ready.contains(&p.media) && !failed.contains(&p.path) && !p.path.is_file())
            })
            .collect())
    }

    /// The portrait of `track`, made from the exact frame of its clearest
    /// face unless it exists. One that fails isn't tried again this session.
    pub fn portrait(&self, tools: &Tools, track: i64, cancel: &CancelToken) -> ProjectResult<Portrait> {
        let Some(plan) = self.portrait_plans(&[track])?.pop() else { return Ok(Portrait::Unavailable) };
        if plan.path.is_file() {
            return Ok(Portrait::Ready(plan.path));
        }
        if self.portraits.lock().failed.contains(&plan.path) {
            return Ok(Portrait::Unavailable);
        }
        let (m, info, index) = match self.ready_source(plan.media) {
            Ok(source) => source,
            Err(ProjectError::NotFound(_) | ProjectError::NotReady(_)) => return Ok(Portrait::Unavailable),
            Err(e) => return Err(e),
        };
        // Detection boxes are fractions of the upright, square-pixel frame.
        // A file indexed again may have fewer frames than its faces know.
        let (w, h) = PORTRAIT;
        let frame = as_media::frames::sample_size(&info, (u32::MAX, u32::MAX), 2);
        let region = (plan.det.frame < index.len()).then(|| portrait_region(plan.det.bbox, frame, w as f32 / h as f32));
        let Some(region) = region.flatten() else {
            self.portraits.lock().failed.insert(plan.path);
            return Ok(Portrait::Unavailable);
        };

        // One decode per file; other portraits are made meanwhile.
        let slot = self.portraits.lock().making.entry(plan.path.clone()).or_default().clone();
        let made = {
            let _one = slot.lock();
            if plan.path.is_file() {
                // Made while this one waited.
                Ok(false)
            } else {
                let path = PathBuf::from(&m.path);
                let src = Source { path: &path, info: &info, index: &index };
                let outs = [(StillSize::Region(region, w, h), plan.path.as_path())];
                extract_frame_jpegs(tools, src, plan.det.frame, &outs, cancel).map(|()| true)
            }
        };
        {
            let mut work = self.portraits.lock();
            // The map's copy and this one: nobody else waits for this file.
            if Arc::strong_count(&slot) == 2 {
                work.making.remove(&plan.path);
            }
        }
        // The track may have gone meanwhile (its faces found again, its media
        // removed): its folder may have been made again just for this file.
        let current = self.portrait_plans(&[track])?.pop().is_some_and(|p| p.path == plan.path);
        match made {
            Ok(_) if !current => {
                let _ = std::fs::remove_file(&plan.path);
                let _ = std::fs::remove_dir(self.media_faces_dir(plan.media));
                Ok(Portrait::Unavailable)
            }
            Ok(true) => {
                self.remove_other_portraits(plan.media, plan.track, &plan.path);
                Ok(Portrait::Made(plan.path))
            }
            Ok(false) => Ok(Portrait::Ready(plan.path)),
            Err(e) => {
                if current && !e.is_cancelled() {
                    self.portraits.lock().failed.insert(plan.path);
                }
                Err(e.into())
            }
        }
    }

    /// Delete portraits of `track` other than `keep` (of an earlier size).
    fn remove_other_portraits(&self, media: MediaId, track: i64, keep: &Path) {
        let prefix = format!("{track}-");
        let Ok(entries) = std::fs::read_dir(self.media_faces_dir(media)) else { return };
        for e in entries.flatten() {
            let (path, name) = (e.path(), e.file_name());
            let name = name.to_string_lossy();
            if name.starts_with(&prefix) && name.ends_with(".jpg") && path != keep {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// The upright, square-pixel size of `id`'s frames (detection boxes are
    /// fractions of it); 16:9 while it isn't known.
    fn frame_size(&self, id: MediaId) -> ProjectResult<(u32, u32)> {
        let info = self.db().media(id)?.and_then(|m| m.info);
        Ok(info.map_or((16, 9), |info| as_media::frames::sample_size(&info, (u32::MAX, u32::MAX), 2)))
    }

    /// Look at the portraits of faces found before that was stored for how
    /// much other faces take of them (see [`rivals`]). Returns how many.
    pub fn fill_rivals(&self, cancel: &CancelToken) -> ProjectResult<usize> {
        let mut n = 0;
        for id in self.db().media_without_rivals()? {
            cancel.check().map_err(|_| ProjectError::Cancelled)?;
            let rows = self.db().media_track_dets(id)?;
            let dets: Vec<Vec<FaceDet>> =
                rows.iter().map(|(_, _, _, d, _)| tracks::dets_from_bytes(d).unwrap_or_default()).collect();
            let rival = rivals(&dets.iter().map(Vec::as_slice).collect::<Vec<_>>(), self.frame_size(id)?);
            let found: Vec<(i64, f32)> = rows.iter().map(|r| r.0).zip(rival).collect();
            self.db().set_rivals(&found)?;
            n += found.len();
        }
        Ok(n)
    }

    /// Whether the cast should be gone over on the upkeep worker (see
    /// [`Self::cast_upkeep`]).
    pub fn cast_needs_upkeep(&self) -> ProjectResult<bool> {
        Ok(self.grouped_by_older_version()? || !self.db().media_without_rivals()?.is_empty())
    }

    /// Bring a cast made by an older version up to date: faces whose
    /// portraits weren't looked at for other faces are, and a cast grouped
    /// by an older version of the grouping is grouped again (the user's
    /// decisions stay). Returns the number of characters when it was
    /// grouped again.
    pub fn cast_upkeep(&self, cancel: &CancelToken) -> ProjectResult<Option<usize>> {
        let looked = self.fill_rivals(cancel)?;
        if looked > 0 {
            tracing::info!(project = %self.id, tracks = looked, "portraits looked at for other faces");
        }
        cancel.check().map_err(|_| ProjectError::Cancelled)?;
        if self.grouped_by_older_version()? { self.recluster().map(Some) } else { Ok(None) }
    }

    // -- face pass ----------------------------------------------------------

    /// Where the face pass of `id` with embedder `model` resumes.
    /// Checkpoints of another embedder, or that don't line up, are dropped
    /// with everything after them; a fresh start clears old thumbnails.
    pub fn faces_plan(&self, id: MediaId, model: &str) -> ProjectResult<FacesPlan> {
        let (m, info, index) = self.ready_source(id)?;
        if m.shots != StageState::Ready {
            return Err(ProjectError::NotReady("Shots of this media aren't found yet.".into()));
        }
        if self.db().use_faces_model(id, model)? {
            tracing::info!(file = %m.file_name, model, "faces are found with a new model");
        }
        let frames = index.len();
        let shots: Vec<(FrameIdx, FrameIdx)> =
            self.segments(id)?.into_iter().filter(|s| s.kind == SegmentKind::Shot).map(|s| (s.start, s.end)).collect();
        let mut resume = 0;
        for (start, end) in self.db().face_chunks(id)? {
            // Chunks end at a shot start (or the end of the media).
            let fits = start == resume
                && end > start
                && end <= frames
                && (end == frames || shots.binary_search_by_key(&end, |s| s.0).is_ok());
            if !fits {
                tracing::info!(file = %m.file_name, start, "dropping face checkpoints that don't line up");
                self.db().drop_face_chunks_from(id, start)?;
                break;
            }
            resume = end;
        }
        if resume == 0 {
            let _ = std::fs::remove_dir_all(self.media_faces_dir(id));
        }
        Ok(FacesPlan {
            path: PathBuf::from(&m.path),
            info,
            index: index.to_bytes(),
            shots,
            frames,
            resume,
            chunk: FACE_CHUNK,
        })
    }

    /// Store one chunk from the engine (tracks as `as_analysis::tracks::encode`,
    /// the sprites of its shots, and — when people were looked for — the
    /// people found, `as_analysis::bodies::encode`; empty when they weren't):
    /// tracks in the database, thumbnails and sprites as files, atomically,
    /// then the people. Returns the number of tracks.
    pub fn store_face_chunk(
        &self,
        id: MediaId,
        start: FrameIdx,
        end: FrameIdx,
        blob: &[u8],
        sprites: &[ShotSprite],
        bodies: &[u8],
    ) -> ProjectResult<usize> {
        let found: Vec<FaceTrack> =
            tracks::decode(blob).ok_or_else(|| ProjectError::Invalid("unreadable face tracks".into()))?;
        let rival = rivals(&found.iter().map(|t| t.dets.as_slice()).collect::<Vec<_>>(), self.frame_size(id)?);
        let rows: Vec<NewTrack> = found
            .iter()
            .zip(rival)
            .map(|(t, rival)| NewTrack {
                start: t.start,
                end: t.end,
                shot: t.shot,
                quality: t.quality,
                face_h: t.face_height(),
                dets: tracks::dets_to_bytes(&t.dets),
                emb: tracks::embedding_to_bytes(&t.embedding),
                rival,
            })
            .collect();
        let sprites: Vec<&ShotSprite> = sprites.iter().filter(|s| (start..end).contains(&s.shot)).collect();
        let cells: Vec<(FrameIdx, FrameIdx, u32)> = sprites.iter().map(|s| (s.shot, s.end, s.cells)).collect();
        let shots_dir = self.media_faces_dir(id).join("shots");
        let replaced = self.db().chunk_tracks(id, start, end)?;
        let ids = self.db().put_face_chunk(id, start, end, &rows, &cells, |ids| {
            std::fs::create_dir_all(&shots_dir).map_err(|e| format!("{}: {e}", shots_dir.display()))?;
            for (track, t) in ids.iter().zip(&found) {
                let path = self.thumb_path(id, *track);
                std::fs::write(&path, &t.thumb).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            for s in &sprites {
                let path = self.sprite_path(id, s.shot, s.end);
                std::fs::write(&path, &s.jpeg).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            Ok(())
        })?;
        self.remove_track_pictures(id, &replaced);
        // The people come after: a chunk stored without them (a crash in
        // between) is looked at again by the people pass.
        if !bodies.is_empty() {
            self.store_chunk_bodies(id, start, end, bodies, &ids)?;
        }
        Ok(found.len())
    }

    /// The thumbnails and portraits of tracks that are gone (their ids are
    /// never handed out again, so this only frees the disk).
    fn remove_track_pictures(&self, media: MediaId, tracks: &[i64]) {
        if tracks.is_empty() {
            return;
        }
        let gone: HashSet<String> = tracks.iter().map(i64::to_string).collect();
        let Ok(entries) = std::fs::read_dir(self.media_faces_dir(media)) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // `<track>.jpg` and `<track>-<frame>-<box>-<size>.jpg`.
            let track = name.split(['.', '-']).next().unwrap_or_default();
            if gone.contains(track) && entry.path().is_file() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    pub fn set_faces_state(&self, id: MediaId, state: StageState, error: Option<&str>) -> ProjectResult<()> {
        Ok(self.db().set_faces_state(id, state, error)?)
    }

    /// The face pass of `id` is complete: put the user's earlier decisions
    /// back on the new tracks (see [`same_face`] and [`resolve`]), then mark
    /// the faces ready.
    pub fn finish_faces(&self, id: MediaId) -> ProjectResult<()> {
        let edits = self.db().track_edits(id)?;
        let changed = source_changed_key(id);
        let must_match = self.db().meta(&changed)?.is_some();
        if !edits.is_empty() {
            let model = self.db().faces_model(id)?;
            let new: Vec<NewFace> = self
                .db()
                .media_track_dets(id)?
                .into_iter()
                .filter_map(|(t, s, e, d, emb)| {
                    Some((t, FrameRange::new(s, e), tracks::dets_from_bytes(&d)?, tracks::embedding_from_bytes(&emb)))
                })
                .collect();
            let mut found: Vec<(u32, i64, EditKind)> = Vec::new();
            let mut kept = 0;
            for e in &edits {
                let span = FrameRange::new(e.start, e.end);
                let dets = tracks::dets_from_bytes(&e.dets).unwrap_or_default();
                let before = found.len();
                // The face it was about, when the file changed and the same
                // embedder made both.
                let face = match (&e.emb, &e.model) {
                    (Some(emb), Some(m)) if must_match && Some(m) == model.as_ref() => {
                        tracks::embedding_from_bytes(emb)
                    }
                    _ => None,
                };
                // Another file, and no face to compare with (another model,
                // a decision older than kept faces): what the analysis gave
                // isn't put back by place alone; the user's own answers are.
                if must_match && face.is_none() && matches!(e.kind, EditKind::Was(_) | EditKind::Cover(_)) {
                    continue;
                }
                let looks = |emb: &Option<Vec<f32>>| match (&face, emb) {
                    (Some(a), Some(b)) => a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() >= REANCHOR_MIN_COS,
                    _ => true,
                };
                let matches = new
                    .iter()
                    .filter(|(_, _, _, emb)| looks(emb))
                    .filter_map(|(t, r, d, _)| same_face((span, &dets), (*r, d)).map(|s| (s, *t, e.kind)));
                if matches!(e.kind, EditKind::Cover(_)) {
                    // A picture is one face: the new track sharing most of it.
                    found.extend(matches.max_by_key(|&(s, t, _)| (s, std::cmp::Reverse(t))));
                } else {
                    found.extend(matches);
                }
                kept += usize::from(found.len() > before);
            }
            let matched = resolve(found);
            tracing::info!(media = %id, edits = edits.len(), kept, tracks = matched.len(), "re-anchored cast edits");
            self.db().reanchor_edits(id, &matched)?;
        }
        if must_match {
            self.db().delete_meta(&changed)?;
        }
        Ok(self.db().set_faces_state(id, StageState::Ready, None)?)
    }

    /// Media whose faces another embedder than `model` found, untouched.
    pub fn faces_by_other_model(&self, model: &str) -> ProjectResult<Vec<MediaId>> {
        Ok(self.db().faces_by_other_model(model)?)
    }

    /// Media whose faces another embedder than `model` found: their faces
    /// are forgotten (decisions are kept) so they are found again.
    pub fn refresh_outdated_faces(&self, model: &str) -> ProjectResult<Vec<MediaId>> {
        let ids = self.db().outdated_faces(model)?;
        for &id in &ids {
            let _ = std::fs::remove_dir_all(self.media_faces_dir(id));
        }
        Ok(ids)
    }

    /// Forget the faces of `id` (decisions are kept for the next pass).
    pub fn reset_faces(&self, id: MediaId) -> ProjectResult<()> {
        self.db().reset_faces(id)?;
        let _ = std::fs::remove_dir_all(self.media_faces_dir(id));
        Ok(())
    }

    /// Media whose shots are found but whose faces aren't, in episode order.
    pub fn pending_faces(&self) -> ProjectResult<Vec<MediaId>> {
        Ok(self.db().pending_faces()?)
    }

    // -- identity -----------------------------------------------------------

    /// Group every track of the project into characters, keeping characters
    /// (names, edits) stable. Returns the number of characters. A decision
    /// the user makes meanwhile is never overwritten: the clustering is
    /// computed again.
    pub fn recluster(&self) -> ProjectResult<usize> {
        let _one = self.clustering.lock();
        for attempt in 1..=CLUSTER_ATTEMPTS {
            if let Some(n) = self.cluster_once()? {
                return Ok(n);
            }
            tracing::info!(attempt, "the cast changed while clustering; clustering again");
        }
        Err(ProjectError::NotReady("The cast kept changing while it was being grouped. Try again.".into()))
    }

    /// One clustering, stored if the cast didn't change meanwhile.
    fn cluster_once(&self) -> ProjectResult<Option<usize>> {
        // The revision first: anything decided after it makes it stale.
        let rev = self.db().cast_rev()?;
        let rows: Vec<IdentityTrack> = self.db().identity_tracks()?;
        let cast = IdentityInput::new(&rows, self.db().characters()?.into_iter().map(|c| c.id).collect());
        let started = std::time::Instant::now();
        let out = identify(&cast.tracks, &self.identity_params()?);
        let people = self.people_outcome(&rows, &cast, &out)?;
        // With it, what it was made with: decisions place faces in its space.
        let (space, version) = (hex(&out.space.to_bytes()), as_identity::VERSION.to_string());
        let made_with = [(GROUPED_IN, space.as_str()), (GROUPED_WITH, version.as_str())];
        let Some(characters) = self.store_identity(rev, &rows, &cast, &out, people, &made_with)? else {
            return Ok(None);
        };
        let ms = started.elapsed().as_millis() as u64;
        tracing::info!(tracks = cast.tracks.len(), characters, ms, "cast clustered");
        Ok(Some(characters))
    }

    /// Whether the cast was grouped by an older way of grouping (see
    /// [`as_identity::VERSION`]) and should be grouped again.
    pub fn grouped_by_older_version(&self) -> ProjectResult<bool> {
        let (_, _, media_with_faces, _) = self.db().cast_counts()?;
        // Unknown (grouped before versions were kept): the first version. A
        // newer version's grouping is never undone by an older one.
        let grouped = self.db().meta(GROUPED_WITH)?.and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
        Ok(media_with_faces > 0 && grouped < as_identity::VERSION)
    }

    /// The cast was grouped for other footage (the user said what it is
    /// after it was grouped): it counts as grouped by no version, so the
    /// next upkeep groups it again, also after a restart.
    pub(crate) fn cast_grouped_for_other_footage(&self) -> ProjectResult<()> {
        Ok(self.db().set_meta(GROUPED_WITH, "0")?)
    }

    /// The grouping's parameters for this project's footage.
    fn identity_params(&self) -> ProjectResult<Params> {
        Ok(Params { animated: self.content()? == ContentKind::Animation, ..Params::default() })
    }

    /// The cast after a decision: every character keeps its members (pins
    /// and the tracks clustered into it) and every other face goes to whom
    /// it resembles now ([`as_identity::reassign`]) — at once, without
    /// grouping everything again. Like [`Self::recluster`], a decision made
    /// meanwhile is never overwritten.
    pub fn reassign(&self) -> ProjectResult<()> {
        let _one = self.clustering.lock();
        for attempt in 1..=CLUSTER_ATTEMPTS {
            if self.reassign_once()? {
                return Ok(());
            }
            tracing::info!(attempt, "the cast changed while placing faces; placing them again");
        }
        Err(ProjectError::NotReady("The cast kept changing while it was being grouped. Try again.".into()))
    }

    fn reassign_once(&self) -> ProjectResult<bool> {
        let rev = self.db().cast_rev()?;
        let rows: Vec<IdentityTrack> = self.db().identity_tracks()?;
        let cast = IdentityInput::new(&rows, self.db().characters()?.into_iter().map(|c| c.id).collect());
        let mut members: Vec<(CharKey, Vec<usize>)> =
            (0..cast.existing.len()).map(|k| (k as CharKey, Vec::new())).collect();
        for (i, &row) in cast.rows.iter().enumerate() {
            let r = &rows[row];
            // A clustered track has a score of exactly 1, like a pin; a face
            // named by its body is never one.
            let member =
                r.assign == TrackAssign::Pinned || (r.assign == TrackAssign::Auto && r.score >= 1.0 && !r.via_body);
            if let (true, Some(k)) = (member, r.character_id.and_then(|c| cast.key.get(&c))) {
                members[*k as usize].1.push(i);
            }
        }
        let started = std::time::Instant::now();
        // In the space the cast was grouped in (unknown: fitted again).
        let space = self.db().meta(GROUPED_IN)?.and_then(|h| unhex(&h)).and_then(|b| Space::from_bytes(&b));
        let out = as_identity::reassign(&cast.tracks, &members, &self.identity_params()?, space.as_ref());
        let people = self.people_outcome(&rows, &cast, &out)?;
        if self.store_identity(rev, &rows, &cast, &out, people, &[])?.is_none() {
            return Ok(false);
        }
        tracing::info!(tracks = cast.tracks.len(), ms = started.elapsed().as_millis() as u64, "faces placed again");
        Ok(true)
    }

    /// Store an identity outcome made at cast revision `rev`, new characters
    /// included, and with `people` who the bodies are (and the faces they
    /// named), and `meta` in the same write. The number of characters, or
    /// `None` when the cast changed since (nothing is stored then).
    fn store_identity(
        &self,
        rev: i64,
        rows: &[IdentityTrack],
        cast: &IdentityInput,
        out: &Outcome,
        people: Option<(Vec<as_db::IdentityBody>, as_identity::bodies::BodyOutcome)>,
        meta: &[(&str, &str)],
    ) -> ProjectResult<Option<usize>> {
        let ids: Vec<CharacterId> =
            out.characters.iter().map(|c| c.key.map_or_else(CharacterId::new, |k| cast.existing[k as usize])).collect();
        let new: Vec<CharacterId> =
            out.characters.iter().zip(&ids).filter(|(c, _)| c.key.is_none()).map(|(_, id)| *id).collect();
        let named: HashMap<usize, usize> =
            people.as_ref().map(|(_, o)| o.promoted.iter().copied().collect()).unwrap_or_default();
        let updates: Vec<TrackUpdate> = cast
            .rows
            .iter()
            .zip(&out.tracks)
            .enumerate()
            .map(|(i, (&row, &(a, score)))| {
                let id = rows[row].id;
                let (character, assign) = match a {
                    Assign::Auto(c) => (Some(ids[c]), TrackAssign::Auto),
                    Assign::Pinned(c) => (Some(ids[c]), TrackAssign::Pinned),
                    Assign::Unsure(c) => (Some(ids[c]), TrackAssign::Unsure),
                    Assign::None => (None, TrackAssign::None),
                };
                match named.get(&i) {
                    // Named by its body: never a member that shapes the character.
                    Some(&c) => TrackUpdate {
                        id,
                        character: Some(ids[c]),
                        assign: TrackAssign::Auto,
                        score: score.min(0.99),
                        via_body: true,
                    },
                    None => TrackUpdate { id, character, assign, score, via_body: false },
                }
            })
            .collect();
        let bodies = people.as_ref().map(|(b, o)| crate::bodies::body_updates(b, o, &ids)).unwrap_or_default();
        let joined: Vec<(CharacterId, CharacterId)> =
            out.absorbed.iter().map(|&(k, c)| (cast.existing[k as usize], ids[c])).collect();
        if !self.db().apply_identity_noting(rev, &new, &updates, &bodies, &joined, meta)? {
            return Ok(None);
        }
        if !joined.is_empty() {
            tracing::info!(joined = joined.len(), "characters found to be one with another");
        }
        if !new.is_empty() {
            tracing::info!(new = new.len(), "new characters");
        }
        Ok(Some(ids.len()))
    }

    // -- cast ---------------------------------------------------------------

    pub(crate) fn path_string(p: &Path) -> String {
        p.display().to_string()
    }

    pub fn cast(&self) -> ProjectResult<Cast> {
        let mut rows = self.db().characters()?;
        let extra = self.body_extras()?;
        if !extra.is_empty() {
            for c in &mut rows {
                if let Some(x) = extra.get(&c.id) {
                    c.screen_sec += x.seconds;
                    c.shots += x.shots;
                    c.media += x.media.len() as u32;
                }
            }
            // Most on screen first, as before.
            rows.sort_by(|a, b| b.screen_sec.total_cmp(&a.screen_sec).then(a.number.cmp(&b.number)));
        }
        let covers: Vec<i64> = rows.iter().filter_map(|c| c.cover.map(|(t, _)| t)).collect();
        let portraits: HashMap<i64, PathBuf> =
            self.portrait_plans(&covers)?.into_iter().filter(|p| p.path.is_file()).map(|p| (p.track, p.path)).collect();
        let characters = rows
            .into_iter()
            .map(|c| Character {
                id: c.id,
                label: c.name.clone().unwrap_or_else(|| format!("Character {}", c.number)),
                name: c.name,
                hidden: c.hidden,
                tracks: c.tracks,
                shots: c.shots,
                media: c.media,
                screen_sec: c.screen_sec,
                cover: c.cover.map(|(t, m)| match portraits.get(&t) {
                    Some(p) => Self::path_string(p),
                    None => Self::path_string(&self.thumb_path(m, t)),
                }),
                cover_small: c.cover.map(|(t, m)| Self::path_string(&self.thumb_path(m, t))),
                unsure: c.unsure,
                waiting: c.waiting,
            })
            .collect();
        let (unassigned, unsure, media_done, media_total) = self.db().cast_counts()?;
        Ok(Cast { characters, unassigned, unsure, media_done, media_total })
    }

    fn track_item(&self, r: TrackRow) -> FaceTrackItem {
        FaceTrackItem {
            id: track_id(r.id),
            media_id: r.media_id,
            start: r.start,
            end: r.end,
            quality: r.quality,
            face_height: r.face_h,
            thumb: Self::path_string(&self.thumb_path(r.media_id, r.id)),
            character_id: r.character_id,
            assign: r.assign,
            score: r.score,
        }
    }

    pub fn cast_tracks(&self, filter: TrackFilter) -> ProjectResult<Vec<FaceTrackItem>> {
        Ok(self.db().tracks(filter)?.into_iter().map(|r| self.track_item(r)).collect())
    }

    /// Apply a decision. Returns the character it created, if any.
    pub fn cast_edit(&self, edit: &CastEdit) -> ProjectResult<Option<CharacterId>> {
        let ids = |t: &[u32]| t.iter().map(|&x| x as i64).collect::<Vec<i64>>();
        let known = |c: CharacterId| -> ProjectResult<()> {
            if self.db().character_exists(c)? {
                Ok(())
            } else {
                Err(ProjectError::NotFound("This character doesn't exist any more.".into()))
            }
        };
        match edit {
            CastEdit::Assign { tracks, character_id } => {
                known(*character_id)?;
                self.db().pin_tracks(&ids(tracks), *character_id, None)?;
            }
            CastEdit::NewCharacter { tracks, name } => {
                let id = CharacterId::new();
                let name = name.as_deref().map(str::trim).filter(|n| !n.is_empty());
                self.db().pin_tracks(&ids(tracks), id, Some(name))?;
                return Ok(Some(id));
            }
            CastEdit::NotThem { tracks, character_id } => self.db().forbid_tracks(&ids(tracks), *character_id)?,
            CastEdit::NotAFace { tracks } => self.db().mark_not_face(&ids(tracks))?,
            CastEdit::Confirm { tracks } => self.db().confirm_tracks(&ids(tracks))?,
            CastEdit::Rename { character_id, name } => {
                known(*character_id)?;
                let name = name.as_deref().map(str::trim).filter(|n| !n.is_empty());
                self.db().rename_character(*character_id, name, NAME_ANCHORS)?;
            }
            CastEdit::SetHidden { character_id, hidden } => {
                self.db().set_character_hidden(*character_id, *hidden, NAME_ANCHORS)?
            }
            CastEdit::Merge { from, into } => {
                known(*from)?;
                known(*into)?;
                self.db().merge_characters(*from, *into)?;
            }
            CastEdit::SetCover { character_id, track } => {
                self.db().set_character_cover(*character_id, *track as i64)?
            }
            CastEdit::BodyAssign { bodies, character_id } => {
                known(*character_id)?;
                self.db().pin_bodies(&ids(bodies), *character_id)?;
            }
            CastEdit::BodyNotThem { bodies, character_id } => self.db().forbid_bodies(&ids(bodies), *character_id)?,
            CastEdit::NotAPerson { bodies } => self.db().mark_not_person(&ids(bodies))?,
        }
        Ok(None)
    }

    /// Where characters are on screen in `id`: their faces, and where only
    /// their bodies show.
    pub fn presence(&self, id: MediaId) -> ProjectResult<Vec<Presence>> {
        let faces = self.db().presence(id)?;
        let mut out: Vec<Presence> = faces
            .iter()
            .map(|&(character_id, t, start, end)| Presence {
                character_id,
                track_id: track_id(t),
                start,
                end,
                body: false,
            })
            .collect();
        let bodies = self.shown_bodies(|db| db.media_bodies(id))?;
        if !bodies.is_empty() {
            let mut face_of: HashMap<CharacterId, Vec<(FrameIdx, FrameIdx)>> = HashMap::new();
            for &(c, _, start, end) in &faces {
                face_of.entry(c).or_default().push((start, end));
            }
            face_of.values_mut().for_each(|v| v.sort_unstable());
            for b in &bodies {
                let spans = crate::bodies::spans_of(&b.dets, b.end);
                let theirs = face_of.get(&b.character_id).map_or(&[][..], Vec::as_slice);
                out.extend(crate::bodies::outside(&spans, theirs).into_iter().map(|(start, end)| Presence {
                    character_id: b.character_id,
                    track_id: track_id(b.id),
                    start,
                    end,
                    body: true,
                }));
            }
            out.sort_by_key(|p| (p.start, p.end));
        }
        Ok(out)
    }

    /// What the bodies of the cast add to each character: the time they
    /// are on screen without their face, the shots without it that make a
    /// clip (see `pack::shots_of`), the episodes only those are in.
    fn body_extras(&self) -> ProjectResult<HashMap<CharacterId, BodyExtra>> {
        let bodies = self.shown_bodies(|db| db.assigned_bodies())?;
        if bodies.is_empty() {
            return Ok(HashMap::new());
        }
        let mut faces: HashMap<(CharacterId, MediaId, FrameIdx), Vec<(FrameIdx, FrameIdx)>> = HashMap::new();
        let mut seen_in: HashSet<(CharacterId, MediaId)> = HashSet::new();
        for (c, m, shot, start, end) in self.db().face_ranges()? {
            faces.entry((c, m, shot)).or_default().push((start, end));
            seen_in.insert((c, m));
        }
        // The media's frame rates only: not counted, nor looked for on disk.
        let frame_sec: HashMap<MediaId, f64> = self
            .db()
            .media_list_uncounted()?
            .into_iter()
            .filter_map(|m| Some((m.id, m.fps.filter(|f| f.num > 0).map(|f| f.den as f64 / f.num as f64)?)))
            .collect();
        // Per character and shot: when their bodies are there, and whether one makes a clip.
        type Spans = (Vec<(FrameIdx, FrameIdx)>, bool);
        let mut by_shot: HashMap<(CharacterId, MediaId, FrameIdx), Spans> = HashMap::new();
        for b in &bodies {
            let e = by_shot.entry((b.character_id, b.media_id, b.shot)).or_default();
            e.0.extend(crate::bodies::spans_of(&b.dets, b.end));
            e.1 |= crate::bodies::showing_of(b).prominent();
        }
        let mut out: HashMap<CharacterId, BodyExtra> = HashMap::new();
        for ((c, m, shot), (mut spans, clip)) in by_shot {
            spans.sort_unstable();
            let face = faces.get(&(c, m, shot));
            let mut face_spans = face.cloned().unwrap_or_default();
            face_spans.sort_unstable();
            let alone = crate::bodies::covered(crate::bodies::outside(&spans, &face_spans));
            let x = out.entry(c).or_default();
            x.seconds += f64::from(alone) * frame_sec.get(&m).copied().unwrap_or(0.0);
            if face.is_none() && clip {
                x.shots += 1;
                if !seen_in.contains(&(c, m)) {
                    x.media.insert(m);
                }
            }
        }
        Ok(out)
    }
}

/// What a character's bodies add to what their faces say.
#[derive(Default)]
struct BodyExtra {
    seconds: f64,
    shots: u32,
    media: HashSet<MediaId>,
}

#[cfg(test)]
pub(crate) mod tests {
    use as_api::Segment;

    use super::*;
    use crate::analysis::tests::project_with_media;

    const MODEL: &str = "test-embedder";

    /// A face of person `who` (a direction in embedding space) in shot `shot`.
    pub(crate) fn face(who: usize, shot: FrameIdx, x: f32, quality: f32) -> FaceTrack {
        let mut embedding = vec![0.02f32; tracks::DIM];
        embedding[who] = 1.0;
        let n = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        embedding.iter_mut().for_each(|v| *v /= n);
        let det = |frame| FaceDet { frame, bbox: [x, 0.2, x + 0.2, 0.6], score: 0.9, norm: 22.0, quality };
        FaceTrack {
            start: shot,
            end: shot + 60,
            shot,
            dets: vec![det(shot), det(shot + 30)],
            embedding,
            quality,
            thumb: vec![0xFF, 0xD8, who as u8],
        }
    }

    /// Person `who` in shot `shot`, seen from `start` to `end` (a detection
    /// every 15 frames), at `x` at frame `shot` and moving `speed` per frame.
    pub(crate) fn moving(who: usize, shot: FrameIdx, start: FrameIdx, end: FrameIdx, x: f32, speed: f32) -> FaceTrack {
        let mut t = face(who, shot, x, 0.9);
        t.start = start;
        t.end = end;
        let first = t.dets[0];
        t.dets = (start..end)
            .step_by(15)
            .map(|frame| {
                let dx = speed * (frame - shot) as f32;
                FaceDet { frame, bbox: [first.bbox[0] + dx, first.bbox[1], first.bbox[2] + dx, first.bbox[3]], ..first }
            })
            .collect();
        t
    }

    pub(crate) fn seen(who: usize, shot: FrameIdx, start: FrameIdx, end: FrameIdx, x: f32) -> FaceTrack {
        moving(who, shot, start, end, x, 0.0)
    }

    #[test]
    fn a_moving_face_is_found_again_in_pieces() {
        // 0.2 wide, crossing 0.6 of the frame: the halves' boxes are far
        // from the whole track's mean box, but match it where they share frames.
        let whole = moving(1, 0, 0, 90, 0.0, 0.006);
        let (a, b) = (moving(1, 0, 0, 45, 0.0, 0.006), moving(1, 0, 45, 90, 0.0, 0.006));
        let r = |t: &FaceTrack| FrameRange::new(t.start, t.end);
        assert_eq!(same_face((r(&whole), &whole.dets), (r(&a), &a.dets)), Some(45));
        assert_eq!(same_face((r(&whole), &whole.dets), (r(&b), &b.dets)), Some(45));
        assert_eq!(same_face((r(&a), &a.dets), (r(&whole), &whole.dets)), Some(45));
        // Someone else at the other side of the frame at the same time isn't.
        let other = seen(2, 0, 0, 90, 0.7);
        assert_eq!(same_face((r(&whole), &whole.dets), (r(&other), &other.dets)), None);
        // A short glimpse inside a long track: most of the shorter one is shared.
        let glimpse = seen(1, 0, 30, 33, 0.18);
        assert_eq!(same_face((r(&glimpse), &glimpse.dets), (r(&whole), &whole.dets)), Some(3));
    }

    #[test]
    fn covers_show_their_portrait_once_it_is_made() {
        let frames = 400;
        let (_dir, project, id) = project_with_media(frames);
        // Seen at 100, 115, 130, …: clearest at 130, the moment of its thumbnail.
        let mut t = seen(1, 100, 100, 190, 0.1);
        t.dets[2].quality = 0.95;
        project.store_face_chunk(id, 0, frames, &tracks::encode(&[t]), &[], &[]).unwrap();
        project.finish_faces(id).unwrap();
        let track = project.cast_tracks(TrackFilter::Unassigned).unwrap()[0].id;
        project.cast_edit(&CastEdit::NewCharacter { tracks: vec![track], name: Some("Walter".into()) }).unwrap();
        let track = i64::from(track);
        let cover = |p: &Project| {
            let c = p.cast().unwrap().characters.remove(0);
            (c.cover.unwrap(), c.cover_small.unwrap())
        };

        // No portrait yet: the thumbnail stands in, and one is owed.
        let (big, small) = cover(&project);
        assert_eq!(big, small);
        assert_eq!(small, project.thumb_path(id, track).display().to_string());
        assert_eq!(project.missing_portraits().unwrap(), [track]);

        // Named after its frame, box and size; once it exists the card shows it.
        let plan = project.portrait_plans(&[track]).unwrap().pop().unwrap();
        assert_eq!(plan.det.frame, 130);
        let portrait = plan.path;
        let name = format!("{track}-130-{}-450x600.jpg", box_tag(plan.det.bbox));
        assert!(portrait.ends_with(&name), "{}", portrait.display());
        std::fs::write(&portrait, b"jpeg").unwrap();
        assert_eq!(cover(&project), (portrait.display().to_string(), small.clone()));
        assert!(project.missing_portraits().unwrap().is_empty());
        assert!(project.portrait_plans(&[9999]).unwrap().is_empty(), "no such track");

        // A media that can't be decoded now owes none.
        std::fs::remove_file(&portrait).unwrap();
        project.db().set_media_status(id, MediaStatus::Missing, None).unwrap();
        assert!(project.missing_portraits().unwrap().is_empty());
        assert_eq!(cover(&project), (small.clone(), small));
        // Nor does one whose portrait failed this session.
        project.db().set_media_status(id, MediaStatus::Ready, None).unwrap();
        assert_eq!(project.missing_portraits().unwrap(), [track]);
        project.portraits.lock().failed.insert(portrait);
        assert!(project.missing_portraits().unwrap().is_empty());
    }

    #[test]
    fn the_cast_keeps_the_space_it_was_grouped_in() {
        let frames = 400;
        let (_dir, project, id) = project_with_media(frames);
        let faces = [seen(1, 0, 0, 90, 0.0), seen(2, 0, 0, 90, 0.6), seen(1, 100, 100, 190, 0.0)];
        project.store_face_chunk(id, 0, frames, &tracks::encode(&faces), &[], &[]).unwrap();
        project.finish_faces(id).unwrap();
        project.recluster().unwrap();
        let stored =
            || project.db().meta(GROUPED_IN).unwrap().and_then(|h| unhex(&h)).and_then(|b| Space::from_bytes(&b));
        // Live action: ArcFace's own space.
        assert_eq!(stored(), Some(Space::Faces));
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(unhex(&hex(&bytes)), Some(bytes));
        assert_eq!((unhex("abc"), unhex("zz")), (None, None));
        // Damaged or missing, a decision still places the faces (fitted again).
        project.db().set_meta(GROUPED_IN, "zz").unwrap();
        project.reassign().unwrap();
        project.db().delete_meta(GROUPED_IN).unwrap();
        project.reassign().unwrap();
        project.recluster().unwrap();
        assert_eq!(stored(), Some(Space::Faces));
    }

    #[test]
    fn a_cast_from_an_older_version_is_brought_up_to_date() {
        let frames = 600;
        let (_dir, project, id) = project_with_media(frames);
        let at = |mut t: FaceTrack, bbox: [f32; 4], quality: f32| {
            t.dets.iter_mut().for_each(|d| (d.bbox, d.quality) = (bbox, quality));
            t.quality = quality;
            t
        };
        // Person 1, sharp, behind person 2's bigger face; later person 1
        // alone, a little less sharp.
        let behind = at(seen(1, 0, 0, 90, 0.0), [0.50, 0.15, 0.56, 0.30], 0.95);
        let front = at(seen(2, 0, 0, 90, 0.0), [0.40, 0.15, 0.53, 0.50], 0.9);
        let alone = at(seen(1, 100, 100, 190, 0.0), [0.30, 0.20, 0.45, 0.45], 0.8);
        project.store_face_chunk(id, 0, frames, &tracks::encode(&[behind, front, alone]), &[], &[]).unwrap();
        project.finish_faces(id).unwrap();
        let ids: Vec<u32> = project.cast_tracks(TrackFilter::Unassigned).unwrap().iter().map(|t| t.id).collect();
        let (behind, alone) = (ids[0].min(ids[1]), ids[2]);
        project
            .cast_edit(&CastEdit::NewCharacter { tracks: vec![behind, alone], name: Some("Walter".into()) })
            .unwrap();
        let cover = |p: &Project| p.cast().unwrap().characters[0].cover_small.clone().unwrap();
        let shows = |track: u32| project.thumb_path(id, i64::from(track)).display().to_string();
        // The picture of them alone, not the one where someone else's face
        // is bigger than theirs.
        assert_eq!(cover(&project), shows(alone));
        project.recluster().unwrap();
        assert!(!project.cast_needs_upkeep().unwrap());

        // A cast from before: grouped by an older version, no portrait looked
        // at for other faces.
        let db = rusqlite::Connection::open(project.dir.join(crate::project::PROJECT_DB)).unwrap();
        db.execute_batch("UPDATE tracks SET rival = NULL; DELETE FROM meta WHERE key = 'identity_version';").unwrap();
        drop(db);
        assert_eq!(cover(&project), shows(behind));
        assert!(project.cast_needs_upkeep().unwrap());
        assert_eq!(project.cast_upkeep(&CancelToken::new()).unwrap(), Some(1));
        assert!(!project.cast_needs_upkeep().unwrap());
        assert_eq!(project.cast_upkeep(&CancelToken::new()).unwrap(), None, "once");
        // The decision stays; the cover shows them.
        let walter = project.cast().unwrap().characters.remove(0);
        assert_eq!((walter.name.as_deref(), walter.tracks), (Some("Walter"), 2));
        assert_eq!(cover(&project), shows(alone));

        // A cast grouped by a newer version (the app was downgraded) isn't
        // grouped again by this one; one marked unreadably is.
        let db = rusqlite::Connection::open(project.dir.join(crate::project::PROJECT_DB)).unwrap();
        let mark = |v: &str| {
            db.execute("UPDATE meta SET value = ?1 WHERE key = 'identity_version'", [v]).unwrap();
        };
        mark(&(as_identity::VERSION + 1).to_string());
        assert!(!project.grouped_by_older_version().unwrap());
        mark("?");
        assert!(project.grouped_by_older_version().unwrap());
    }

    #[test]
    fn box_tags_tell_faces_of_one_frame_apart() {
        let (a, b) = ([0.1, 0.2, 0.3, 0.6], [0.5, 0.2, 0.7, 0.6]);
        assert_eq!(box_tag(a), box_tag(a));
        assert_ne!(box_tag(a), box_tag(b));
        assert_eq!(box_tag(a).len(), 6);
    }

    #[test]
    fn decisions_on_one_track_are_made_consistent() {
        let (x, y) = (CharacterId::new(), CharacterId::new());
        let got = |found: Vec<(u32, i64, EditKind)>| {
            let mut v = resolve(found);
            v.sort_by_key(|(t, k)| (*t, format!("{k:?}")));
            v
        };
        // The decision sharing most frames wins: here "not a face".
        assert_eq!(got(vec![(10, 1, EditKind::Pin(x)), (30, 1, EditKind::NotFace)]), [(1, EditKind::NotFace)]);
        // "This is X" against "not X": the bigger one holds, the other goes.
        assert_eq!(got(vec![(40, 2, EditKind::Pin(x)), (10, 2, EditKind::Forbid(x))]), [(2, EditKind::Pin(x))]);
        assert_eq!(got(vec![(10, 3, EditKind::Pin(x)), (40, 3, EditKind::Forbid(x))]), [(3, EditKind::Forbid(x))]);
        // Clustering's word only without a decision, the biggest, never a "not".
        assert_eq!(
            got(vec![(5, 4, EditKind::Was(x)), (50, 4, EditKind::Was(y)), (1, 4, EditKind::Forbid(y))]),
            [(4, EditKind::Forbid(y)), (4, EditKind::Was(x))]
        );
        assert_eq!(got(vec![(50, 5, EditKind::Was(y)), (5, 5, EditKind::Pin(x))]), [(5, EditKind::Pin(x))]);
        assert_eq!(got(vec![(5, 6, EditKind::Was(x)), (50, 6, EditKind::Was(y))]), [(6, EditKind::Was(y))]);
    }

    #[test]
    fn decisions_follow_a_face_the_tracker_now_joins_or_splits() {
        let frames = 400;
        let (_dir, project, id) = project_with_media(frames);
        let segments: Vec<Segment> = (0..frames)
            .step_by(100)
            .map(|s| Segment { start: s, end: s + 100, kind: SegmentKind::Shot, tiny: false })
            .collect();
        project.db().store_segments(id, &segments).unwrap();
        // Walter in two pieces in shot 0; Jesse walking across shot 100 in
        // one piece; an extra in shot 200 the user said isn't Walter.
        let first = [
            seen(1, 0, 0, 30, 0.1),
            seen(1, 0, 30, 60, 0.1),
            moving(2, 100, 100, 190, 0.0, 0.007),
            seen(3, 200, 200, 260, 0.3),
        ];
        project.store_face_chunk(id, 0, frames, &tracks::encode(&first), &[], &[]).unwrap();
        project.finish_faces(id).unwrap();
        let ids = |f: TrackFilter| -> Vec<u32> { project.cast_tracks(f).unwrap().iter().map(|t| t.id).collect() };
        let all = ids(TrackFilter::Unassigned);
        assert_eq!(all.len(), 4);
        let by_start = |start| {
            project.cast_tracks(TrackFilter::Unassigned).unwrap().into_iter().find(|t| t.start == start).unwrap().id
        };
        let (w1, w2, j, extra) = (by_start(0), by_start(30), by_start(100), by_start(200));
        let walter = project
            .cast_edit(&CastEdit::NewCharacter { tracks: vec![w1, w2], name: Some("Walter".into()) })
            .unwrap()
            .unwrap();
        let jesse = project
            .cast_edit(&CastEdit::NewCharacter { tracks: vec![j], name: Some("Jesse".into()) })
            .unwrap()
            .unwrap();
        project.cast_edit(&CastEdit::NotThem { tracks: vec![extra], character_id: walter }).unwrap();

        // A better tracker: Walter in one piece, Jesse in two, the extra a little longer.
        project.reset_faces(id).unwrap();
        let again = [
            seen(1, 0, 0, 60, 0.1),
            moving(2, 100, 100, 145, 0.0, 0.007),
            moving(2, 100, 145, 190, 0.0, 0.007),
            seen(3, 200, 200, 290, 0.3),
        ];
        project.store_face_chunk(id, 0, frames, &tracks::encode(&again), &[], &[]).unwrap();
        project.finish_faces(id).unwrap();
        let spans = |c| -> Vec<(FrameIdx, FrameIdx)> {
            let mut v: Vec<_> =
                project.cast_tracks(TrackFilter::Character(c)).unwrap().iter().map(|t| (t.start, t.end)).collect();
            v.sort();
            v
        };
        assert_eq!(spans(walter), [(0, 60)]);
        assert_eq!(spans(jesse), [(100, 145), (145, 190)]);
        // Clustering can't put the extra on Walter.
        project.recluster().unwrap();
        assert!(spans(walter).iter().all(|s| s.0 != 200));
        let extra = project.db().identity_tracks().unwrap().into_iter().find(|t| t.start == 200).unwrap();
        assert_eq!(extra.forbids, [walter]);
    }

    #[test]
    fn a_decision_goes_back_only_onto_a_face_that_looks_like_it() {
        let frames = 400;
        let (_dir, project, id) = project_with_media(frames);
        let whole = Segment { start: 0, end: frames, kind: SegmentKind::Shot, tiny: false };
        project.db().store_segments(id, &[whole]).unwrap();
        let pass = |who: usize| {
            project.faces_plan(id, MODEL).unwrap();
            project.store_face_chunk(id, 0, frames, &tracks::encode(&[seen(who, 0, 100, 190, 0.1)]), &[], &[]).unwrap();
            project.finish_faces(id).unwrap();
        };
        pass(1);
        let t = project.cast_tracks(TrackFilter::Unassigned).unwrap()[0].id;
        let walter = project
            .cast_edit(&CastEdit::NewCharacter { tracks: vec![t], name: Some("Walter".into()) })
            .unwrap()
            .unwrap();
        // Found again in another cut: someone else sits where he was.
        let another_file = || project.db().set_meta(&source_changed_key(id), "1").unwrap();
        another_file();
        project.reset_faces(id).unwrap();
        pass(2);
        assert!(project.cast_tracks(TrackFilter::Character(walter)).unwrap().is_empty(), "not on someone else");
        // The same face found again takes his decision back.
        project
            .cast_edit(&CastEdit::Assign {
                tracks: vec![project.cast_tracks(TrackFilter::Unassigned).unwrap()[0].id],
                character_id: walter,
            })
            .unwrap();
        another_file();
        project.reset_faces(id).unwrap();
        pass(2);
        assert_eq!(project.cast_tracks(TrackFilter::Character(walter)).unwrap().len(), 1);
        // The same file analysed again: the decision goes by place, even
        // where the face came out looking different.
        project.reset_faces(id).unwrap();
        pass(1);
        assert_eq!(project.cast_tracks(TrackFilter::Character(walter)).unwrap().len(), 1, "same file, same place");
    }

    #[test]
    fn naming_one_face_brings_the_look_alikes_at_once() {
        let frames = 2000;
        let (_dir, project, id) = project_with_media(frames);
        let segments: Vec<Segment> = (0..frames)
            .step_by(100)
            .map(|s| Segment { start: s, end: s + 100, kind: SegmentKind::Shot, tiny: false })
            .collect();
        project.db().store_segments(id, &segments).unwrap();
        project.faces_plan(id, MODEL).unwrap();
        // People 1 and 2 clearly, taking turns; person 3 only blurry, in the
        // first four shots: not a character of their own.
        let mut found: Vec<FaceTrack> = (0..20).map(|k| face(1 + k % 2, k as u32 * 100, 0.1, 0.9)).collect();
        found.extend((0..4).map(|k| face(3, k * 100, 0.6, 0.2)));
        project.store_face_chunk(id, 0, frames, &tracks::encode(&found), &[], &[]).unwrap();
        project.finish_faces(id).unwrap();
        assert_eq!(project.recluster().unwrap(), 2);
        let blurry = project.cast_tracks(TrackFilter::Unassigned).unwrap();
        assert_eq!(blurry.len(), 4);
        let before: Vec<(CharacterId, u32)> =
            project.cast().unwrap().characters.iter().map(|c| (c.id, c.tracks)).collect();

        // One of them named: the others follow at once, nobody else moves.
        let jesse = project
            .cast_edit(&CastEdit::NewCharacter { tracks: vec![blurry[0].id], name: Some("Jesse".into()) })
            .unwrap()
            .unwrap();
        let theirs = |c| project.cast_tracks(TrackFilter::Character(c)).unwrap();
        assert_eq!(theirs(jesse).len(), 1, "the decision alone");
        project.reassign().unwrap();
        assert_eq!(theirs(jesse).len(), 4);
        let cast = project.cast().unwrap();
        assert_eq!(cast.characters.len(), 3, "no character made by placing faces");
        for (c, tracks) in before {
            assert_eq!(cast.characters.iter().find(|x| x.id == c).unwrap().tracks, tracks);
        }

        // "Not Jesse" on one: it leaves, and stays out when faces are placed again.
        let wrong = theirs(jesse).iter().find(|t| t.id != blurry[0].id).unwrap().id;
        project.cast_edit(&CastEdit::NotThem { tracks: vec![wrong], character_id: jesse }).unwrap();
        project.reassign().unwrap();
        let now = theirs(jesse);
        assert_eq!(now.len(), 3);
        assert!(now.iter().all(|t| t.id != wrong));
        assert!(now.iter().any(|t| t.id == blurry[0].id && t.assign == TrackAssign::Pinned));
    }

    #[test]
    fn faces_become_a_cast_whose_edits_survive_reanalysis() {
        let frames = 2000;
        let (_dir, project, id) = project_with_media(frames);
        // Shots of 100 frames, no transitions.
        let segments: Vec<Segment> = (0..frames)
            .step_by(100)
            .map(|s| Segment { start: s, end: s + 100, kind: SegmentKind::Shot, tiny: false })
            .collect();
        project.db().store_segments(id, &segments).unwrap();
        let plan = project.faces_plan(id, MODEL).unwrap();
        assert_eq!((plan.resume, plan.shots.len(), plan.frames), (0, 20, frames));

        // Two people, one in the even shots and one in the odd shots (both in shot 0).
        let all = |shift: f32| -> Vec<FaceTrack> {
            let mut v = vec![face(1, 0, 0.6 + shift, 0.9)];
            v.extend((0..frames).step_by(100).map(|s| face((s / 100 % 2) as usize, s, 0.1 + shift, 0.9)));
            v
        };
        let first = all(0.0);
        let (a, b): (Vec<_>, Vec<_>) = first.iter().cloned().partition(|t| t.start < 1000);
        let sprite = |shot| ShotSprite { shot, end: shot + 100, cells: 8, jpeg: vec![0xFF, 0xD8, 1] };
        // A sprite outside the chunk is not this chunk's.
        assert_eq!(
            project.store_face_chunk(id, 0, 1000, &tracks::encode(&a), &[sprite(0), sprite(1000)], &[]).unwrap(),
            11
        );
        assert_eq!(project.faces_plan(id, MODEL).unwrap().resume, 1000);
        project.store_face_chunk(id, 1000, frames, &tracks::encode(&b), &[], &[]).unwrap();
        project.finish_faces(id).unwrap();
        assert!(project.pending_faces().unwrap().is_empty());
        assert_eq!(project.recluster().unwrap(), 2);

        let cast = project.cast().unwrap();
        assert_eq!(cast.characters.len(), 2);
        let walter = cast.characters[0].clone();
        let other = cast.characters[1].clone();
        assert_eq!(walter.tracks, 11);
        assert!(std::path::Path::new(walter.cover.as_deref().unwrap()).is_file());
        project.cast_edit(&CastEdit::Rename { character_id: walter.id, name: Some(" Walter ".into()) }).unwrap();
        let shots = project.character_shots(walter.id).unwrap();
        assert_eq!(shots.len(), 11);
        assert_eq!((shots[0].start, shots[0].end), (0, 100));
        assert!(std::path::Path::new(shots[0].sprite.as_deref().unwrap()).is_file());
        assert_eq!(shots[0].cells, 8);
        assert!(shots[1..].iter().all(|s| s.sprite.is_none() && s.cells == 0));
        assert!(shots.iter().all(|s| std::path::Path::new(&s.face).is_file()));
        assert_eq!(project.presence(id).unwrap().len(), 21);

        // "Not them" on one of Walter's tracks.
        let tracks = project.cast_tracks(TrackFilter::Character(walter.id)).unwrap();
        let wrong = tracks.iter().find(|t| t.start == 300).unwrap().id;
        project.cast_edit(&CastEdit::NotThem { tracks: vec![wrong], character_id: walter.id }).unwrap();
        assert_eq!(project.cast_tracks(TrackFilter::Character(walter.id)).unwrap().len(), 10);
        let picture = tracks.iter().find(|t| t.start == 500).unwrap().id;
        project.cast_edit(&CastEdit::SetCover { character_id: walter.id, track: picture }).unwrap();
        let cover_start = |p: &Project| {
            let rows = p.db().characters().unwrap();
            let (t, _) = rows.iter().find(|c| c.id == walter.id).unwrap().cover.unwrap();
            p.db().tracks_by_id(&[t]).unwrap()[0].start
        };
        assert_eq!(cover_start(&project), 500);

        // Re-analysis: tracks are replaced (slightly moved boxes), the name and the decision stay.
        project.reset_faces(id).unwrap();
        assert!(project.cast().unwrap().characters.iter().any(|c| c.name.as_deref() == Some("Walter")));
        let again = all(0.02);
        let (a, b): (Vec<_>, Vec<_>) = again.into_iter().partition(|t| t.start < 1000);
        project.store_face_chunk(id, 0, 1000, &tracks::encode(&a), &[], &[]).unwrap();
        // Another media's restart reclusters while this pass is half done:
        // its new faces must not become new characters meanwhile.
        project.recluster().unwrap();
        let mid: Vec<CharacterId> = project.cast().unwrap().characters.iter().map(|c| c.id).collect();
        assert_eq!(mid.len(), 2);
        assert!(mid.contains(&walter.id) && mid.contains(&other.id));
        project.store_face_chunk(id, 1000, frames, &tracks::encode(&b), &[], &[]).unwrap();
        project.finish_faces(id).unwrap();
        project.recluster().unwrap();
        let cast = project.cast().unwrap();
        let w = cast.characters.iter().find(|c| c.name.as_deref() == Some("Walter")).expect("Walter survived");
        assert_eq!(w.id, walter.id);
        let mine = project.cast_tracks(TrackFilter::Character(w.id)).unwrap();
        assert_eq!(mine.len(), 10, "the 'not them' decision survived");
        assert!(mine.iter().all(|t| t.start != 300));
        assert_eq!(cover_start(&project), 500, "the chosen picture survived");
        // The character nobody named or touched is still the same one.
        let o = cast.characters.iter().find(|c| c.id == other.id).expect("the unnamed character survived");
        assert_eq!((o.label.as_str(), o.tracks), (other.label.as_str(), other.tracks));
        assert_eq!(cast.characters.len(), 2);
    }
}
