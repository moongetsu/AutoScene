//! Analysis: the resumable passes over a media and what they produce —
//! frame signals fused into shots and transitions, face tracks, and the
//! people around the faces (body tracks tied to them).

pub mod bodies;
pub mod body_tracker;
pub mod faces;
pub mod fusion;
pub mod link;
pub mod pipeline;
pub mod portrait;
pub mod signals;
pub mod sprites;
pub mod tracker;
pub mod tracks;

pub use faces::{DETECT_BATCH, FaceChunk, FaceParams, face_tracks, frame_sizes};
pub use fusion::{FusionParams, Segment, SegmentKind, TransitionKind, fuse};
pub use pipeline::{CHUNK, shot_signals};
pub use portrait::portrait_region;
pub use signals::FrameSignals;
pub use sprites::ShotSprite;
pub use tracks::{FaceDet, FaceTrack};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error(transparent)]
    Media(#[from] as_media::MediaError),
    #[error(transparent)]
    Infer(#[from] as_infer::InferError),
    #[error("Something went wrong while analysing. Details are in the log.")]
    Internal(String),
    /// A checkpoint couldn't be stored (the caller's error, as text).
    #[error("{0}")]
    Store(String),
}

impl AnalysisError {
    pub fn detail(&self) -> String {
        match self {
            Self::Media(e) => e.detail(),
            Self::Infer(e) => e.detail(),
            Self::Internal(e) => format!("analysis: {e}"),
            Self::Store(e) => format!("checkpoint: {e}"),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Media(e) if e.is_cancelled())
    }
}
