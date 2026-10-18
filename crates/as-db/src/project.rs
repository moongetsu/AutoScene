//! `project.db`: one per `<Title>.autoscene` folder.

use std::collections::HashSet;
use std::path::Path;

use as_api::{
    ClipOutcome, ExportItem, ExportPreset, ExportStatus, ExportSummary, MediaInfo, MediaItem, MediaStatus, MergedFile,
    ProxyState, Segment, StageState, now_ms,
};
use as_core::{ExportId, FrameIdx, MediaId, ProjectId, Rational};
use rusqlite::{OptionalExtension, Row, params};
use rusqlite_migration::{M, Migrations};

use crate::{Db, DbError, DbResult, enum_from_sql, enum_to_sql, json_from_sql, json_to_sql};

/// Append-only: never edit a released migration, add a new one.
const STEPS: &[M<'static>] = &[
    M::up(
        "CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;

    CREATE TABLE media (
        id            TEXT PRIMARY KEY,
        path          TEXT NOT NULL UNIQUE,
        file_name     TEXT NOT NULL,
        size_bytes    INTEGER NOT NULL,
        mtime         INTEGER,
        season        INTEGER,
        episode       INTEGER,
        position      INTEGER NOT NULL,
        status        TEXT NOT NULL,
        error         TEXT,
        info          TEXT,        -- MediaInfo JSON
        frames        INTEGER,
        duration_sec  REAL,
        fps_num       INTEGER,
        fps_den       INTEGER,
        vfr           INTEGER NOT NULL DEFAULT 0,
        interlaced    INTEGER NOT NULL DEFAULT 0,
        audio_ordinal INTEGER,
        proxy         TEXT NOT NULL DEFAULT 'unknown',
        proxy_path    TEXT,
        proxy_t0      REAL,
        poster        TEXT,
        added_at      INTEGER NOT NULL
    ) STRICT;

    -- FrameIndex::to_bytes (zstd). Kept apart from `media` so listing stays cheap.
    CREATE TABLE frame_index (
        media_id TEXT PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
        data     BLOB NOT NULL
    ) STRICT;

    CREATE TABLE exports (
        id         TEXT PRIMARY KEY,
        created_at INTEGER NOT NULL,
        preset     TEXT NOT NULL,  -- ExportPreset JSON, frozen at export time
        out_dir    TEXT NOT NULL,
        status     TEXT NOT NULL
    ) STRICT;

    CREATE TABLE export_items (
        id          INTEGER PRIMARY KEY,
        export_id   TEXT NOT NULL REFERENCES exports(id) ON DELETE CASCADE,
        media_id    TEXT NOT NULL,
        start_frame INTEGER NOT NULL,
        end_frame   INTEGER NOT NULL,
        name        TEXT NOT NULL,
        status      TEXT NOT NULL,
        path        TEXT,
        verified    INTEGER NOT NULL DEFAULT 0,
        encoder     TEXT,
        bytes       INTEGER,
        error       TEXT
    ) STRICT;
    CREATE INDEX export_items_by_export ON export_items(export_id);",
    ),
    // The audio track is part of the export plan, frozen when it's queued.
    M::up("ALTER TABLE export_items ADD COLUMN audio_ordinal INTEGER;"),
    // M2: shot detection. Signals are checkpointed per chunk so an analysis
    // resumes where it stopped; segments are what fusion made of them.
    M::up(
        "ALTER TABLE media ADD COLUMN shots TEXT NOT NULL DEFAULT 'pending';
    ALTER TABLE media ADD COLUMN shots_error TEXT;

    -- as_analysis::signals::encode of frames start.. (one chunk)
    CREATE TABLE shot_signals (
        media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        start    INTEGER NOT NULL,
        data     BLOB NOT NULL,
        PRIMARY KEY (media_id, start)
    ) STRICT;

    CREATE TABLE segments (
        media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        start    INTEGER NOT NULL,
        end      INTEGER NOT NULL,
        kind     TEXT NOT NULL,
        tiny     INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (media_id, start)
    ) STRICT;",
    ),
    // M3: faces and characters. Tracks are stored per checkpoint chunk (a
    // face_chunks row says frames start..end are done); characters are the
    // project's people, tracks point at them. User edits are pins, forbids
    // and not-a-face marks on tracks — never cluster numbers — and survive a
    // re-analysis through track_edits (re-anchored by time and position).
    M::up(
        "ALTER TABLE media ADD COLUMN faces TEXT NOT NULL DEFAULT 'pending';
    ALTER TABLE media ADD COLUMN faces_error TEXT;

    CREATE TABLE face_chunks (
        media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        start    INTEGER NOT NULL,
        end      INTEGER NOT NULL,
        PRIMARY KEY (media_id, start)
    ) STRICT;

    CREATE TABLE characters (
        id         TEXT PRIMARY KEY,
        number     INTEGER NOT NULL,    -- 'Character N' until named; never reused
        name       TEXT,
        hidden     INTEGER NOT NULL DEFAULT 0,
        cover      INTEGER,             -- track id; NULL = the best track
        created_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE tracks (
        id           INTEGER PRIMARY KEY,
        media_id     TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        start        INTEGER NOT NULL,
        end          INTEGER NOT NULL,
        shot         INTEGER NOT NULL,  -- start frame of its shot
        quality      REAL NOT NULL,
        face_h       REAL NOT NULL,
        dets         BLOB NOT NULL,     -- as_analysis::tracks::dets_to_bytes
        emb          BLOB NOT NULL,     -- 512 x f16
        character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
        assign       TEXT NOT NULL DEFAULT 'none',  -- none | auto | unsure | pinned
        score        REAL NOT NULL DEFAULT 0,
        not_face     INTEGER NOT NULL DEFAULT 0
    ) STRICT;
    CREATE INDEX tracks_by_media ON tracks(media_id, start);
    CREATE INDEX tracks_by_character ON tracks(character_id, assign);

    CREATE TABLE track_forbids (
        track_id     INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
        character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
        PRIMARY KEY (track_id, character_id)
    ) STRICT;

    CREATE TABLE track_edits (
        media_id     TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        start        INTEGER NOT NULL,
        end          INTEGER NOT NULL,
        dets         BLOB NOT NULL,
        kind         TEXT NOT NULL,     -- pin | forbid | not_face
        character_id TEXT REFERENCES characters(id) ON DELETE CASCADE
    ) STRICT;",
    ),
    // Shot sprites, made by the face pass with each chunk's tracks (the
    // files live next to the face thumbnails).
    M::up(
        "CREATE TABLE sprites (
        media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        shot     INTEGER NOT NULL,  -- start frame of the shot
        end      INTEGER NOT NULL,  -- and its end (part of the file name)
        cells    INTEGER NOT NULL,
        PRIMARY KEY (media_id, shot)
    ) STRICT;

    -- A re-analysis keeps every face's character in track_edits ('was'):
    -- the cast looks them up per character, the face pass per media.
    CREATE INDEX track_edits_by_character ON track_edits(character_id);
    CREATE INDEX track_edits_by_media ON track_edits(media_id);",
    ),
    // Which face embedder made a media's tracks: embeddings of two models
    // can't be compared, so tracks of another one are found again.
    M::up("ALTER TABLE media ADD COLUMN faces_model TEXT;"),
    // A small poster for small places (rail tiles, lists, cards).
    M::up("ALTER TABLE media ADD COLUMN poster_small TEXT;"),
    // Scenepacks: per character, the filters last used (PackSpec and
    // PackNaming JSON) and the shots picked or dropped by hand, keyed by the
    // shot's start frame. A pack is never stored as a list of clips: it is
    // made from the analysis each time, so it follows a new analysis.
    M::up(
        "CREATE TABLE packs (
        character_id TEXT PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
        spec         TEXT NOT NULL,
        naming       TEXT,
        updated_at   INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE pack_overrides (
        character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
        media_id     TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        shot         INTEGER NOT NULL,
        include      INTEGER NOT NULL,  -- 1 picked, 0 dropped
        updated_at   INTEGER NOT NULL,
        PRIMARY KEY (character_id, media_id, shot)
    ) STRICT;

    -- Everyone in a shot (who else is on screen with a character).
    CREATE INDEX tracks_by_shot ON tracks(media_id, shot);",
    ),
    // What an export is ("Walter · scenepack"), for the export history.
    M::up("ALTER TABLE exports ADD COLUMN label TEXT;"),
    // 10: a decision waiting for a new face pass remembers the face itself
    // (its embedding and the model that made it), so it only lands on a
    // face that looks like it; track ids continue from the highest ever.
    M::up(
        "ALTER TABLE track_edits ADD COLUMN emb BLOB;
         ALTER TABLE track_edits ADD COLUMN model TEXT;
         INSERT INTO meta (key, value)
           SELECT 'track_seq', CAST(COALESCE((SELECT MAX(id) FROM tracks), 0) AS TEXT)
            WHERE NOT EXISTS (SELECT 1 FROM meta WHERE key = 'track_seq');",
    ),
    // 11: an export can join its clips into one file (MergedFile JSON).
    M::up("ALTER TABLE exports ADD COLUMN merged TEXT;"),
    // 12: people followed by their bodies (an optional mode). Body tracks
    // are stored per checkpoint chunk like faces (a body_chunks row says
    // frames start..end are done), tied to the face tracks on them, with
    // how they look and how they show. Who they are is derived after every
    // clustering (character, how: face | clothes | conflict, similarity)
    // and never shapes a character; faces named by their body carry
    // via_body. The user's word on a body (pin, forbid, not a person) is
    // kept like a face's and waits in body_edits while the media is
    // analysed again.
    M::up(
        "ALTER TABLE media ADD COLUMN bodies TEXT NOT NULL DEFAULT 'pending';
    ALTER TABLE media ADD COLUMN bodies_error TEXT;
    ALTER TABLE tracks ADD COLUMN via_body INTEGER NOT NULL DEFAULT 0;

    CREATE TABLE body_chunks (
        media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        start    INTEGER NOT NULL,
        end      INTEGER NOT NULL,
        PRIMARY KEY (media_id, start)
    ) STRICT;

    CREATE TABLE bodies (
        id           INTEGER PRIMARY KEY,
        media_id     TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        start        INTEGER NOT NULL,
        end          INTEGER NOT NULL,
        shot         INTEGER NOT NULL,  -- start frame of its shot
        dets         BLOB NOT NULL,     -- as_analysis::bodies::dets_to_bytes
        reid         BLOB,              -- 256 x f16: how they look
        reid_away    BLOB,              -- ... while no face of theirs shows
        height       REAL NOT NULL,     -- as_analysis::bodies::Showing
        aspect       REAL NOT NULL,
        top          REAL NOT NULL,
        foreground   REAL NOT NULL,
        score        REAL NOT NULL,
        character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
        source       TEXT NOT NULL DEFAULT 'none',  -- none | face | clothes | conflict | pinned
        similarity   REAL NOT NULL DEFAULT 0,
        not_person   INTEGER NOT NULL DEFAULT 0
    ) STRICT;
    CREATE INDEX bodies_by_media ON bodies(media_id, shot);
    CREATE INDEX bodies_by_character ON bodies(character_id);

    CREATE TABLE body_faces (
        body_id  INTEGER NOT NULL REFERENCES bodies(id) ON DELETE CASCADE,
        track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
        samples  INTEGER NOT NULL,
        PRIMARY KEY (body_id, track_id)
    ) STRICT;
    CREATE INDEX body_faces_by_track ON body_faces(track_id);

    CREATE TABLE body_forbids (
        body_id      INTEGER NOT NULL REFERENCES bodies(id) ON DELETE CASCADE,
        character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
        PRIMARY KEY (body_id, character_id)
    ) STRICT;

    CREATE TABLE body_edits (
        media_id     TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
        start        INTEGER NOT NULL,
        end          INTEGER NOT NULL,
        dets         BLOB NOT NULL,
        kind         TEXT NOT NULL,     -- pin | forbid | not_person
        character_id TEXT REFERENCES characters(id) ON DELETE CASCADE
    ) STRICT;
    CREATE INDEX body_edits_by_media ON body_edits(media_id);
    CREATE INDEX body_edits_by_character ON body_edits(character_id);",
    ),
    // 13: how much another face takes of a track's portrait (the most any
    // covers of it, as a multiple of the track's own face; NULL: not looked
    // at yet): a character's cover shows them, not the person in front.
    M::up("ALTER TABLE tracks ADD COLUMN rival REAL;"),
    // 14: a character's faces (their cover, the cast's counts) and everyone
    // in their shots (a scenepack) are read from these indexes alone (a
    // chosen cover aside), not past the tracks' detections and embeddings.
    // Each keeps its order — the track id right after its key — so faces
    // that tie come out as before; the columns after it are the ones those
    // queries read. Any wider, `tracks_by_shot` would lose a media's tracks
    // to `tracks_by_media`, read in another order: what a re-analysis keeps
    // would be put back differently (see the cast's tests).
    M::up(
        "DROP INDEX IF EXISTS tracks_by_character;
    CREATE INDEX tracks_by_character ON tracks(character_id, assign, id, media_id, start, end, shot, quality,
                                               face_h, score, not_face, via_body, rival);
    DROP INDEX IF EXISTS tracks_by_shot;
    CREATE INDEX tracks_by_shot ON tracks(media_id, shot, id, start, end, quality, face_h, character_id, assign,
                                          not_face);",
    ),
];
const MIGRATIONS: Migrations<'static> = Migrations::from_slice(STEPS);

pub struct ProjectDb(pub(crate) Db);

/// A file being added to the project, before it is probed.
#[derive(Clone, Debug)]
pub struct NewMedia {
    pub id: MediaId,
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub mtime: Option<i64>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

/// What ingest learned about a media file.
#[derive(Clone, Debug)]
pub struct IngestRecord {
    pub info: MediaInfo,
    pub frames: u32,
    pub duration_sec: f64,
    pub fps: Rational,
    pub vfr: bool,
    pub interlaced: bool,
    pub proxy: ProxyState,
    pub poster: Option<String>,
    pub poster_small: Option<String>,
    pub index_blob: Vec<u8>,
    /// Audio track for playback and export (`None` = the file has no audio).
    pub audio_ordinal: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectStats {
    pub media_count: u32,
    pub ready_count: u32,
    pub duration_sec: f64,
    pub poster: Option<String>,
    pub poster_small: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewExportItem {
    pub media_id: MediaId,
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub name: String,
    /// Audio track at the time of export (`None` = no audio / file default).
    pub audio_ordinal: Option<u32>,
}

/// The columns `media_row` reads. `counted`: with the media's shots and
/// faces counted — a pass over every track of it (`NULL` without).
fn media_cols(counted: bool) -> String {
    let (shots, faces) = match counted {
        true => (
            "CASE WHEN shots = 'ready' THEN (SELECT COUNT(*) FROM segments s WHERE s.media_id = media.id AND s.kind = 'shot') END",
            "CASE WHEN faces = 'ready' THEN (SELECT COUNT(*) FROM tracks t WHERE t.media_id = media.id AND t.not_face = 0) END",
        ),
        false => ("NULL", "NULL"),
    };
    format!(
        "id, path, file_name, size_bytes, season, episode, status, error, info, frames, duration_sec, \
         fps_num, fps_den, vfr, interlaced, audio_ordinal, proxy, poster, added_at, shots, shots_error, {shots}, \
         faces, faces_error, {faces}, poster_small, bodies, bodies_error"
    )
}

/// Episode order (same as `as_project::naming::episode_cmp`): numbered episodes by
/// (season, episode), then the rest in the order they were added.
const MEDIA_ORDER: &str = "ORDER BY episode IS NULL, COALESCE(season, 0), episode, position, file_name COLLATE NOCASE";

fn parse_id<T: std::str::FromStr>(s: &str) -> DbResult<T>
where
    T::Err: std::fmt::Display,
{
    s.parse().map_err(|e: T::Err| DbError::Corrupt(e.to_string()))
}

fn media_row(r: &Row, position: u32) -> DbResult<MediaItem> {
    let season: Option<u32> = r.get(4)?;
    let episode: Option<u32> = r.get(5)?;
    let info: Option<String> = r.get(8)?;
    let fps = match (r.get::<_, Option<i64>>(11)?, r.get::<_, Option<i64>>(12)?) {
        (Some(n), Some(d)) if d != 0 => Some(Rational::new(n, d)),
        _ => None,
    };
    Ok(MediaItem {
        id: parse_id(&r.get::<_, String>(0)?)?,
        path: r.get(1)?,
        file_name: r.get(2)?,
        size_bytes: r.get::<_, i64>(3)? as f64,
        label: episode_label(season, episode),
        season,
        episode,
        position,
        status: enum_from_sql(&r.get::<_, String>(6)?)?,
        error: r.get(7)?,
        info: info.as_deref().map(json_from_sql).transpose()?,
        frames: r.get(9)?,
        duration_sec: r.get(10)?,
        fps,
        vfr: r.get(13)?,
        interlaced: r.get(14)?,
        audio_ordinal: r.get(15)?,
        proxy: enum_from_sql(&r.get::<_, String>(16)?)?,
        poster: r.get(17)?,
        poster_small: r.get(25)?,
        added_at: r.get(18)?,
        shots: enum_from_sql(&r.get::<_, String>(19)?)?,
        shots_error: r.get(20)?,
        shot_count: r.get(21)?,
        faces: enum_from_sql(&r.get::<_, String>(22)?)?,
        faces_error: r.get(23)?,
        face_count: r.get(24)?,
        bodies: enum_from_sql(&r.get::<_, String>(26)?)?,
        bodies_error: r.get(27)?,
    })
}

/// `S01E03`, `E12`, or empty.
pub fn episode_label(season: Option<u32>, episode: Option<u32>) -> String {
    match (season, episode) {
        (Some(s), Some(e)) => format!("S{s:02}E{e:02}"),
        (None, Some(e)) => format!("E{e:02}"),
        _ => String::new(),
    }
}

fn export_item_row(r: &Row) -> DbResult<ExportItem> {
    Ok(ExportItem {
        id: r.get(0)?,
        media_id: parse_id(&r.get::<_, String>(1)?)?,
        start: r.get(2)?,
        end: r.get(3)?,
        name: r.get(4)?,
        status: enum_from_sql(&r.get::<_, String>(5)?)?,
        path: r.get(6)?,
        verified: r.get(7)?,
        encoder: r.get(8)?,
        bytes: r.get::<_, Option<i64>>(9)?.map(|b| b as f64),
        error: r.get(10)?,
    })
}

fn clear_shots(tx: &rusqlite::Transaction, id: MediaId) -> DbResult<()> {
    tx.execute("DELETE FROM shot_signals WHERE media_id = ?1", [id.to_string()])?;
    tx.execute("DELETE FROM segments WHERE media_id = ?1", [id.to_string()])?;
    tx.execute("UPDATE media SET shots = 'pending', shots_error = NULL WHERE id = ?1", [id.to_string()])?;
    // Tracks belong to shots.
    crate::cast::clear_faces(tx, id)
}

impl ProjectDb {
    pub fn open(path: &Path) -> DbResult<Self> {
        Ok(Self(Db::open(path, &MIGRATIONS)?))
    }

    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self(Db::memory(&MIGRATIONS, None))
    }

    /// An empty database at schema `version`, as an older AutoScene left it.
    #[cfg(test)]
    pub(crate) fn memory_at(version: usize) -> Self {
        Self(Db::memory(&MIGRATIONS, Some(version)))
    }

    /// Bring a database of [`Self::memory_at`] up to date, as opening it does.
    #[cfg(test)]
    pub(crate) fn migrate(&self) {
        MIGRATIONS.to_latest(&mut self.0.conn.lock()).unwrap();
    }

    pub fn meta(&self, key: &str) -> DbResult<Option<String>> {
        self.0.read(|c| Ok(c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?))
    }

    pub fn delete_meta(&self, key: &str) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("DELETE FROM meta WHERE key = ?1", [key])?;
            Ok(())
        })
    }

    pub fn set_meta(&self, key: &str, value: &str) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [key, value],
            )?;
            Ok(())
        })
    }

    // -- media --------------------------------------------------------------

    /// Insert new media (status `queued`). Paths already in the project are skipped.
    /// Returns the ids actually inserted.
    pub fn add_media(&self, items: &[NewMedia]) -> DbResult<Vec<MediaId>> {
        self.0.write(|tx| {
            let mut pos: i64 = tx.query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM media", [], |r| r.get(0))?;
            let mut st = tx.prepare(
                "INSERT INTO media (id, path, file_name, size_bytes, mtime, season, episode, position, status, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued', ?9)
                 ON CONFLICT(path) DO NOTHING",
            )?;
            let now = now_ms();
            let mut added = Vec::new();
            for m in items {
                let n = st.execute(params![
                    m.id.to_string(),
                    m.path,
                    m.file_name,
                    m.size_bytes as i64,
                    m.mtime,
                    m.season,
                    m.episode,
                    pos,
                    now
                ])?;
                if n == 1 {
                    added.push(m.id);
                    pos += 1;
                }
            }
            Ok(added)
        })
    }

    pub fn media_list(&self) -> DbResult<Vec<MediaItem>> {
        self.media_rows(true)
    }

    /// [`Self::media_list`] without each media's shots and faces counted
    /// (`shot_count` and `face_count` are `None`): counting takes a pass over
    /// every track, and only what shows the counts needs it.
    pub fn media_list_uncounted(&self) -> DbResult<Vec<MediaItem>> {
        self.media_rows(false)
    }

    fn media_rows(&self, counted: bool) -> DbResult<Vec<MediaItem>> {
        self.0.read(|c| {
            let mut st = c.prepare(&format!("SELECT {} FROM media {MEDIA_ORDER}", media_cols(counted)))?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(media_row(r, out.len() as u32)?);
            }
            Ok(out)
        })
    }

    /// One media, as [`Self::media_list`] has it.
    pub fn media(&self, id: MediaId) -> DbResult<Option<MediaItem>> {
        self.0.read(|c| {
            // Its place in episode order: from the ids, not every media's row.
            let key = id.to_string();
            let mut st = c.prepare(&format!("SELECT id FROM media {MEDIA_ORDER}"))?;
            let mut rows = st.query([])?;
            let mut position = 0;
            loop {
                match rows.next()? {
                    Some(r) if r.get::<_, String>(0)? == key => break,
                    Some(_) => position += 1,
                    None => return Ok(None),
                }
            }
            let mut st = c.prepare(&format!("SELECT {} FROM media WHERE id = ?1", media_cols(true)))?;
            let mut rows = st.query([&key])?;
            rows.next()?.map(|r| media_row(r, position)).transpose()
        })
    }

    pub fn media_paths(&self) -> DbResult<HashSet<String>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT path FROM media")?;
            let paths = st.query_map([], |r| r.get(0))?.collect::<Result<HashSet<String>, _>>()?;
            Ok(paths)
        })
    }

    pub fn set_media_status(&self, id: MediaId, status: MediaStatus, error: Option<&str>) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE media SET status = ?2, error = ?3 WHERE id = ?1",
                params![id.to_string(), enum_to_sql(status), error],
            )?;
            Ok(())
        })
    }

    /// Store a finished ingest (and its frame index) atomically; status
    /// becomes `ready`. A frame index that numbers the frames differently
    /// starts the analysis over; the same frames (a file indexed again, or
    /// found again as another copy) keep it.
    pub fn record_ingest(&self, id: MediaId, rec: &IngestRecord) -> DbResult<()> {
        self.0.write(|tx| {
            let old: Option<Vec<u8>> = tx
                .query_row("SELECT data FROM frame_index WHERE media_id = ?1", [id.to_string()], |r| r.get(0))
                .optional()?;
            let decoded = |b: &[u8]| as_core::FrameIndex::from_bytes(b).ok();
            let same_frames = old.as_deref().and_then(decoded).is_some_and(|o| decoded(&rec.index_blob) == Some(o));
            tx.execute(
                "UPDATE media SET status = 'ready', error = NULL, info = ?2, frames = ?3, duration_sec = ?4,
                        fps_num = ?5, fps_den = ?6, vfr = ?7, interlaced = ?8, proxy = ?9, poster = ?10,
                        audio_ordinal = ?11, poster_small = ?12
                 WHERE id = ?1",
                params![
                    id.to_string(),
                    json_to_sql(&rec.info),
                    rec.frames,
                    rec.duration_sec,
                    rec.fps.num,
                    rec.fps.den,
                    rec.vfr,
                    rec.interlaced,
                    enum_to_sql(rec.proxy),
                    rec.poster,
                    rec.audio_ordinal,
                    rec.poster_small,
                ],
            )?;
            tx.execute(
                "INSERT INTO frame_index (media_id, data) VALUES (?1, ?2)
                 ON CONFLICT(media_id) DO UPDATE SET data = excluded.data",
                params![id.to_string(), rec.index_blob],
            )?;
            if !same_frames {
                clear_shots(tx, id)?;
            }
            Ok(())
        })
    }

    /// The media's file is at `path` now (found again after it moved).
    pub fn relink_media(
        &self,
        id: MediaId,
        path: &str,
        file_name: &str,
        size_bytes: u64,
        mtime: Option<i64>,
    ) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE media SET path = ?2, file_name = ?3, size_bytes = ?4, mtime = ?5 WHERE id = ?1",
                params![id.to_string(), path, file_name, size_bytes as i64, mtime],
            )?;
            Ok(())
        })
    }

    pub fn frame_index_blob(&self, id: MediaId) -> DbResult<Option<Vec<u8>>> {
        self.0.read(|c| {
            Ok(c.query_row("SELECT data FROM frame_index WHERE media_id = ?1", [id.to_string()], |r| r.get(0))
                .optional()?)
        })
    }

    /// Point a media at newly made posters. `false` when the media is gone.
    pub fn set_posters(&self, id: MediaId, poster: &str, poster_small: &str) -> DbResult<bool> {
        self.0.write(|tx| {
            let n = tx.execute(
                "UPDATE media SET poster = ?2, poster_small = ?3 WHERE id = ?1",
                params![id.to_string(), poster, poster_small],
            )?;
            Ok(n > 0)
        })
    }

    pub fn set_proxy(&self, id: MediaId, state: ProxyState, path: Option<&str>, t0: Option<f64>) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE media SET proxy = ?2, proxy_path = ?3, proxy_t0 = ?4 WHERE id = ?1",
                params![id.to_string(), enum_to_sql(state), path, t0],
            )?;
            Ok(())
        })
    }

    /// `(proxy_path, proxy_t0)` once a proxy is ready.
    pub fn proxy(&self, id: MediaId) -> DbResult<Option<(String, f64)>> {
        self.0.read(|c| {
            let row: Option<(Option<String>, Option<f64>)> = c
                .query_row(
                    "SELECT proxy_path, proxy_t0 FROM media WHERE id = ?1 AND proxy = 'ready'",
                    [id.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            Ok(row.and_then(|(p, t)| Some((p?, t?))))
        })
    }

    pub fn set_audio(&self, id: MediaId, ordinal: Option<u32>) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("UPDATE media SET audio_ordinal = ?2 WHERE id = ?1", params![id.to_string(), ordinal])?;
            Ok(())
        })
    }

    pub fn remove_media(&self, id: MediaId) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("DELETE FROM media WHERE id = ?1", [id.to_string()])?;
            Ok(())
        })
    }

    // -- shots --------------------------------------------------------------

    pub fn set_shots_state(&self, id: MediaId, state: StageState, error: Option<&str>) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE media SET shots = ?2, shots_error = ?3 WHERE id = ?1",
                params![id.to_string(), enum_to_sql(state), error],
            )?;
            Ok(())
        })
    }

    /// Checkpointed signal chunks `(start, data)`, in frame order.
    pub fn shot_chunks(&self, id: MediaId) -> DbResult<Vec<(FrameIdx, Vec<u8>)>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT start, data FROM shot_signals WHERE media_id = ?1 ORDER BY start")?;
            let rows = st.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    pub fn put_shot_chunk(&self, id: MediaId, start: FrameIdx, data: &[u8]) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "INSERT INTO shot_signals (media_id, start, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT(media_id, start) DO UPDATE SET data = excluded.data",
                params![id.to_string(), start, data],
            )?;
            Ok(())
        })
    }

    /// Drop checkpoints from `start` on (unreadable or out of order).
    pub fn drop_shot_chunks_from(&self, id: MediaId, start: FrameIdx) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("DELETE FROM shot_signals WHERE media_id = ?1 AND start >= ?2", params![id.to_string(), start])?;
            Ok(())
        })
    }

    /// Replace the media's segments and mark its shots ready, atomically.
    pub fn store_segments(&self, id: MediaId, segments: &[Segment]) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("DELETE FROM segments WHERE media_id = ?1", [id.to_string()])?;
            let mut st =
                tx.prepare("INSERT INTO segments (media_id, start, end, kind, tiny) VALUES (?1, ?2, ?3, ?4, ?5)")?;
            for seg in segments {
                st.execute(params![id.to_string(), seg.start, seg.end, enum_to_sql(seg.kind), seg.tiny])?;
            }
            tx.execute("UPDATE media SET shots = 'ready', shots_error = NULL WHERE id = ?1", [id.to_string()])?;
            Ok(())
        })
    }

    pub fn segments(&self, id: MediaId) -> DbResult<Vec<Segment>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT start, end, kind, tiny FROM segments WHERE media_id = ?1 ORDER BY start")?;
            let mut rows = st.query([id.to_string()])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(Segment {
                    start: r.get(0)?,
                    end: r.get(1)?,
                    kind: enum_from_sql(&r.get::<_, String>(2)?)?,
                    tiny: r.get(3)?,
                });
            }
            Ok(out)
        })
    }

    /// Forget a media's shot analysis (signals and segments).
    /// Analysis stages of a media that failed go back to pending (its file
    /// was found again: what failed for want of it can run now).
    pub fn retry_failed_analysis(&self, id: MediaId) -> DbResult<()> {
        self.0.write(|tx| {
            let m = id.to_string();
            tx.execute(
                "UPDATE media SET shots = 'pending', shots_error = NULL WHERE id = ?1 AND shots = 'failed'",
                [&m],
            )?;
            tx.execute(
                "UPDATE media SET faces = 'pending', faces_error = NULL WHERE id = ?1 AND faces = 'failed'",
                [&m],
            )?;
            tx.execute(
                "UPDATE media SET bodies = 'pending', bodies_error = NULL WHERE id = ?1 AND bodies = 'failed'",
                [&m],
            )?;
            Ok(())
        })
    }

    pub fn reset_shots(&self, id: MediaId) -> DbResult<()> {
        self.0.write(|tx| clear_shots(tx, id))
    }

    pub fn stats(&self) -> DbResult<ProjectStats> {
        let media = self.media_list_uncounted()?;
        let ready: Vec<_> = media.iter().filter(|m| m.status == MediaStatus::Ready).collect();
        let pictured = media.iter().find(|m| m.poster.is_some());
        Ok(ProjectStats {
            media_count: media.len() as u32,
            ready_count: ready.len() as u32,
            duration_sec: ready.iter().filter_map(|m| m.duration_sec).sum(),
            poster: pictured.and_then(|m| m.poster.clone()),
            poster_small: pictured.and_then(|m| m.poster_small.clone()),
        })
    }

    // -- exports ------------------------------------------------------------

    pub fn insert_export(
        &self,
        id: ExportId,
        preset: &ExportPreset,
        out_dir: &str,
        label: Option<&str>,
        merged: Option<&MergedFile>,
        items: &[NewExportItem],
    ) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "INSERT INTO exports (id, created_at, preset, out_dir, status, label, merged)
                 VALUES (?1, ?2, ?3, ?4, 'queued', ?5, ?6)",
                params![id.to_string(), now_ms(), json_to_sql(preset), out_dir, label, merged.map(json_to_sql)],
            )?;
            let mut st = tx.prepare(
                "INSERT INTO export_items (export_id, media_id, start_frame, end_frame, name, status, audio_ordinal)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6)",
            )?;
            for it in items {
                st.execute(params![
                    id.to_string(),
                    it.media_id.to_string(),
                    it.start,
                    it.end,
                    it.name,
                    it.audio_ordinal
                ])?;
            }
            Ok(())
        })
    }

    pub fn set_export_status(&self, id: ExportId, status: ExportStatus) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("UPDATE exports SET status = ?2 WHERE id = ?1", params![id.to_string(), enum_to_sql(status)])?;
            Ok(())
        })
    }

    /// Audio track frozen into an export item when it was queued.
    pub fn export_item_audio(&self, item_id: i64) -> DbResult<Option<u32>> {
        self.0
            .read(|c| Ok(c.query_row("SELECT audio_ordinal FROM export_items WHERE id = ?1", [item_id], |r| r.get(0))?))
    }

    pub fn update_export_item(
        &self,
        item_id: i64,
        status: ExportStatus,
        outcome: Option<&ClipOutcome>,
        error: Option<&str>,
    ) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE export_items SET status = ?2, path = ?3, verified = ?4, encoder = ?5, bytes = ?6, error = ?7
                 WHERE id = ?1",
                params![
                    item_id,
                    enum_to_sql(status),
                    outcome.map(|o| o.path.as_str()),
                    outcome.is_some_and(|o| o.verified),
                    outcome.map(|o| o.encoder.as_str()),
                    outcome.map(|o| o.bytes as i64),
                    error,
                ],
            )?;
            Ok(())
        })
    }

    /// A clip of an export that joins them into one file: rendered (or not)
    /// as a part of it, which has no path or size of its own.
    pub fn update_export_part(
        &self,
        item_id: i64,
        status: ExportStatus,
        outcome: Option<&ClipOutcome>,
        error: Option<&str>,
    ) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE export_items SET status = ?2, path = NULL, verified = ?3, encoder = ?4, bytes = NULL, error = ?5
                 WHERE id = ?1",
                params![
                    item_id,
                    enum_to_sql(status),
                    outcome.is_some_and(|o| o.verified),
                    outcome.map(|o| o.encoder.as_str()),
                    error,
                ],
            )?;
            Ok(())
        })
    }

    /// The joined file of an export, and the clips that are in it (their
    /// `path` is the file's).
    pub fn set_export_merged(&self, id: ExportId, merged: &MergedFile, in_file: &[i64]) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("UPDATE exports SET merged = ?2 WHERE id = ?1", params![id.to_string(), json_to_sql(merged)])?;
            let mut st = tx.prepare("UPDATE export_items SET path = ?2 WHERE id = ?1")?;
            for item in in_file {
                st.execute(params![item, merged.path])?;
            }
            Ok(())
        })
    }

    /// Newest first.
    pub fn exports(&self, project_id: ProjectId) -> DbResult<Vec<ExportSummary>> {
        let ids: Vec<String> = self.0.read(|c| {
            let mut st = c.prepare("SELECT id FROM exports ORDER BY created_at DESC, id DESC")?;
            let ids = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok(ids)
        })?;
        ids.iter().filter_map(|id| self.export(project_id, parse_id(id).ok()?).transpose()).collect()
    }

    pub fn export(&self, project_id: ProjectId, id: ExportId) -> DbResult<Option<ExportSummary>> {
        self.0.read(|c| {
            type Head = (i64, String, String, String, Option<String>, Option<String>);
            let head: Option<Head> = c
                .query_row(
                    "SELECT created_at, preset, out_dir, status, label, merged FROM exports WHERE id = ?1",
                    [id.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .optional()?;
            let Some((created_at, preset, out_dir, status, label, merged)) = head else { return Ok(None) };
            let mut st = c.prepare(
                "SELECT id, media_id, start_frame, end_frame, name, status, path, verified, encoder, bytes, error
                 FROM export_items WHERE export_id = ?1 ORDER BY id",
            )?;
            let mut rows = st.query([id.to_string()])?;
            let mut items = Vec::new();
            while let Some(r) = rows.next()? {
                items.push(export_item_row(r)?);
            }
            Ok(Some(ExportSummary {
                id,
                project_id,
                created_at,
                preset: json_from_sql(&preset)?,
                out_dir,
                status: enum_from_sql(&status)?,
                items,
                label,
                merged: merged.as_deref().map(json_from_sql).transpose()?,
            }))
        })
    }

    /// Exports left `queued`/`running` by a crash or quit can't resume (M1);
    /// mark them cancelled so the UI doesn't show them as live. The clips an
    /// unfinished file was made of are in no file either.
    pub fn close_stale_exports(&self) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE export_items SET status = 'cancelled'
                 WHERE status = 'done' AND export_id IN
                   (SELECT id FROM exports WHERE status IN ('queued', 'running') AND merged IS NOT NULL)",
                [],
            )?;
            tx.execute("UPDATE export_items SET status = 'cancelled' WHERE status IN ('queued', 'running')", [])?;
            tx.execute("UPDATE exports SET status = 'cancelled' WHERE status IN ('queued', 'running')", [])?;
            Ok(())
        })
    }

    /// Every export that joins its clips into one file, with its folder
    /// (where its parts are rendered).
    pub fn merged_exports(&self) -> DbResult<Vec<(ExportId, String)>> {
        self.0.read(|c| {
            let mut st = c.prepare("SELECT id, out_dir FROM exports WHERE merged IS NOT NULL")?;
            let rows = st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows.into_iter().map(|(id, dir)| Ok((parse_id(&id)?, dir))).collect()
        })
    }

    /// Media left mid-ingest by a crash or quit go back to the queue.
    pub fn requeue_interrupted(&self) -> DbResult<Vec<MediaId>> {
        self.0.write(|tx| {
            tx.execute("UPDATE media SET status = 'queued' WHERE status = 'indexing'", [])?;
            tx.execute("UPDATE media SET proxy = 'pending' WHERE proxy = 'building'", [])?;
            let mut st = tx.prepare("SELECT id FROM media WHERE status = 'queued' ORDER BY position")?;
            let ids = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            ids.iter().map(|s| parse_id(s)).collect()
        })
    }

    /// Indexed media whose shot analysis hasn't finished (an interrupted run
    /// counts as pending again), in episode order.
    pub fn pending_shots(&self) -> DbResult<Vec<MediaId>> {
        self.0.write(|tx| {
            tx.execute("UPDATE media SET shots = 'pending' WHERE shots = 'running'", [])?;
            let mut st = tx
                .prepare(&format!("SELECT id FROM media WHERE status = 'ready' AND shots = 'pending' {MEDIA_ORDER}"))?;
            let ids = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            ids.iter().map(|s| parse_id(s)).collect()
        })
    }

    /// Ready media whose proxy still has to be built.
    pub fn pending_proxies(&self) -> DbResult<Vec<MediaId>> {
        self.0.read(|c| {
            let mut st =
                c.prepare("SELECT id FROM media WHERE status = 'ready' AND proxy = 'pending' ORDER BY position")?;
            let ids = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            ids.iter().map(|s| parse_id(s)).collect()
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use as_api::{AudioStream, HdrKind, VideoStream};

    pub(crate) fn new_media(path: &str, season: Option<u32>, episode: Option<u32>) -> NewMedia {
        NewMedia {
            id: MediaId::new(),
            path: path.into(),
            file_name: path.rsplit('/').next().unwrap().into(),
            size_bytes: 1000,
            mtime: None,
            season,
            episode,
        }
    }

    pub(crate) fn info() -> MediaInfo {
        MediaInfo {
            container: "matroska,webm".into(),
            duration_sec: Some(10.0),
            start_sec: 0.0,
            size_bytes: Some(1000.0),
            bit_rate: None,
            video: VideoStream {
                index: 0,
                codec: "h264".into(),
                profile: None,
                pix_fmt: "yuv420p".into(),
                bit_depth: 8,
                width: 1920,
                height: 1080,
                sar: Rational::new(1, 1),
                r_frame_rate: Rational::new(24000, 1001),
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
            audio: vec![AudioStream {
                index: 1,
                ordinal: 0,
                codec: "aac".into(),
                profile: None,
                channels: 2,
                channel_layout: None,
                sample_rate: 48000,
                language: Some("jpn".into()),
                title: None,
                default: true,
                commentary: false,
            }],
            subtitles: vec![],
        }
    }

    #[test]
    fn media_lifecycle_and_episode_order() {
        let db = ProjectDb::memory();
        let e2 = new_media("/s/Show - 02.mkv", None, Some(2));
        let e1 = new_media("/s/Show - 01.mkv", None, Some(1));
        let extra = new_media("/s/Show - NCOP.mkv", None, None);
        let added = db.add_media(&[e2.clone(), extra.clone(), e1.clone()]).unwrap();
        assert_eq!(added.len(), 3);
        // Same path again is ignored.
        assert!(db.add_media(&[new_media("/s/Show - 01.mkv", None, Some(1))]).unwrap().is_empty());

        let list = db.media_list().unwrap();
        let order: Vec<_> = list.iter().map(|m| (m.id, m.label.as_str(), m.position)).collect();
        assert_eq!(order, vec![(e1.id, "E01", 0), (e2.id, "E02", 1), (extra.id, "", 2)]);
        assert!(list.iter().all(|m| m.status == MediaStatus::Queued && m.proxy == ProxyState::Unknown));
        // One of them by its id: in its place too.
        for m in &list {
            assert_eq!(db.media(m.id).unwrap().unwrap().position, m.position);
        }
        assert!(db.media(MediaId::new()).unwrap().is_none());

        db.set_media_status(e1.id, MediaStatus::Indexing, None).unwrap();
        let rec = IngestRecord {
            info: info(),
            frames: 240,
            duration_sec: 10.01,
            fps: Rational::new(24000, 1001),
            vfr: false,
            interlaced: false,
            proxy: ProxyState::Pending,
            poster: Some("/thumbs/e1.jpg".into()),
            poster_small: Some("/thumbs/e1-small.jpg".into()),
            index_blob: vec![1, 2, 3],
            audio_ordinal: Some(0),
        };
        db.record_ingest(e1.id, &rec).unwrap();
        let m = db.media(e1.id).unwrap().unwrap();
        assert_eq!(m.status, MediaStatus::Ready);
        assert_eq!(m.frames, Some(240));
        assert_eq!(m.fps, Some(Rational::new(24000, 1001)));
        assert_eq!(m.info.unwrap().audio[0].language.as_deref(), Some("jpn"));
        assert_eq!(m.audio_ordinal, Some(0), "ingest records the chosen track");
        assert_eq!(db.frame_index_blob(e1.id).unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(db.pending_proxies().unwrap(), vec![e1.id]);

        db.set_proxy(e1.id, ProxyState::Ready, Some("/proxies/e1.mp4"), Some(0.0)).unwrap();
        assert_eq!(db.proxy(e1.id).unwrap(), Some(("/proxies/e1.mp4".into(), 0.0)));
        assert!(db.proxy(e2.id).unwrap().is_none());

        let stats = db.stats().unwrap();
        assert_eq!(
            stats,
            ProjectStats {
                media_count: 3,
                ready_count: 1,
                duration_sec: 10.01,
                poster: Some("/thumbs/e1.jpg".into()),
                poster_small: Some("/thumbs/e1-small.jpg".into()),
            }
        );

        // Posters made again at new sizes.
        assert!(db.set_posters(e1.id, "/thumbs/e1-720.jpg", "/thumbs/e1-270.jpg").unwrap());
        let m = db.media(e1.id).unwrap().unwrap();
        assert_eq!(
            (m.poster.as_deref(), m.poster_small.as_deref()),
            (Some("/thumbs/e1-720.jpg"), Some("/thumbs/e1-270.jpg"))
        );

        // A crash mid-ingest: back to the queue on next open.
        db.set_media_status(e2.id, MediaStatus::Indexing, None).unwrap();
        assert_eq!(db.requeue_interrupted().unwrap(), vec![e2.id, extra.id]);

        db.remove_media(e1.id).unwrap();
        assert!(db.frame_index_blob(e1.id).unwrap().is_none(), "frame index cascades");
        assert!(!db.set_posters(e1.id, "/a.jpg", "/b.jpg").unwrap(), "a removed media gets no posters");
    }

    #[test]
    fn shot_checkpoints_and_segments() {
        use as_api::SegmentKind;
        let db = ProjectDb::memory();
        let m = new_media("/s/Show - 01.mkv", None, Some(1));
        db.add_media(std::slice::from_ref(&m)).unwrap();
        let rec = IngestRecord {
            info: info(),
            frames: 240,
            duration_sec: 10.0,
            fps: Rational::new(24, 1),
            vfr: false,
            interlaced: false,
            proxy: ProxyState::NotNeeded,
            poster: None,
            poster_small: None,
            index_blob: vec![1],
            audio_ordinal: None,
        };
        db.record_ingest(m.id, &rec).unwrap();
        let item = db.media(m.id).unwrap().unwrap();
        assert_eq!((item.shots, item.shot_count), (StageState::Pending, None));

        db.set_shots_state(m.id, StageState::Running, None).unwrap();
        db.put_shot_chunk(m.id, 100, &[2]).unwrap();
        db.put_shot_chunk(m.id, 0, &[1]).unwrap();
        db.put_shot_chunk(m.id, 200, &[3]).unwrap();
        assert_eq!(db.shot_chunks(m.id).unwrap(), vec![(0, vec![1]), (100, vec![2]), (200, vec![3])]);
        db.drop_shot_chunks_from(m.id, 200).unwrap();
        assert_eq!(db.shot_chunks(m.id).unwrap().len(), 2);

        let segs = [
            Segment { start: 0, end: 100, kind: SegmentKind::Shot, tiny: false },
            Segment { start: 100, end: 112, kind: SegmentKind::Dissolve, tiny: false },
            Segment { start: 112, end: 114, kind: SegmentKind::Shot, tiny: true },
            Segment { start: 114, end: 240, kind: SegmentKind::Shot, tiny: false },
        ];
        db.store_segments(m.id, &segs).unwrap();
        assert_eq!(db.segments(m.id).unwrap(), segs);
        let item = db.media(m.id).unwrap().unwrap();
        assert_eq!((item.shots, item.shot_count), (StageState::Ready, Some(3)));
        // Not counted: the same media, without its count.
        let quick = db.media_list_uncounted().unwrap().remove(0);
        assert_eq!((quick.id, quick.shots, quick.shot_count, quick.frames), (m.id, StageState::Ready, None, Some(240)));

        // Re-indexing starts the analysis over.
        db.record_ingest(m.id, &rec).unwrap();
        assert!(db.shot_chunks(m.id).unwrap().is_empty() && db.segments(m.id).unwrap().is_empty());
        assert_eq!(db.media(m.id).unwrap().unwrap().shots, StageState::Pending);
    }

    #[test]
    fn uncounted_media_lack_only_their_counts() {
        use crate::cast::tests::{ready_media, track};
        use as_api::SegmentKind;
        let db = ProjectDb::memory();
        let a = ready_media(&db, "/s/Show - 01.mkv");
        ready_media(&db, "/s/Show - 02.mkv");
        let shot = |start, end| Segment { start, end, kind: SegmentKind::Shot, tiny: false };
        db.store_segments(a, &[shot(0, 500), shot(500, 1000)]).unwrap();
        db.put_face_chunk(a, 0, 1000, &[track(10, 0.9), track(600, 0.5)], &[], |_| Ok(())).unwrap();
        db.set_faces_state(a, StageState::Ready, None).unwrap();
        assert!(db.set_posters(a, "/thumbs/a-720.jpg", "/thumbs/a-480x270.jpg").unwrap());

        let counted = db.media_list().unwrap();
        let counts: Vec<_> = counted.iter().map(|m| (m.id == a, m.shot_count, m.face_count)).collect();
        assert_eq!(counts, [(true, Some(2), Some(2)), (false, Some(0), None)]);
        // The same rows, in the same order, but for the counts.
        let rows = |list: Vec<MediaItem>| -> Vec<serde_json::Value> {
            let uncounted = |m| MediaItem { shot_count: None, face_count: None, ..m };
            list.into_iter().map(|m| serde_json::to_value(uncounted(m)).unwrap()).collect()
        };
        let quick = db.media_list_uncounted().unwrap();
        assert!(quick.iter().all(|m| m.shot_count.is_none() && m.face_count.is_none()));
        assert_eq!(rows(quick), rows(counted));
        // The library's card, from them.
        assert_eq!(
            db.stats().unwrap(),
            ProjectStats {
                media_count: 2,
                ready_count: 2,
                duration_sec: 83.4,
                poster: Some("/thumbs/a-720.jpg".into()),
                poster_small: Some("/thumbs/a-480x270.jpg".into()),
            }
        );
    }

    #[test]
    fn exports_round_trip() {
        let db = ProjectDb::memory();
        let m = new_media("/a.mp4", None, None);
        db.add_media(std::slice::from_ref(&m)).unwrap();
        let preset = as_export::builtin_presets().remove(0);
        let eid = ExportId::new();
        let items = [
            NewExportItem { media_id: m.id, start: 10, end: 34, name: "a_1".into(), audio_ordinal: Some(1) },
            NewExportItem { media_id: m.id, start: 50, end: 60, name: "a_2".into(), audio_ordinal: None },
        ];
        db.insert_export(eid, &preset, "/out", Some("Walter · scenepack"), None, &items).unwrap();
        let pid = ProjectId::new();
        let ex = db.export(pid, eid).unwrap().unwrap();
        assert_eq!(ex.items.len(), 2);
        assert_eq!(ex.preset, preset);
        assert_eq!(ex.label.as_deref(), Some("Walter · scenepack"));
        assert_eq!(db.export_item_audio(ex.items[0].id).unwrap(), Some(1), "audio track is frozen per item");
        assert_eq!(ex.status, ExportStatus::Queued);

        let outcome = ClipOutcome {
            path: "/out/a_1.mov".into(),
            frames: 24,
            verified: true,
            encoder: "prores_ks".into(),
            bytes: 9e6,
        };
        db.update_export_item(ex.items[0].id, ExportStatus::Done, Some(&outcome), None).unwrap();
        db.update_export_item(ex.items[1].id, ExportStatus::Failed, None, Some("disk full")).unwrap();
        db.set_export_status(eid, ExportStatus::Failed).unwrap();

        let ex = &db.exports(pid).unwrap()[0];
        assert!(ex.items[0].verified);
        assert_eq!(ex.items[0].path.as_deref(), Some("/out/a_1.mov"));
        assert_eq!(ex.items[1].error.as_deref(), Some("disk full"));
        assert_eq!(ex.status, ExportStatus::Failed);
        assert_eq!(ex.merged, None);
    }

    #[test]
    fn merged_exports_round_trip() {
        let db = ProjectDb::memory();
        let m = new_media("/a.mp4", None, None);
        db.add_media(std::slice::from_ref(&m)).unwrap();
        let preset = as_export::builtin_presets().remove(0);
        let eid = ExportId::new();
        let items = [
            NewExportItem { media_id: m.id, start: 10, end: 34, name: "a_1".into(), audio_ordinal: None },
            NewExportItem { media_id: m.id, start: 50, end: 60, name: "a_2".into(), audio_ordinal: None },
        ];
        let file =
            MergedFile { name: "Show_Walter_scenepack".into(), path: None, verified: false, bytes: None, error: None };
        db.insert_export(eid, &preset, "/out", None, Some(&file), &items).unwrap();
        let pid = ProjectId::new();
        let ex = db.export(pid, eid).unwrap().unwrap();
        assert_eq!(ex.merged.as_ref(), Some(&file));

        let part = ClipOutcome {
            path: "/out/.autoscene-partial/x/part-00001.mov".into(),
            frames: 24,
            verified: true,
            encoder: "prores_ks".into(),
            bytes: 9e6,
        };
        db.update_export_part(ex.items[0].id, ExportStatus::Done, Some(&part), None).unwrap();
        db.update_export_part(ex.items[1].id, ExportStatus::Failed, None, Some("gone")).unwrap();
        let ex = db.export(pid, eid).unwrap().unwrap();
        assert_eq!((ex.items[0].path.as_deref(), ex.items[0].bytes), (None, None), "a part isn't a file of its own");
        assert!(ex.items[0].verified);

        let done = MergedFile {
            path: Some("/out/Show_Walter_scenepack.mov".into()),
            verified: true,
            bytes: Some(1e7),
            ..file
        };
        db.set_export_merged(eid, &done, &[ex.items[0].id]).unwrap();
        let ex = db.export(pid, eid).unwrap().unwrap();
        assert_eq!(ex.merged.as_ref(), Some(&done));
        assert_eq!(ex.items[0].path, done.path, "a clip in the file points at it");
        assert_eq!(ex.items[1].path, None);
    }

    #[test]
    fn stale_exports_are_closed() {
        let db = ProjectDb::memory();
        let m = new_media("/a.mp4", None, None);
        db.add_media(std::slice::from_ref(&m)).unwrap();
        let eid = ExportId::new();
        let preset = as_export::builtin_presets().remove(0);
        let item = [NewExportItem { media_id: m.id, start: 0, end: 5, name: "x".into(), audio_ordinal: None }];
        db.insert_export(eid, &preset, "/out", None, None, &item).unwrap();
        // A file of two clips, one of them rendered already.
        let joining = ExportId::new();
        let file = MergedFile { name: "x".into(), path: None, verified: false, bytes: None, error: None };
        let two = [item[0].clone(), item[0].clone()];
        db.insert_export(joining, &preset, "/joined", None, Some(&file), &two).unwrap();
        let pid = ProjectId::new();
        let rendered = db.export(pid, joining).unwrap().unwrap().items[0].id;
        db.update_export_part(rendered, ExportStatus::Done, None, None).unwrap();
        db.close_stale_exports().unwrap();
        let ex = db.export(pid, eid).unwrap().unwrap();
        assert_eq!(ex.status, ExportStatus::Cancelled);
        assert_eq!(ex.items[0].status, ExportStatus::Cancelled);
        let ex = db.export(pid, joining).unwrap().unwrap();
        assert_eq!(ex.status, ExportStatus::Cancelled);
        assert!(ex.items.iter().all(|i| i.status == ExportStatus::Cancelled), "no clip is in a file that wasn't made");
        assert_eq!(db.merged_exports().unwrap(), vec![(joining, "/joined".to_string())]);
    }

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
    }
}
