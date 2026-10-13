use as_project::ProjectError;
use serde::Serialize;
use specta::Type;

/// Error returned by every command. Serialised as `{ code, message }` and
/// thrown on the TypeScript side, so UI code can branch on `code` and show
/// `message` verbatim.
#[derive(Debug, Serialize, Type)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    /// The request itself is wrong (bad range, empty name…); `message` says why.
    Invalid,
    /// Exists but isn't usable yet (still indexing, preview being prepared).
    NotReady,
    Cancelled,
    /// ffmpeg is missing or failed.
    Media,
    Io,
    Update,
    Internal,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        let code = if e.kind() == std::io::ErrorKind::NotFound { ErrorCode::NotFound } else { ErrorCode::Io };
        Self::new(code, e.to_string())
    }
}

impl From<ProjectError> for AppError {
    fn from(e: ProjectError) -> Self {
        let code = match &e {
            ProjectError::NotFound(_) => ErrorCode::NotFound,
            ProjectError::Invalid(_) => ErrorCode::Invalid,
            ProjectError::NotReady(_) => ErrorCode::NotReady,
            ProjectError::Cancelled => ErrorCode::Cancelled,
            ProjectError::Media(_) | ProjectError::Export(_) => ErrorCode::Media,
            ProjectError::Io(_) => ErrorCode::Io,
            ProjectError::Db(_) => ErrorCode::Internal,
        };
        Self::new(code, e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
