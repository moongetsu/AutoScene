//! Faces and characters in `project.db`.
//!
//! Tracks arrive per checkpoint chunk from the face pass; clustering then
//! points them at characters. Everything the user decides is stored on the
//! tracks themselves — `pinned` (this is X), a forbid (this is not X),
//! `not_face` — so a recluster can never undo it. Every decision bumps the
//! cast's revision; a clustering computed before it is not stored.

use as_api::{SegmentKind, StageState, TrackAssign, TrackFilter, now_ms};
use as_core::{CharacterId, FrameIdx, MediaId};
use rusqlite::{OptionalExtension, Transaction, params, params_from_iter};

use crate::project::ProjectDb;
use crate::{DbError, DbResult, enum_from_sql, enum_to_sql};

/// A track from the face pass, ready to store.
#[derive(Clone, Debug)]
pub struct NewTrack {
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub shot: FrameIdx,
    pub quality: f32,
    pub face_h: f32,
    /// `as_analysis::tracks::dets_to_bytes`.
    pub dets: Vec<u8>,
    /// 512 × f16.
    pub emb: Vec<u8>,
}

/// What clustering needs of a track.
#[derive(Clone, Debug)]
pub struct IdentityTrack {
    pub id: i64,
    pub media_id: MediaId,
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub quality: f32,
    pub dets: u32,
    /// `as_analysis::tracks::dets_to_bytes`.
    pub det_bytes: Vec<u8>,
    pub emb: Vec<u8>,
    pub character_id: Option<CharacterId>,
    pub assign: TrackAssign,
    /// Similarity behind the assignment; 1 for pins and clustered tracks.
    pub score: f32,
    pub forbids: Vec<CharacterId>,
    /// Its character came from its body, not from the face itself.
    pub via_body: bool,
}

/// What a clustering made of a track.
#[derive(Clone, Copy, Debug)]
pub struct TrackUpdate {
    pub id: i64,
    pub character: Option<CharacterId>,
    pub assign: TrackAssign,
    pub score: f32,
    /// The character came from the track's body.
    pub via_body: bool,
}

#[derive(Clone, Debug)]
pub struct TrackRow {
    pub id: i64,
    pub media_id: MediaId,
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub shot: FrameIdx,
    pub quality: f32,
    pub face_h: f32,
    pub character_id: Option<CharacterId>,
    pub assign: TrackAssign,
    pub score: f32,
}

#[derive(Clone, Debug)]
pub struct CharacterRow {
    pub id: CharacterId,
    pub number: u32,
    pub name: Option<String>,
    pub hidden: bool,
    /// The chosen cover track, else the best track; with its media.
    pub cover: Option<(i64, MediaId)>,
    pub tracks: u32,
    pub shots: u32,
    pub media: u32,
    pub screen_sec: f64,
    pub unsure: u32,
    /// Decisions and faces waiting for a media being analysed again (not
    /// for one whose analysis failed or was stopped).
    pub waiting: u32,
}

/// A shot a character is seen in.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterShotRow {
    pub media_id: MediaId,
    pub start: FrameIdx,
    pub end: FrameIdx,
    /// The shot's sprite, if it has one: the shot end it was made for, and its cells.
    pub sprite: Option<(FrameIdx, u32)>,
    /// The character's clearest face in the shot, and the biggest face
    /// height (fraction of the frame) it has there.
    pub track: i64,
    pub face_h: f32,
}

/// `(track, start, end, dets)`.
/// A track as decisions are re-anchored to it: id, range, detections, embedding.
pub type TrackDets = (i64, FrameIdx, FrameIdx, Vec<u8>, Vec<u8>);

/// A user decision — or the character clustering gave it — on a track that
/// was deleted for a re-analysis.
#[derive(Clone, Debug)]
pub struct TrackEdit {
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub dets: Vec<u8>,
    pub kind: EditKind,
    /// The face's embedding and the model that made it (decisions made
    /// before they were kept have neither).
    pub emb: Option<Vec<u8>>,
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Pin(CharacterId),
    Forbid(CharacterId),
    NotFace,
    /// Clustering had put the track on this character: the new track starts
    /// out there, so the next clustering keeps the character (its number,
    /// its place in the cast) instead of making a new one.
    Was(CharacterId),
    /// The user chose this face as the character's picture.
    Cover(CharacterId),
}

pub(crate) fn parse_id<T: std::str::FromStr>(s: &str) -> DbResult<T>
where
    T::Err: std::fmt::Display,
{
    s.parse().map_err(|e: T::Err| DbError::Corrupt(e.to_string()))
}

fn opt_id(s: Option<String>) -> DbResult<Option<CharacterId>> {
    s.as_deref().map(parse_id).transpose()
}

/// A character's (`c`) cover track: the chosen one while it is still theirs,
/// else their clearest big face.
pub(crate) const COVER: &str = "COALESCE((SELECT x.id FROM tracks x WHERE x.id = c.cover AND x.character_id = c.id
                                 AND x.assign IN ('auto', 'pinned') AND x.not_face = 0),
                              (SELECT b.id FROM tracks b WHERE b.character_id = c.id AND b.assign IN ('auto', 'pinned')
                                           AND b.not_face = 0
                                         ORDER BY b.face_h >= 0.12 DESC, b.quality DESC, b.face_h DESC LIMIT 1))";

/// `?, ?, ?` for an `IN (...)` list.
pub(crate) fn marks(n: usize) -> String {
    vec!["?"; n].join(", ")
}

/// Forget a media's faces, keeping the user's decisions on them — and who
/// clustering said they were — in `track_edits` for the next face pass to
/// re-anchor.
pub(crate) fn clear_faces(tx: &Transaction, id: MediaId) -> DbResult<()> {
    forget_tracks(tx, id)?;
    tx.execute("UPDATE media SET faces = 'pending', faces_error = NULL WHERE id = ?1", [id.to_string()])?;
    bump_rev(tx)
}

/// Delete a media's tracks, sprites and checkpoints, keeping the decisions
/// on its tracks (and who clustering said they were) in `track_edits`.
fn forget_tracks(tx: &Transaction, id: MediaId) -> DbResult<()> {
    let m = id.to_string();
    tx.execute(
        "INSERT INTO track_edits (media_id, start, end, dets, kind, character_id, emb, model)
         SELECT e.*, (SELECT faces_model FROM media WHERE id = ?1) FROM (
           SELECT media_id, start, end, dets, 'pin', character_id, emb FROM tracks
            WHERE media_id = ?1 AND assign = 'pinned' AND character_id IS NOT NULL
           UNION ALL
           SELECT t.media_id, t.start, t.end, t.dets, 'forbid', f.character_id, t.emb
             FROM tracks t JOIN track_forbids f ON f.track_id = t.id WHERE t.media_id = ?1
           UNION ALL
           SELECT media_id, start, end, dets, 'not_face', NULL, emb FROM tracks WHERE media_id = ?1 AND not_face = 1
           UNION ALL
           SELECT media_id, start, end, dets, 'was', character_id, emb FROM tracks
            WHERE media_id = ?1 AND assign = 'auto' AND character_id IS NOT NULL AND not_face = 0 AND via_body = 0
           UNION ALL
           SELECT t.media_id, t.start, t.end, t.dets, 'cover', c.id, t.emb FROM characters c JOIN tracks t ON t.id = c.cover
            WHERE t.media_id = ?1
         ) e",
        [&m],
    )?;
    // Track ids are handed out again: a cover must not point at someone else's face.
    tx.execute("UPDATE characters SET cover = NULL WHERE cover IN (SELECT id FROM tracks WHERE media_id = ?1)", [&m])?;
    // Bodies are tied to the faces that go.
    crate::bodies::forget_bodies(tx, id)?;
    tx.execute("DELETE FROM tracks WHERE media_id = ?1", [&m])?;
    tx.execute("DELETE FROM sprites WHERE media_id = ?1", [&m])?;
    tx.execute("DELETE FROM face_chunks WHERE media_id = ?1", [&m])?;
    Ok(())
}

/// A decision (or a stored clustering) changed the cast.
pub(crate) fn bump_rev(tx: &Transaction) -> DbResult<()> {
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('cast_rev', '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
        [],
    )?;
    Ok(())
}

fn rev_of(c: &rusqlite::Connection) -> DbResult<i64> {
    Ok(c.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'cast_rev'", [], |r| r.get(0))
        .optional()?
        .unwrap_or(0))
}

fn track_row(r: &rusqlite::Row) -> DbResult<TrackRow> {
    Ok(TrackRow {
        id: r.get(0)?,
        media_id: parse_id(&r.get::<_, String>(1)?)?,
        start: r.get(2)?,
        end: r.get(3)?,
        shot: r.get(4)?,
        quality: r.get::<_, f64>(5)? as f32,
        face_h: r.get::<_, f64>(6)? as f32,
        character_id: opt_id(r.get(7)?)?,
        assign: enum_from_sql(&r.get::<_, String>(8)?)?,
        score: r.get::<_, f64>(9)? as f32,
    })
}

const TRACK_COLS: &str =
    "t.id, t.media_id, t.start, t.end, t.shot, t.quality, t.face_h, t.character_id, t.assign, t.score";

/// Insert a character (numbered after every character there ever was).
fn insert_character(tx: &Transaction, id: CharacterId, name: Option<&str>) -> DbResult<()> {
    let number: i64 = tx.query_row(
        "SELECT MAX(COALESCE((SELECT MAX(number) FROM characters), 0), CAST(COALESCE((SELECT value FROM meta WHERE key = 'character_number'), '0') AS INTEGER)) + 1",
        [],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO characters (id, number, name, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id.to_string(), number, name, now_ms()],
    )?;
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('character_number', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [number.to_string()],
    )?;
    Ok(())
}

/// Characters nobody is in any more go — unless the user named or hid them,
/// or decisions on a media being analysed again wait for them. Suggestions
/// pointing at gone characters are dropped.
fn drop_empty_characters(tx: &Transaction) -> DbResult<()> {
    tx.execute(
        "DELETE FROM characters WHERE name IS NULL AND hidden = 0
           AND NOT EXISTS (SELECT 1 FROM tracks t WHERE t.character_id = characters.id
                             AND t.assign IN ('auto', 'pinned') AND t.not_face = 0 AND t.via_body = 0)
           AND NOT EXISTS (SELECT 1 FROM track_edits e WHERE e.character_id = characters.id)
           AND NOT EXISTS (SELECT 1 FROM bodies b WHERE b.character_id = characters.id AND b.source = 'pinned')
           AND NOT EXISTS (SELECT 1 FROM body_edits e WHERE e.character_id = characters.id AND e.kind = 'pin')",
        [],
    )?;
    tx.execute("UPDATE tracks SET assign = 'none', score = 0 WHERE character_id IS NULL AND assign <> 'none'", [])?;
    tx.execute("UPDATE characters SET cover = NULL WHERE cover IS NOT NULL AND NOT EXISTS (SELECT 1 FROM tracks t WHERE t.id = characters.cover AND t.character_id = characters.id AND t.assign IN ('auto', 'pinned'))", [])?;
    Ok(())
}

pub(crate) fn ids_param(ids: &[i64]) -> Vec<rusqlite::types::Value> {
    ids.iter().map(|&i| rusqlite::types::Value::Integer(i)).collect()
}

impl ProjectDb {
    // -- face pass ----------------------------------------------------------

    pub fn set_faces_state(&self, id: MediaId, state: StageState, error: Option<&str>) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE media SET faces = ?2, faces_error = ?3 WHERE id = ?1",
                params![id.to_string(), enum_to_sql(state), error],
            )?;
            Ok(())
        })
    }

    /// The embedder a media's stored tracks (finished or not) come from;
    /// `None` before models were recorded.
    pub fn faces_model(&self, id: MediaId) -> DbResult<Option<String>> {
        self.0.read(|c| {
            Ok(c.query_row("SELECT faces_model FROM media WHERE id = ?1", [id.to_string()], |r| r.get(0))
                .optional()?
                .flatten())
        })
    }

    /// The tracks of `id` are to come from `model`: what another one stored
    /// (tracks, sprites, checkpoints) goes, decisions on it wait in
    /// `track_edits`, and the pass starts over. Returns whether it did.
    pub fn use_faces_model(&self, id: MediaId, model: &str) -> DbResult<bool> {
        self.0.write(|tx| {
            let m = id.to_string();
            let old: Option<String> =
                tx.query_row("SELECT faces_model FROM media WHERE id = ?1", [&m], |r| r.get(0)).optional()?.flatten();
            if old.as_deref() == Some(model) {
                return Ok(false);
            }
            forget_tracks(tx, id)?;
            tx.execute("UPDATE media SET faces_model = ?2 WHERE id = ?1", params![m, model])?;
            bump_rev(tx)?;
            Ok(true)
        })
    }

    /// Media whose faces are found by another embedder than `model` (or one
    /// not recorded), left as they are.
    pub fn faces_by_other_model(&self, model: &str) -> DbResult<Vec<MediaId>> {
        self.0.read(|c| {
            let mut st = c.prepare(
                "SELECT id FROM media WHERE faces = 'ready' AND (faces_model IS NULL OR faces_model <> ?1) ORDER BY position",
            )?;
            let ids: Vec<String> = st.query_map([model], |r| r.get(0))?.collect::<Result<_, _>>()?;
            ids.iter().map(|s| parse_id(s)).collect()
        })
    }

    /// Media whose faces are found, by another embedder than `model` (or
    /// one not recorded): their faces are forgotten — the user's decisions
    /// and who clustering said they were wait in `track_edits` — so the
    /// face pass runs again. Returns them.
    pub fn outdated_faces(&self, model: &str) -> DbResult<Vec<MediaId>> {
        self.0.write(|tx| {
            let ids: Vec<String> = {
                let mut st = tx.prepare(
                    "SELECT id FROM media WHERE faces = 'ready' AND (faces_model IS NULL OR faces_model <> ?1)",
                )?;
                st.query_map([model], |r| r.get(0))?.collect::<Result<_, _>>()?
            };
            for id in &ids {
                clear_faces(tx, parse_id(id)?)?;
            }
            ids.iter().map(|s| parse_id(s)).collect()
        })
    }

    /// Finished chunks `(start, end)` of the face pass, in order.
    pub fn face_chunks(&self, id: MediaId) -> DbResult<Vec<(FrameIdx, FrameIdx)>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT start, end FROM face_chunks WHERE media_id = ?1 ORDER BY start")?;
            let rows = st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Drop chunks (and their tracks, sprites and the bodies tied to them)
    /// from `start` on.
    pub fn drop_face_chunks_from(&self, id: MediaId, start: FrameIdx) -> DbResult<()> {
        self.0.write(|tx| {
            crate::bodies::drop_bodies_from(tx, id, start)?;
            tx.execute("DELETE FROM tracks WHERE media_id = ?1 AND shot >= ?2", params![id.to_string(), start])?;
            tx.execute("DELETE FROM sprites WHERE media_id = ?1 AND shot >= ?2", params![id.to_string(), start])?;
            tx.execute("DELETE FROM face_chunks WHERE media_id = ?1 AND start >= ?2", params![id.to_string(), start])?;
            Ok(())
        })
    }

    /// The tracks a new chunk `start..end` replaces (a face pass resumed
    /// over a chunk it had stored before).
    pub fn chunk_tracks(&self, id: MediaId, start: FrameIdx, end: FrameIdx) -> DbResult<Vec<i64>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT id FROM tracks WHERE media_id = ?1 AND shot >= ?2 AND shot < ?3")?;
            let rows = st.query_map(params![id.to_string(), start, end], |r| r.get(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Store the tracks of chunk `start..end`, its shots' sprites `(shot,
    /// end, cells)` and the chunk itself, atomically. `files(ids)` runs before
    /// the commit (thumbnails, sprites); its error rolls back. Track ids are
    /// never handed out twice — not even ids of tracks deleted since: files
    /// named after a track (and the webview's cache of them) never show
    /// another face.
    pub fn put_face_chunk(
        &self,
        id: MediaId,
        start: FrameIdx,
        end: FrameIdx,
        tracks: &[NewTrack],
        sprites: &[(FrameIdx, FrameIdx, u32)],
        files: impl FnOnce(&[i64]) -> Result<(), String>,
    ) -> DbResult<Vec<i64>> {
        self.0.write(|tx| {
            let m = id.to_string();
            // Before the chunk's own tracks go: their ids count as handed out.
            let mut last: i64 = tx.query_row(
                "SELECT MAX(COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'track_seq'), 0),
                            COALESCE((SELECT MAX(id) FROM tracks), 0))",
                [],
                |r| r.get(0),
            )?;
            tx.execute("DELETE FROM tracks WHERE media_id = ?1 AND shot >= ?2 AND shot < ?3", params![m, start, end])?;
            tx.execute("DELETE FROM sprites WHERE media_id = ?1 AND shot >= ?2 AND shot < ?3", params![m, start, end])?;
            // People of the chunk were tied to the faces it replaces: they
            // come again with it (or from the people pass).
            tx.execute("DELETE FROM bodies WHERE media_id = ?1 AND shot >= ?2 AND shot < ?3", params![m, start, end])?;
            tx.execute(
                "DELETE FROM body_chunks WHERE media_id = ?1 AND start < ?3 AND end > ?2",
                params![m, start, end],
            )?;
            let mut st = tx.prepare("INSERT INTO sprites (media_id, shot, end, cells) VALUES (?1, ?2, ?3, ?4)")?;
            for &(shot, end, cells) in sprites {
                st.execute(params![m, shot, end, cells])?;
            }
            drop(st);
            let mut st = tx.prepare(
                "INSERT INTO tracks (id, media_id, start, end, shot, quality, face_h, dets, emb)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            let mut ids = Vec::with_capacity(tracks.len());
            for t in tracks {
                last += 1;
                st.execute(params![last, m, t.start, t.end, t.shot, t.quality as f64, t.face_h as f64, t.dets, t.emb])?;
                ids.push(last);
            }
            drop(st);
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('track_seq', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [last.to_string()],
            )?;
            tx.execute(
                "INSERT INTO face_chunks (media_id, start, end) VALUES (?1, ?2, ?3)
                 ON CONFLICT(media_id, start) DO UPDATE SET end = excluded.end",
                params![m, start, end],
            )?;
            files(&ids).map_err(DbError::Io)?;
            Ok(ids)
        })
    }

    /// Edits waiting to be re-anchored on the media's new tracks.
    pub fn track_edits(&self, id: MediaId) -> DbResult<Vec<TrackEdit>> {
        self.0.read(|c| {
            let mut st = c.prepare(
                "SELECT start, end, dets, kind, character_id, emb, model FROM track_edits WHERE media_id = ?1",
            )?;
            let mut rows = st.query([id.to_string()])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let kind: String = r.get(3)?;
                let ch = opt_id(r.get(4)?)?;
                let kind = match (kind.as_str(), ch) {
                    ("pin", Some(c)) => EditKind::Pin(c),
                    ("forbid", Some(c)) => EditKind::Forbid(c),
                    ("not_face", _) => EditKind::NotFace,
                    ("was", Some(c)) => EditKind::Was(c),
                    ("cover", Some(c)) => EditKind::Cover(c),
                    _ => continue,
                };
                out.push(TrackEdit {
                    start: r.get(0)?,
                    end: r.get(1)?,
                    dets: r.get(2)?,
                    kind,
                    emb: r.get(5)?,
                    model: r.get(6)?,
                });
            }
            Ok(out)
        })
    }

    /// Re-apply edits to new tracks (`(track, edit)`, one consistent set per
    /// track) and forget all of the media's pending edits. Decisions go
    /// first, each like the user made it; a former character only lands on a
    /// face nobody decided about and that may be it.
    pub fn reanchor_edits(&self, id: MediaId, matched: &[(i64, EditKind)]) -> DbResult<()> {
        self.0.write(|tx| {
            for &(track, kind) in matched {
                match kind {
                    EditKind::Pin(c) => {
                        let n = tx.execute(
                            "UPDATE tracks SET character_id = ?2, assign = 'pinned', score = 1, not_face = 0 WHERE id = ?1
                               AND EXISTS (SELECT 1 FROM characters WHERE id = ?2)",
                            params![track, c.to_string()],
                        )?;
                        if n > 0 {
                            tx.execute(
                                "DELETE FROM track_forbids WHERE track_id = ?1 AND character_id = ?2",
                                params![track, c.to_string()],
                            )?;
                        }
                    }
                    EditKind::Forbid(c) => {
                        tx.execute(
                            "INSERT OR IGNORE INTO track_forbids (track_id, character_id)
                             SELECT ?1, id FROM characters WHERE id = ?2",
                            params![track, c.to_string()],
                        )?;
                        tx.execute(
                            "UPDATE tracks SET character_id = NULL, assign = 'none', score = 0
                              WHERE id = ?1 AND character_id = ?2",
                            params![track, c.to_string()],
                        )?;
                    }
                    EditKind::NotFace => {
                        tx.execute(
                            "UPDATE tracks SET not_face = 1, character_id = NULL, assign = 'none' WHERE id = ?1",
                            [track],
                        )?;
                    }
                    EditKind::Was(_) | EditKind::Cover(_) => {}
                }
            }
            for &(track, kind) in matched {
                if let EditKind::Was(c) = kind {
                    tx.execute(
                        // Below 1: not a member that shapes the character
                        // until clustering says so again (see reassign).
                        "UPDATE tracks SET character_id = ?2, assign = 'auto', score = 0.99
                          WHERE id = ?1 AND assign = 'none' AND not_face = 0
                            AND EXISTS (SELECT 1 FROM characters WHERE id = ?2)
                            AND NOT EXISTS (SELECT 1 FROM track_forbids f WHERE f.track_id = ?1 AND f.character_id = ?2)",
                        params![track, c.to_string()],
                    )?;
                }
            }
            // The chosen picture comes back once its face is the character's again.
            for &(track, kind) in matched {
                if let EditKind::Cover(c) = kind {
                    tx.execute(
                        "UPDATE characters SET cover = ?1 WHERE id = ?2
                            AND EXISTS (SELECT 1 FROM tracks WHERE id = ?1 AND character_id = ?2
                                          AND assign IN ('auto', 'pinned') AND not_face = 0)",
                        params![track, c.to_string()],
                    )?;
                }
            }
            tx.execute("DELETE FROM track_edits WHERE media_id = ?1", [id.to_string()])?;
            bump_rev(tx)
        })
    }

    /// Tracks by id with their media and detections (for portraits); ids
    /// that don't exist are left out.
    pub fn track_dets(&self, ids: &[i64]) -> DbResult<Vec<(i64, MediaId, Vec<u8>)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.0.read(|c| {
            let mut st =
                c.prepare(&format!("SELECT id, media_id, dets FROM tracks WHERE id IN ({})", marks(ids.len())))?;
            let mut rows = st.query(params_from_iter(ids_param(ids)))?;
            let mut out = Vec::with_capacity(ids.len());
            while let Some(r) = rows.next()? {
                out.push((r.get(0)?, parse_id(&r.get::<_, String>(1)?)?, r.get(2)?));
            }
            Ok(out)
        })
    }

    /// Every track of a media with its detections (for re-anchoring edits).
    pub fn media_track_dets(&self, id: MediaId) -> DbResult<Vec<TrackDets>> {
        self.0.read(|c| {
            let mut st =
                c.prepare("SELECT id, start, end, dets, emb FROM tracks WHERE media_id = ?1 ORDER BY start")?;
            let rows =
                st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Media ready for the face pass (shots found) that haven't had it (an
    /// interrupted run counts as pending again), in episode order.
    pub fn pending_faces(&self) -> DbResult<Vec<MediaId>> {
        self.0.write(|tx| {
            tx.execute("UPDATE media SET faces = 'pending' WHERE faces = 'running'", [])?;
            let mut st = tx.prepare(
                "SELECT id FROM media WHERE status = 'ready' AND shots = 'ready' AND faces = 'pending'
                 ORDER BY episode IS NULL, COALESCE(season, 0), episode, position",
            )?;
            let ids = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            ids.iter().map(|s| parse_id(s)).collect()
        })
    }

    pub fn reset_faces(&self, id: MediaId) -> DbResult<()> {
        self.0.write(|tx| clear_faces(tx, id))
    }

    // -- cast ---------------------------------------------------------------

    /// The cast's revision. Read it before [`ProjectDb::identity_tracks`]
    /// and hand it to [`ProjectDb::apply_identity`].
    pub fn cast_rev(&self) -> DbResult<i64> {
        self.0.read(rev_of)
    }

    /// Every track that is a face, with what clustering needs — of media
    /// whose face pass is complete: a pass still running has stored only
    /// part of its tracks, without the earlier decisions on them yet.
    pub fn identity_tracks(&self) -> DbResult<Vec<IdentityTrack>> {
        self.0.read(|c| {
            let mut forbids: std::collections::HashMap<i64, Vec<CharacterId>> = Default::default();
            {
                let mut st = c.prepare("SELECT track_id, character_id FROM track_forbids")?;
                let mut rows = st.query([])?;
                while let Some(r) = rows.next()? {
                    forbids.entry(r.get(0)?).or_default().push(parse_id(&r.get::<_, String>(1)?)?);
                }
            }
            let mut st = c.prepare(
                "SELECT id, media_id, start, end, quality, length(dets) / 32, emb, character_id, assign, dets, score,
                        via_body
                   FROM tracks WHERE not_face = 0 AND media_id IN (SELECT id FROM media WHERE faces = 'ready')
                  ORDER BY media_id, start, id",
            )?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                out.push(IdentityTrack {
                    id,
                    media_id: parse_id(&r.get::<_, String>(1)?)?,
                    start: r.get(2)?,
                    end: r.get(3)?,
                    quality: r.get::<_, f64>(4)? as f32,
                    dets: r.get(5)?,
                    emb: r.get(6)?,
                    character_id: opt_id(r.get(7)?)?,
                    assign: enum_from_sql(&r.get::<_, String>(8)?)?,
                    det_bytes: r.get(9)?,
                    score: r.get::<_, f64>(10)? as f32,
                    forbids: forbids.remove(&id).unwrap_or_default(),
                    via_body: r.get(11)?,
                });
            }
            Ok(out)
        })
    }

    /// Store a clustering of the cast at revision `rev`: new characters,
    /// then every track's assignment (pinned tracks and faces that aren't
    /// keep theirs), and who every body is (what was derived before goes;
    /// the user's pins stay). Characters left empty go. Stores nothing and
    /// returns `false` when the cast changed since `rev`.
    pub fn apply_identity(
        &self,
        rev: i64,
        new_characters: &[CharacterId],
        tracks: &[TrackUpdate],
        bodies: &[crate::BodyUpdate],
    ) -> DbResult<bool> {
        self.0.write(|tx| {
            if rev_of(tx)? != rev {
                return Ok(false);
            }
            for &c in new_characters {
                insert_character(tx, c, None)?;
            }
            let mut st = tx.prepare(
                "UPDATE tracks SET character_id = ?2, assign = ?3, score = ?4, via_body = ?5
                  WHERE id = ?1 AND assign <> 'pinned' AND not_face = 0",
            )?;
            for t in tracks {
                let via_body = t.via_body && t.assign == TrackAssign::Auto;
                st.execute(params![
                    t.id,
                    t.character.map(|c| c.to_string()),
                    enum_to_sql(t.assign),
                    t.score as f64,
                    via_body
                ])?;
            }
            drop(st);
            crate::bodies::apply_bodies(tx, bodies)?;
            drop_empty_characters(tx)?;
            bump_rev(tx)?;
            Ok(true)
        })
    }

    /// Each character's cover track (as [`CharacterRow::cover`]), in the
    /// order characters were found: a fraction of what the whole cast costs.
    pub fn cover_tracks(&self) -> DbResult<Vec<i64>> {
        self.0.read(|c| {
            let mut st = c.prepare(&format!("SELECT {COVER} FROM characters c ORDER BY c.number"))?;
            let rows = st.query_map([], |r| r.get::<_, Option<i64>>(0))?;
            Ok(rows.filter_map(|r| r.transpose()).collect::<Result<_, _>>()?)
        })
    }

    pub fn characters(&self) -> DbResult<Vec<CharacterRow>> {
        self.0.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT c.id, c.number, c.name, c.hidden, {COVER},
                        COUNT(t.id),
                        COUNT(DISTINCT t.media_id || ':' || t.shot),
                        COUNT(DISTINCT t.media_id),
                        COALESCE(SUM((t.end - t.start) * 1.0 * m.fps_den / m.fps_num), 0),
                        (SELECT COUNT(*) FROM tracks u WHERE u.character_id = c.id AND u.assign = 'unsure' AND u.not_face = 0),
                        (SELECT COUNT(*) FROM track_edits e WHERE e.character_id = c.id AND e.kind IN ('pin', 'was')
                            AND e.media_id IN (SELECT id FROM media WHERE faces IN ('pending', 'running')))
                   FROM characters c
                   LEFT JOIN tracks t ON t.character_id = c.id AND t.assign IN ('auto', 'pinned') AND t.not_face = 0
                   LEFT JOIN media m ON m.id = t.media_id
                  GROUP BY c.id
                  ORDER BY 9 DESC, c.number"
            ))?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let cover: Option<i64> = r.get(4)?;
                let cover = match cover {
                    Some(t) => c
                        .query_row("SELECT media_id FROM tracks WHERE id = ?1", [t], |r| r.get::<_, String>(0))
                        .optional()?
                        .map(|m| parse_id(&m).map(|m| (t, m)))
                        .transpose()?,
                    None => None,
                };
                out.push(CharacterRow {
                    id: parse_id(&r.get::<_, String>(0)?)?,
                    number: r.get(1)?,
                    name: r.get(2)?,
                    hidden: r.get(3)?,
                    cover,
                    tracks: r.get(5)?,
                    shots: r.get(6)?,
                    media: r.get(7)?,
                    screen_sec: r.get(8)?,
                    unsure: r.get(9)?,
                    waiting: r.get(10)?,
                });
            }
            Ok(out)
        })
    }

    /// (unassigned, unsure, media with faces done, media total).
    pub fn cast_counts(&self) -> DbResult<(u32, u32, u32, u32)> {
        self.0.read(|c| {
            Ok(c.query_row(
                "SELECT (SELECT COUNT(*) FROM tracks WHERE not_face = 0 AND assign = 'none'
                           AND media_id IN (SELECT id FROM media WHERE faces = 'ready')),
                        (SELECT COUNT(*) FROM tracks WHERE not_face = 0 AND assign = 'unsure'),
                        (SELECT COUNT(*) FROM media WHERE faces = 'ready'),
                        (SELECT COUNT(*) FROM media)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
    }

    /// Tracks of the cast, best first.
    pub fn tracks(&self, filter: TrackFilter) -> DbResult<Vec<TrackRow>> {
        let (cond, arg) = match filter {
            TrackFilter::Character(c) => {
                ("t.character_id = ?1 AND t.assign IN ('auto', 'pinned')", Some(c.to_string()))
            }
            TrackFilter::Unsure(c) => ("t.character_id = ?1 AND t.assign = 'unsure'", Some(c.to_string())),
            TrackFilter::AllUnsure => ("t.assign = 'unsure'", None),
            TrackFilter::Unassigned => {
                ("t.assign = 'none' AND t.media_id IN (SELECT id FROM media WHERE faces = 'ready')", None)
            }
        };
        self.0.read(|c| {
            let order = match filter {
                TrackFilter::Character(_) => "t.assign = 'pinned' DESC, t.quality DESC",
                _ => "t.score DESC, t.quality DESC",
            };
            let mut st = c.prepare(&format!(
                "SELECT {TRACK_COLS} FROM tracks t WHERE t.not_face = 0 AND {cond} ORDER BY {order}, t.id"
            ))?;
            let mut rows = match &arg {
                Some(a) => st.query([a])?,
                None => st.query([])?,
            };
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(track_row(r)?);
            }
            Ok(out)
        })
    }

    pub fn tracks_by_id(&self, ids: &[i64]) -> DbResult<Vec<TrackRow>> {
        self.0.read(|c| {
            let mut st =
                c.prepare(&format!("SELECT {TRACK_COLS} FROM tracks t WHERE t.id IN ({})", marks(ids.len())))?;
            let mut rows = st.query(params_from_iter(ids_param(ids)))?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(track_row(r)?);
            }
            Ok(out)
        })
    }

    /// Spans of assigned tracks in one media (for the timeline).
    pub fn presence(&self, id: MediaId) -> DbResult<Vec<(CharacterId, i64, FrameIdx, FrameIdx)>> {
        self.0.read(|c| {
            let mut st = c.prepare(
                "SELECT character_id, id, start, end FROM tracks
                  WHERE media_id = ?1 AND not_face = 0 AND assign IN ('auto', 'pinned') AND character_id IS NOT NULL
                  ORDER BY start",
            )?;
            let mut rows = st.query([id.to_string()])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push((parse_id(&r.get::<_, String>(0)?)?, r.get(1)?, r.get(2)?, r.get(3)?));
            }
            Ok(out)
        })
    }

    /// The shots a character is seen in, by media then time.
    pub fn character_shots(&self, c: CharacterId) -> DbResult<Vec<CharacterShotRow>> {
        self.0.read(|conn| {
            let mut st = conn.prepare(
                "SELECT t.media_id, t.shot, g.end, sp.end, sp.cells, t.id, t.face_h
                   FROM tracks t
                   JOIN segments g ON g.media_id = t.media_id AND g.start = t.shot AND g.kind = ?2
                   LEFT JOIN sprites sp ON sp.media_id = t.media_id AND sp.shot = t.shot
                  WHERE t.character_id = ?1 AND t.assign IN ('auto', 'pinned') AND t.not_face = 0
                  ORDER BY t.media_id, t.shot, t.face_h >= 0.12 DESC, t.quality DESC, t.id",
            )?;
            let mut rows = st.query(params![c.to_string(), enum_to_sql(SegmentKind::Shot)])?;
            let mut out: Vec<CharacterShotRow> = Vec::new();
            while let Some(r) = rows.next()? {
                let media_id = parse_id(&r.get::<_, String>(0)?)?;
                let start: FrameIdx = r.get(1)?;
                let face_h = r.get::<_, f64>(6)? as f32;
                // The first row of a shot is its clearest face.
                if let Some(last) = out.last_mut().filter(|s| s.media_id == media_id && s.start == start) {
                    last.face_h = last.face_h.max(face_h);
                    continue;
                }
                out.push(CharacterShotRow {
                    media_id,
                    start,
                    end: r.get(2)?,
                    sprite: r.get::<_, Option<FrameIdx>>(3)?.zip(r.get::<_, Option<u32>>(4)?),
                    track: r.get(5)?,
                    face_h,
                });
            }
            Ok(out)
        })
    }

    pub fn character_exists(&self, c: CharacterId) -> DbResult<bool> {
        self.0.read(|conn| {
            Ok(conn
                .query_row("SELECT 1 FROM characters WHERE id = ?1", [c.to_string()], |_| Ok(()))
                .optional()?
                .is_some())
        })
    }

    // -- edits --------------------------------------------------------------

    /// Pin tracks to a character (creating it first when `create` names it).
    pub fn pin_tracks(&self, ids: &[i64], c: CharacterId, create: Option<Option<&str>>) -> DbResult<()> {
        self.0.write(|tx| {
            if let Some(name) = create {
                insert_character(tx, c, name)?;
            }
            let mut st = tx.prepare(
                "UPDATE tracks SET character_id = ?2, assign = 'pinned', score = 1, not_face = 0, via_body = 0 WHERE id = ?1",
            )?;
            let mut unforbid = tx.prepare("DELETE FROM track_forbids WHERE track_id = ?1 AND character_id = ?2")?;
            for &id in ids {
                st.execute(params![id, c.to_string()])?;
                unforbid.execute(params![id, c.to_string()])?;
            }
            drop((st, unforbid));
            drop_empty_characters(tx)?;
            bump_rev(tx)
        })
    }

    /// These tracks are not `c`: forbid it and take them off it.
    pub fn forbid_tracks(&self, ids: &[i64], c: CharacterId) -> DbResult<()> {
        self.0.write(|tx| {
            let mut add = tx.prepare("INSERT OR IGNORE INTO track_forbids (track_id, character_id) VALUES (?1, ?2)")?;
            let mut off = tx.prepare(
                "UPDATE tracks SET character_id = NULL, assign = 'none', score = 0 WHERE id = ?1 AND character_id = ?2",
            )?;
            for &id in ids {
                add.execute(params![id, c.to_string()])?;
                off.execute(params![id, c.to_string()])?;
            }
            drop((add, off));
            drop_empty_characters(tx)?;
            bump_rev(tx)
        })
    }

    pub fn mark_not_face(&self, ids: &[i64]) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                &format!(
                    "UPDATE tracks SET not_face = 1, character_id = NULL, assign = 'none', score = 0 WHERE id IN ({})",
                    marks(ids.len())
                ),
                params_from_iter(ids_param(ids)),
            )?;
            drop_empty_characters(tx)?;
            bump_rev(tx)
        })
    }

    /// Accept the suggestion (or automatic assignment) of these tracks.
    pub fn confirm_tracks(&self, ids: &[i64]) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                &format!(
                    "UPDATE tracks SET assign = 'pinned', via_body = 0
                      WHERE character_id IS NOT NULL AND assign IN ('unsure', 'auto') AND not_face = 0 AND id IN ({})",
                    marks(ids.len())
                ),
                params_from_iter(ids_param(ids)),
            )?;
            bump_rev(tx)
        })
    }

    /// Name a character and pin its best `anchors` tracks, so it stays this
    /// character whatever later clustering does.
    pub fn rename_character(&self, c: CharacterId, name: Option<&str>, anchors: usize) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("UPDATE characters SET name = ?2 WHERE id = ?1", params![c.to_string(), name])?;
            pin_anchors(tx, c, anchors)?;
            bump_rev(tx)
        })
    }

    /// Hide or show a character; hiding pins its best `anchors` tracks, so the
    /// character it hides stays the same.
    pub fn set_character_hidden(&self, c: CharacterId, hidden: bool, anchors: usize) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("UPDATE characters SET hidden = ?2 WHERE id = ?1", params![c.to_string(), hidden])?;
            if hidden {
                pin_anchors(tx, c, anchors)?;
            }
            bump_rev(tx)
        })
    }

    pub fn set_character_cover(&self, c: CharacterId, track: i64) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("UPDATE characters SET cover = ?2 WHERE id = ?1", params![c.to_string(), track])?;
            Ok(())
        })
    }

    /// Everyone of `from` is `into`: its tracks are pinned there, its
    /// suggestions, forbids and waiting decisions move along, `from` goes.
    /// A face the user said isn't `into` doesn't follow; a face only its
    /// body named follows as it is (never a pin). `into` takes `from`'s name
    /// if it has none.
    pub fn merge_characters(&self, from: CharacterId, into: CharacterId) -> DbResult<()> {
        if from == into {
            return Ok(());
        }
        self.0.write(|tx| {
            let (f, i) = (from.to_string(), into.to_string());
            tx.execute(
                "UPDATE tracks SET character_id = NULL, assign = 'none', score = 0
                  WHERE character_id = ?1 AND id IN (SELECT track_id FROM track_forbids WHERE character_id = ?2)",
                [&f, &i],
            )?;
            tx.execute(
                "UPDATE tracks SET character_id = ?2, assign = 'pinned', score = 1
                  WHERE character_id = ?1 AND (assign = 'pinned' OR (assign = 'auto' AND via_body = 0))",
                [&f, &i],
            )?;
            tx.execute("UPDATE tracks SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute("UPDATE OR IGNORE track_forbids SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute("UPDATE track_edits SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            // Their bodies: those the user said aren't `into` are no one.
            tx.execute(
                "UPDATE bodies SET character_id = NULL, source = 'none', similarity = 0
                  WHERE character_id = ?1 AND id IN (SELECT body_id FROM body_forbids WHERE character_id = ?2)",
                [&f, &i],
            )?;
            tx.execute("UPDATE bodies SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute("UPDATE OR IGNORE body_forbids SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute("UPDATE body_edits SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute(
                "UPDATE characters SET name = (SELECT name FROM characters WHERE id = ?1) WHERE id = ?2 AND name IS NULL",
                [&f, &i],
            )?;
            // Their pack too: shots picked or dropped by hand (the one who
            // stays decides where both did), and the filters if they had none.
            tx.execute("UPDATE OR IGNORE pack_overrides SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute(
                "INSERT OR IGNORE INTO packs (character_id, spec, naming, updated_at)
                 SELECT ?2, spec, naming, updated_at FROM packs WHERE character_id = ?1",
                [&f, &i],
            )?;
            tx.execute("DELETE FROM characters WHERE id = ?1", [&f])?;
            bump_rev(tx)
        })
    }
}

/// Pin a character's best `anchors` automatic tracks (its own faces: one
/// its body named never shapes it).
fn pin_anchors(tx: &Transaction, c: CharacterId, anchors: usize) -> DbResult<()> {
    tx.execute(
        "UPDATE tracks SET assign = 'pinned' WHERE id IN
           (SELECT id FROM tracks WHERE character_id = ?1 AND assign = 'auto' AND not_face = 0 AND via_body = 0
             ORDER BY quality DESC LIMIT ?2)",
        params![c.to_string(), anchors as i64],
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::project::tests::{info, new_media};
    use crate::{IngestRecord, ProjectDb};
    use as_api::ProxyState;
    use as_core::Rational;

    /// A clustering stored as the cast stores one, without bodies.
    pub(crate) fn apply(
        db: &ProjectDb,
        rev: i64,
        new: &[CharacterId],
        tracks: &[(i64, Option<CharacterId>, TrackAssign, f32)],
    ) -> DbResult<bool> {
        let t: Vec<TrackUpdate> = tracks
            .iter()
            .map(|&(id, character, assign, score)| TrackUpdate { id, character, assign, score, via_body: false })
            .collect();
        db.apply_identity(rev, new, &t, &[])
    }

    pub(crate) fn ready_media(db: &ProjectDb, name: &str) -> MediaId {
        let id = db.add_media(&[new_media(name, Some(1), Some(1))]).unwrap()[0];
        db.record_ingest(
            id,
            &IngestRecord {
                info: info(),
                frames: 1000,
                duration_sec: 41.7,
                fps: Rational::new(24, 1),
                vfr: false,
                interlaced: false,
                proxy: ProxyState::NotNeeded,
                poster: None,
                poster_small: None,
                index_blob: vec![1],
                audio_ordinal: None,
            },
        )
        .unwrap();
        db.store_segments(id, &[]).unwrap();
        id
    }

    pub(crate) fn track(start: FrameIdx, quality: f32) -> NewTrack {
        NewTrack { start, end: start + 24, shot: start, quality, face_h: 0.3, dets: vec![0; 64], emb: vec![0; 1024] }
    }

    #[test]
    fn track_ids_are_never_handed_out_twice() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let first = db.put_face_chunk(m, 0, 500, &[track(10, 0.9), track(300, 0.5)], &[], |_| Ok(())).unwrap();
        // The chunk stored again (a resumed pass): what it replaces is named first.
        assert_eq!(db.chunk_tracks(m, 0, 500).unwrap(), first);
        let again = db.put_face_chunk(m, 0, 500, &[track(10, 0.9), track(300, 0.5)], &[], |_| Ok(())).unwrap();
        assert!(again.iter().all(|t| *t > first[1]), "{again:?} after {first:?}");
        // Faces forgotten (the newest ids gone): still never again.
        db.reset_faces(m).unwrap();
        let after = db.put_face_chunk(m, 0, 500, &[track(10, 0.9)], &[], |_| Ok(())).unwrap();
        assert!(after[0] > again[1], "{after:?} after {again:?}");
        // A rolled-back chunk hands out nothing.
        assert!(db.put_face_chunk(m, 500, 1000, &[track(600, 0.9)], &[], |_| Err("disk full".into())).is_err());
        let next = db.put_face_chunk(m, 500, 1000, &[track(600, 0.9)], &[], |_| Ok(())).unwrap();
        assert_eq!(next[0], after[0] + 1);
    }

    #[test]
    fn chunks_tracks_and_resume() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        assert_eq!(db.pending_faces().unwrap(), vec![m]);
        let mut seen = vec![];
        let ids = db
            .put_face_chunk(m, 0, 500, &[track(10, 0.9), track(300, 0.5)], &[], |ids| {
                seen = ids.to_vec();
                Ok(())
            })
            .unwrap();
        assert_eq!(ids, seen);
        // A failing file write rolls the chunk back.
        assert!(db.put_face_chunk(m, 500, 1000, &[track(600, 0.9)], &[], |_| Err("disk full".into())).is_err());
        assert_eq!(db.face_chunks(m).unwrap(), [(0, 500)]);
        db.put_face_chunk(m, 500, 1000, &[track(600, 0.9)], &[(600, 624, 8)], |_| Ok(())).unwrap();
        // A pass that isn't finished isn't clustered (nor listed as nobody's).
        assert!(db.identity_tracks().unwrap().is_empty());
        assert!(db.tracks(TrackFilter::Unassigned).unwrap().is_empty());
        assert_eq!(db.cast_counts().unwrap().0, 0);
        db.set_faces_state(m, StageState::Ready, None).unwrap();
        assert_eq!(db.identity_tracks().unwrap().len(), 3);
        assert_eq!(db.tracks(TrackFilter::Unassigned).unwrap().len(), 3);
        let sprites = |db: &ProjectDb| -> i64 {
            db.0.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM sprites", [], |r| r.get(0))?)).unwrap()
        };
        assert_eq!(sprites(&db), 1);
        db.drop_face_chunks_from(m, 500).unwrap();
        assert_eq!(db.identity_tracks().unwrap().len(), 2);
        assert_eq!(db.face_chunks(m).unwrap(), [(0, 500)]);
        assert_eq!(sprites(&db), 0);
    }

    #[test]
    fn edits_stick_and_survive_reanalysis() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db
            .put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8), track(200, 0.7)], &[], |_| Ok(()))
            .unwrap();
        db.set_faces_state(m, StageState::Ready, None).unwrap();
        // Clustering made one character of all three, then the user named it.
        let (a, b) = (CharacterId::new(), CharacterId::new());
        apply(
            &db,
            db.cast_rev().unwrap(),
            &[a],
            &ids.iter().map(|&t| (t, Some(a), TrackAssign::Auto, 0.9)).collect::<Vec<_>>(),
        )
        .unwrap();
        db.rename_character(a, Some("Walter"), 1).unwrap();
        let rows = db.characters().unwrap();
        assert_eq!((rows.len(), rows[0].name.as_deref(), rows[0].tracks, rows[0].number), (1, Some("Walter"), 3, 1));
        assert_eq!(rows[0].cover.map(|c| c.0), Some(ids[0]));
        // The best track became an anchor; a recluster can't move it.
        apply(
            &db,
            db.cast_rev().unwrap(),
            &[],
            &ids.iter().map(|&t| (t, None, TrackAssign::None, 0.0)).collect::<Vec<_>>(),
        )
        .unwrap();
        let tracks = db.tracks_by_id(&ids).unwrap();
        assert_eq!(tracks.iter().filter(|t| t.character_id == Some(a)).count(), 1);
        // Someone new, then "not them", then not a face.
        db.pin_tracks(&[ids[1]], b, Some(None)).unwrap();
        assert_eq!(db.characters().unwrap().iter().find(|c| c.id == b).unwrap().number, 2);
        db.forbid_tracks(&[ids[1]], b).unwrap();
        // b is empty and unnamed: gone. Its number is never reused.
        assert!(!db.character_exists(b).unwrap());
        db.mark_not_face(&[ids[2]]).unwrap();
        assert_eq!(db.identity_tracks().unwrap().len(), 2);
        assert_eq!(db.cast_counts().unwrap(), (1, 0, 1, 1));
        db.set_character_cover(a, ids[0]).unwrap();

        // Re-analysis: the decisions wait in track_edits for the new tracks.
        db.reset_faces(m).unwrap();
        assert!(db.identity_tracks().unwrap().is_empty());
        let edits = db.track_edits(m).unwrap();
        let kinds: Vec<_> = edits.iter().map(|e| (e.start, e.kind)).collect();
        assert_eq!(kinds.len(), 3);
        for k in [(10, EditKind::Pin(a)), (10, EditKind::Cover(a)), (200, EditKind::NotFace)] {
            assert!(kinds.contains(&k), "{k:?} waits");
        }
        // The new pass gets new track ids, and no stale cover meanwhile.
        let new = db.put_face_chunk(m, 0, 1000, &[track(200, 0.7), track(10, 0.9)], &[], |_| Ok(())).unwrap();
        assert!(new.iter().all(|t| !ids.contains(t)), "track ids are never handed out twice");
        assert_eq!(db.characters().unwrap()[0].cover, None);
        db.reanchor_edits(m, &[(new[1], EditKind::Pin(a)), (new[1], EditKind::Cover(a)), (new[0], EditKind::NotFace)])
            .unwrap();
        assert_eq!(db.characters().unwrap()[0].cover.map(|c| c.0), Some(new[1]), "the chosen picture is back");
        db.set_faces_state(m, StageState::Ready, None).unwrap();
        assert!(db.track_edits(m).unwrap().is_empty());
        assert_eq!(db.tracks(TrackFilter::Character(a)).unwrap().len(), 1);
        assert_eq!(db.identity_tracks().unwrap().len(), 1);
        let c = db.pin_tracks(&[], CharacterId::new(), Some(Some("Jesse")));
        assert!(c.is_ok());
        assert_eq!(db.characters().unwrap().iter().map(|c| c.number).max(), Some(3));
    }

    #[test]
    fn merging_moves_everything() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let shot = |start| as_api::Segment { start, end: start + 50, kind: SegmentKind::Shot, tiny: false };
        db.store_segments(m, &[shot(10), shot(100), shot(200)]).unwrap();
        let ids = db
            .put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8), track(200, 0.2)], &[], |_| Ok(()))
            .unwrap();
        let (a, b) = (CharacterId::new(), CharacterId::new());
        apply(
            &db,
            db.cast_rev().unwrap(),
            &[a, b],
            &[
                (ids[0], Some(a), TrackAssign::Auto, 0.9),
                (ids[1], Some(b), TrackAssign::Auto, 0.9),
                (ids[2], Some(b), TrackAssign::Unsure, 0.35),
            ],
        )
        .unwrap();
        db.rename_character(b, Some("Skyler"), 0).unwrap();
        db.merge_characters(b, a).unwrap();
        let rows = db.characters().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].id, rows[0].name.as_deref(), rows[0].tracks, rows[0].unsure), (a, Some("Skyler"), 2, 1));
        assert_eq!(db.tracks(TrackFilter::Unsure(a)).unwrap().len(), 1);
        db.confirm_tracks(&[ids[2]]).unwrap();
        assert_eq!(db.tracks(TrackFilter::Character(a)).unwrap().len(), 3);
        assert_eq!(db.presence(m).unwrap().len(), 3);
        let shots = db.character_shots(a).unwrap();
        assert_eq!(
            shots.iter().map(|s| (s.start, s.end, s.sprite)).collect::<Vec<_>>(),
            [(10, 60, None), (100, 150, None), (200, 250, None)]
        );
    }

    #[test]
    fn faces_named_by_their_body_are_never_pinned_but_by_the_user() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db
            .put_face_chunk(
                m,
                0,
                1000,
                &[track(10, 0.5), track(100, 0.9), track(200, 0.6), track(300, 0.4)],
                &[],
                |_| Ok(()),
            )
            .unwrap();
        db.set_faces_state(m, StageState::Ready, None).unwrap();
        let (k, j) = (CharacterId::new(), CharacterId::new());
        let t =
            |id, c, via_body| TrackUpdate { id, character: Some(c), assign: TrackAssign::Auto, score: 0.9, via_body };
        let updates = [t(ids[0], k, false), t(ids[1], k, true), t(ids[2], j, false), t(ids[3], j, true)];
        assert!(db.apply_identity(db.cast_rev().unwrap(), &[k, j], &updates, &[]).unwrap());
        let state = |id: i64| {
            let r = db.identity_tracks().unwrap().into_iter().find(|r| r.id == id).unwrap();
            (r.character_id, r.assign, r.via_body)
        };
        // Naming (or hiding) a character pins its best faces: its own, not
        // the clearer one its body named.
        db.rename_character(k, Some("Skyler"), 5).unwrap();
        assert_eq!(state(ids[0]), (Some(k), TrackAssign::Pinned, false));
        assert_eq!(state(ids[1]), (Some(k), TrackAssign::Auto, true));
        db.set_character_hidden(j, true, 5).unwrap();
        assert_eq!(state(ids[3]), (Some(j), TrackAssign::Auto, true));
        db.set_character_hidden(j, false, 0).unwrap();
        // A merge pins the character's own faces; the others follow as they were.
        db.merge_characters(j, k).unwrap();
        assert_eq!(state(ids[2]), (Some(k), TrackAssign::Pinned, false));
        assert_eq!(state(ids[3]), (Some(k), TrackAssign::Auto, true));
        // The user's word makes a face theirs.
        db.confirm_tracks(&[ids[1]]).unwrap();
        assert_eq!(state(ids[1]), (Some(k), TrackAssign::Pinned, false));
    }

    #[test]
    fn a_clustering_older_than_a_decision_is_not_stored() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], &[], |_| Ok(())).unwrap();
        let a = CharacterId::new();
        let all = |c: Option<CharacterId>| ids.iter().map(|&t| (t, c, TrackAssign::Auto, 0.9)).collect::<Vec<_>>();
        assert!(apply(&db, db.cast_rev().unwrap(), &[a], &all(Some(a))).unwrap());
        db.rename_character(a, Some("A"), 0).unwrap();
        // Clustering reads the cast, the user says "not them" meanwhile.
        let rev = db.cast_rev().unwrap();
        db.forbid_tracks(&[ids[0]], a).unwrap();
        db.mark_not_face(&[ids[1]]).unwrap();
        assert!(!apply(&db, rev, &[], &all(Some(a))).unwrap());
        assert!(db.tracks(TrackFilter::Character(a)).unwrap().is_empty());
        // Even a current one never puts a face that isn't one on anyone.
        assert!(apply(&db, db.cast_rev().unwrap(), &[], &[(ids[1], Some(a), TrackAssign::Auto, 0.9)]).unwrap());
        assert!(db.tracks_by_id(&[ids[1]]).unwrap()[0].character_id.is_none());
    }

    #[test]
    fn decisions_waiting_for_a_reanalysis_keep_their_character() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], &[], |_| Ok(())).unwrap();
        // An unnamed character made by moving a face, and a hidden one.
        let (a, h) = (CharacterId::new(), CharacterId::new());
        db.pin_tracks(&[ids[0]], a, Some(None)).unwrap();
        apply(&db, db.cast_rev().unwrap(), &[h], &[(ids[1], Some(h), TrackAssign::Auto, 0.9)]).unwrap();
        db.set_character_hidden(h, true, 5).unwrap();
        // Analysed again: both are empty until the new tracks come.
        db.reset_faces(m).unwrap();
        apply(&db, db.cast_rev().unwrap(), &[], &[]).unwrap();
        assert!(db.character_exists(a).unwrap() && db.character_exists(h).unwrap());
        let mut waiting: Vec<_> = db.track_edits(m).unwrap().into_iter().map(|e| e.kind).collect();
        waiting.sort_by_key(|k| format!("{k:?}"));
        assert_eq!(waiting.len(), 2);
        assert!(waiting.contains(&EditKind::Pin(a)) && waiting.contains(&EditKind::Pin(h)));
        // Merged meanwhile: the waiting decisions follow.
        db.merge_characters(a, h).unwrap();
        assert!(db.track_edits(m).unwrap().iter().all(|e| e.kind == EditKind::Pin(h)));
    }

    #[test]
    fn characters_nobody_decided_about_survive_a_reanalysis() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], &[], |_| Ok(())).unwrap();
        let a = CharacterId::new();
        apply(
            &db,
            db.cast_rev().unwrap(),
            &[a],
            &ids.iter().map(|&t| (t, Some(a), TrackAssign::Auto, 0.9)).collect::<Vec<_>>(),
        )
        .unwrap();
        // Analysed again; another media's clustering runs meanwhile.
        db.reset_faces(m).unwrap();
        apply(&db, db.cast_rev().unwrap(), &[], &[]).unwrap();
        assert!(db.character_exists(a).unwrap(), "kept while its faces are found again");
        assert_eq!((db.characters().unwrap()[0].tracks, db.characters().unwrap()[0].waiting), (0, 2));
        assert!(db.track_edits(m).unwrap().iter().all(|e| e.kind == EditKind::Was(a)));
        // The new tracks start out on it — unless a decision says otherwise.
        let new = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], &[], |_| Ok(())).unwrap();
        db.reanchor_edits(m, &[(new[0], EditKind::Was(a)), (new[1], EditKind::Was(a)), (new[1], EditKind::Forbid(a))])
            .unwrap();
        let t = db.tracks_by_id(&new).unwrap();
        let on = |id| t.iter().find(|r| r.id == id).map(|r| (r.character_id, r.assign)).unwrap();
        assert_eq!(on(new[0]), (Some(a), TrackAssign::Auto));
        assert_eq!(on(new[1]), (None, TrackAssign::None));
        assert_eq!(db.characters().unwrap()[0].number, 1);
        // Faces that didn't come back leave the character to the next clustering.
        db.reset_faces(m).unwrap();
        db.reanchor_edits(m, &[]).unwrap();
        apply(&db, db.cast_rev().unwrap(), &[], &[]).unwrap();
        assert!(!db.character_exists(a).unwrap());
    }

    #[test]
    fn faces_of_another_model_are_found_again() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        // Found before models were recorded, a character made of them.
        assert!(db.use_faces_model(m, "old").unwrap());
        assert!(!db.use_faces_model(m, "old").unwrap(), "same model: nothing to drop");
        let ids = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], &[], |_| Ok(())).unwrap();
        db.set_faces_state(m, StageState::Ready, None).unwrap();
        let a = CharacterId::new();
        apply(
            &db,
            db.cast_rev().unwrap(),
            &[a],
            &ids.iter().map(|&t| (t, Some(a), TrackAssign::Auto, 0.9)).collect::<Vec<_>>(),
        )
        .unwrap();
        // The app now embeds with "new": the media is found again, its character waits.
        assert!(db.outdated_faces("old").unwrap().is_empty());
        assert_eq!(db.outdated_faces("new").unwrap(), [m]);
        assert_eq!(db.pending_faces().unwrap(), [m]);
        assert!(db.character_exists(a).unwrap());
        assert_eq!(db.track_edits(m).unwrap().len(), 2);
        // A pass of the old model half done when the app changed starts over.
        db.put_face_chunk(m, 0, 500, &[track(10, 0.9)], &[], |_| Ok(())).unwrap();
        assert!(db.use_faces_model(m, "new").unwrap());
        assert!(db.face_chunks(m).unwrap().is_empty());
        assert_eq!(db.faces_model(m).unwrap().as_deref(), Some("new"));
        assert_eq!(db.track_edits(m).unwrap().len(), 2, "decisions still wait");
    }

    #[test]
    fn merging_respects_not_them() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], &[], |_| Ok(())).unwrap();
        let (a, b) = (CharacterId::new(), CharacterId::new());
        apply(
            &db,
            db.cast_rev().unwrap(),
            &[a, b],
            &[(ids[0], Some(a), TrackAssign::Auto, 0.9), (ids[1], Some(b), TrackAssign::Auto, 0.9)],
        )
        .unwrap();
        db.rename_character(b, Some("B"), 0).unwrap();
        // ids[1] is on b and the user said it is not a.
        db.forbid_tracks(&[ids[1]], a).unwrap();
        db.merge_characters(b, a).unwrap();
        let t = &db.tracks_by_id(&[ids[1]]).unwrap()[0];
        assert_eq!((t.character_id, t.assign), (None, TrackAssign::None));
        assert_eq!(db.tracks(TrackFilter::Character(a)).unwrap().len(), 1);
    }

    #[test]
    fn the_cast_lists_most_screen_time_first_with_a_clear_cover() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let small = NewTrack { face_h: 0.05, ..track(400, 0.95) };
        let ids = db
            .put_face_chunk(m, 0, 1000, &[track(10, 0.6), track(100, 0.7), small, track(700, 0.9)], &[], |_| Ok(()))
            .unwrap();
        let (a, b) = (CharacterId::new(), CharacterId::new());
        // `a` is created first but `b` is on screen longer.
        apply(
            &db,
            db.cast_rev().unwrap(),
            &[a, b],
            &[
                (ids[0], Some(a), TrackAssign::Auto, 1.0),
                (ids[1], Some(b), TrackAssign::Auto, 1.0),
                (ids[2], Some(b), TrackAssign::Auto, 1.0),
            ],
        )
        .unwrap();
        let rows = db.characters().unwrap();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [b, a]);
        // The tiny face has the best quality, but a clear, big face makes the cover.
        assert_eq!(rows[0].cover.map(|c| c.0), Some(ids[1]));
        // The covers alone, in the order characters were found: the same tracks.
        assert_eq!(db.cover_tracks().unwrap(), [rows[1].cover.unwrap().0, rows[0].cover.unwrap().0]);

        // A portrait needs the cover's media and detections; unknown tracks are left out.
        let dets = NewTrack { dets: vec![7; 32], ..track(800, 0.5) };
        let more = db.put_face_chunk(m, 1000, 2000, &[dets], &[], |_| Ok(())).unwrap();
        let mut got = db.track_dets(&[more[0], ids[1], 9999]).unwrap();
        got.sort_by_key(|t| t.0);
        assert_eq!(got, [(ids[1], m, vec![0; 64]), (more[0], m, vec![7; 32])]);
        assert!(db.track_dets(&[]).unwrap().is_empty());
    }
}
