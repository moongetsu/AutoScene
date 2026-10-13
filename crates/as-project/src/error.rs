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
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Media(MediaError),
    #[error(transparent)]
    Export(ExportError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ProjectError {
    /// Technical description for logs.
    pub fn detail(&self) -> String {
        match self {
            Self::Media(e) => e.detail(),
            Self::Export(ExportError::Media(e)) => e.detail(),
            Self::Export(e) => format!("{e:?}"),
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
