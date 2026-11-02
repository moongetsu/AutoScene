use as_plugin::PluginError;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
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
        log::warn!("file: {e}");
        let code = if e.kind() == std::io::ErrorKind::NotFound { ErrorCode::NotFound } else { ErrorCode::Io };
        Self::new(code, as_core::io_message(&e))
    }
}

/// People read the error's `Display`; the log gets its technical side, so
/// "Details are in the log" holds for commands as it does for jobs.
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
        match code {
            // Expected answers (a stale id, a file still indexing): no noise.
            ErrorCode::NotFound | ErrorCode::Invalid | ErrorCode::NotReady | ErrorCode::Cancelled => {
                log::debug!("{}", e.detail());
            }
            _ => log::warn!("{}", e.detail()),
        }
        Self::new(code, e.to_string())
    }
}

/// As for [`ProjectError`]. A signature that fails or an author key that
/// changed says only that to the person; the log has which and why.
impl From<PluginError> for AppError {
    fn from(e: PluginError) -> Self {
        let code = match &e {
            PluginError::NotFound(_) => ErrorCode::NotFound,
            PluginError::Busy(_) => ErrorCode::NotReady,
            PluginError::Cancelled => ErrorCode::Cancelled,
            PluginError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            PluginError::Io(_) => ErrorCode::Io,
            PluginError::Db(_) | PluginError::Failed { .. } | PluginError::Crashed { .. } => ErrorCode::Internal,
            PluginError::Invalid(_)
            | PluginError::TooNew(_)
            | PluginError::Platform
            | PluginError::BadSignature(_)
            | PluginError::KeyChanged { .. }
            | PluginError::NotAllowed
            | PluginError::UvMissing => ErrorCode::Invalid,
        };
        match (&e, code) {
            (PluginError::BadSignature(_) | PluginError::KeyChanged { .. }, _) => log::warn!("{}", e.detail()),
            (_, ErrorCode::NotFound | ErrorCode::Invalid | ErrorCode::NotReady | ErrorCode::Cancelled) => {
                log::debug!("{}", e.detail());
            }
            _ => log::warn!("{}", e.detail()),
        }
        Self::new(code, e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use super::*;

    #[test]
    fn plugin_errors_get_their_codes_and_keep_their_words() {
        let said = |e: PluginError| {
            let e = AppError::from(e);
            (e.code, e.message)
        };
        let invalid = [
            PluginError::Invalid("Its name is missing.".into()),
            PluginError::TooNew("engines.autoscene".into()),
            PluginError::Platform,
            PluginError::BadSignature("hash".into()),
            PluginError::KeyChanged { pinned: "A".into(), got: "B".into() },
            PluginError::NotAllowed,
            PluginError::UvMissing,
        ];
        for e in invalid {
            let words = e.to_string();
            assert_eq!(said(e), (ErrorCode::Invalid, words));
        }
        assert_eq!(said(PluginError::NotFound("dev.a.b".into())).0, ErrorCode::NotFound);
        assert_eq!(
            said(PluginError::Busy("Poster".into())),
            (ErrorCode::NotReady, "Poster is running. Wait for it to finish, or cancel it in Activity.".into())
        );
        assert_eq!(said(PluginError::Cancelled).0, ErrorCode::Cancelled);
        // A file: missing, else any other trouble.
        assert_eq!(said(PluginError::Io(Error::from(ErrorKind::NotFound))).0, ErrorCode::NotFound);
        assert_eq!(said(PluginError::Io(Error::from(ErrorKind::PermissionDenied))).0, ErrorCode::Io);
        // A run's own failures are the plugin's, not the request's.
        let failed = PluginError::Failed { message: "The webhook said no.".into(), detail: None };
        assert_eq!(said(failed), (ErrorCode::Internal, "The webhook said no.".into()));
        let crashed = PluginError::Crashed { name: "Poster".into(), detail: "exit 1".into() };
        assert_eq!(said(crashed), (ErrorCode::Internal, "Poster stopped unexpectedly.".into()));
    }
}
