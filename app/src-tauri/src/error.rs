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
    Io,
    Update,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
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

pub type AppResult<T> = Result<T, AppError>;
