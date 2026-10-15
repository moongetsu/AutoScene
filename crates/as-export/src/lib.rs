//! Export: turning frame ranges into files editors can trust.

pub mod clip;
pub mod preset;

pub use clip::{ClipJob, ClipOutcome, export_clip};
pub use preset::{ExportPreset, builtin_presets};

use thiserror::Error;

pub type ExportResult<T> = Result<T, ExportError>;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("This export format isn't available on this computer.")]
    EncoderMissing(String),
    #[error("Those frames are outside the video.")]
    BadRange(String),
    #[error("This export preset can't be used: {0}.")]
    BadPreset(String),
    #[error(transparent)]
    Media(#[from] as_media::MediaError),
    #[error("{}", as_core::io_message(.0))]
    Io(#[from] std::io::Error),
}

impl ExportError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Media(m) if m.is_cancelled())
    }
}
