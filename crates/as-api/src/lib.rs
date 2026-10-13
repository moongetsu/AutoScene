//! Types that cross the IPC boundary to the React UI. Everything here derives
//! `specta::Type`, and `app/src/bindings.ts` is generated from it — the UI
//! never hand-writes a backend type.

use as_core::{ExportId, FrameIdx, JobId, MediaId, ProjectId, Rational};
use serde::{Deserialize, Serialize};
use specta::Type;

pub use as_export::{ClipOutcome, ExportPreset};
pub use as_media::{AudioStream, HdrKind, MediaInfo, SubtitleStream, VideoStream};

/// Static facts about the running app, shown in Settings → About and used by
/// the shell for platform-specific chrome.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub platform: Platform,
    pub arch: String,
    pub data_dir: String,
    pub logs_dir: String,
    pub library_root: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

/// The media engine as found on this machine (Settings → About / Hardware).
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaToolsStatus {
    pub ready: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub error: Option<String>,
}

/// User preferences stored in `library.db`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Folder that holds `<Title>.autoscene` project folders.
    pub library_root: String,
    /// Preset preselected in the export dialog.
    pub export_preset_id: String,
    /// Audio language picked automatically for new episodes (ISO 639-2 code,
    /// e.g. `eng`); empty = use each file's default track.
    #[serde(default)]
    pub audio_language: String,
}

// ---------------------------------------------------------------------------
// Library & projects
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ProjectKind {
    Series,
    Movie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    /// Live-action footage (faces).
    Live,
    /// Anime / animation (heads, character art).
    Anime,
    Mixed,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub title: String,
    pub kind: ProjectKind,
    pub content: ContentKind,
    /// The `<Title>.autoscene` folder.
    pub dir: String,
    /// Unix milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    pub media_count: u32,
    pub ready_count: u32,
    pub duration_sec: f64,
    /// Poster image of the first episode, once indexed.
    pub poster: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NewProject {
    pub title: String,
    pub kind: ProjectKind,
    pub content: ContentKind,
    /// Video files (folders are expanded by `import_scan` beforehand).
    pub files: Vec<String>,
}

/// A video found by `import_scan`, with its parsed episode number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub path: String,
    pub file_name: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// `S01E03`, `E12`, or empty when no number was recognised.
    pub label: String,
    pub size_bytes: f64,
    /// Already part of the project (only when scanning into an existing one).
    pub duplicate: bool,
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MediaStatus {
    /// Waiting for its ingest job.
    Queued,
    /// Probing and building the frame index.
    Indexing,
    Ready,
    Failed,
    /// The source file is gone or unreadable (moved drive, renamed).
    Missing,
}

/// Playback proxy state. Sources the webview plays frame-accurately as-is
/// never get one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProxyState {
    /// Not decided yet (media not indexed).
    Unknown,
    NotNeeded,
    Pending,
    Building,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: MediaId,
    pub path: String,
    pub file_name: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// `S01E03` / `E12` / empty.
    pub label: String,
    /// Order within the project (episode order).
    pub position: u32,
    pub size_bytes: f64,
    pub status: MediaStatus,
    pub error: Option<String>,
    pub info: Option<MediaInfo>,
    /// Decoded frame count (from the frame index).
    pub frames: Option<u32>,
    pub duration_sec: Option<f64>,
    /// Nominal frame rate used for timecodes.
    pub fps: Option<Rational>,
    pub vfr: bool,
    pub interlaced: bool,
    /// Audio stream ordinal used for playback and export (`None` = default track).
    pub audio_ordinal: Option<u32>,
    pub proxy: ProxyState,
    pub poster: Option<String>,
    pub added_at: i64,
}

/// Everything the player needs to map `<video>` time to source frames.
///
/// `player_time(f) = offset + rel(f)` where `rel(f) = f * step` for
/// constant-rate media, or `times[f]` otherwise (`times` has `frames + 1`
/// entries; the last is the end of the final frame).
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameMap {
    pub media_id: MediaId,
    /// File to play (the source itself, or its proxy).
    pub src: String,
    pub is_proxy: bool,
    pub frames: u32,
    pub fps: Rational,
    /// Timecodes use SMPTE drop-frame counting (29.97 / 59.94).
    pub drop_frame: bool,
    pub offset: f64,
    pub step: Option<f64>,
    pub times: Option<Vec<f64>>,
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClipRequest {
    pub media_id: MediaId,
    /// First frame (inclusive).
    pub start: FrameIdx,
    /// End frame (exclusive).
    pub end: FrameIdx,
    /// Output file name without extension; generated when empty.
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub preset_id: String,
    /// Destination folder; the project's default export folder when `None`.
    pub out_dir: Option<String>,
    pub clips: Vec<ClipRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportItem {
    pub id: i64,
    pub media_id: MediaId,
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub name: String,
    pub status: ExportStatus,
    pub path: Option<String>,
    /// Every frame was checked against the source frame grid.
    pub verified: bool,
    pub encoder: Option<String>,
    pub bytes: Option<f64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub id: ExportId,
    pub project_id: ProjectId,
    pub created_at: i64,
    pub preset: ExportPreset,
    pub out_dir: String,
    pub status: ExportStatus,
    pub items: Vec<ExportItem>,
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Ingest,
    Proxy,
    Export,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub id: JobId,
    pub kind: JobKind,
    pub title: String,
    pub subtitle: Option<String>,
    pub project_id: Option<ProjectId>,
    pub media_id: Option<MediaId>,
    pub export_id: Option<ExportId>,
    pub state: JobState,
    /// 0..1, `None` while indeterminate.
    pub progress: Option<f64>,
    pub stage: Option<String>,
    pub eta_ms: Option<i64>,
    pub error: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    /// Increases with every change to any job. Events can arrive out of
    /// order; the UI keeps the copy with the highest revision.
    pub rev: u64,
}

/// Throttled progress for a running job (≤ 4 Hz per job).
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub id: JobId,
    /// Revision of the job this progress belongs to (see [`JobInfo::rev`]).
    pub rev: u64,
    pub progress: Option<f64>,
    pub stage: Option<String>,
    pub eta_ms: Option<i64>,
}

/// Unix time in milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
