//! People followed by their bodies in `project.db` (an optional mode):
//! body tracks per checkpoint chunk, tied to the face tracks on them; who
//! they are, derived after every clustering; and the user's word on them,
//! kept like a face's — on the body while it exists, in `body_edits` while
//! its media is analysed again.

use as_api::StageState;
use as_core::{CharacterId, FrameIdx, MediaId};
use rusqlite::{OptionalExtension, Transaction, params, params_from_iter};

use crate::cast::{ids_param, marks, parse_id};
use crate::project::ProjectDb;
use crate::{DbError, DbResult, enum_to_sql};

/// A body track from the analysis, ready to store.
#[derive(Clone, Debug, PartialEq)]
pub struct NewBody {
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub shot: FrameIdx,
    /// `as_analysis::bodies::dets_to_bytes`.
    pub dets: Vec<u8>,
    /// How they look (256 × f16), and while no face of theirs shows.
    pub reid: Option<Vec<u8>>,
    pub reid_away: Option<Vec<u8>>,
    /// How they show (`as_analysis::bodies::Showing`).
    pub height: f32,
    pub aspect: f32,
    pub top: f32,
    pub foreground: f32,
    pub score: f32,
    /// Face tracks tied to it: `(track id, samples that tied them)`.
    pub faces: Vec<(i64, u32)>,
}

/// How a body got to its character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodySource {
    None,
    /// The face on it is the character.
    Face,
    /// Found again by its clothes.
    Clothes,
    /// Its faces are different characters.
    Conflict,
    /// The user's decision.
    Pinned,
}

impl BodySource {
    fn as_sql(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Face => "face",
            Self::Clothes => "clothes",
            Self::Conflict => "conflict",
            Self::Pinned => "pinned",
        }
    }

    fn from_sql(s: &str) -> DbResult<Self> {
        Ok(match s {
            "none" => Self::None,
            "face" => Self::Face,
            "clothes" => Self::Clothes,
            "conflict" => Self::Conflict,
            "pinned" => Self::Pinned,
            other => return Err(DbError::Corrupt(format!("unknown body source '{other}'"))),
        })
    }
}

/// What the identity step needs of a body.
#[derive(Clone, Debug)]
pub struct IdentityBody {
    pub id: i64,
    pub media_id: MediaId,
    pub end: FrameIdx,
    /// `as_analysis::bodies::dets_to_bytes`.
    pub dets: Vec<u8>,
    pub reid: Option<Vec<u8>>,
    pub reid_away: Option<Vec<u8>>,
    /// The surest of its detections.
    pub score: f32,
    /// Face tracks tied to it: `(track id, samples)`.
    pub faces: Vec<(i64, u32)>,
    /// The user put it on this character, or said it isn't these.
    pub pin: Option<CharacterId>,
    pub forbids: Vec<CharacterId>,
}

/// A body with a character: in a pack's shots, an episode's timeline, the cast's counts.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyRow {
    pub id: i64,
    pub media_id: MediaId,
    pub shot: FrameIdx,
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub dets: Vec<u8>,
    pub character_id: CharacterId,
    pub source: BodySource,
    /// The character is hidden in the cast.
    pub hidden: bool,
    pub height: f32,
    pub aspect: f32,
    pub top: f32,
    pub foreground: f32,
    pub score: f32,
}

/// A body in a shot a character is seen in, with the shot.
#[derive(Clone, Debug, PartialEq)]
pub struct PackBodyRow {
    pub body: BodyRow,
    /// The shot's end (exclusive), whether it is under 3 frames, and its
    /// sprite (the shot end it was made for, and its cells).
    pub shot_end: FrameIdx,
    pub tiny: bool,
    pub sprite: Option<(FrameIdx, u32)>,
}

/// What the identity step made of a body.
#[derive(Clone, Copy, Debug)]
pub struct BodyUpdate {
    pub id: i64,
    pub character: Option<CharacterId>,
    pub source: BodySource,
    pub similarity: f32,
}

/// The user's word on a body whose media is being analysed again.
#[derive(Clone, Debug)]
pub struct BodyEdit {
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub dets: Vec<u8>,
    pub kind: BodyEditKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyEditKind {
    Pin(CharacterId),
    Forbid(CharacterId),
    NotPerson,
}

/// A face track as a pass for people ties them: `(id, start, end, shot, dets)`.
pub type KnownFaceRow = (i64, FrameIdx, FrameIdx, FrameIdx, Vec<u8>);
/// A body with its range and detections: `(id, start, end, dets)`.
pub type BodyDets = (i64, FrameIdx, FrameIdx, Vec<u8>);
/// A face of a character as a range of its shot: `(character, media, shot, start, end)`.
pub type FaceRange = (CharacterId, MediaId, FrameIdx, FrameIdx, FrameIdx);

const BODY_COLS: &str = "b.id, b.media_id, b.shot, b.start, b.end, b.dets, b.character_id, b.source, ch.hidden, \
     b.height, b.aspect, b.top, b.foreground, b.score";

fn waiting_bodies(c: &rusqlite::Connection) -> DbResult<Vec<MediaId>> {
    let mut st = c.prepare(
        "SELECT id FROM media WHERE status = 'ready' AND faces = 'ready' AND bodies = 'pending'
         ORDER BY episode IS NULL, COALESCE(season, 0), episode, position",
    )?;
    let ids = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|s| parse_id(s)).collect()
}

/// Bodies that count as someone's: by a face, by clothes, or the user's.
const ASSIGNED: &str = "b.character_id IS NOT NULL AND b.not_person = 0 AND b.source IN ('face', 'clothes', 'pinned')";

fn body_row(r: &rusqlite::Row) -> DbResult<BodyRow> {
    Ok(BodyRow {
        id: r.get(0)?,
        media_id: parse_id(&r.get::<_, String>(1)?)?,
        shot: r.get(2)?,
        start: r.get(3)?,
        end: r.get(4)?,
        dets: r.get(5)?,
        character_id: parse_id(&r.get::<_, String>(6)?)?,
        source: BodySource::from_sql(&r.get::<_, String>(7)?)?,
        hidden: r.get(8)?,
        height: r.get::<_, f64>(9)? as f32,
        aspect: r.get::<_, f64>(10)? as f32,
        top: r.get::<_, f64>(11)? as f32,
        foreground: r.get::<_, f64>(12)? as f32,
        score: r.get::<_, f64>(13)? as f32,
    })
}

/// Forget a media's bodies, keeping the user's word on them in
/// `body_edits` for the next pass to put back.
pub(crate) fn forget_bodies(tx: &Transaction, id: MediaId) -> DbResult<()> {
    let m = id.to_string();
    tx.execute(
        "INSERT INTO body_edits (media_id, start, end, dets, kind, character_id)
           SELECT media_id, start, end, dets, 'pin', character_id FROM bodies
            WHERE media_id = ?1 AND source = 'pinned' AND character_id IS NOT NULL
           UNION ALL
           SELECT b.media_id, b.start, b.end, b.dets, 'forbid', f.character_id
             FROM bodies b JOIN body_forbids f ON f.body_id = b.id WHERE b.media_id = ?1
           UNION ALL
           SELECT media_id, start, end, dets, 'not_person', NULL FROM bodies WHERE media_id = ?1 AND not_person = 1",
        [&m],
    )?;
    tx.execute("DELETE FROM bodies WHERE media_id = ?1", [&m])?;
    tx.execute("DELETE FROM body_chunks WHERE media_id = ?1", [&m])?;
    tx.execute("UPDATE media SET bodies = 'pending', bodies_error = NULL WHERE id = ?1", [&m])?;
    Ok(())
}

/// Drop a media's bodies (and their checkpoints) from frame `start` on:
/// they were tied to faces that go too.
pub(crate) fn drop_bodies_from(tx: &Transaction, id: MediaId, start: FrameIdx) -> DbResult<()> {
    let m = id.to_string();
    tx.execute("DELETE FROM bodies WHERE media_id = ?1 AND shot >= ?2", params![m, start])?;
    tx.execute("DELETE FROM body_chunks WHERE media_id = ?1 AND end > ?2", params![m, start])?;
    Ok(())
}

impl ProjectDb {
    pub fn set_bodies_state(&self, id: MediaId, state: StageState, error: Option<&str>) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE media SET bodies = ?2, bodies_error = ?3 WHERE id = ?1",
                params![id.to_string(), enum_to_sql(state), error],
            )?;
            Ok(())
        })
    }

    /// Finished chunks `(start, end)` of a media's bodies, in order.
    pub fn body_chunks(&self, id: MediaId) -> DbResult<Vec<(FrameIdx, FrameIdx)>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT start, end FROM body_chunks WHERE media_id = ?1 ORDER BY start")?;
            let rows = st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Drop body chunks (and their bodies) from `start` on.
    pub fn drop_body_chunks_from(&self, id: MediaId, start: FrameIdx) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("DELETE FROM bodies WHERE media_id = ?1 AND shot >= ?2", params![id.to_string(), start])?;
            tx.execute("DELETE FROM body_chunks WHERE media_id = ?1 AND start >= ?2", params![id.to_string(), start])?;
            Ok(())
        })
    }

    /// Store the bodies of chunk `start..end` (replacing what it held) and
    /// the chunk itself, atomically. Ties to face tracks that are gone are
    /// left out. Body ids are never handed out twice. Returns them.
    pub fn put_body_chunk(
        &self,
        id: MediaId,
        start: FrameIdx,
        end: FrameIdx,
        bodies: &[NewBody],
    ) -> DbResult<Vec<i64>> {
        self.0.write(|tx| {
            let m = id.to_string();
            let mut last: i64 = tx.query_row(
                "SELECT MAX(COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'body_seq'), 0),
                            COALESCE((SELECT MAX(id) FROM bodies), 0))",
                [],
                |r| r.get(0),
            )?;
            tx.execute("DELETE FROM bodies WHERE media_id = ?1 AND shot >= ?2 AND shot < ?3", params![m, start, end])?;
            let mut put = tx.prepare(
                "INSERT INTO bodies (id, media_id, start, end, shot, dets, reid, reid_away, height, aspect, top,
                                     foreground, score)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )?;
            let mut tie = tx.prepare(
                "INSERT OR IGNORE INTO body_faces (body_id, track_id, samples)
                 SELECT ?1, id, ?3 FROM tracks WHERE id = ?2 AND media_id = ?4",
            )?;
            let mut ids = Vec::with_capacity(bodies.len());
            for b in bodies {
                last += 1;
                put.execute(params![
                    last,
                    m,
                    b.start,
                    b.end,
                    b.shot,
                    b.dets,
                    b.reid,
                    b.reid_away,
                    b.height as f64,
                    b.aspect as f64,
                    b.top as f64,
                    b.foreground as f64,
                    b.score as f64,
                ])?;
                for &(track, samples) in &b.faces {
                    tie.execute(params![last, track, samples, m])?;
                }
                ids.push(last);
            }
            drop((put, tie));
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('body_seq', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [last.to_string()],
            )?;
            tx.execute(
                "INSERT INTO body_chunks (media_id, start, end) VALUES (?1, ?2, ?3)
                 ON CONFLICT(media_id, start) DO UPDATE SET end = excluded.end",
                params![m, start, end],
            )?;
            Ok(ids)
        })
    }

    /// Media whose faces are found and whose people aren't (an interrupted
    /// run counts as pending again: for a launch, when nothing runs yet), in
    /// episode order.
    pub fn pending_bodies(&self) -> DbResult<Vec<MediaId>> {
        self.0.write(|tx| {
            tx.execute("UPDATE media SET bodies = 'pending' WHERE bodies = 'running'", [])?;
            waiting_bodies(tx)
        })
    }

    /// Media whose faces are found and whose people wait to be found, in
    /// episode order — those being found now left as they are.
    pub fn waiting_bodies(&self) -> DbResult<Vec<MediaId>> {
        self.0.read(waiting_bodies)
    }

    /// The face tracks of a media, as a pass for its people ties them
    /// (faces the user said aren't, left out).
    pub fn known_faces(&self, id: MediaId) -> DbResult<Vec<KnownFaceRow>> {
        self.0.read(|c| {
            let mut st = c.prepare(
                "SELECT id, start, end, shot, dets FROM tracks WHERE media_id = ?1 AND not_face = 0 ORDER BY shot, start, id",
            )?;
            let rows =
                st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// The user's word on bodies waiting for a media's new bodies.
    pub fn body_edits(&self, id: MediaId) -> DbResult<Vec<BodyEdit>> {
        self.0.read(|c| {
            let mut st =
                c.prepare("SELECT start, end, dets, kind, character_id FROM body_edits WHERE media_id = ?1")?;
            let mut rows = st.query([id.to_string()])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let kind: String = r.get(3)?;
                let ch: Option<String> = r.get(4)?;
                let ch = ch.as_deref().map(parse_id).transpose()?;
                let kind = match (kind.as_str(), ch) {
                    ("pin", Some(c)) => BodyEditKind::Pin(c),
                    ("forbid", Some(c)) => BodyEditKind::Forbid(c),
                    ("not_person", _) => BodyEditKind::NotPerson,
                    _ => continue,
                };
                out.push(BodyEdit { start: r.get(0)?, end: r.get(1)?, dets: r.get(2)?, kind });
            }
            Ok(out)
        })
    }

    /// Every body of a media with its range and detections (to put the
    /// user's word back on).
    pub fn media_body_dets(&self, id: MediaId) -> DbResult<Vec<BodyDets>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT id, start, end, dets FROM bodies WHERE media_id = ?1 ORDER BY start")?;
            let rows = st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Put the user's word back on new bodies (`(body, edit)`, one
    /// consistent set per body) and forget all of the media's waiting ones.
    pub fn reanchor_body_edits(&self, id: MediaId, matched: &[(i64, BodyEditKind)]) -> DbResult<()> {
        self.0.write(|tx| {
            for &(body, kind) in matched {
                match kind {
                    BodyEditKind::Pin(c) => pin_body(tx, body, c)?,
                    BodyEditKind::Forbid(c) => forbid_body(tx, body, c)?,
                    BodyEditKind::NotPerson => {
                        tx.execute(
                            "UPDATE bodies SET not_person = 1, character_id = NULL, source = 'none', similarity = 0
                              WHERE id = ?1",
                            [body],
                        )?;
                    }
                }
            }
            tx.execute("DELETE FROM body_edits WHERE media_id = ?1", [id.to_string()])?;
            crate::cast::bump_rev(tx)
        })
    }

    /// Every body that may be someone — of media whose faces and people
    /// are both found (a pass still running has stored only part of them,
    /// without the user's earlier word on them yet).
    pub fn identity_bodies(&self) -> DbResult<Vec<IdentityBody>> {
        self.0.read(|c| {
            let mut faces: std::collections::HashMap<i64, Vec<(i64, u32)>> = Default::default();
            {
                let mut st = c.prepare("SELECT body_id, track_id, samples FROM body_faces")?;
                let mut rows = st.query([])?;
                while let Some(r) = rows.next()? {
                    faces.entry(r.get(0)?).or_default().push((r.get(1)?, r.get(2)?));
                }
            }
            let mut forbids: std::collections::HashMap<i64, Vec<CharacterId>> = Default::default();
            {
                let mut st = c.prepare("SELECT body_id, character_id FROM body_forbids")?;
                let mut rows = st.query([])?;
                while let Some(r) = rows.next()? {
                    forbids.entry(r.get(0)?).or_default().push(parse_id(&r.get::<_, String>(1)?)?);
                }
            }
            let mut st = c.prepare(
                "SELECT id, media_id, end, dets, reid, reid_away, score, source, character_id FROM bodies
                  WHERE not_person = 0
                    AND media_id IN (SELECT id FROM media WHERE faces = 'ready' AND bodies = 'ready')
                  ORDER BY media_id, start, id",
            )?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                let pinned = r.get::<_, String>(7)? == "pinned";
                let ch: Option<String> = r.get(8)?;
                out.push(IdentityBody {
                    id,
                    media_id: parse_id(&r.get::<_, String>(1)?)?,
                    end: r.get(2)?,
                    dets: r.get(3)?,
                    reid: r.get(4)?,
                    reid_away: r.get(5)?,
                    score: r.get::<_, f64>(6)? as f32,
                    faces: faces.remove(&id).unwrap_or_default(),
                    pin: if pinned { ch.as_deref().map(parse_id).transpose()? } else { None },
                    forbids: forbids.remove(&id).unwrap_or_default(),
                });
            }
            Ok(out)
        })
    }

    /// Whether anything the bodies said is stored: faces their body named,
    /// bodies their faces or clothes named (the user's own word aside).
    pub fn has_body_names(&self) -> DbResult<bool> {
        self.0.read(|c| {
            Ok(c.query_row(
                "SELECT EXISTS (SELECT 1 FROM tracks WHERE via_body = 1)
                     OR EXISTS (SELECT 1 FROM bodies WHERE source IN ('face', 'clothes', 'conflict'))",
                [],
                |r| r.get(0),
            )?)
        })
    }

    /// Every body that is someone's.
    pub fn assigned_bodies(&self) -> DbResult<Vec<BodyRow>> {
        self.0.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {BODY_COLS} FROM bodies b JOIN characters ch ON ch.id = b.character_id
                  WHERE {ASSIGNED} ORDER BY b.media_id, b.shot, b.id"
            ))?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(body_row(r)?);
            }
            Ok(out)
        })
    }

    /// The bodies of one media that are someone's.
    pub fn media_bodies(&self, id: MediaId) -> DbResult<Vec<BodyRow>> {
        self.0.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {BODY_COLS} FROM bodies b JOIN characters ch ON ch.id = b.character_id
                  WHERE b.media_id = ?1 AND {ASSIGNED} ORDER BY b.start, b.id"
            ))?;
            let mut rows = st.query([id.to_string()])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(body_row(r)?);
            }
            Ok(out)
        })
    }

    /// Every body that is someone's in the shots `c` is seen in (by face or
    /// by body), by media and shot; within a shot `c`'s own first, the
    /// tallest first.
    pub fn pack_bodies(&self, c: CharacterId) -> DbResult<Vec<PackBodyRow>> {
        self.0.read(|conn| {
            let mut st = conn.prepare(&format!(
                "WITH mine AS ({MINE})
                 SELECT {BODY_COLS}, g.end, g.tiny, sp.end, sp.cells FROM mine
                   JOIN segments g ON g.media_id = mine.media_id AND g.start = mine.shot AND g.kind = ?2
                   JOIN bodies b ON b.media_id = mine.media_id AND b.shot = mine.shot
                   JOIN characters ch ON ch.id = b.character_id
                   LEFT JOIN sprites sp ON sp.media_id = mine.media_id AND sp.shot = mine.shot
                  WHERE {ASSIGNED}
                  ORDER BY b.media_id, b.shot, b.character_id = ?1 DESC, b.height DESC, b.id"
            ))?;
            let mut rows = st.query(params![c.to_string(), enum_to_sql(as_api::SegmentKind::Shot)])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(PackBodyRow {
                    body: body_row(r)?,
                    shot_end: r.get(14)?,
                    tiny: r.get(15)?,
                    sprite: r.get::<_, Option<FrameIdx>>(16)?.zip(r.get::<_, Option<u32>>(17)?),
                });
            }
            Ok(out)
        })
    }

    /// The small picture of `c`'s cover face (its track and media), as
    /// [`ProjectDb::characters`] picks it — for one character.
    pub fn character_cover(&self, c: CharacterId) -> DbResult<Option<(i64, MediaId)>> {
        self.0.read(|conn| {
            let cover: Option<i64> = conn
                .query_row(
                    &format!("SELECT {} FROM characters c WHERE c.id = ?1", crate::cast::COVER),
                    [c.to_string()],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let Some(track) = cover else { return Ok(None) };
            let media: Option<String> =
                conn.query_row("SELECT media_id FROM tracks WHERE id = ?1", [track], |r| r.get(0)).optional()?;
            media.map(|m| Ok((track, parse_id(&m)?))).transpose()
        })
    }

    /// Every face of a character, as ranges of the shots they are in.
    pub fn face_ranges(&self) -> DbResult<Vec<FaceRange>> {
        self.0.read(|c| {
            let mut st = c.prepare(
                "SELECT character_id, media_id, shot, start, end FROM tracks
                  WHERE character_id IS NOT NULL AND assign IN ('auto', 'pinned') AND not_face = 0
                  ORDER BY media_id, shot, start",
            )?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push((
                    parse_id(&r.get::<_, String>(0)?)?,
                    parse_id(&r.get::<_, String>(1)?)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                ));
            }
            Ok(out)
        })
    }

    /// These bodies are `c` (the user's word).
    pub fn pin_bodies(&self, ids: &[i64], c: CharacterId) -> DbResult<()> {
        self.0.write(|tx| {
            for &id in ids {
                pin_body(tx, id, c)?;
            }
            crate::cast::bump_rev(tx)
        })
    }

    /// These bodies are not `c`.
    pub fn forbid_bodies(&self, ids: &[i64], c: CharacterId) -> DbResult<()> {
        self.0.write(|tx| {
            for &id in ids {
                forbid_body(tx, id, c)?;
            }
            crate::cast::bump_rev(tx)
        })
    }

    /// These bodies aren't people.
    pub fn mark_not_person(&self, ids: &[i64]) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                &format!(
                    "UPDATE bodies SET not_person = 1, character_id = NULL, source = 'none', similarity = 0
                      WHERE id IN ({})",
                    marks(ids.len())
                ),
                params_from_iter(ids_param(ids)),
            )?;
            crate::cast::bump_rev(tx)
        })
    }

    /// Whether `body` exists.
    pub fn body_exists(&self, body: i64) -> DbResult<bool> {
        self.0.read(|c| Ok(c.query_row("SELECT 1 FROM bodies WHERE id = ?1", [body], |_| Ok(())).optional()?.is_some()))
    }
}

/// The shots `?1` is seen in: by a face, or by a body that is theirs.
pub(crate) const MINE: &str = "SELECT DISTINCT media_id, shot FROM tracks
      WHERE character_id = ?1 AND assign IN ('auto', 'pinned') AND not_face = 0
     UNION
     SELECT DISTINCT media_id, shot FROM bodies
      WHERE character_id = ?1 AND not_person = 0 AND source IN ('face', 'clothes', 'pinned')";

fn pin_body(tx: &Transaction, body: i64, c: CharacterId) -> DbResult<()> {
    let n = tx.execute(
        "UPDATE bodies SET character_id = ?2, source = 'pinned', similarity = 1, not_person = 0 WHERE id = ?1
           AND EXISTS (SELECT 1 FROM characters WHERE id = ?2)",
        params![body, c.to_string()],
    )?;
    if n > 0 {
        tx.execute("DELETE FROM body_forbids WHERE body_id = ?1 AND character_id = ?2", params![body, c.to_string()])?;
    }
    Ok(())
}

fn forbid_body(tx: &Transaction, body: i64, c: CharacterId) -> DbResult<()> {
    tx.execute(
        "INSERT OR IGNORE INTO body_forbids (body_id, character_id)
         SELECT ?1, id FROM characters WHERE id = ?2 AND EXISTS (SELECT 1 FROM bodies WHERE id = ?1)",
        params![body, c.to_string()],
    )?;
    tx.execute(
        "UPDATE bodies SET character_id = NULL, source = 'none', similarity = 0 WHERE id = ?1 AND character_id = ?2",
        params![body, c.to_string()],
    )?;
    Ok(())
}

/// Store what the identity step made of the bodies: everything derived
/// before goes (a body nobody is now is no one), the user's pins stay.
pub(crate) fn apply_bodies(tx: &Transaction, bodies: &[BodyUpdate]) -> DbResult<()> {
    tx.execute("UPDATE bodies SET character_id = NULL, source = 'none', similarity = 0 WHERE source <> 'pinned'", [])?;
    let mut st = tx.prepare(
        "UPDATE bodies SET character_id = ?2, source = ?3, similarity = ?4
          WHERE id = ?1 AND source <> 'pinned' AND not_person = 0",
    )?;
    for b in bodies.iter().filter(|b| b.source != BodySource::Pinned) {
        st.execute(params![b.id, b.character.map(|c| c.to_string()), b.source.as_sql(), b.similarity as f64])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackUpdate;
    use crate::cast::tests::{ready_media, track};
    use as_api::{SegmentKind, TrackAssign};

    pub(crate) fn body(shot: FrameIdx, start: FrameIdx, faces: Vec<(i64, u32)>) -> NewBody {
        NewBody {
            start,
            end: start + 30,
            shot,
            dets: vec![0; 52],
            reid: Some(vec![0; 512]),
            reid_away: None,
            height: 0.8,
            aspect: 0.5,
            top: 0.1,
            foreground: 0.0,
            score: 0.9,
            faces,
        }
    }

    #[test]
    fn bodies_are_stored_per_chunk_tied_to_their_faces() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        let faces = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9), track(300, 0.5)], &[], |_| Ok(())).unwrap();
        let ids = db
            .put_body_chunk(m, 0, 1000, &[body(10, 10, vec![(faces[0], 6), (9999, 2)]), body(300, 300, vec![])])
            .unwrap();
        assert_eq!(db.body_chunks(m).unwrap(), [(0, 1000)]);
        // Faces not done: no one to name yet.
        assert!(db.identity_bodies().unwrap().is_empty());
        db.set_faces_state(m, StageState::Ready, None).unwrap();
        assert_eq!(db.pending_bodies().unwrap(), [m]);
        // Being found: left alone while the app runs, waiting again on a launch.
        db.set_bodies_state(m, StageState::Running, None).unwrap();
        assert!(db.waiting_bodies().unwrap().is_empty());
        assert_eq!(db.pending_bodies().unwrap(), [m]);
        assert_eq!(db.waiting_bodies().unwrap(), [m]);
        db.set_bodies_state(m, StageState::Ready, None).unwrap();
        let got = db.identity_bodies().unwrap();
        assert_eq!(
            got.iter().map(|b| (b.id, b.faces.clone())).collect::<Vec<_>>(),
            [(ids[0], vec![(faces[0], 6)]), (ids[1], vec![])]
        );
        // The chunk again (a resumed pass): replaced, never the same ids.
        let again = db.put_body_chunk(m, 0, 1000, &[body(10, 10, vec![])]).unwrap();
        assert!(again[0] > ids[1]);
        assert_eq!(db.identity_bodies().unwrap().len(), 1);
        // Faces dropped from a chunk take their bodies along.
        db.drop_face_chunks_from(m, 0).unwrap();
        assert!(db.media_body_dets(m).unwrap().is_empty() && db.body_chunks(m).unwrap().is_empty());
        // A media's faces for a pass of its people, by shot.
        let faces = db.put_face_chunk(m, 0, 1000, &[track(300, 0.5), track(10, 0.9)], &[], |_| Ok(())).unwrap();
        db.mark_not_face(&[faces[0]]).unwrap();
        assert_eq!(db.known_faces(m).unwrap().iter().map(|f| f.0).collect::<Vec<_>>(), [faces[1]]);
    }

    #[test]
    fn who_a_body_is_is_derived_and_the_users_word_holds_through_a_reanalysis() {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        db.store_segments(m, &[as_api::Segment { start: 0, end: 1000, kind: SegmentKind::Shot, tiny: false }]).unwrap();
        let faces = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9)], &[], |_| Ok(())).unwrap();
        let ids = db.put_body_chunk(m, 0, 1000, &[body(0, 10, vec![(faces[0], 6)]), body(0, 400, vec![])]).unwrap();
        db.set_faces_state(m, StageState::Ready, None).unwrap();
        db.set_bodies_state(m, StageState::Ready, None).unwrap();
        let (walter, jesse) = (CharacterId::new(), CharacterId::new());
        let rev = db.cast_rev().unwrap();
        let update = |id, c: Option<CharacterId>, source| BodyUpdate { id, character: c, source, similarity: 0.8 };
        let face = TrackUpdate {
            id: faces[0],
            character: Some(walter),
            assign: TrackAssign::Auto,
            score: 1.0,
            via_body: false,
        };
        assert!(
            db.apply_identity(
                rev,
                &[walter],
                &[face],
                &[update(ids[0], Some(walter), BodySource::Face), update(ids[1], Some(walter), BodySource::Clothes)]
            )
            .unwrap()
        );
        // Someone named, with no face of theirs found yet.
        db.pin_tracks(&[], jesse, Some(Some("Jesse"))).unwrap();
        assert_eq!(db.assigned_bodies().unwrap().len(), 2);
        assert_eq!(
            db.pack_bodies(walter).unwrap().iter().map(|b| (b.body.id, b.shot_end)).collect::<Vec<_>>(),
            [(ids[0], 1000), (ids[1], 1000)]
        );
        assert_eq!(db.character_cover(walter).unwrap(), Some((faces[0], m)));
        assert_eq!(db.face_ranges().unwrap().len(), 1);
        assert_eq!(db.media_bodies(m).unwrap()[1].source, BodySource::Clothes);
        // The user: the second is Jesse; then "not Jesse" on the first.
        db.pin_bodies(&[ids[1]], jesse).unwrap();
        db.forbid_bodies(&[ids[0]], walter).unwrap();
        // A new clustering keeps the pin and starts from nothing for the rest.
        let rev = db.cast_rev().unwrap();
        assert!(db.apply_identity(rev, &[], &[], &[update(ids[1], Some(walter), BodySource::Clothes)]).unwrap());
        let got = db.identity_bodies().unwrap();
        assert_eq!(got[1].pin, Some(jesse));
        assert_eq!(got[0].forbids, [walter]);
        assert!(db.assigned_bodies().unwrap().iter().all(|b| b.id == ids[1] && b.character_id == jesse));
        // Analysed again: the user's word waits, then lands on the same bodies.
        db.reset_faces(m).unwrap();
        assert!(db.media_body_dets(m).unwrap().is_empty());
        assert_eq!(db.media(m).unwrap().unwrap().bodies, StageState::Pending);
        let mut kinds: Vec<_> = db.body_edits(m).unwrap().into_iter().map(|e| (e.start, e.kind)).collect();
        kinds.sort_by_key(|k| k.0);
        assert_eq!(kinds, [(10, BodyEditKind::Forbid(walter)), (400, BodyEditKind::Pin(jesse))]);
        assert!(db.character_exists(jesse).unwrap(), "a body's pin keeps its character");
        let faces = db.put_face_chunk(m, 0, 1000, &[track(10, 0.9)], &[], |_| Ok(())).unwrap();
        let new = db.put_body_chunk(m, 0, 1000, &[body(0, 10, vec![(faces[0], 6)]), body(0, 400, vec![])]).unwrap();
        db.reanchor_body_edits(m, &[(new[0], BodyEditKind::Forbid(walter)), (new[1], BodyEditKind::Pin(jesse))])
            .unwrap();
        assert!(db.body_edits(m).unwrap().is_empty());
        db.set_faces_state(m, StageState::Ready, None).unwrap();
        db.set_bodies_state(m, StageState::Ready, None).unwrap();
        let got = db.identity_bodies().unwrap();
        assert_eq!((got[0].forbids.clone(), got[1].pin), (vec![walter], Some(jesse)));
        // Not a person: never anyone's again.
        db.mark_not_person(&[new[1]]).unwrap();
        assert_eq!(db.identity_bodies().unwrap().len(), 1);
        // Merged: everything of one character goes to the other.
        db.pin_bodies(&[new[0]], walter).unwrap();
        db.merge_characters(walter, jesse).unwrap();
        assert!(db.assigned_bodies().unwrap().iter().all(|b| b.character_id == jesse));
    }
}
