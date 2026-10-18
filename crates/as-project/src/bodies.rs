//! People followed by their bodies (an optional mode, the library's
//! setting): their checkpoints, the user's word on them put back after a
//! new analysis, who they are after every clustering, and what they add to
//! a character — screen time, shots without their face, the timeline.

use std::collections::HashMap;
use std::path::PathBuf;

use as_analysis::PeopleParams;
use as_analysis::bodies::{self, BodyDet, FoundBody, Showing};
use as_analysis::tracks::{self, KnownFace};
use as_api::{MediaInfo, SegmentKind, StageState};
use as_core::{CharacterId, FrameIdx, FrameRange, MediaId};
use as_db::{BodyEditKind, BodyRow, BodySource, BodyUpdate, DbResult, IdentityBody, IdentityTrack, NewBody, ProjectDb};
use as_identity::bodies::{BodyAssign, BodyIn, BodyOutcome, BodyParams, FaceOn, propagate};
use as_identity::{CharKey, Outcome};

use crate::cast::{FACE_CHUNK, IdentityInput};
use crate::{Project, ProjectError, ProjectResult};

/// What the engine needs to find a media's people for its faces found before.
#[derive(Clone, Debug)]
pub struct BodiesPlan {
    pub path: PathBuf,
    pub info: MediaInfo,
    /// `FrameIndex::to_bytes`.
    pub index: Vec<u8>,
    pub shots: Vec<(FrameIdx, FrameIdx)>,
    pub frames: u32,
    /// First frame without a checkpoint (`frames` = done).
    pub resume: FrameIdx,
    pub chunk: u32,
    /// The media's faces (`as_analysis::tracks::encode_known`), by id.
    pub known: Vec<u8>,
}

/// A found body as the database keeps it, its faces named by `face`.
fn new_body(b: &FoundBody, face: impl Fn(u32) -> Option<i64>) -> NewBody {
    let Showing { height, aspect, top, foreground, score } = b.showing;
    NewBody {
        start: b.start,
        end: b.end,
        shot: b.shot,
        dets: bodies::dets_to_bytes(&b.dets),
        reid: b.reid.as_deref().map(bodies::reid_to_bytes),
        reid_away: b.reid_away.as_deref().map(bodies::reid_to_bytes),
        height,
        aspect,
        top,
        foreground,
        score,
        faces: b.faces.iter().filter_map(|&(f, samples)| Some((face(f)?, samples))).collect(),
    }
}

/// The shape a stored body shows in.
pub(crate) fn showing_of(b: &BodyRow) -> Showing {
    Showing { height: b.height, aspect: b.aspect, top: b.top, foreground: b.foreground, score: b.score }
}

/// When a stored body is on screen.
pub(crate) fn spans_of(dets: &[u8], end: FrameIdx) -> Vec<(FrameIdx, FrameIdx)> {
    let p = PeopleParams::default();
    bodies::dets_from_bytes(dets).map(|d| bodies::spans_of(&d, end, p.stride, p.span_gap())).unwrap_or_default()
}

/// Whether two of a media's bodies are the same person: they share most of
/// the shorter of the two in time, at the same place on screen while they
/// do. Returns the frames they share.
fn same_body(old: (FrameRange, &[BodyDet]), new: (FrameRange, &[BodyDet])) -> Option<u32> {
    let both = old.0.intersect(&new.0)?;
    if 2 * both.len() < old.0.len().min(new.0.len()) {
        return None;
    }
    let mean = |dets: &[BodyDet]| -> Option<[f32; 4]> {
        let inside: Vec<[f32; 4]> = dets.iter().filter(|d| both.contains(d.frame)).map(|d| d.visible).collect();
        let pick = if inside.is_empty() {
            let mid = both.start + both.len() / 2;
            vec![dets.iter().min_by_key(|d| d.frame.abs_diff(mid))?.visible]
        } else {
            inside
        };
        let n = pick.len() as f32;
        Some(std::array::from_fn(|k| pick.iter().map(|b| b[k]).sum::<f32>() / n))
    };
    let (a, b) = (mean(old.1)?, mean(new.1)?);
    (crate::cast::box_iou(&a, &b) >= 0.5).then_some(both.len())
}

/// The user's word found again on new bodies (`(frames shared, body, word)`)
/// made consistent, one body at a time: "not a person" or "this is X",
/// whichever shares most, holds ("this is X" only if no "not X" shares
/// more); every other "not Y" holds too.
fn resolve(mut found: Vec<(u32, i64, BodyEditKind)>) -> Vec<(i64, BodyEditKind)> {
    found.sort_by_key(|&(shared, body, _)| (body, std::cmp::Reverse(shared)));
    let mut out = Vec::new();
    for group in found.chunk_by(|a, b| a.1 == b.1) {
        let body = group[0].1;
        let forbids: Vec<(u32, CharacterId)> = group
            .iter()
            .filter_map(|&(s, _, k)| if let BodyEditKind::Forbid(c) = k { Some((s, c)) } else { None })
            .collect();
        let decided =
            group.iter().find(|(_, _, k)| matches!(k, BodyEditKind::Pin(_) | BodyEditKind::NotPerson)).filter(
                |&&(s, _, k)| !matches!(k, BodyEditKind::Pin(c) if forbids.iter().any(|&(fs, f)| f == c && fs > s)),
            );
        out.extend(
            forbids
                .iter()
                .filter(|&&(_, c)| !matches!(decided, Some(&(_, _, BodyEditKind::Pin(p))) if p == c))
                .map(|&(_, c)| (body, BodyEditKind::Forbid(c))),
        );
        if let Some(&(_, _, k)) = decided {
            out.push((body, k));
        }
    }
    out
}

impl Project {
    pub fn set_bodies_state(&self, id: MediaId, state: StageState, error: Option<&str>) -> ProjectResult<()> {
        Ok(self.db().set_bodies_state(id, state, error)?)
    }

    /// Media whose faces are found and whose people aren't, in episode
    /// order — an interrupted run too: for a launch, when nothing runs yet.
    pub fn pending_bodies(&self) -> ProjectResult<Vec<MediaId>> {
        Ok(self.db().pending_bodies()?)
    }

    /// Media whose people wait to be found, in episode order (those being
    /// found now left as they are).
    pub fn waiting_bodies(&self) -> ProjectResult<Vec<MediaId>> {
        Ok(self.db().waiting_bodies()?)
    }

    /// Where the people pass of `id` resumes — over its faces found before,
    /// which the plan carries. Checkpoints that don't line up are dropped
    /// with everything after them.
    pub fn bodies_plan(&self, id: MediaId) -> ProjectResult<BodiesPlan> {
        let (m, info, index) = self.ready_source(id)?;
        if m.faces != StageState::Ready {
            return Err(ProjectError::NotReady("The faces of this media aren't found yet.".into()));
        }
        let frames = index.len();
        let shots: Vec<(FrameIdx, FrameIdx)> =
            self.segments(id)?.into_iter().filter(|s| s.kind == SegmentKind::Shot).map(|s| (s.start, s.end)).collect();
        let mut resume = 0;
        for (start, end) in self.db().body_chunks(id)? {
            let fits = start == resume
                && end > start
                && end <= frames
                && (end == frames || shots.binary_search_by_key(&end, |s| s.0).is_ok());
            if !fits {
                tracing::info!(file = %m.file_name, start, "dropping people checkpoints that don't line up");
                self.db().drop_body_chunks_from(id, start)?;
                break;
            }
            resume = end;
        }
        let known: Vec<KnownFace> = self
            .db()
            .known_faces(id)?
            .into_iter()
            .filter_map(|(t, start, end, shot, dets)| {
                Some(KnownFace {
                    id: crate::cast::track_id(t),
                    start,
                    end,
                    shot,
                    dets: tracks::dets_from_bytes(&dets)?,
                })
            })
            .collect();
        Ok(BodiesPlan {
            path: PathBuf::from(&m.path),
            info,
            index: index.to_bytes(),
            shots,
            frames,
            resume,
            chunk: FACE_CHUNK,
            known: tracks::encode_known(&known),
        })
    }

    /// Store the people of a face chunk (`as_analysis::bodies::encode`),
    /// their faces named by their place in the chunk's tracks (`ids`).
    pub(crate) fn store_chunk_bodies(
        &self,
        id: MediaId,
        start: FrameIdx,
        end: FrameIdx,
        blob: &[u8],
        ids: &[i64],
    ) -> ProjectResult<usize> {
        let found = bodies::decode(blob).ok_or_else(|| ProjectError::Invalid("unreadable people".into()))?;
        let rows: Vec<NewBody> = found.iter().map(|b| new_body(b, |f| ids.get(f as usize).copied())).collect();
        self.db().put_body_chunk(id, start, end, &rows)?;
        Ok(rows.len())
    }

    /// Store one chunk of a people pass over faces found before: its faces
    /// are named by their track ids. Returns the number of people.
    pub fn store_body_chunk(&self, id: MediaId, start: FrameIdx, end: FrameIdx, blob: &[u8]) -> ProjectResult<usize> {
        let found = bodies::decode(blob).ok_or_else(|| ProjectError::Invalid("unreadable people".into()))?;
        let rows: Vec<NewBody> = found.iter().map(|b| new_body(b, |f| Some(i64::from(f)))).collect();
        self.db().put_body_chunk(id, start, end, &rows)?;
        Ok(rows.len())
    }

    /// The people of `id` are all found: put the user's earlier word on
    /// them back (see [`same_body`] and [`resolve`]), then mark them ready.
    pub fn finish_bodies(&self, id: MediaId) -> ProjectResult<()> {
        let edits = self.db().body_edits(id)?;
        if !edits.is_empty() {
            let new: Vec<(i64, FrameRange, Vec<BodyDet>)> = self
                .db()
                .media_body_dets(id)?
                .into_iter()
                .filter_map(|(b, s, e, d)| Some((b, FrameRange::new(s, e), bodies::dets_from_bytes(&d)?)))
                .collect();
            let mut found: Vec<(u32, i64, BodyEditKind)> = Vec::new();
            for e in &edits {
                let span = FrameRange::new(e.start, e.end);
                let dets = bodies::dets_from_bytes(&e.dets).unwrap_or_default();
                found.extend(
                    new.iter().filter_map(|(b, r, d)| same_body((span, &dets), (*r, d)).map(|s| (s, *b, e.kind))),
                );
            }
            let matched = resolve(found);
            tracing::info!(media = %id, edits = edits.len(), bodies = matched.len(), "re-anchored decisions on people");
            self.db().reanchor_body_edits(id, &matched)?;
        }
        Ok(self.db().set_bodies_state(id, StageState::Ready, None)?)
    }

    /// What `read` gives of the stored bodies — nothing while characters
    /// aren't looked for by their bodies: the cast, packs and timelines are
    /// their faces' alone then, the user's word on a body too (the bodies
    /// stay for when they are looked for again).
    pub(crate) fn shown_bodies<T>(&self, read: impl FnOnce(&ProjectDb) -> DbResult<Vec<T>>) -> ProjectResult<Vec<T>> {
        if !self.finds_bodies() {
            return Ok(Vec::new());
        }
        Ok(read(self.db())?)
    }

    /// Who the people are, after `out` placed the faces of `cast` — when
    /// characters are looked for by their bodies at all.
    pub(crate) fn people_outcome(
        &self,
        rows: &[IdentityTrack],
        cast: &IdentityInput,
        out: &Outcome,
    ) -> ProjectResult<Option<(Vec<IdentityBody>, BodyOutcome)>> {
        if !self.finds_bodies() {
            return Ok(None);
        }
        let found = self.db().identity_bodies()?;
        let track_at: HashMap<i64, usize> = cast.rows.iter().enumerate().map(|(i, &row)| (rows[row].id, i)).collect();
        // The media's frame rates only: not counted, nor looked for on disk.
        let fps: HashMap<MediaId, f32> = self
            .db()
            .media_list_uncounted()?
            .into_iter()
            .filter_map(|m| Some((m.id, m.fps.map(|f| f.num as f32 / f.den.max(1) as f32)?)))
            .collect();
        let mut media = cast.media.clone();
        let mut kept = Vec::with_capacity(found.len());
        let mut input = Vec::with_capacity(found.len());
        for b in found {
            let spans = spans_of(&b.dets, b.end);
            if spans.is_empty() {
                continue;
            }
            let next = media.len() as u32;
            input.push(BodyIn {
                media: *media.entry(b.media_id).or_insert(next),
                fps: fps.get(&b.media_id).copied().unwrap_or(24.0),
                spans,
                faces: b
                    .faces
                    .iter()
                    .filter_map(|&(t, samples)| Some(FaceOn { track: *track_at.get(&t)?, samples }))
                    .collect(),
                reid: b.reid.as_deref().and_then(bodies::reid_from_bytes),
                reid_away: b.reid_away.as_deref().and_then(bodies::reid_from_bytes),
                score: b.score,
                // A character the cast doesn't know is no one's key: the body stays the user's.
                pin: b.pin.map(|c| cast.key.get(&c).copied().unwrap_or(CharKey::MAX)),
                forbid: b.forbids.iter().filter_map(|c| cast.key.get(c).copied()).collect(),
            });
            kept.push(b);
        }
        let outcome = propagate(&cast.tracks, out, &input, &BodyParams::default());
        Ok(Some((kept, outcome)))
    }
}

/// What an identity outcome makes of the bodies, characters by `ids`.
pub(crate) fn body_updates(found: &[IdentityBody], outcome: &BodyOutcome, ids: &[CharacterId]) -> Vec<BodyUpdate> {
    found
        .iter()
        .zip(&outcome.bodies)
        .filter_map(|(b, a)| {
            let (character, source, similarity) = match *a {
                BodyAssign::Face(c) => (Some(ids[c]), BodySource::Face, 1.0),
                BodyAssign::Reid { character, score, .. } => (Some(ids[character]), BodySource::Clothes, score),
                BodyAssign::Conflict => (None, BodySource::Conflict, 0.0),
                BodyAssign::Pinned(_) | BodyAssign::None => return None,
            };
            Some(BodyUpdate { id: b.id, character, source, similarity })
        })
        .collect()
}

/// Union length of frame ranges.
pub(crate) fn covered(mut v: Vec<(FrameIdx, FrameIdx)>) -> u32 {
    v.sort_unstable();
    let (mut total, mut reach) = (0u32, 0u32);
    for (a, b) in v {
        let a = a.max(reach);
        if b > a {
            total += b - a;
            reach = b;
        }
    }
    total
}

/// The parts of `spans` outside `faces` (both sorted ranges).
pub(crate) fn outside(spans: &[(FrameIdx, FrameIdx)], faces: &[(FrameIdx, FrameIdx)]) -> Vec<(FrameIdx, FrameIdx)> {
    let mut out = Vec::new();
    for &(a, b) in spans {
        let mut from = a;
        for &(fa, fb) in faces.iter().filter(|&&(fa, fb)| fa < b && a < fb) {
            if fa > from {
                out.push((from, fa));
            }
            from = from.max(fb);
        }
        if from < b {
            out.push((from, b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(frame: FrameIdx, x: f32) -> BodyDet {
        let b = [x, 0.1, x + 0.2, 0.9];
        BodyDet { frame, bbox: b, visible: b, score: 0.9, sharpness: 100.0, luma: 100.0, rival: 0.0 }
    }

    #[test]
    fn a_body_found_again_is_the_one_at_the_same_place() {
        let old: Vec<BodyDet> = (0..10).map(|i| det(i * 6, 0.1)).collect();
        let near: Vec<BodyDet> = (2..12).map(|i| det(i * 6, 0.12)).collect();
        let other: Vec<BodyDet> = (0..10).map(|i| det(i * 6, 0.6)).collect();
        let r = |d: &[BodyDet]| FrameRange::new(d[0].frame, d.last().unwrap().frame + 6);
        assert_eq!(same_body((r(&old), &old), (r(&near), &near)), Some(48));
        assert_eq!(same_body((r(&old), &old), (r(&other), &other)), None, "someone else at the same time");
    }

    #[test]
    fn the_users_word_on_one_body_is_made_consistent() {
        let (x, y) = (CharacterId::new(), CharacterId::new());
        let mut got = resolve(vec![
            (30, 1, BodyEditKind::Pin(x)),
            (10, 1, BodyEditKind::Forbid(x)),
            (5, 1, BodyEditKind::Forbid(y)),
        ]);
        got.sort_by_key(|(_, k)| format!("{k:?}"));
        assert_eq!(got, [(1, BodyEditKind::Forbid(y)), (1, BodyEditKind::Pin(x))]);
        assert_eq!(
            resolve(vec![(10, 2, BodyEditKind::Pin(x)), (30, 2, BodyEditKind::Forbid(x))]),
            [(2, BodyEditKind::Forbid(x))]
        );
        assert_eq!(
            resolve(vec![(10, 3, BodyEditKind::Pin(x)), (30, 3, BodyEditKind::NotPerson)]),
            [(3, BodyEditKind::NotPerson)]
        );
    }

    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use as_analysis::tracks::FaceTrack;
    use as_api::{CastEdit, PackOrder, PackSpec, Segment, TrackAssign, TrackFilter};

    use crate::analysis::tests::project_with_media;
    use crate::cast::tests::seen;
    use crate::cast::track_id;

    /// Someone seen every 6 frames of `from..to` in shot `shot` at `x`, as
    /// tall as `height`, looking like `look` (a direction of 4).
    fn person(
        shot: FrameIdx,
        from: FrameIdx,
        to: FrameIdx,
        x: f32,
        width: f32,
        height: f32,
        look: [f32; 4],
    ) -> FoundBody {
        let v = [x, 1.0 - height, x + width, 1.0];
        let dets: Vec<BodyDet> = (from..to)
            .step_by(6)
            .map(|frame| BodyDet { frame, bbox: v, visible: v, score: 0.9, sharpness: 100.0, luma: 100.0, rival: 0.0 })
            .collect();
        let mut reid = vec![0f32; bodies::REID_DIM];
        reid[..4].copy_from_slice(&look);
        FoundBody {
            start: from,
            end: to,
            shot,
            showing: bodies::showing(&dets, 16.0 / 9.0),
            dets,
            faces: Vec::new(),
            reid: Some(reid),
            reid_away: None,
        }
    }

    const WALTER: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const JESSE: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    /// Walter's clothes, a little different (cos 0.9).
    const LIKE_WALTER: [f32; 4] = [0.9, 0.0, 0.0, 0.43589];

    /// One episode of 100-frame shots: Walter in shot 0 (and, too blurred
    /// to tell, again later on the same body), Jesse in shot 100; in shot
    /// 200 Walter from behind, and in shot 300 only his hand.
    fn episode(project: &Project, id: MediaId) {
        let shots: Vec<Segment> = (0..2400)
            .step_by(100)
            .map(|s| Segment { start: s, end: s + 100, kind: SegmentKind::Shot, tiny: false })
            .collect();
        project.db().store_segments(id, &shots).unwrap();
        // Too blurred to look like anyone.
        let mut blurred = seen(5, 0, 66, 90, 0.4);
        blurred.quality = 0.1;
        blurred.dets.iter_mut().for_each(|d| d.quality = 0.1);
        let faces: Vec<FaceTrack> = vec![seen(1, 0, 0, 60, 0.4), seen(2, 100, 100, 160, 0.4), blurred];
        let mut walter = person(0, 0, 96, 0.35, 0.3, 0.9, WALTER);
        walter.faces = vec![(0, 8), (2, 4)];
        let mut jesse = person(100, 100, 160, 0.35, 0.3, 0.9, JESSE);
        jesse.faces = vec![(1, 8)];
        let back = person(200, 200, 290, 0.3, 0.3, 0.9, LIKE_WALTER);
        let hand = person(300, 300, 360, 0.0, 0.9, 0.97, LIKE_WALTER);
        let found = bodies::encode(&[walter, jesse, back, hand]);
        project.store_face_chunk(id, 0, 2400, &tracks::encode(&faces), &[], &found).unwrap();
        project.finish_faces(id).unwrap();
        project.finish_bodies(id).unwrap();
    }

    #[test]
    fn characters_are_found_by_their_bodies_when_asked_to() {
        let (_dir, project, id) = project_with_media(2400);
        episode(&project, id);
        let faces = project.cast_tracks(TrackFilter::Unassigned).unwrap();
        let face = |start| faces.iter().find(|t| t.start == start).unwrap().id;
        let new = |track, name: &str| {
            let edit = CastEdit::NewCharacter { tracks: vec![track], name: Some(name.into()) };
            project.cast_edit(&edit).unwrap().unwrap()
        };
        let (walter, jesse) = (new(face(0), "Walter"), new(face(100), "Jesse"));
        // Every face of the episode: when, whose, how.
        let state = |p: &Project| -> Vec<(FrameIdx, Option<CharacterId>, TrackAssign)> {
            let ids: Vec<i64> = p.db().media_track_dets(id).unwrap().into_iter().map(|t| t.0).collect();
            let mut v = p.db().tracks_by_id(&ids).unwrap();
            v.sort_by_key(|t| t.start);
            v.into_iter().map(|t| (t.start, t.character_id, t.assign)).collect()
        };

        // Faces only: the blurred face is nobody's, Walter is in one shot.
        project.reassign().unwrap();
        let face_only = state(&project);
        assert_eq!(face_only[1].1, None, "the blurred face says nothing");
        let shots =
            |p: &Project| p.character_shots(walter).unwrap().iter().map(|s| (s.start, s.faceless)).collect::<Vec<_>>();
        assert_eq!(shots(&project), [(0, false)]);

        // With bodies: his body names the blurred face, and his clothes find him from behind.
        project.set_finds_bodies(true);
        project.reassign().unwrap();
        let mut with = state(&project);
        assert_eq!((with[1].1, with[1].2), (Some(walter), TrackAssign::Auto), "named by his body");
        with[1] = face_only[1];
        assert_eq!(with, face_only, "every other face as before");
        assert_eq!(shots(&project), [(0, false), (200, true)], "not the shot of his hand");
        let back = project.character_shots(walter).unwrap().pop().unwrap();
        assert!(back.bodies.len() == 1 && back.track_id == 0 && back.face.ends_with(".jpg"));
        assert!(project.character_shots(jesse).unwrap().iter().all(|s| !s.faceless));

        // The pack: the shot without his face behind the one with it, and a filter for it.
        let preview = |spec: &PackSpec| project.pack_preview(walter, spec).unwrap();
        let best = preview(&PackSpec { order: PackOrder::Best, ..PackSpec::default() });
        assert_eq!(best.clips.iter().map(|c| (c.shot.start, c.included)).collect::<Vec<_>>(), [(0, true), (200, true)]);
        let skip = preview(&PackSpec { skip_faceless: true, ..PackSpec::default() });
        assert_eq!(skip.clips[1].reason, Some(as_api::PackReason::Faceless));
        let one = preview(&PackSpec { limit: Some(1), ..PackSpec::default() });
        assert_eq!(one.clips.iter().filter(|c| c.included).map(|c| c.shot.start).collect::<Vec<_>>(), [0]);
        assert!(one.clips[1].coverage > 0.8, "on screen by his body");

        // The cast and the timeline count his body too.
        let card = |p: &Project| p.cast().unwrap().characters.into_iter().find(|c| c.id == walter).unwrap();
        let w = card(&project);
        assert_eq!(w.shots, 2);
        assert!(w.screen_sec > 9.0, "{}", w.screen_sec);
        let presence = project.presence(id).unwrap();
        let bodies: Vec<_> =
            presence.iter().filter(|p| p.body && p.character_id == walter).map(|p| (p.start, p.end)).collect();
        assert_eq!(bodies.first(), Some(&(60, 66)), "where neither face of his shows");
        assert!(bodies.iter().any(|&(a, _)| a == 300), "his hand is him, on screen");

        // "Not Walter" on the back, "not a person" on the hand: and after a new analysis still.
        let ids =
            |p: &Project| p.character_shots(walter).unwrap().into_iter().flat_map(|s| s.bodies).collect::<Vec<_>>();
        let back_id = ids(&project)[0];
        project.cast_edit(&CastEdit::BodyNotThem { bodies: vec![back_id], character_id: walter }).unwrap();
        let hand = presence.iter().find(|p| p.body && p.start == 300).unwrap().track_id;
        project.cast_edit(&CastEdit::NotAPerson { bodies: vec![hand] }).unwrap();
        project.reassign().unwrap();
        assert_eq!(shots(&project), [(0, false)]);
        project.reset_faces(id).unwrap();
        episode(&project, id);
        project.recluster().unwrap();
        assert_eq!(shots(&project), [(0, false)], "the decisions came back");
        assert!(project.presence(id).unwrap().iter().all(|p| !(p.body && p.start >= 300)));
        // "This is Walter" on the back after all: in his pack again.
        let back_now = project.db().media_body_dets(id).unwrap().into_iter().find(|b| b.1 == 200).unwrap().0;
        assert_ne!(track_id(back_now), back_id, "a new analysis, new bodies");
        project.cast_edit(&CastEdit::BodyAssign { bodies: vec![track_id(back_now)], character_id: walter }).unwrap();
        project.reassign().unwrap();
        assert_eq!(shots(&project), [(0, false), (200, true)]);

        // Asked not to any more: everything as with faces alone, the user's
        // own word on a body too — until they are looked for again.
        let faces_alone = |p: &Project| {
            let now = state(p).iter().map(|t| (t.0, t.1)).collect::<Vec<_>>();
            now == face_only.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>()
        };
        project.set_finds_bodies(false);
        project.reassign().unwrap();
        assert!(faces_alone(&project));
        assert!(project.presence(id).unwrap().iter().all(|p| !p.body));
        assert_eq!((card(&project).shots, shots(&project)), (1, vec![(0, false)]));
        project.set_finds_bodies(true);
        project.reassign().unwrap();
        assert_eq!(shots(&project), [(0, false), (200, true)], "the pinned back is his again");
        assert!(!faces_alone(&project) && project.db().has_body_names().unwrap());

        // Turned off while the project wasn't open (or its faces couldn't be
        // placed again then): what the bodies said goes when it opens.
        let off = Arc::new(AtomicBool::new(false));
        let reopened = Project::open(project.id, project.dir.clone(), off).unwrap();
        assert!(faces_alone(&reopened) && !reopened.db().has_body_names().unwrap());
        assert!(reopened.presence(id).unwrap().iter().all(|p| !p.body));
    }

    #[test]
    fn spans_outside_faces() {
        assert_eq!(outside(&[(0, 100)], &[(10, 20), (50, 60)]), [(0, 10), (20, 50), (60, 100)]);
        assert_eq!(outside(&[(0, 10), (30, 40)], &[(5, 35)]), [(0, 5), (35, 40)]);
        assert!(outside(&[(10, 20)], &[(0, 30)]).is_empty());
        assert_eq!(covered(vec![(0, 10), (5, 20), (30, 31)]), 21);
    }
}
