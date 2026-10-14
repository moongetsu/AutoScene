//! Faces and characters: the face pass's checkpoints (tracks plus thumbnail
//! files), clustering tracks into the project's cast, and every edit the
//! user makes to it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use as_analysis::FaceParams;
use as_analysis::tracker::TrackerParams;
use as_analysis::tracks::{self, FaceTrack};
use as_analysis::{FaceDet, ShotSprite};
use as_api::{
    Cast, CastEdit, Character, CharacterShot, ClipRequest, FaceTrackItem, MediaInfo, Presence, SegmentKind, StageState,
    TrackAssign, TrackFilter,
};
use as_core::{CharacterId, FrameIdx, FrameRange, MediaId};
use as_db::{EditKind, IdentityTrack, NewTrack, TrackRow};
use as_identity::{Assign, CharKey, Params, TrackIn, identify};

use crate::{Project, ProjectError, ProjectResult};

/// Frames per face checkpoint chunk (closed at the next shot start).
pub const FACE_CHUNK: u32 = 2400;
/// Tracks pinned when a character is named, so it keeps its identity.
const NAME_ANCHORS: usize = 5;
/// Clusterings in a row that may find the cast changed by the user before
/// one is stored.
const CLUSTER_ATTEMPTS: usize = 8;

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

fn box_iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

fn track_id(id: i64) -> u32 {
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

    fn thumb_path(&self, media: MediaId, track: i64) -> PathBuf {
        self.media_faces_dir(media).join(format!("{track}.jpg"))
    }

    /// Named by the shot's start and end: a shot found again differently is
    /// a new file, never a stale one from the webview's cache.
    fn sprite_path(&self, media: MediaId, shot: FrameIdx, end: FrameIdx) -> PathBuf {
        self.media_faces_dir(media).join("shots").join(format!("{shot}-{end}.jpg"))
    }

    // -- face pass ----------------------------------------------------------

    /// Where the face pass of `id` resumes. Checkpoints that don't line up
    /// are dropped with everything after them; a fresh start clears old
    /// thumbnails.
    pub fn faces_plan(&self, id: MediaId) -> ProjectResult<FacesPlan> {
        let (m, info, index) = self.ready_source(id)?;
        if m.shots != StageState::Ready {
            return Err(ProjectError::NotReady("Shots of this media aren't found yet.".into()));
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
    /// and the sprites of its shots): tracks in the database, thumbnails and
    /// sprites as files, atomically. Returns the number of tracks.
    pub fn store_face_chunk(
        &self,
        id: MediaId,
        start: FrameIdx,
        end: FrameIdx,
        blob: &[u8],
        sprites: &[ShotSprite],
    ) -> ProjectResult<usize> {
        let found: Vec<FaceTrack> =
            tracks::decode(blob).ok_or_else(|| ProjectError::Invalid("unreadable face tracks".into()))?;
        let rows: Vec<NewTrack> = found
            .iter()
            .map(|t| NewTrack {
                start: t.start,
                end: t.end,
                shot: t.shot,
                quality: t.quality,
                face_h: t.face_height(),
                dets: tracks::dets_to_bytes(&t.dets),
                emb: tracks::embedding_to_bytes(&t.embedding),
            })
            .collect();
        let sprites: Vec<&ShotSprite> = sprites.iter().filter(|s| (start..end).contains(&s.shot)).collect();
        let cells: Vec<(FrameIdx, FrameIdx, u32)> = sprites.iter().map(|s| (s.shot, s.end, s.cells)).collect();
        let shots_dir = self.media_faces_dir(id).join("shots");
        self.db().put_face_chunk(id, start, end, &rows, &cells, |ids| {
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
        Ok(found.len())
    }

    pub fn set_faces_state(&self, id: MediaId, state: StageState, error: Option<&str>) -> ProjectResult<()> {
        Ok(self.db().set_faces_state(id, state, error)?)
    }

    /// The face pass of `id` is complete: put the user's earlier decisions
    /// back on the new tracks (see [`same_face`] and [`resolve`]), then mark
    /// the faces ready.
    pub fn finish_faces(&self, id: MediaId) -> ProjectResult<()> {
        let edits = self.db().track_edits(id)?;
        if !edits.is_empty() {
            let new: Vec<(i64, FrameRange, Vec<FaceDet>)> = self
                .db()
                .media_track_dets(id)?
                .into_iter()
                .filter_map(|(t, s, e, d)| Some((t, FrameRange::new(s, e), tracks::dets_from_bytes(&d)?)))
                .collect();
            let mut found: Vec<(u32, i64, EditKind)> = Vec::new();
            let mut kept = 0;
            for e in &edits {
                let span = FrameRange::new(e.start, e.end);
                let dets = tracks::dets_from_bytes(&e.dets).unwrap_or_default();
                let before = found.len();
                for (t, r, new_dets) in &new {
                    if let Some(shared) = same_face((span, &dets), (*r, new_dets)) {
                        found.push((shared, *t, e.kind));
                    }
                }
                kept += usize::from(found.len() > before);
            }
            let matched = resolve(found);
            tracing::info!(media = %id, edits = edits.len(), kept, tracks = matched.len(), "re-anchored cast edits");
            self.db().reanchor_edits(id, &matched)?;
        }
        Ok(self.db().set_faces_state(id, StageState::Ready, None)?)
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
        let face = FaceParams::default();
        let max_gap = TrackerParams::new(face.stride).max_gap;
        let existing: Vec<CharacterId> = self.db().characters()?.into_iter().map(|c| c.id).collect();
        let key: HashMap<CharacterId, CharKey> = existing.iter().enumerate().map(|(i, c)| (*c, i as CharKey)).collect();
        let mut media: HashMap<MediaId, u32> = HashMap::new();
        let mut input = Vec::with_capacity(rows.len());
        let mut kept = Vec::with_capacity(rows.len());
        for r in &rows {
            let Some(embedding) = tracks::embedding_from_bytes(&r.emb) else { continue };
            let m = media.len() as u32;
            let m = *media.entry(r.media_id).or_insert(m);
            let assigned = r.character_id.and_then(|c| key.get(&c).copied());
            let spans = tracks::dets_from_bytes(&r.det_bytes)
                .map(|d| tracks::presence_spans(&d, r.end, face.stride, max_gap))
                .unwrap_or_default();
            input.push(TrackIn {
                media: m,
                start: r.start,
                end: r.end,
                spans,
                embedding,
                quality: r.quality,
                dets: r.dets,
                pin: (r.assign == TrackAssign::Pinned).then_some(assigned).flatten(),
                forbid: r.forbids.iter().filter_map(|c| key.get(c).copied()).collect(),
                previous: matches!(r.assign, TrackAssign::Auto | TrackAssign::Pinned).then_some(assigned).flatten(),
            });
            kept.push(r.id);
        }
        let started = std::time::Instant::now();
        let out = identify(&input, &Params::default());
        let ids: Vec<CharacterId> =
            out.characters.iter().map(|c| c.key.map_or_else(CharacterId::new, |k| existing[k as usize])).collect();
        let new: Vec<CharacterId> =
            out.characters.iter().zip(&ids).filter(|(c, _)| c.key.is_none()).map(|(_, id)| *id).collect();
        let updates: Vec<(i64, Option<CharacterId>, TrackAssign, f32)> = kept
            .iter()
            .zip(&out.tracks)
            .map(|(&t, &(a, score))| match a {
                Assign::Auto(c) => (t, Some(ids[c]), TrackAssign::Auto, score),
                Assign::Pinned(c) => (t, Some(ids[c]), TrackAssign::Pinned, score),
                Assign::Unsure(c) => (t, Some(ids[c]), TrackAssign::Unsure, score),
                Assign::None => (t, None, TrackAssign::None, score),
            })
            .collect();
        if !self.db().apply_identity(rev, &new, &updates)? {
            return Ok(None);
        }
        tracing::info!(
            tracks = kept.len(),
            characters = ids.len(),
            new = new.len(),
            ms = started.elapsed().as_millis() as u64,
            "cast clustered"
        );
        Ok(Some(ids.len()))
    }

    // -- cast ---------------------------------------------------------------

    fn path_string(p: &Path) -> String {
        p.display().to_string()
    }

    pub fn cast(&self) -> ProjectResult<Cast> {
        let characters = self
            .db()
            .characters()?
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
                cover: c.cover.map(|(t, m)| Self::path_string(&self.thumb_path(m, t))),
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
        }
        Ok(None)
    }

    /// Where characters are on screen in `id`.
    pub fn presence(&self, id: MediaId) -> ProjectResult<Vec<Presence>> {
        Ok(self
            .db()
            .presence(id)?
            .into_iter()
            .map(|(character_id, t, start, end)| Presence { character_id, track_id: track_id(t), start, end })
            .collect())
    }

    /// Every shot a character is seen in, in episode order.
    pub fn character_shots(&self, c: CharacterId) -> ProjectResult<Vec<CharacterShot>> {
        let mut by_media: HashMap<MediaId, Vec<as_db::CharacterShotRow>> = HashMap::new();
        for r in self.db().character_shots(c)? {
            by_media.entry(r.media_id).or_default().push(r);
        }
        let mut shots = Vec::new();
        for m in self.media_list()? {
            for r in by_media.remove(&m.id).unwrap_or_default() {
                shots.push(CharacterShot {
                    media_id: r.media_id,
                    start: r.start,
                    end: r.end,
                    sprite: r.sprite.map(|(end, _)| Self::path_string(&self.sprite_path(r.media_id, r.start, end))),
                    cells: r.sprite.map_or(0, |s| s.1),
                    track_id: track_id(r.track),
                    face: Self::path_string(&self.thumb_path(r.media_id, r.track)),
                    face_height: r.face_h,
                });
            }
        }
        Ok(shots)
    }

    /// Every shot a character is seen in, as clips (the shot's clean range),
    /// in episode order.
    pub fn character_clips(&self, c: CharacterId) -> ProjectResult<Vec<ClipRequest>> {
        Ok(self
            .character_shots(c)?
            .into_iter()
            .map(|s| ClipRequest { media_id: s.media_id, start: s.start, end: s.end, name: None })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use as_api::Segment;

    use super::*;
    use crate::analysis::tests::project_with_media;

    /// A face of person `who` (a direction in embedding space) in shot `shot`.
    fn face(who: usize, shot: FrameIdx, x: f32, quality: f32) -> FaceTrack {
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
    fn moving(who: usize, shot: FrameIdx, start: FrameIdx, end: FrameIdx, x: f32, speed: f32) -> FaceTrack {
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

    fn seen(who: usize, shot: FrameIdx, start: FrameIdx, end: FrameIdx, x: f32) -> FaceTrack {
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
        project.store_face_chunk(id, 0, frames, &tracks::encode(&first), &[]).unwrap();
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
        project.store_face_chunk(id, 0, frames, &tracks::encode(&again), &[]).unwrap();
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
    fn faces_become_a_cast_whose_edits_survive_reanalysis() {
        let frames = 2000;
        let (_dir, project, id) = project_with_media(frames);
        // Shots of 100 frames, no transitions.
        let segments: Vec<Segment> = (0..frames)
            .step_by(100)
            .map(|s| Segment { start: s, end: s + 100, kind: SegmentKind::Shot, tiny: false })
            .collect();
        project.db().store_segments(id, &segments).unwrap();
        let plan = project.faces_plan(id).unwrap();
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
        assert_eq!(project.store_face_chunk(id, 0, 1000, &tracks::encode(&a), &[sprite(0), sprite(1000)]).unwrap(), 11);
        assert_eq!(project.faces_plan(id).unwrap().resume, 1000);
        project.store_face_chunk(id, 1000, frames, &tracks::encode(&b), &[]).unwrap();
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
        let clips = project.character_clips(walter.id).unwrap();
        assert_eq!(clips.len(), 11);
        assert_eq!((clips[0].start, clips[0].end), (0, 100));
        let shots = project.character_shots(walter.id).unwrap();
        assert_eq!(shots.len(), 11);
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

        // Re-analysis: tracks are replaced (slightly moved boxes), the name and the decision stay.
        project.reset_faces(id).unwrap();
        assert!(project.cast().unwrap().characters.iter().any(|c| c.name.as_deref() == Some("Walter")));
        let again = all(0.02);
        let (a, b): (Vec<_>, Vec<_>) = again.into_iter().partition(|t| t.start < 1000);
        project.store_face_chunk(id, 0, 1000, &tracks::encode(&a), &[]).unwrap();
        // Another media's restart reclusters while this pass is half done:
        // its new faces must not become new characters meanwhile.
        project.recluster().unwrap();
        let mid: Vec<CharacterId> = project.cast().unwrap().characters.iter().map(|c| c.id).collect();
        assert_eq!(mid.len(), 2);
        assert!(mid.contains(&walter.id) && mid.contains(&other.id));
        project.store_face_chunk(id, 1000, frames, &tracks::encode(&b), &[]).unwrap();
        project.finish_faces(id).unwrap();
        project.recluster().unwrap();
        let cast = project.cast().unwrap();
        let w = cast.characters.iter().find(|c| c.name.as_deref() == Some("Walter")).expect("Walter survived");
        assert_eq!(w.id, walter.id);
        let mine = project.cast_tracks(TrackFilter::Character(w.id)).unwrap();
        assert_eq!(mine.len(), 10, "the 'not them' decision survived");
        assert!(mine.iter().all(|t| t.start != 300));
        // The character nobody named or touched is still the same one.
        let o = cast.characters.iter().find(|c| c.id == other.id).expect("the unnamed character survived");
        assert_eq!((o.label.as_str(), o.tracks), (other.label.as_str(), other.tracks));
        assert_eq!(cast.characters.len(), 2);
    }
}
