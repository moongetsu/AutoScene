//! Faces and characters: the face pass's checkpoints (tracks plus thumbnail
//! files), clustering tracks into the project's cast, and every edit the
//! user makes to it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use as_analysis::FaceParams;
use as_analysis::tracker::TrackerParams;
use as_analysis::tracks::{self, FaceTrack};
use as_api::{
    Cast, CastEdit, Character, ClipRequest, FaceTrackItem, MediaInfo, Presence, SegmentKind, StageState, TrackAssign,
    TrackFilter,
};
use as_core::{CharacterId, FrameIdx, FrameRange, MediaId};
use as_db::{IdentityTrack, NewTrack, TrackRow};
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

fn mean_box(dets: &[as_analysis::FaceDet]) -> [f32; 4] {
    let n = dets.len().max(1) as f32;
    let mut b = [0f32; 4];
    for d in dets {
        for (x, v) in b.iter_mut().zip(d.bbox) {
            *x += v / n;
        }
    }
    b
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
    /// Face thumbnails, one folder per media.
    pub fn faces_dir(&self) -> PathBuf {
        self.dir.join("faces")
    }

    fn media_faces_dir(&self, media: MediaId) -> PathBuf {
        self.faces_dir().join(media.to_string())
    }

    fn thumb_path(&self, media: MediaId, track: i64) -> PathBuf {
        self.media_faces_dir(media).join(format!("{track}.jpg"))
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

    /// Store one chunk from the engine (`as_analysis::tracks::encode`):
    /// tracks in the database, thumbnails as files, atomically. Returns the
    /// number of tracks.
    pub fn store_face_chunk(&self, id: MediaId, start: FrameIdx, end: FrameIdx, blob: &[u8]) -> ProjectResult<usize> {
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
        let dir = self.media_faces_dir(id);
        self.db().put_face_chunk(id, start, end, &rows, |ids| {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            for (track, t) in ids.iter().zip(&found) {
                let path = self.thumb_path(id, *track);
                std::fs::write(&path, &t.thumb).map_err(|e| format!("{}: {e}", path.display()))?;
            }
            Ok(())
        })?;
        Ok(found.len())
    }

    pub fn set_faces_state(&self, id: MediaId, state: StageState, error: Option<&str>) -> ProjectResult<()> {
        Ok(self.db().set_faces_state(id, state, error)?)
    }

    /// The face pass of `id` is complete: put the user's earlier decisions
    /// back on the new tracks (same time span and place on screen), then
    /// mark the faces ready.
    pub fn finish_faces(&self, id: MediaId) -> ProjectResult<()> {
        let edits = self.db().track_edits(id)?;
        if !edits.is_empty() {
            let new: Vec<(i64, FrameRange, [f32; 4])> = self
                .db()
                .media_track_dets(id)?
                .into_iter()
                .filter_map(|(t, s, e, d)| Some((t, FrameRange::new(s, e), mean_box(&tracks::dets_from_bytes(&d)?))))
                .collect();
            let mut matched = Vec::new();
            for e in &edits {
                let span = FrameRange::new(e.start, e.end);
                let bx = mean_box(&tracks::dets_from_bytes(&e.dets).unwrap_or_default());
                let best = new
                    .iter()
                    .map(|(t, r, b)| (r.iou(&span), box_iou(b, &bx), *t))
                    .filter(|&(ti, bi, _)| ti >= 0.5 && bi >= 0.5)
                    .max_by(|a, b| (a.0 + a.1).total_cmp(&(b.0 + b.1)));
                if let Some((_, _, t)) = best {
                    matched.push((t, e.kind));
                }
            }
            tracing::info!(media = %id, edits = edits.len(), kept = matched.len(), "re-anchored cast edits");
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

    /// Every shot a character is seen in, as clips (the shot's clean range),
    /// in episode order.
    pub fn character_clips(&self, c: CharacterId) -> ProjectResult<Vec<ClipRequest>> {
        let mut by_media: HashMap<MediaId, Vec<FrameIdx>> = HashMap::new();
        for (m, shot) in self.db().character_shots(c)? {
            by_media.entry(m).or_default().push(shot);
        }
        let mut clips = Vec::new();
        for m in self.media_list()? {
            let Some(mut shots) = by_media.remove(&m.id) else { continue };
            shots.sort_unstable();
            let segments = self.segments(m.id)?;
            for s in shots {
                if let Ok(i) = segments.binary_search_by_key(&s, |seg| seg.start)
                    && segments[i].kind == SegmentKind::Shot
                {
                    clips.push(ClipRequest { media_id: m.id, start: s, end: segments[i].end, name: None });
                }
            }
        }
        Ok(clips)
    }
}

#[cfg(test)]
mod tests {
    use as_analysis::FaceDet;
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
        assert_eq!(project.store_face_chunk(id, 0, 1000, &tracks::encode(&a)).unwrap(), 11);
        assert_eq!(project.faces_plan(id).unwrap().resume, 1000);
        project.store_face_chunk(id, 1000, frames, &tracks::encode(&b)).unwrap();
        project.finish_faces(id).unwrap();
        assert!(project.pending_faces().unwrap().is_empty());
        assert_eq!(project.recluster().unwrap(), 2);

        let cast = project.cast().unwrap();
        assert_eq!(cast.characters.len(), 2);
        let walter = cast.characters[0].clone();
        assert_eq!(walter.tracks, 11);
        assert!(std::path::Path::new(walter.cover.as_deref().unwrap()).is_file());
        project.cast_edit(&CastEdit::Rename { character_id: walter.id, name: Some(" Walter ".into()) }).unwrap();
        let clips = project.character_clips(walter.id).unwrap();
        assert_eq!(clips.len(), 11);
        assert_eq!((clips[0].start, clips[0].end), (0, 100));
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
        project.store_face_chunk(id, 0, frames, &tracks::encode(&again)).unwrap();
        project.finish_faces(id).unwrap();
        project.recluster().unwrap();
        let cast = project.cast().unwrap();
        let w = cast.characters.iter().find(|c| c.name.as_deref() == Some("Walter")).expect("Walter survived");
        assert_eq!(w.id, walter.id);
        let mine = project.cast_tracks(TrackFilter::Character(w.id)).unwrap();
        assert_eq!(mine.len(), 10, "the 'not them' decision survived");
        assert!(mine.iter().all(|t| t.start != 300));
    }
}
