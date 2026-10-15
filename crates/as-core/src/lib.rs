//! Core types shared by every AutoScene crate.
//!
//! The one rule this crate enforces: **time is an integer frame index** on a
//! media's decoded frame grid. Seconds only exist at the edges (ffmpeg
//! arguments, the `<video>` element), and are always derived through a
//! [`FrameIndex`] built from the decoder's real output timestamps.

pub mod cancel;
pub mod error;
pub mod ids;
pub mod paths;
pub mod progress;
pub mod time;

pub use cancel::CancelToken;
pub use error::{CoreError, CoreResult, io_message};
pub use ids::*;
pub use paths::AppPaths;
pub use progress::{Progress, ProgressSink};
pub use time::{FrameIdx, FrameIndex, FrameRange, Rational, Timecode};
