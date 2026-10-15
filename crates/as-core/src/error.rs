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
    const IN_USE: &str = "The file is open in another program. Close it there and try again.";
    // Windows reports a file another program holds open as a sharing or lock
    // violation, which std doesn't sort into a kind.
    #[cfg(windows)]
    if matches!(e.raw_os_error(), Some(32 | 33)) {
        return IN_USE;
    }
    match e.kind() {
        K::NotFound => "A file or folder AutoScene needs isn't there any more.",
        K::PermissionDenied => "AutoScene doesn't have permission to use that file or folder.",
        K::ReadOnlyFilesystem => "That drive is read-only.",
        K::StorageFull | K::QuotaExceeded => "The disk is full.",
        K::FileTooLarge => "The file is too big for that disk.",
        K::ResourceBusy | K::ExecutableFileBusy => IN_USE,
        K::InvalidFilename => "That name can't be used for a file.",
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
        assert!(io_message(&Error::from(ErrorKind::PermissionDenied)).contains("permission"));
        assert!(io_message(&Error::other("boom")).contains("Details are in the log"));
    }

    #[cfg(windows)]
    #[test]
    fn a_file_open_elsewhere_says_so() {
        for code in [32, 33] {
            assert!(io_message(&Error::from_raw_os_error(code)).contains("open in another program"));
        }
    }
}
