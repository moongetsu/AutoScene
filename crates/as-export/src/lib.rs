//! Export: turning frame ranges into files editors can trust.

pub mod clip;
pub mod estimate;
pub mod lossless;
pub mod merge;
pub mod preset;

pub use clip::{ClipJob, ClipOutcome, Part, export_clip};
pub use estimate::{EstimateClip, estimate_bytes};
pub use lossless::{LosslessCodec, exact_format};
pub use merge::{
    JoinJob, JoinPart, LosslessShape, MergePlan, MergeShape, MergeSource, PartTiming, join_parts,
    lossless_merge_blocker, plan_lossless_merge, plan_merge,
};
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
    /// A Bit-Perfect clip can't be, or didn't come out, exactly the source.
    #[error("This clip can't be exported bit-perfect: {0}.")]
    Inexact(String),
    /// The parts of a merged export couldn't be joined (the detail says why).
    #[error("The clips couldn't be joined into one file.")]
    Join(String),
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
