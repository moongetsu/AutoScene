//! Faces and characters in `project.db`.
//!
//! Tracks arrive per checkpoint chunk from the face pass; clustering then
//! points them at characters. Everything the user decides is stored on the
//! tracks themselves — `pinned` (this is X), a forbid (this is not X),
//! `not_face` — so a recluster can never undo it. Every decision bumps the
//! cast's revision; a clustering computed before it is not stored.

use as_api::{StageState, TrackAssign, TrackFilter, now_ms};
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
    pub forbids: Vec<CharacterId>,
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
}

/// `(track, start, end, dets)`.
pub type TrackDets = (i64, FrameIdx, FrameIdx, Vec<u8>);

/// A user decision on a track that was deleted for a re-analysis.
#[derive(Clone, Debug)]
pub struct TrackEdit {
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub dets: Vec<u8>,
    pub kind: EditKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Pin(CharacterId),
    Forbid(CharacterId),
    NotFace,
}

fn parse_id<T: std::str::FromStr>(s: &str) -> DbResult<T>
where
    T::Err: std::fmt::Display,
{
    s.parse().map_err(|e: T::Err| DbError::Corrupt(e.to_string()))
}

fn opt_id(s: Option<String>) -> DbResult<Option<CharacterId>> {
    s.as_deref().map(parse_id).transpose()
}

/// `?, ?, ?` for an `IN (...)` list.
fn marks(n: usize) -> String {
    vec!["?"; n].join(", ")
}

/// Forget a media's faces, keeping the user's decisions on them in
/// `track_edits` for the next face pass to re-anchor.
pub(crate) fn clear_faces(tx: &Transaction, id: MediaId) -> DbResult<()> {
    let m = id.to_string();
    tx.execute(
        "INSERT INTO track_edits (media_id, start, end, dets, kind, character_id)
         SELECT media_id, start, end, dets, 'pin', character_id FROM tracks
          WHERE media_id = ?1 AND assign = 'pinned' AND character_id IS NOT NULL
         UNION ALL
         SELECT t.media_id, t.start, t.end, t.dets, 'forbid', f.character_id
           FROM tracks t JOIN track_forbids f ON f.track_id = t.id WHERE t.media_id = ?1
         UNION ALL
         SELECT media_id, start, end, dets, 'not_face', NULL FROM tracks WHERE media_id = ?1 AND not_face = 1",
        [&m],
    )?;
    tx.execute("DELETE FROM tracks WHERE media_id = ?1", [&m])?;
    tx.execute("DELETE FROM face_chunks WHERE media_id = ?1", [&m])?;
    tx.execute("UPDATE media SET faces = 'pending', faces_error = NULL WHERE id = ?1", [&m])?;
    bump_rev(tx)
}

/// A decision (or a stored clustering) changed the cast.
fn bump_rev(tx: &Transaction) -> DbResult<()> {
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
                             AND t.assign IN ('auto', 'pinned') AND t.not_face = 0)
           AND NOT EXISTS (SELECT 1 FROM track_edits e WHERE e.character_id = characters.id)",
        [],
    )?;
    tx.execute("UPDATE tracks SET assign = 'none', score = 0 WHERE character_id IS NULL AND assign <> 'none'", [])?;
    tx.execute("UPDATE characters SET cover = NULL WHERE cover IS NOT NULL AND NOT EXISTS (SELECT 1 FROM tracks t WHERE t.id = characters.cover AND t.character_id = characters.id AND t.assign IN ('auto', 'pinned'))", [])?;
    Ok(())
}

fn ids_param(ids: &[i64]) -> Vec<rusqlite::types::Value> {
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

    /// Finished chunks `(start, end)` of the face pass, in order.
    pub fn face_chunks(&self, id: MediaId) -> DbResult<Vec<(FrameIdx, FrameIdx)>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT start, end FROM face_chunks WHERE media_id = ?1 ORDER BY start")?;
            let rows = st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Drop chunks (and their tracks) from `start` on.
    pub fn drop_face_chunks_from(&self, id: MediaId, start: FrameIdx) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("DELETE FROM tracks WHERE media_id = ?1 AND shot >= ?2", params![id.to_string(), start])?;
            tx.execute("DELETE FROM face_chunks WHERE media_id = ?1 AND start >= ?2", params![id.to_string(), start])?;
            Ok(())
        })
    }

    /// Store the tracks of chunk `start..end` and the chunk itself, atomically.
    /// `files(ids)` runs before the commit (thumbnails); its error rolls back.
    pub fn put_face_chunk(
        &self,
        id: MediaId,
        start: FrameIdx,
        end: FrameIdx,
        tracks: &[NewTrack],
        files: impl FnOnce(&[i64]) -> Result<(), String>,
    ) -> DbResult<Vec<i64>> {
        self.0.write(|tx| {
            let m = id.to_string();
            tx.execute("DELETE FROM tracks WHERE media_id = ?1 AND shot >= ?2 AND shot < ?3", params![m, start, end])?;
            let mut st = tx.prepare(
                "INSERT INTO tracks (media_id, start, end, shot, quality, face_h, dets, emb)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            let mut ids = Vec::with_capacity(tracks.len());
            for t in tracks {
                st.execute(params![m, t.start, t.end, t.shot, t.quality as f64, t.face_h as f64, t.dets, t.emb])?;
                ids.push(tx.last_insert_rowid());
            }
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
            let mut st =
                c.prepare("SELECT start, end, dets, kind, character_id FROM track_edits WHERE media_id = ?1")?;
            let mut rows = st.query([id.to_string()])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let kind: String = r.get(3)?;
                let ch = opt_id(r.get(4)?)?;
                let kind = match (kind.as_str(), ch) {
                    ("pin", Some(c)) => EditKind::Pin(c),
                    ("forbid", Some(c)) => EditKind::Forbid(c),
                    ("not_face", _) => EditKind::NotFace,
                    _ => continue,
                };
                out.push(TrackEdit { start: r.get(0)?, end: r.get(1)?, dets: r.get(2)?, kind });
            }
            Ok(out)
        })
    }

    /// Re-apply edits to new tracks (`(track, edit)`) and forget all of the media's pending edits.
    pub fn reanchor_edits(&self, id: MediaId, matched: &[(i64, EditKind)]) -> DbResult<()> {
        self.0.write(|tx| {
            for &(track, kind) in matched {
                match kind {
                    EditKind::Pin(c) => {
                        tx.execute(
                            "UPDATE tracks SET character_id = ?2, assign = 'pinned', score = 1 WHERE id = ?1
                               AND EXISTS (SELECT 1 FROM characters WHERE id = ?2)",
                            params![track, c.to_string()],
                        )?;
                    }
                    EditKind::Forbid(c) => {
                        tx.execute(
                            "INSERT OR IGNORE INTO track_forbids (track_id, character_id)
                             SELECT ?1, id FROM characters WHERE id = ?2",
                            params![track, c.to_string()],
                        )?;
                    }
                    EditKind::NotFace => {
                        tx.execute(
                            "UPDATE tracks SET not_face = 1, character_id = NULL, assign = 'none' WHERE id = ?1",
                            [track],
                        )?;
                    }
                }
            }
            tx.execute("DELETE FROM track_edits WHERE media_id = ?1", [id.to_string()])?;
            bump_rev(tx)
        })
    }

    /// Every track of a media with its detections (for re-anchoring edits).
    pub fn media_track_dets(&self, id: MediaId) -> DbResult<Vec<TrackDets>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT id, start, end, dets FROM tracks WHERE media_id = ?1 ORDER BY start")?;
            let rows = st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
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

    /// Every track that is a face, with what clustering needs.
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
                "SELECT id, media_id, start, end, quality, length(dets) / 32, emb, character_id, assign, dets
                   FROM tracks WHERE not_face = 0 ORDER BY media_id, start, id",
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
                    forbids: forbids.remove(&id).unwrap_or_default(),
                });
            }
            Ok(out)
        })
    }

    /// Store a clustering of the cast at revision `rev`: new characters,
    /// then every track's assignment (pinned tracks and faces that aren't
    /// keep theirs). Characters left empty go. Stores nothing and returns
    /// `false` when the cast changed since `rev`.
    pub fn apply_identity(
        &self,
        rev: i64,
        new_characters: &[CharacterId],
        tracks: &[(i64, Option<CharacterId>, TrackAssign, f32)],
    ) -> DbResult<bool> {
        self.0.write(|tx| {
            if rev_of(tx)? != rev {
                return Ok(false);
            }
            for &c in new_characters {
                insert_character(tx, c, None)?;
            }
            let mut st = tx.prepare(
                "UPDATE tracks SET character_id = ?2, assign = ?3, score = ?4
                  WHERE id = ?1 AND assign <> 'pinned' AND not_face = 0",
            )?;
            for &(id, c, a, score) in tracks {
                st.execute(params![id, c.map(|c| c.to_string()), enum_to_sql(a), score as f64])?;
            }
            drop(st);
            drop_empty_characters(tx)?;
            bump_rev(tx)?;
            Ok(true)
        })
    }

    pub fn characters(&self) -> DbResult<Vec<CharacterRow>> {
        self.0.read(|c| {
            let mut st = c.prepare(
                "SELECT c.id, c.number, c.name, c.hidden,
                        COALESCE(c.cover, (SELECT b.id FROM tracks b WHERE b.character_id = c.id AND b.assign IN ('auto', 'pinned')
                                              AND b.not_face = 0
                                            ORDER BY b.face_h >= 0.12 DESC, b.quality DESC, b.face_h DESC LIMIT 1)),
                        COUNT(t.id),
                        COUNT(DISTINCT t.media_id || ':' || t.shot),
                        COUNT(DISTINCT t.media_id),
                        COALESCE(SUM((t.end - t.start) * 1.0 * m.fps_den / m.fps_num), 0),
                        (SELECT COUNT(*) FROM tracks u WHERE u.character_id = c.id AND u.assign = 'unsure' AND u.not_face = 0)
                   FROM characters c
                   LEFT JOIN tracks t ON t.character_id = c.id AND t.assign IN ('auto', 'pinned') AND t.not_face = 0
                   LEFT JOIN media m ON m.id = t.media_id
                  GROUP BY c.id
                  ORDER BY 9 DESC, c.number",
            )?;
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
                });
            }
            Ok(out)
        })
    }

    /// (unassigned, unsure, media with faces done, media total).
    pub fn cast_counts(&self) -> DbResult<(u32, u32, u32, u32)> {
        self.0.read(|c| {
            Ok(c.query_row(
                "SELECT (SELECT COUNT(*) FROM tracks WHERE not_face = 0 AND assign = 'none'),
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
            TrackFilter::Unassigned => ("t.assign = 'none'", None),
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

    /// Shots `(media, shot start)` a character is seen in.
    pub fn character_shots(&self, c: CharacterId) -> DbResult<Vec<(MediaId, FrameIdx)>> {
        self.0.read(|conn| {
            let mut st = conn.prepare(
                "SELECT DISTINCT media_id, shot FROM tracks
                  WHERE character_id = ?1 AND assign IN ('auto', 'pinned') AND not_face = 0",
            )?;
            let mut rows = st.query([c.to_string()])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push((parse_id(&r.get::<_, String>(0)?)?, r.get(1)?));
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
                "UPDATE tracks SET character_id = ?2, assign = 'pinned', score = 1, not_face = 0 WHERE id = ?1",
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
                    "UPDATE tracks SET assign = 'pinned' WHERE character_id IS NOT NULL AND assign IN ('unsure', 'auto')
                       AND not_face = 0 AND id IN ({})",
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
    /// A face the user said isn't `into` doesn't follow. `into` takes
    /// `from`'s name if it has none.
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
                  WHERE character_id = ?1 AND assign IN ('auto', 'pinned')",
                [&f, &i],
            )?;
            tx.execute("UPDATE tracks SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute("UPDATE OR IGNORE track_forbids SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute("UPDATE track_edits SET character_id = ?2 WHERE character_id = ?1", [&f, &i])?;
            tx.execute(
                "UPDATE characters SET name = (SELECT name FROM characters WHERE id = ?1) WHERE id = ?2 AND name IS NULL",
                [&f, &i],
            )?;
            tx.execute("DELETE FROM characters WHERE id = ?1", [&f])?;
            bump_rev(tx)
        })
    }
}

/// Pin a character's best `anchors` automatic tracks.
fn pin_anchors(tx: &Transaction, c: CharacterId, anchors: usize) -> DbResult<()> {
    tx.execute(
        "UPDATE tracks SET assign = 'pinned' WHERE id IN
           (SELECT id FROM tracks WHERE character_id = ?1 AND assign = 'auto' AND not_face = 0
             ORDER BY quality DESC LIMIT ?2)",
        params![c.to_string(), anchors as i64],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::tests::{info, new_media};
    use crate::{IngestRecord, ProjectDb};
    use as_api::ProxyState;
    use as_core::Rational;

    fn ready_media(db: &ProjectDb, name: &str) -> MediaId {
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
                index_blob: vec![1],
                audio_ordinal: None,
            },
        )
        .unwrap();
        db.store_segments(id, &[]).unwrap();
        id
    }

    fn track(start: FrameIdx, quality: f32) -> NewTrack {
        NewTrack { start, end: start + 24, shot: start, quality, face_h: 0.3, dets: vec![0; 64], emb: vec![0; 1024] }
    }

    #[test]
    fn chunks_tracks_and_resume() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        assert_eq!(db.pending_faces().unwrap(), vec![m]);
        let mut seen = vec![];
        let ids = db
            .put_face_chunk(m, 0, 500, &[track(10, 0.9), track(300, 0.5)], |ids| {
                seen = ids.to_vec();
                Ok(())
            })
            .unwrap();
        assert_eq!(ids, seen);
        // A failing file write rolls the chunk back.
        assert!(db.put_face_chunk(m, 500, 1000, &[track(600, 0.9)], |_| Err("disk full".into())).is_err());
        assert_eq!(db.face_chunks(m).unwrap(), [(0, 500)]);
        db.put_face_chunk(m, 500, 1000, &[track(600, 0.9)], |_| Ok(())).unwrap();
        assert_eq!(db.identity_tracks().unwrap().len(), 3);
        db.drop_face_chunks_from(m, 500).unwrap();
        assert_eq!(db.identity_tracks().unwrap().len(), 2);
        assert_eq!(db.face_chunks(m).unwrap(), [(0, 500)]);
    }

    #[test]
    fn edits_stick_and_survive_reanalysis() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids =
            db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8), track(200, 0.7)], |_| Ok(())).unwrap();
        // Clustering made one character of all three, then the user named it.
        let (a, b) = (CharacterId::new(), CharacterId::new());
        db.apply_identity(
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
        db.apply_identity(
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
        assert_eq!(db.cast_counts().unwrap(), (1, 0, 0, 1));

        // Re-analysis: the decisions wait in track_edits for the new tracks.
        db.reset_faces(m).unwrap();
        assert!(db.identity_tracks().unwrap().is_empty());
        let edits = db.track_edits(m).unwrap();
        let mut kinds: Vec<_> = edits.iter().map(|e| (e.start, e.kind)).collect();
        kinds.sort_by_key(|k| k.0);
        assert_eq!(kinds, [(10, EditKind::Pin(a)), (200, EditKind::NotFace)]);
        let new = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(200, 0.7)], |_| Ok(())).unwrap();
        db.reanchor_edits(m, &[(new[0], EditKind::Pin(a)), (new[1], EditKind::NotFace)]).unwrap();
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
        let ids =
            db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8), track(200, 0.2)], |_| Ok(())).unwrap();
        let (a, b) = (CharacterId::new(), CharacterId::new());
        db.apply_identity(
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
        assert_eq!(db.character_shots(a).unwrap().len(), 3);
    }

    #[test]
    fn a_clustering_older_than_a_decision_is_not_stored() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], |_| Ok(())).unwrap();
        let a = CharacterId::new();
        let all = |c: Option<CharacterId>| ids.iter().map(|&t| (t, c, TrackAssign::Auto, 0.9)).collect::<Vec<_>>();
        assert!(db.apply_identity(db.cast_rev().unwrap(), &[a], &all(Some(a))).unwrap());
        db.rename_character(a, Some("A"), 0).unwrap();
        // Clustering reads the cast, the user says "not them" meanwhile.
        let rev = db.cast_rev().unwrap();
        db.forbid_tracks(&[ids[0]], a).unwrap();
        db.mark_not_face(&[ids[1]]).unwrap();
        assert!(!db.apply_identity(rev, &[], &all(Some(a))).unwrap());
        assert!(db.tracks(TrackFilter::Character(a)).unwrap().is_empty());
        // Even a current one never puts a face that isn't one on anyone.
        assert!(db.apply_identity(db.cast_rev().unwrap(), &[], &[(ids[1], Some(a), TrackAssign::Auto, 0.9)]).unwrap());
        assert!(db.tracks_by_id(&[ids[1]]).unwrap()[0].character_id.is_none());
    }

    #[test]
    fn decisions_waiting_for_a_reanalysis_keep_their_character() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], |_| Ok(())).unwrap();
        // An unnamed character made by moving a face, and a hidden one.
        let (a, h) = (CharacterId::new(), CharacterId::new());
        db.pin_tracks(&[ids[0]], a, Some(None)).unwrap();
        db.apply_identity(db.cast_rev().unwrap(), &[h], &[(ids[1], Some(h), TrackAssign::Auto, 0.9)]).unwrap();
        db.set_character_hidden(h, true, 5).unwrap();
        // Analysed again: both are empty until the new tracks come.
        db.reset_faces(m).unwrap();
        db.apply_identity(db.cast_rev().unwrap(), &[], &[]).unwrap();
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
    fn merging_respects_not_them() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let ids = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(100, 0.8)], |_| Ok(())).unwrap();
        let (a, b) = (CharacterId::new(), CharacterId::new());
        db.apply_identity(
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
            .put_face_chunk(m, 0, 1000, &[track(10, 0.6), track(100, 0.7), small, track(700, 0.9)], |_| Ok(()))
            .unwrap();
        let (a, b) = (CharacterId::new(), CharacterId::new());
        // `a` is created first but `b` is on screen longer.
        db.apply_identity(
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
    }
}
