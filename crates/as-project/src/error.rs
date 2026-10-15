use as_db::DbError;
use as_export::ExportError;
use as_media::MediaError;
use thiserror::Error;

pub type ProjectResult<T> = Result<T, ProjectError>;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    /// The thing exists but isn't usable yet (e.g. still indexing).
    #[error("{0}")]
    NotReady(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{}", db_message(.0))]
    Db(#[from] DbError),
    #[error(transparent)]
    Media(MediaError),
    #[error(transparent)]
    Export(ExportError),
    #[error("{}", as_core::io_message(.0))]
    Io(#[from] std::io::Error),
}

/// What people read when the project's database fails; SQLite's own words
/// are in [`ProjectError::detail`].
fn db_message(e: &DbError) -> &'static str {
    match e {
        DbError::TooNew => "This project was made by a newer version of AutoScene. Update AutoScene to open it.",
        _ => "AutoScene couldn't read or save its data. Details are in the log.",
    }
}

impl ProjectError {
    /// Technical description for logs.
    pub fn detail(&self) -> String {
        match self {
            Self::Media(e) => e.detail(),
            Self::Export(ExportError::Media(e)) => e.detail(),
            Self::Export(e) => format!("{e:?}"),
            Self::Db(e) => format!("database: {e}"),
            Self::Io(e) => format!("file: {e}"),
            e => e.to_string(),
        }
    }
}

impl From<MediaError> for ProjectError {
    fn from(e: MediaError) -> Self {
        if e.is_cancelled() { Self::Cancelled } else { Self::Media(e) }
    }
}

impl From<ExportError> for ProjectError {
    fn from(e: ExportError) -> Self {
        if e.is_cancelled() { Self::Cancelled } else { Self::Export(e) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_and_file_failures_read_like_the_app() {
        let db = ProjectError::Db(DbError::Corrupt("tracks row 7".into()));
        assert!(!db.to_string().contains("corrupt record"), "{db}");
        assert!(db.detail().contains("corrupt record: tracks row 7"), "{}", db.detail());
        assert!(ProjectError::Db(DbError::TooNew).to_string().contains("Update AutoScene"));

        let io = ProjectError::Io(std::io::Error::other("The device is not ready. (os error 21)"));
        assert!(!io.to_string().contains("os error"), "{io}");
        assert!(io.detail().contains("os error 21"), "{}", io.detail());
    }
}
