//! Types that cross the IPC boundary to the React UI. Everything here derives
//! `specta::Type`, and `app/src/bindings.ts` is generated from it — the UI
//! never hand-writes a backend type.

use as_core::{CharacterId, ExportId, FrameIdx, JobId, MediaId, ProjectId, Rational};
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
    /// Poster image of the first episode, once indexed (720p, for artwork).
    pub poster: Option<String>,
    /// The same picture small (covers 480×270), for rail tiles and lists.
    pub poster_small: Option<String>,
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
    /// A still of the episode, 720p, for artwork (heroes, backdrops).
    pub poster: Option<String>,
    /// The same still small (covers 480×270), for cards, lists and tiles.
    pub poster_small: Option<String>,
    pub added_at: i64,
    /// Shot detection.
    pub shots: StageState,
    pub shots_error: Option<String>,
    /// Number of shots once detected.
    pub shot_count: Option<u32>,
    /// Face analysis (characters).
    pub faces: StageState,
    pub faces_error: Option<String>,
    /// Face tracks once found.
    pub face_count: Option<u32>,
}

/// A stage of a media's analysis, to run again from scratch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStage {
    /// Shots (and everything built on them: faces).
    Shots,
    /// Faces and characters only.
    Faces,
}

/// Progress of one analysis stage of a media.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    /// Not done yet (waiting for indexing or its job).
    Pending,
    Running,
    Ready,
    Failed,
    /// Stopped by the user; continues only when asked to.
    Stopped,
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Shot,
    /// Cross-dissolve, wipe or another blend between two shots.
    Dissolve,
    /// From a shot to black or white.
    FadeOut,
    /// From black or white into a shot.
    FadeIn,
    /// Black (or, inside a fade, white) frames.
    Black,
    /// A brief flash (muzzle flash, lightning, anime impact frame) inside a shot.
    Flash,
}

/// Frames `[start, end)` of a media. Segments cover it without gaps; clips
/// come from `shot` segments, so they never start mid-transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub kind: SegmentKind,
    /// A shot under 3 frames, kept because both cuts around it are clear.
    pub tiny: bool,
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
// Cast
// ---------------------------------------------------------------------------

/// How a face track got to its character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TrackAssign {
    /// Nobody (an extra, a background face).
    None,
    /// Found by the analysis.
    Auto,
    /// Looks like the character, but not clearly: waits for a decision.
    Unsure,
    /// Decided by the user.
    Pinned,
}

/// A person the analysis found across the project's media.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    pub id: CharacterId,
    /// Given by the user; `None` until then.
    pub name: Option<String>,
    /// The name, or "Character N" (numbered in the order characters were
    /// found; a number is never reused).
    pub label: String,
    pub hidden: bool,
    /// Face tracks (auto and pinned).
    pub tracks: u32,
    /// Shots the character is seen in.
    pub shots: u32,
    /// Media (episodes) the character is seen in.
    pub media: u32,
    /// Time the character's face is on screen.
    pub screen_sec: f64,
    /// Portrait for big places (cards, heroes): 3:4 from the exact frame at
    /// full resolution once it is made, the face thumbnail until then
    /// (absolute path).
    pub cover: Option<String>,
    /// The same face as a small square thumbnail, for avatars and lists.
    pub cover_small: Option<String>,
    /// Tracks that look like the character and wait for a decision.
    pub unsure: u32,
    /// Faces of the character in media waiting for or in a new analysis:
    /// they come back when it is done.
    pub waiting: u32,
}

/// The cast of a project.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Cast {
    /// Most screen time first.
    pub characters: Vec<Character>,
    /// Tracks of nobody in particular.
    pub unassigned: u32,
    /// Tracks waiting for a decision.
    pub unsure: u32,
    /// Media whose faces are analysed / all media.
    pub media_done: u32,
    pub media_total: u32,
}

/// One face followed through one shot.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FaceTrackItem {
    pub id: u32,
    pub media_id: MediaId,
    pub start: FrameIdx,
    pub end: FrameIdx,
    /// 0..1: how clearly the face shows who it is.
    pub quality: f32,
    /// Median face height as a share of the frame height.
    pub face_height: f32,
    /// Square JPEG of the face (absolute path).
    pub thumb: String,
    /// Assigned character, or the suggestion when `assign` is `unsure`.
    pub character_id: Option<CharacterId>,
    pub assign: TrackAssign,
    /// Similarity behind an automatic assignment or suggestion.
    pub score: f32,
}

/// A shot a character is seen in.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterShot {
    pub media_id: MediaId,
    pub start: FrameIdx,
    pub end: FrameIdx,
    /// JPEG of `cells` frames spread over the shot, side by side (absolute
    /// path); `None` for media analysed before shots had sprites.
    pub sprite: Option<String>,
    pub cells: u32,
    /// The character's clearest face in the shot.
    pub track_id: u32,
    /// Its thumbnail (absolute path).
    pub face: String,
    /// The biggest the character's face is in the shot, as a share of the frame height.
    pub face_height: f32,
}

/// Which tracks of the cast to list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum TrackFilter {
    /// Tracks of a character (auto and pinned).
    Character(CharacterId),
    /// Tracks suggested for a character, waiting for a decision.
    Unsure(CharacterId),
    /// Every track waiting for a decision.
    AllUnsure,
    /// Tracks of nobody.
    Unassigned,
}

/// A user decision about face tracks.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum CastEdit {
    /// These tracks are this character.
    Assign {
        tracks: Vec<u32>,
        character_id: CharacterId,
    },
    /// These tracks are someone new (returns the new character).
    NewCharacter {
        tracks: Vec<u32>,
        name: Option<String>,
    },
    /// These tracks are not this character.
    NotThem {
        tracks: Vec<u32>,
        character_id: CharacterId,
    },
    /// These tracks aren't faces at all.
    NotAFace {
        tracks: Vec<u32>,
    },
    /// Accept the suggestions of these unsure tracks.
    Confirm {
        tracks: Vec<u32>,
    },
    Rename {
        character_id: CharacterId,
        name: Option<String>,
    },
    SetHidden {
        character_id: CharacterId,
        hidden: bool,
    },
    /// Everyone of `from` is `into`.
    Merge {
        from: CharacterId,
        into: CharacterId,
    },
    SetCover {
        character_id: CharacterId,
        track: u32,
    },
}

/// Where characters are on screen in one media.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub character_id: CharacterId,
    pub track_id: u32,
    pub start: FrameIdx,
    pub end: FrameIdx,
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
    /// AI analysis of a media: shots, then faces and characters.
    Analyze,
    /// Posters of a project made again at the current sizes.
    Posters,
    /// Sharp portraits of a project's characters, cut from the exact frames.
    Portraits,
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
