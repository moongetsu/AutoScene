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
