//! `project.db`: one per `<Title>.autoscene` folder.

use std::collections::HashSet;
use std::path::Path;

use as_api::{
    ClipOutcome, ExportItem, ExportPreset, ExportStatus, ExportSummary, MediaInfo, MediaItem, MediaStatus, ProxyState,
    now_ms,
};
use as_core::{ExportId, FrameIdx, MediaId, ProjectId, Rational};
use rusqlite::{OptionalExtension, Row, params};
use rusqlite_migration::{M, Migrations};

use crate::{Db, DbError, DbResult, enum_from_sql, enum_to_sql, json_from_sql, json_to_sql};

/// Append-only: never edit a released migration, add a new one.
const STEPS: &[M<'static>] = &[M::up(
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
)];
const MIGRATIONS: Migrations<'static> = Migrations::from_slice(STEPS);

pub struct ProjectDb(Db);

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
    pub index_blob: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectStats {
    pub media_count: u32,
    pub ready_count: u32,
    pub duration_sec: f64,
    pub poster: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NewExportItem {
    pub media_id: MediaId,
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub name: String,
}

const MEDIA_COLS: &str = "id, path, file_name, size_bytes, season, episode, status, error, info, frames, duration_sec, \
     fps_num, fps_den, vfr, interlaced, audio_ordinal, proxy, poster, added_at";

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
        added_at: r.get(18)?,
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

impl ProjectDb {
    pub fn open(path: &Path) -> DbResult<Self> {
        Ok(Self(Db::open(path, &MIGRATIONS)?))
    }

    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self(Db::memory(&MIGRATIONS))
    }

    pub fn meta(&self, key: &str) -> DbResult<Option<String>> {
        self.0.read(|c| Ok(c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?))
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
        self.0.read(|c| {
            let mut st = c.prepare(&format!("SELECT {MEDIA_COLS} FROM media {MEDIA_ORDER}"))?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(media_row(r, out.len() as u32)?);
            }
            Ok(out)
        })
    }

    pub fn media(&self, id: MediaId) -> DbResult<Option<MediaItem>> {
        Ok(self.media_list()?.into_iter().find(|m| m.id == id))
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

    /// Store a finished ingest (and its frame index) atomically; status becomes `ready`.
    pub fn record_ingest(&self, id: MediaId, rec: &IngestRecord) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE media SET status = 'ready', error = NULL, info = ?2, frames = ?3, duration_sec = ?4,
                        fps_num = ?5, fps_den = ?6, vfr = ?7, interlaced = ?8, proxy = ?9, poster = ?10
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
                ],
            )?;
            tx.execute(
                "INSERT INTO frame_index (media_id, data) VALUES (?1, ?2)
                 ON CONFLICT(media_id) DO UPDATE SET data = excluded.data",
                params![id.to_string(), rec.index_blob],
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

    pub fn stats(&self) -> DbResult<ProjectStats> {
        let media = self.media_list()?;
        let ready: Vec<_> = media.iter().filter(|m| m.status == MediaStatus::Ready).collect();
        Ok(ProjectStats {
            media_count: media.len() as u32,
            ready_count: ready.len() as u32,
            duration_sec: ready.iter().filter_map(|m| m.duration_sec).sum(),
            poster: media.iter().find_map(|m| m.poster.clone()),
        })
    }

    // -- exports ------------------------------------------------------------

    pub fn insert_export(
        &self,
        id: ExportId,
        preset: &ExportPreset,
        out_dir: &str,
        items: &[NewExportItem],
    ) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "INSERT INTO exports (id, created_at, preset, out_dir, status) VALUES (?1, ?2, ?3, ?4, 'queued')",
                params![id.to_string(), now_ms(), json_to_sql(preset), out_dir],
            )?;
            let mut st = tx.prepare(
                "INSERT INTO export_items (export_id, media_id, start_frame, end_frame, name, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'queued')",
            )?;
            for it in items {
                st.execute(params![id.to_string(), it.media_id.to_string(), it.start, it.end, it.name])?;
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
            let head: Option<(i64, String, String, String)> = c
                .query_row(
                    "SELECT created_at, preset, out_dir, status FROM exports WHERE id = ?1",
                    [id.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((created_at, preset, out_dir, status)) = head else { return Ok(None) };
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
            }))
        })
    }

    /// Exports left `queued`/`running` by a crash or quit can't resume (M1);
    /// mark them cancelled so the UI doesn't show them as live.
    pub fn close_stale_exports(&self) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("UPDATE export_items SET status = 'cancelled' WHERE status IN ('queued', 'running')", [])?;
            tx.execute("UPDATE exports SET status = 'cancelled' WHERE status IN ('queued', 'running')", [])?;
            Ok(())
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
mod tests {
    use super::*;
    use as_api::{AudioStream, HdrKind, VideoStream};

    fn new_media(path: &str, season: Option<u32>, episode: Option<u32>) -> NewMedia {
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

    fn info() -> MediaInfo {
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
                channels: 2,
                channel_layout: None,
                sample_rate: 48000,
                language: Some("jpn".into()),
                title: None,
                default: true,
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
            index_blob: vec![1, 2, 3],
        };
        db.record_ingest(e1.id, &rec).unwrap();
        let m = db.media(e1.id).unwrap().unwrap();
        assert_eq!(m.status, MediaStatus::Ready);
        assert_eq!(m.frames, Some(240));
        assert_eq!(m.fps, Some(Rational::new(24000, 1001)));
        assert_eq!(m.info.unwrap().audio[0].language.as_deref(), Some("jpn"));
        assert_eq!(db.frame_index_blob(e1.id).unwrap(), Some(vec![1, 2, 3]));
        assert_eq!(db.pending_proxies().unwrap(), vec![e1.id]);

        db.set_proxy(e1.id, ProxyState::Ready, Some("/proxies/e1.mp4"), Some(0.0)).unwrap();
        assert_eq!(db.proxy(e1.id).unwrap(), Some(("/proxies/e1.mp4".into(), 0.0)));
        assert!(db.proxy(e2.id).unwrap().is_none());

        let stats = db.stats().unwrap();
        assert_eq!(
            stats,
            ProjectStats { media_count: 3, ready_count: 1, duration_sec: 10.01, poster: Some("/thumbs/e1.jpg".into()) }
        );

        // A crash mid-ingest: back to the queue on next open.
        db.set_media_status(e2.id, MediaStatus::Indexing, None).unwrap();
        assert_eq!(db.requeue_interrupted().unwrap(), vec![e2.id, extra.id]);

        db.remove_media(e1.id).unwrap();
        assert!(db.frame_index_blob(e1.id).unwrap().is_none(), "frame index cascades");
    }

    #[test]
    fn exports_round_trip() {
        let db = ProjectDb::memory();
        let m = new_media("/a.mp4", None, None);
        db.add_media(std::slice::from_ref(&m)).unwrap();
        let preset = as_export::builtin_presets().remove(0);
        let eid = ExportId::new();
        let items = [
            NewExportItem { media_id: m.id, start: 10, end: 34, name: "a_1".into() },
            NewExportItem { media_id: m.id, start: 50, end: 60, name: "a_2".into() },
        ];
        db.insert_export(eid, &preset, "/out", &items).unwrap();
        let pid = ProjectId::new();
        let ex = db.export(pid, eid).unwrap().unwrap();
        assert_eq!(ex.items.len(), 2);
        assert_eq!(ex.preset, preset);
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
    }

    #[test]
    fn stale_exports_are_closed() {
        let db = ProjectDb::memory();
        let m = new_media("/a.mp4", None, None);
        db.add_media(std::slice::from_ref(&m)).unwrap();
        let eid = ExportId::new();
        let preset = as_export::builtin_presets().remove(0);
        db.insert_export(eid, &preset, "/out", &[NewExportItem { media_id: m.id, start: 0, end: 5, name: "x".into() }])
            .unwrap();
        db.close_stale_exports().unwrap();
        let ex = db.export(ProjectId::new(), eid).unwrap().unwrap();
        assert_eq!(ex.status, ExportStatus::Cancelled);
        assert_eq!(ex.items[0].status, ExportStatus::Cancelled);
    }

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
    }
}
