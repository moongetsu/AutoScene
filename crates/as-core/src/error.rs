use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("cancelled")]
    Cancelled,
    #[error("invalid rational '{0}'")]
    BadRational(String),
    #[error("frame index is empty")]
    EmptyIndex,
    #[error("frame index timestamps are not strictly increasing at frame {0}")]
    NonMonotonic(usize),
    #[error("corrupt frame index blob: {0}")]
    CorruptIndex(String),
}

/// What people read when a file or folder couldn't be read or written. The
/// system's own message (paths, OS error codes) belongs in the log, next to it.
pub fn io_message(e: &std::io::Error) -> &'static str {
    use std::io::ErrorKind as K;
    match e.kind() {
        K::NotFound => "A file or folder AutoScene needs isn't there any more.",
        K::PermissionDenied | K::ReadOnlyFilesystem => "AutoScene isn't allowed to write there.",
        K::StorageFull | K::QuotaExceeded => "The disk is full.",
        K::FileTooLarge => "The file is too big for that disk.",
        _ => "A file couldn't be read or written. Details are in the log.",
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use super::io_message;

    #[test]
    fn file_errors_read_like_the_app_not_the_os() {
        let os = Error::from_raw_os_error(5);
        assert!(!io_message(&os).contains("os error"), "{}", io_message(&os));
        assert_eq!(io_message(&Error::from(ErrorKind::StorageFull)), "The disk is full.");
        assert!(io_message(&Error::from(ErrorKind::NotFound)).contains("isn't there"));
        assert!(io_message(&Error::other("boom")).contains("Details are in the log"));
    }
}
