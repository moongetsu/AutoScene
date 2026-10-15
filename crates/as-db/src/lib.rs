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
mod pack;
mod project;

pub use cast::{CharacterRow, CharacterShotRow, EditKind, IdentityTrack, NewTrack, TrackEdit, TrackRow};
pub use library::LibraryDb;
pub use pack::PackRow;
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

impl DbError {
    /// What people read when the database fails. This error's `Display`, with
    /// SQLite's own words, belongs in the log.
    pub fn user_message(&self) -> &'static str {
        const GENERIC: &str = "AutoScene couldn't read or save its data. Details are in the log.";
        let sqlite = |e: &rusqlite::Error| {
            use rusqlite::ErrorCode as C;
            let rusqlite::Error::SqliteFailure(f, _) = e else { return GENERIC };
            match f.code {
                C::DiskFull => "The disk is full.",
                C::ReadOnly => "AutoScene can't save its data there: the drive or folder is read-only.",
                C::CannotOpen => "AutoScene couldn't open its data. Check that the drive is connected.",
                C::PermissionDenied => "AutoScene doesn't have permission to open its data.",
                C::DatabaseBusy | C::DatabaseLocked => {
                    "Another program is using AutoScene's data. Close it and try again."
                }
                C::DatabaseCorrupt | C::NotADatabase => "AutoScene's data file is damaged. Details are in the log.",
                C::SystemIoFailure => "The drive couldn't be read or written. Details are in the log.",
                _ => GENERIC,
            }
        };
        match self {
            Self::TooNew => "This project was made by a newer version of AutoScene. Update AutoScene to open it.",
            Self::Sqlite(e) => sqlite(e),
            Self::Migration(rusqlite_migration::Error::RusqliteError { err, .. }) => match sqlite(err) {
                GENERIC => "AutoScene couldn't update this project's data. Details are in the log.",
                message => message,
            },
            Self::Migration(_) => "AutoScene couldn't update this project's data. Details are in the log.",
            Self::Corrupt(_) | Self::Io(_) => GENERIC,
        }
    }
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
