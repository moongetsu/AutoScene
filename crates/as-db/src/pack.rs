//! Scenepacks: the faces a pack is made from, a character's last filters,
//! and the shots picked or dropped by hand. Packs themselves are never
//! stored: `as_project::pack` makes them from these rows each time, so a
//! pack follows every new analysis and every decision in the cast.

use as_api::{SegmentKind, now_ms};
use as_core::{CharacterId, FrameIdx, MediaId};
use rusqlite::{OptionalExtension, params};

use crate::cast::parse_id;
use crate::{DbResult, ProjectDb, enum_to_sql};

/// One face in a shot a character is seen in: theirs, or someone else's on
/// screen with them.
#[derive(Clone, Debug, PartialEq)]
pub struct PackRow {
    pub media_id: MediaId,
    /// The shot: its first frame and its end (exclusive).
    pub shot: FrameIdx,
    pub shot_end: FrameIdx,
    /// Under 3 frames (kept only because both cuts around it are clear).
    pub tiny: bool,
    /// The shot's sprite, if it has one: the shot end it was made for, and its cells.
    pub sprite: Option<(FrameIdx, u32)>,
    pub track: i64,
    pub character_id: CharacterId,
    /// The character is hidden in the cast.
    pub hidden: bool,
    /// When the face is on screen (within the shot).
    pub start: FrameIdx,
    pub end: FrameIdx,
    /// Median face height, as a share of the frame height.
    pub face_h: f32,
    pub quality: f32,
}

impl ProjectDb {
    /// Every assigned face in the shots `c` is seen in, by media and shot;
    /// within a shot `c`'s own faces come first, clearest first.
    pub fn pack_rows(&self, c: CharacterId) -> DbResult<Vec<PackRow>> {
        self.0.read(|conn| {
            let mut st = conn.prepare(
                "WITH mine AS (
                     SELECT DISTINCT media_id, shot FROM tracks
                      WHERE character_id = ?1 AND assign IN ('auto', 'pinned') AND not_face = 0)
                 SELECT t.media_id, t.shot, g.end, g.tiny, sp.end, sp.cells, t.id, t.character_id, ch.hidden,
                        t.start, t.end, t.face_h, t.quality
                   FROM mine
                   JOIN segments g ON g.media_id = mine.media_id AND g.start = mine.shot AND g.kind = ?2
                   JOIN tracks t ON t.media_id = mine.media_id AND t.shot = mine.shot
                              AND t.assign IN ('auto', 'pinned') AND t.not_face = 0
                   JOIN characters ch ON ch.id = t.character_id
                   LEFT JOIN sprites sp ON sp.media_id = mine.media_id AND sp.shot = mine.shot
                  ORDER BY t.media_id, t.shot, t.character_id = ?1 DESC, t.face_h >= 0.12 DESC, t.quality DESC, t.id",
            )?;
            let mut rows = st.query(params![c.to_string(), enum_to_sql(SegmentKind::Shot)])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(PackRow {
                    media_id: parse_id(&r.get::<_, String>(0)?)?,
                    shot: r.get(1)?,
                    shot_end: r.get(2)?,
                    tiny: r.get(3)?,
                    sprite: r.get::<_, Option<FrameIdx>>(4)?.zip(r.get::<_, Option<u32>>(5)?),
                    track: r.get(6)?,
                    character_id: parse_id(&r.get::<_, String>(7)?)?,
                    hidden: r.get(8)?,
                    start: r.get(9)?,
                    end: r.get(10)?,
                    face_h: r.get::<_, f64>(11)? as f32,
                    quality: r.get::<_, f64>(12)? as f32,
                });
            }
            Ok(out)
        })
    }

    /// A character's name, or "Character N" until named; `None` when gone.
    /// (The whole cast costs far more: it counts every track.)
    pub fn character_label(&self, c: CharacterId) -> DbResult<Option<String>> {
        self.0.read(|conn| {
            let row = conn
                .query_row("SELECT name, number FROM characters WHERE id = ?1", [c.to_string()], |r| {
                    Ok((r.get::<_, Option<String>>(0)?, r.get::<_, u32>(1)?))
                })
                .optional()?;
            Ok(row.map(|(name, n)| name.unwrap_or_else(|| format!("Character {n}"))))
        })
    }

    /// A character's last pack filters and naming (JSON, as stored).
    pub fn pack_state(&self, c: CharacterId) -> DbResult<Option<(String, Option<String>)>> {
        self.0.read(|conn| {
            Ok(conn
                .query_row("SELECT spec, naming FROM packs WHERE character_id = ?1", [c.to_string()], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?)
        })
    }

    /// Keep a character's pack filters and naming. Returns `false` when the
    /// character is gone (merged away meanwhile).
    pub fn set_pack_state(&self, c: CharacterId, spec: &str, naming: Option<&str>) -> DbResult<bool> {
        self.0.write(|tx| {
            let n = tx.execute(
                "INSERT INTO packs (character_id, spec, naming, updated_at)
                 SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM characters WHERE id = ?1)
                 ON CONFLICT(character_id) DO UPDATE SET spec = excluded.spec, naming = excluded.naming,
                                                         updated_at = excluded.updated_at",
                params![c.to_string(), spec, naming, now_ms()],
            )?;
            Ok(n > 0)
        })
    }

    /// The shots picked (`true`) or dropped (`false`) by hand for `c`'s pack.
    pub fn pack_overrides(&self, c: CharacterId) -> DbResult<Vec<(MediaId, FrameIdx, bool)>> {
        self.0.read(|conn| {
            let mut st = conn.prepare("SELECT media_id, shot, include FROM pack_overrides WHERE character_id = ?1")?;
            let mut rows = st.query([c.to_string()])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push((parse_id(&r.get::<_, String>(0)?)?, r.get(1)?, r.get(2)?));
            }
            Ok(out)
        })
    }

    /// Pick (`Some(true)`), drop (`Some(false)`) or hand back to the filters
    /// (`None`) these shots of `c`'s pack, all at once. Returns `false` when
    /// the character is gone.
    pub fn set_pack_overrides(
        &self,
        c: CharacterId,
        shots: &[(MediaId, FrameIdx)],
        include: Option<bool>,
    ) -> DbResult<bool> {
        self.0.write(|tx| {
            let exists = tx
                .query_row("SELECT 1 FROM characters WHERE id = ?1", [c.to_string()], |_| Ok(()))
                .optional()?
                .is_some();
            if !exists {
                return Ok(false);
            }
            let now = now_ms();
            let mut put = tx.prepare(
                "INSERT INTO pack_overrides (character_id, media_id, shot, include, updated_at)
                 SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM media WHERE id = ?2)
                 ON CONFLICT(character_id, media_id, shot) DO UPDATE SET include = excluded.include,
                                                                         updated_at = excluded.updated_at",
            )?;
            let mut clear =
                tx.prepare("DELETE FROM pack_overrides WHERE character_id = ?1 AND media_id = ?2 AND shot = ?3")?;
            for (m, shot) in shots {
                match include {
                    Some(inc) => put.execute(params![c.to_string(), m.to_string(), shot, inc, now])?,
                    None => clear.execute(params![c.to_string(), m.to_string(), shot])?,
                };
            }
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use as_api::Segment;

    use super::*;
    use crate::NewTrack;
    use crate::cast::tests::{ready_media, track};

    fn shot(start: FrameIdx, end: FrameIdx) -> Segment {
        Segment { start, end, kind: SegmentKind::Shot, tiny: end - start < 3 }
    }

    fn face(shot: FrameIdx, start: FrameIdx, face_h: f32, quality: f32) -> NewTrack {
        NewTrack { start, end: start + 12, shot, face_h, ..track(start, quality) }
    }

    /// A media with shots 0–100, 100–102 (tiny) and 102–400, and faces in them.
    fn setup() -> (ProjectDb, MediaId, Vec<i64>) {
        let db = ProjectDb::memory();
        let m = ready_media(&db, "/v/a.mkv");
        db.store_segments(m, &[shot(0, 100), shot(100, 102), shot(102, 400)]).unwrap();
        let ids = db
            .put_face_chunk(
                m,
                0,
                1000,
                &[
                    face(0, 10, 0.30, 0.9),    // 0: Walter, close
                    face(0, 20, 0.10, 0.5),    // 1: Walter again, small
                    face(0, 30, 0.20, 0.8),    // 2: Jesse, with him
                    face(102, 150, 0.08, 0.7), // 3: Walter, alone
                    face(102, 200, 0.40, 0.9), // 4: an extra nobody named
                ],
                &[(102, 400, 8)],
                |_| Ok(()),
            )
            .unwrap();
        (db, m, ids)
    }

    fn character(db: &ProjectDb, tracks: &[i64], name: &str) -> CharacterId {
        let c = CharacterId::new();
        db.pin_tracks(tracks, c, Some(Some(name))).unwrap();
        c
    }

    #[test]
    fn rows_cover_everyone_in_the_characters_shots() {
        let (db, m, ids) = setup();
        let walter = character(&db, &[ids[0], ids[1], ids[3]], "Walter");
        let jesse = character(&db, &[ids[2]], "Jesse");
        let rows = db.pack_rows(walter).unwrap();
        // Shot 0: Walter's clearest face first, then his small one, then Jesse.
        let summary: Vec<_> = rows.iter().map(|r| (r.shot, r.track, r.character_id)).collect();
        assert_eq!(
            summary,
            [(0, ids[0], walter), (0, ids[1], walter), (0, ids[2], jesse), (102, ids[3], walter)],
            "the unassigned extra in shot 102 isn't anyone"
        );
        assert_eq!((rows[0].shot_end, rows[0].sprite), (100, None));
        assert_eq!((rows[3].shot_end, rows[3].sprite), (400, Some((400, 8))));
        assert_eq!((rows[0].face_h, rows[0].start, rows[0].end), (0.30, 10, 22));
        // Jesse's pack: his shot, with Walter in it.
        let rows = db.pack_rows(jesse).unwrap();
        assert_eq!(rows.iter().map(|r| r.character_id).collect::<Vec<_>>(), [jesse, walter, walter]);
        assert!(rows.iter().all(|r| r.media_id == m && !r.hidden));
    }

    #[test]
    fn hidden_characters_and_faces_that_arent_say_so() {
        let (db, _, ids) = setup();
        let walter = character(&db, &[ids[0], ids[1], ids[3]], "Walter");
        let jesse = character(&db, &[ids[2]], "Jesse");
        db.set_character_hidden(jesse, true, 0).unwrap();
        db.mark_not_face(&[ids[0]]).unwrap();
        let rows = db.pack_rows(walter).unwrap();
        assert!(rows.iter().all(|r| r.track != ids[0]), "not a face");
        // Still in shot 0 by his other face: Jesse is there with him, hidden.
        assert!(rows.iter().find(|r| r.character_id == jesse).unwrap().hidden);
        // Without any face of his left in a shot, the shot isn't his.
        db.mark_not_face(&[ids[1]]).unwrap();
        assert!(db.pack_rows(walter).unwrap().iter().all(|r| r.shot == 102));
    }

    #[test]
    fn filters_and_hand_picks_are_kept_per_character() {
        let (db, m, ids) = setup();
        let walter = character(&db, &[ids[0]], "Walter");
        assert_eq!(db.character_label(walter).unwrap().as_deref(), Some("Walter"));
        assert_eq!(db.character_label(CharacterId::new()).unwrap(), None);
        assert_eq!(db.pack_state(walter).unwrap(), None);
        assert!(db.set_pack_state(walter, r#"{"solo":true}"#, None).unwrap());
        assert!(db.set_pack_state(walter, r#"{"solo":false}"#, Some(r#"{"folder":true}"#)).unwrap());
        assert_eq!(
            db.pack_state(walter).unwrap(),
            Some((r#"{"solo":false}"#.into(), Some(r#"{"folder":true}"#.into())))
        );

        assert!(db.set_pack_overrides(walter, &[(m, 0), (m, 102)], Some(false)).unwrap());
        assert!(db.set_pack_overrides(walter, &[(m, 102)], Some(true)).unwrap());
        let mut o = db.pack_overrides(walter).unwrap();
        o.sort_by_key(|x| x.1);
        assert_eq!(o, [(m, 0, false), (m, 102, true)]);
        assert!(db.set_pack_overrides(walter, &[(m, 0)], None).unwrap());
        assert_eq!(db.pack_overrides(walter).unwrap(), [(m, 102, true)]);

        // A character that is gone keeps nothing.
        let gone = CharacterId::new();
        assert!(!db.set_pack_state(gone, "{}", None).unwrap());
        assert!(!db.set_pack_overrides(gone, &[(m, 0)], Some(true)).unwrap());
        // A media removed takes its hand picks along; the filters stay.
        db.remove_media(m).unwrap();
        assert!(db.pack_overrides(walter).unwrap().is_empty());
        assert!(db.pack_state(walter).unwrap().is_some());
    }

    #[test]
    fn a_merge_moves_the_pack_to_the_character_that_stays() {
        let (db, m, ids) = setup();
        let walter = character(&db, &[ids[0]], "Walter");
        let heisenberg = character(&db, &[ids[3]], "Heisenberg");
        db.set_pack_state(heisenberg, r#"{"solo":true}"#, None).unwrap();
        db.set_pack_overrides(heisenberg, &[(m, 102)], Some(true)).unwrap();
        db.set_pack_overrides(walter, &[(m, 0)], Some(false)).unwrap();
        db.merge_characters(heisenberg, walter).unwrap();
        let mut o = db.pack_overrides(walter).unwrap();
        o.sort_by_key(|x| x.1);
        assert_eq!(o, [(m, 0, false), (m, 102, true)]);
        // Walter had no filters of his own: Heisenberg's come along.
        assert_eq!(db.pack_state(walter).unwrap(), Some((r#"{"solo":true}"#.into(), None)));
        assert_eq!(db.pack_state(heisenberg).unwrap(), None);
    }
}
