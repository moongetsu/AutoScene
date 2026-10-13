use thiserror::Error;

pub type MediaResult<T> = Result<T, MediaError>;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("ffmpeg was not found. Reinstall AutoScene or set AUTOSCENE_FFMPEG_DIR.")]
    ToolsMissing,
    #[error("ffmpeg {0} is too old; version 7 or newer is required")]
    ToolsTooOld(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0} has no video stream")]
    NoVideo(String),
    #[error("could not read {path}: {message}")]
    Probe { path: String, message: String },
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("verification failed: {0}")]
    Verify(String),
    #[error(transparent)]
    Core(#[from] as_core::CoreError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl MediaError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled | Self::Core(as_core::CoreError::Cancelled))
    }
}
