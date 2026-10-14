//! Storage. Two SQLite databases, both in WAL mode:
//!
//! - `library.db` in the app data folder: settings and the project list (with
//!   denormalised stats so the Library grid never opens every project);
//! - `project.db` inside each `<Title>.autoscene` folder: media, frame
//!   indexes, exports — and, from M2 on, everything analysis produces.
//!
//! Schemas only ever move forward through `rusqlite_migration` (`user_version`).
//! A database written by a newer AutoScene is refused rather than damaged.

mod cast;
mod library;
mod project;

pub use cast::{CharacterRow, CharacterShotRow, EditKind, IdentityTrack, NewTrack, TrackEdit, TrackRow};
pub use library::LibraryDb;
pub use project::{IngestRecord, NewExportItem, NewMedia, ProjectDb, ProjectStats, episode_label};

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{Connection, Transaction};
use rusqlite_migration::Migrations;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not upgrade database: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("this project was created by a newer version of AutoScene")]
    TooNew,
    #[error("corrupt record: {0}")]
    Corrupt(String),
    /// A file written as part of a transaction failed (the transaction rolled back).
    #[error("file error: {0}")]
    Io(String),
}

/// One SQLite connection behind a mutex. Every statement here is short
/// (indexed lookups, small writes), so a single connection per database is
/// simpler and just as fast as a pool at AutoScene's scale.
struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    fn open(path: &Path, migrations: &Migrations<'static>) -> DbResult<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        tracing::debug!(db = %path.display(), "opening database");
        match migrations.to_latest(&mut conn) {
            Err(rusqlite_migration::Error::MigrationDefinition(
                rusqlite_migration::MigrationDefinitionError::DatabaseTooFarAhead,
            )) => return Err(DbError::TooNew),
            r => r?,
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    #[cfg(test)]
    fn memory(migrations: &Migrations<'static>) -> Self {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrations.to_latest(&mut conn).unwrap();
        Self { conn: Mutex::new(conn) }
    }

    fn read<T>(&self, f: impl FnOnce(&Connection) -> DbResult<T>) -> DbResult<T> {
        f(&self.conn.lock())
    }

    fn write<T>(&self, f: impl FnOnce(&Transaction) -> DbResult<T>) -> DbResult<T> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}

/// Enums are stored as their serde (= TypeScript) names, so the database,
/// the IPC payloads and the UI all spell a state the same way.
fn enum_to_sql<T: Serialize>(v: T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        other => panic!("enum_to_sql on a non-unit enum: {other:?}"),
    }
}

fn enum_from_sql<T: DeserializeOwned>(s: &str) -> DbResult<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|_| DbError::Corrupt(format!("unknown value '{s}'")))
}

fn json_to_sql<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("DTOs always serialise")
}

fn json_from_sql<T: DeserializeOwned>(s: &str) -> DbResult<T> {
    serde_json::from_str(s).map_err(|e| DbError::Corrupt(e.to_string()))
}
