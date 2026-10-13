//! Export: turning frame ranges into files editors can trust.

pub mod clip;
pub mod preset;

pub use clip::{ClipJob, ClipOutcome, export_clip};
pub use preset::{ExportPreset, builtin_presets};

use thiserror::Error;

pub type ExportResult<T> = Result<T, ExportError>;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("this ffmpeg build has no {0} encoder")]
    EncoderMissing(String),
    #[error("invalid range: {0}")]
    BadRange(String),
    #[error("invalid preset: {0}")]
    BadPreset(String),
    #[error(transparent)]
    Media(#[from] as_media::MediaError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ExportError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Media(m) if m.is_cancelled())
    }
}
