//! Media layer. Everything that talks to ffmpeg/ffprobe lives here, behind
//! typed APIs that speak in frame indices rather than seconds.

pub mod caps;
pub mod error;
pub mod filters;
pub mod frames;
pub mod idet;
pub mod index;
pub mod lang;
pub mod poster;
pub mod probe;
pub mod proc;
pub mod proxy;
pub mod run;
pub mod tools;

pub use caps::{Caps, CodecFamily, HwDecode, HwEncoder, Vendor};
pub use error::{MediaError, MediaResult};
pub use index::build_frame_index;
pub use probe::{AudioStream, HdrKind, MediaInfo, SubtitleStream, VideoStream, probe, same_source};
pub use run::{FfProgress, run_ffmpeg, run_ffmpeg_counting};
pub use tools::Tools;

use std::path::Path;

/// A source file with everything needed to address its frames exactly.
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    pub path: &'a Path,
    pub info: &'a MediaInfo,
    pub index: &'a as_core::FrameIndex,
}
