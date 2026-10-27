//! `library.db`: settings and the project list.

use std::path::Path;

use as_api::{ContentKind, ProjectSummary, now_ms};
use as_core::ProjectId;
use rusqlite::{OptionalExtension, Row, params};
use rusqlite_migration::{M, Migrations};

use crate::{Db, DbError, DbResult, ProjectStats, enum_from_sql, enum_to_sql};

/// Append-only: never edit a released migration, add a new one.
const STEPS: &[M<'static>] = &[
    M::up(
        "CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;

    CREATE TABLE projects (
        id           TEXT PRIMARY KEY,
        title        TEXT NOT NULL,
        kind         TEXT NOT NULL,
        content      TEXT NOT NULL,
        dir          TEXT NOT NULL UNIQUE,
        created_at   INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL,
        -- Denormalised from project.db so the Library grid stays instant.
        media_count  INTEGER NOT NULL DEFAULT 0,
        ready_count  INTEGER NOT NULL DEFAULT 0,
        duration_sec REAL NOT NULL DEFAULT 0,
        poster       TEXT
    ) STRICT;",
    ),
    M::up("ALTER TABLE projects ADD COLUMN poster_small TEXT;"),
];
const MIGRATIONS: Migrations<'static> = Migrations::from_slice(STEPS);

pub struct LibraryDb(Db);

const PROJECT_COLS: &str = "id, title, kind, content, dir, created_at, updated_at, media_count, ready_count, \
     duration_sec, poster, poster_small";

/// Footage builds before it don't know is stored as the nearest kind they
/// do (3D animation: mixed — faces, film-like cuts), the kind itself in the
/// settings (`footage.<id>`): an older AutoScene still opens the library.
fn stored(content: ContentKind) -> (ContentKind, Option<ContentKind>) {
    match content {
        ContentKind::Animation => (ContentKind::Mixed, Some(content)),
        c => (c, None),
    }
}

fn footage_key(id: ProjectId) -> String {
    format!("footage.{id}")
}

fn set_footage(tx: &rusqlite::Transaction, id: ProjectId, content: ContentKind) -> DbResult<()> {
    match stored(content).1 {
        Some(f) => tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![footage_key(id), enum_to_sql(f)],
        )?,
        None => tx.execute("DELETE FROM settings WHERE key = ?1", [footage_key(id)])?,
    };
    Ok(())
}

/// The footage the settings hold for projects, by key.
fn footages(c: &rusqlite::Connection) -> DbResult<std::collections::HashMap<String, ContentKind>> {
    let mut st = c.prepare("SELECT key, value FROM settings WHERE key LIKE 'footage.%'")?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let (k, v) = row?;
        if let Ok(f) = enum_from_sql(&v) {
            out.insert(k, f);
        }
    }
    Ok(out)
}

fn project_row(r: &Row) -> DbResult<ProjectSummary> {
    let id: String = r.get(0)?;
    Ok(ProjectSummary {
        id: ProjectId::parse(&id).map_err(|e| DbError::Corrupt(e.to_string()))?,
        title: r.get(1)?,
        kind: enum_from_sql(&r.get::<_, String>(2)?)?,
        // Footage a later version stored: the nearest it can be here.
        content: enum_from_sql(&r.get::<_, String>(3)?).unwrap_or(ContentKind::Mixed),
        dir: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        media_count: r.get(7)?,
        ready_count: r.get(8)?,
        duration_sec: r.get(9)?,
        poster: r.get(10)?,
        poster_small: r.get(11)?,
    })
}

impl LibraryDb {
    pub fn open(path: &Path) -> DbResult<Self> {
        Ok(Self(Db::open(path, &MIGRATIONS)?))
    }

    #[cfg(test)]
    pub(crate) fn memory() -> Self {
        Self(Db::memory(&MIGRATIONS, None))
    }

    pub fn setting(&self, key: &str) -> DbResult<Option<String>> {
        self.0.read(|c| Ok(c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [key, value],
            )?;
            Ok(())
        })
    }

    pub fn insert_project(&self, p: &ProjectSummary) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                &format!(
                    "INSERT INTO projects ({PROJECT_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
                ),
                params![
                    p.id.to_string(),
                    p.title,
                    enum_to_sql(p.kind),
                    enum_to_sql(stored(p.content).0),
                    p.dir,
                    p.created_at,
                    p.updated_at,
                    p.media_count,
                    p.ready_count,
                    p.duration_sec,
                    p.poster,
                    p.poster_small,
                ],
            )?;
            set_footage(tx, p.id, p.content)
        })
    }

    /// Most recently touched first.
    pub fn projects(&self) -> DbResult<Vec<ProjectSummary>> {
        self.0.read(|c| {
            let footage = footages(c)?;
            let mut st = c.prepare(&format!("SELECT {PROJECT_COLS} FROM projects ORDER BY updated_at DESC"))?;
            let mut rows = st.query([])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                let mut p = project_row(r)?;
                p.content = footage.get(&footage_key(p.id)).copied().unwrap_or(p.content);
                out.push(p);
            }
            Ok(out)
        })
    }

    pub fn project(&self, id: ProjectId) -> DbResult<Option<ProjectSummary>> {
        self.0.read(|c| {
            let footage = footages(c)?;
            let mut st = c.prepare(&format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1"))?;
            let mut rows = st.query([id.to_string()])?;
            let Some(mut p) = rows.next()?.map(project_row).transpose()? else { return Ok(None) };
            p.content = footage.get(&footage_key(p.id)).copied().unwrap_or(p.content);
            Ok(Some(p))
        })
    }

    pub fn rename_project(&self, id: ProjectId, title: &str) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE projects SET title = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.to_string(), title, now_ms()],
            )?;
            Ok(())
        })
    }

    pub fn set_project_content(&self, id: ProjectId, content: ContentKind) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE projects SET content = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.to_string(), enum_to_sql(stored(content).0), now_ms()],
            )?;
            set_footage(tx, id, content)
        })
    }

    /// Store a project's stats; it counts as touched (moves to the front).
    pub fn update_stats(&self, id: ProjectId, s: &ProjectStats) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute(
                "UPDATE projects SET media_count = ?2, ready_count = ?3, duration_sec = ?4, poster = ?5,
                        poster_small = ?6, updated_at = ?7
                 WHERE id = ?1",
                params![
                    id.to_string(),
                    s.media_count,
                    s.ready_count,
                    s.duration_sec,
                    s.poster,
                    s.poster_small,
                    now_ms()
                ],
            )?;
            Ok(())
        })
    }

    /// Store a project's posters only. Returns whether they changed. Posters
    /// made again in the background don't count as touching the project.
    pub fn set_posters(&self, id: ProjectId, poster: Option<&str>, poster_small: Option<&str>) -> DbResult<bool> {
        self.0.write(|tx| {
            let n = tx.execute(
                "UPDATE projects SET poster = ?2, poster_small = ?3
                 WHERE id = ?1 AND (poster IS NOT ?2 OR poster_small IS NOT ?3)",
                params![id.to_string(), poster, poster_small],
            )?;
            Ok(n > 0)
        })
    }

    pub fn delete_project(&self, id: ProjectId) -> DbResult<()> {
        self.0.write(|tx| {
            tx.execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])?;
            tx.execute("DELETE FROM settings WHERE key = ?1", [footage_key(id)])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use as_api::ProjectKind;

    fn sample(title: &str, dir: &str) -> ProjectSummary {
        ProjectSummary {
            id: ProjectId::new(),
            title: title.into(),
            kind: ProjectKind::Series,
            content: ContentKind::Anime,
            dir: dir.into(),
            created_at: 1,
            updated_at: 1,
            media_count: 0,
            ready_count: 0,
            duration_sec: 0.0,
            poster: None,
            poster_small: None,
        }
    }

    #[test]
    fn projects_round_trip_and_update() {
        let db = LibraryDb::memory();
        let a = sample("Frieren", "/lib/Frieren.autoscene");
        db.insert_project(&a).unwrap();
        let stats = ProjectStats {
            media_count: 3,
            ready_count: 2,
            duration_sec: 4200.5,
            poster: Some("/p.jpg".into()),
            poster_small: Some("/p-small.jpg".into()),
        };
        db.update_stats(a.id, &stats).unwrap();
        db.rename_project(a.id, "Frieren S1").unwrap();
        let got = db.project(a.id).unwrap().unwrap();
        assert_eq!((got.title.as_str(), got.media_count, got.ready_count), ("Frieren S1", 3, 2));
        assert_eq!(got.content, ContentKind::Anime);
        assert_eq!((got.poster.as_deref(), got.poster_small.as_deref()), (Some("/p.jpg"), Some("/p-small.jpg")));

        // New posters don't move the project to the front.
        assert!(!db.set_posters(a.id, Some("/p.jpg"), Some("/p-small.jpg")).unwrap(), "unchanged");
        assert!(db.set_posters(a.id, Some("/p-720.jpg"), Some("/p-270.jpg")).unwrap());
        let after = db.project(a.id).unwrap().unwrap();
        assert_eq!((after.poster.as_deref(), after.poster_small.as_deref()), (Some("/p-720.jpg"), Some("/p-270.jpg")));
        assert_eq!(after.updated_at, got.updated_at);
        // Same folder twice is refused.
        assert!(db.insert_project(&sample("Other", "/lib/Frieren.autoscene")).is_err());
        db.delete_project(a.id).unwrap();
        assert!(db.projects().unwrap().is_empty());
    }

    #[test]
    fn footage_older_versions_dont_know_keeps_the_library_readable_to_them() {
        let db = LibraryDb::memory();
        let a = ProjectSummary { content: ContentKind::Animation, ..sample("Film", "/lib/Film.autoscene") };
        db.insert_project(&a).unwrap();
        // What a build without 3D animation reads: the nearest kind it knows.
        let raw = |id: ProjectId| -> String {
            db.0.read(
                |c| Ok(c.query_row("SELECT content FROM projects WHERE id = ?1", [id.to_string()], |r| r.get(0))?),
            )
            .unwrap()
        };
        assert_eq!(raw(a.id), "mixed");
        assert_eq!(db.project(a.id).unwrap().unwrap().content, ContentKind::Animation);
        assert_eq!(db.projects().unwrap()[0].content, ContentKind::Animation);
        db.set_project_content(a.id, ContentKind::Live).unwrap();
        assert_eq!((raw(a.id).as_str(), db.setting(&footage_key(a.id)).unwrap()), ("live", None));
        assert_eq!(db.project(a.id).unwrap().unwrap().content, ContentKind::Live);
        db.set_project_content(a.id, ContentKind::Animation).unwrap();
        assert_eq!(db.project(a.id).unwrap().unwrap().content, ContentKind::Animation);
        db.delete_project(a.id).unwrap();
        assert_eq!(db.setting(&footage_key(a.id)).unwrap(), None);
        // Footage a later version stored: the nearest kind this one knows.
        let b = sample("Later", "/lib/Later.autoscene");
        db.insert_project(&b).unwrap();
        db.0.write(
            |tx| Ok(tx.execute("UPDATE projects SET content = 'stopmotion' WHERE id = ?1", [b.id.to_string()])?),
        )
        .unwrap();
        assert_eq!(db.project(b.id).unwrap().unwrap().content, ContentKind::Mixed);
    }

    #[test]
    fn settings_upsert() {
        let db = LibraryDb::memory();
        assert_eq!(db.setting("library_root").unwrap(), None);
        db.set_setting("library_root", "D:/A").unwrap();
        db.set_setting("library_root", "D:/B").unwrap();
        assert_eq!(db.setting("library_root").unwrap().as_deref(), Some("D:/B"));
    }

    #[test]
    fn migrations_are_valid() {
        MIGRATIONS.validate().unwrap();
    }

    #[test]
    fn refuses_databases_from_newer_versions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        drop(LibraryDb::open(&path).unwrap());
        let c = rusqlite::Connection::open(&path).unwrap();
        c.pragma_update(None, "user_version", 99).unwrap();
        drop(c);
        assert!(matches!(LibraryDb::open(&path), Err(DbError::TooNew)));
    }
}
