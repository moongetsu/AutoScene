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
    /// Also follow people by their bodies and clothes, so shots where a
    /// character's face doesn't show (turned away, too small) join their
    /// scenepack. Analysis takes longer; off by default.
    #[serde(default)]
    pub find_bodies: bool,
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
    /// 3D / CGI animation (Pixar-like films and series): faces like live
    /// action, but every character of one style looks alike to the face
    /// model, so their similarity bars are always fitted to the footage.
    Animation,
    /// Anime / 2D animation (heads, character art).
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
    /// People followed by their bodies (when the setting is on).
    #[serde(default = "pending")]
    pub bodies: StageState,
    #[serde(default)]
    pub bodies_error: Option<String>,
}

fn pending() -> StageState {
    StageState::Pending
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
    /// Time the character is on screen: their face, or their body.
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
    /// The character's clearest face in the shot (0 in a shot without it).
    pub track_id: u32,
    /// Its thumbnail (absolute path); the character's own in a shot
    /// without their face.
    pub face: String,
    /// The biggest the character's face is in the shot, as a share of the
    /// frame height (as big as their body suggests in a shot without it).
    pub face_height: f32,
    /// Their face doesn't show in the shot: their body does (turned away,
    /// too small), found by the face on it or by their clothes.
    #[serde(default)]
    pub faceless: bool,
    /// Their bodies there, in a shot without their face — every one, so
    /// the user's word on the shot is on all of them.
    #[serde(default)]
    pub bodies: Vec<u32>,
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
    /// These bodies (people seen without their face) are this character.
    BodyAssign {
        bodies: Vec<u32>,
        character_id: CharacterId,
    },
    /// These bodies are not this character.
    BodyNotThem {
        bodies: Vec<u32>,
        character_id: CharacterId,
    },
    /// These bodies aren't people (a coat on a chair, a shadow).
    NotAPerson {
        bodies: Vec<u32>,
    },
}

impl CastEdit {
    /// The decision changes who a face is (so other faces may follow it), not
    /// just a name, a picture or what is shown.
    pub fn moves_faces(&self) -> bool {
        match self {
            Self::Assign { .. }
            | Self::NewCharacter { .. }
            | Self::NotThem { .. }
            | Self::NotAFace { .. }
            | Self::Confirm { .. }
            | Self::Merge { .. }
            | Self::BodyAssign { .. }
            | Self::BodyNotThem { .. }
            | Self::NotAPerson { .. } => true,
            Self::Rename { .. } | Self::SetHidden { .. } | Self::SetCover { .. } => false,
        }
    }
}

/// What became of a missing media pointed at a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Relink {
    /// The same file: everything stays as it was.
    Same,
    /// Another copy of it: indexed again. Its analysis and the decisions on
    /// it stay when its frames are the same.
    Reindex,
    /// Another video (`reason` says how). Taken anyway with `force`: it is
    /// analysed again, the decisions on its faces are kept.
    Different { reason: String },
}

/// Missing media looked for in a folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelinkSummary {
    /// Found as they were.
    pub same: u32,
    /// Found as another copy, being indexed again.
    pub reindex: u32,
    /// Still missing.
    pub missing: u32,
}

/// Where characters are on screen in one media.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub character_id: CharacterId,
    /// The face track — or, with `body`, the body track.
    pub track_id: u32,
    pub start: FrameIdx,
    pub end: FrameIdx,
    /// On screen without their face showing: their body is.
    #[serde(default)]
    pub body: bool,
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
    /// What it is, for the export history ("Walter · scenepack").
    pub label: Option<String>,
    /// Join every clip into one file of this name (without extension), with
    /// a chapter per clip named after it; `None`: a file per clip.
    #[serde(default)]
    pub merge: Option<String>,
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

/// The one file of an export that joins its clips.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MergedFile {
    /// File name without extension.
    pub name: String,
    /// Where it is, once it's made.
    pub path: Option<String>,
    /// Every frame of every clip was checked, and so was the joined file.
    pub verified: bool,
    pub bytes: Option<f64>,
    /// Why it couldn't be made.
    pub error: Option<String>,
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

/// What an export would make, worked out before it starts.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    /// File names (without extension) in clip order, as the export names
    /// them (a name already in the destination folder gets `_2` then).
    pub names: Vec<String>,
    /// Roughly how big it would be with each preset.
    pub sizes: Vec<PresetSize>,
    /// The folder the files go into inside the destination (a scenepack's
    /// folder named after its character), as it will be named.
    pub folder: Option<String>,
    /// The one file's name (without extension) when the clips are joined
    /// into one; `names` are then its chapters.
    pub file: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetSize {
    pub preset_id: String,
    pub bytes: f64,
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
    /// What it is ("Walter · scenepack"); `None` for single clips.
    pub label: Option<String>,
    /// The one file the clips are joined into; `None`: a file per clip.
    pub merged: Option<MergedFile>,
}

// ---------------------------------------------------------------------------
// Packs
// ---------------------------------------------------------------------------

/// How big a face is in a shot, from its height in the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ShotSize {
    ExtremeCloseUp,
    CloseUp,
    MediumCloseUp,
    Medium,
    Wide,
}

impl ShotSize {
    /// From the biggest face's height as a share of the frame height.
    pub fn of(face_height: f32) -> Self {
        match face_height {
            h if h >= 0.45 => Self::ExtremeCloseUp,
            h if h >= 0.25 => Self::CloseUp,
            h if h >= 0.15 => Self::MediumCloseUp,
            h if h >= 0.08 => Self::Medium,
            _ => Self::Wide,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PackOrder {
    /// As they are in the episodes.
    #[default]
    Episodes,
    /// The best first (clear, big, on screen the whole shot).
    Best,
}

/// Which of a character's shots go into their scenepack. (Filters saved by
/// an older version load with the defaults filled in: `as_project::pack`.)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackSpec {
    /// Shots shorter than this many seconds are left out.
    pub min_sec: Option<f64>,
    /// Shots longer than this many seconds are left out.
    pub max_sec: Option<f64>,
    /// Only shots of these sizes (empty: any size).
    pub sizes: Vec<ShotSize>,
    /// Only shots with no one else in them.
    pub solo: bool,
    /// Leave out flash cuts (shots under 3 frames).
    pub skip_tiny: bool,
    /// Leave out shots where the character's face doesn't show (found by
    /// their body).
    #[serde(default)]
    pub skip_faceless: bool,
    /// Only these episodes (empty: all).
    pub media: Vec<MediaId>,
    pub order: PackOrder,
    /// At most this many shots: the best ones.
    pub limit: Option<u32>,
    /// Extra frames before and after each shot (within the episode).
    pub handles: u32,
}

impl Default for PackSpec {
    fn default() -> Self {
        Self {
            min_sec: None,
            max_sec: None,
            sizes: Vec::new(),
            solo: false,
            skip_tiny: true,
            skip_faceless: false,
            media: Vec::new(),
            order: PackOrder::Episodes,
            limit: None,
            handles: 0,
        }
    }
}

/// Why a shot is left out of a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PackReason {
    /// The episode's file can't be found (or isn't indexed yet).
    Missing,
    Episode,
    Tiny,
    Short,
    Long,
    Size,
    NotSolo,
    /// The character's face doesn't show in it.
    Faceless,
    /// Past the limit.
    Limit,
    /// Dropped by hand.
    Removed,
}

/// A shot of a pack, picked or not.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackClip {
    /// The shot (its picture, face and range): picks by hand stick to its media and first frame.
    pub shot: CharacterShot,
    /// The clip: the shot and its handles, within the episode.
    pub start: FrameIdx,
    pub end: FrameIdx,
    pub size: ShotSize,
    /// 0..1: how clearly the character's face shows in it.
    pub quality: f32,
    /// Share of the shot the character is on screen.
    pub coverage: f32,
    /// Other characters on screen in the shot.
    pub others: u32,
    /// 0..1: how good a clip it makes (what "best first" sorts by).
    pub score: f32,
    pub included: bool,
    /// What the filters alone would say.
    pub by_rules: bool,
    /// Why it's left out.
    pub reason: Option<PackReason>,
    /// Picked (`true`) or dropped (`false`) by hand.
    pub manual: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackTotals {
    pub clips: u32,
    pub frames: f64,
    pub seconds: f64,
}

/// A pack as it stands: every shot of the character, the ones it takes
/// marked, in the pack's order.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackPreview {
    pub clips: Vec<PackClip>,
    pub totals: PackTotals,
    /// Names exactly the clips it takes: an export of it is refused once the
    /// pack changed, so what is exported is what was shown.
    pub stamp: String,
    /// Names its clips, in order, as they are apart from their [`PackChoice`]s.
    pub shape: String,
}

/// Whether a shot is in its pack, and why: the part of a [`PackClip`] that
/// picking shots changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackChoice {
    pub included: bool,
    pub by_rules: bool,
    pub reason: Option<PackReason>,
    pub manual: Option<bool>,
}

/// A pack after shots were picked: every clip's choice, in the pack's
/// order — the rest of the pack is its preview's, when the `shape` is.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackChoices {
    pub choices: Vec<PackChoice>,
    pub totals: PackTotals,
    pub stamp: String,
    pub shape: String,
}

/// How a pack's files are named and where they go.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackNaming {
    /// Each clip's name (its file's, or its chapter's in one file), with
    /// `{series}`, `{character}`, `{episode}`, `{index}`, `{tc}`, `{frame}`.
    pub template: String,
    /// Into a folder named after the character (a file per clip only).
    pub folder: bool,
    /// Every clip in one file, with a chapter each.
    pub merge: bool,
    /// The one file's name, with `{series}` and `{character}`.
    pub file: String,
}

impl Default for PackNaming {
    fn default() -> Self {
        Self {
            template: "{series} {character} {episode} {tc}".into(),
            folder: true,
            merge: true,
            file: "{series} {character} scenepack".into(),
        }
    }
}

/// A character's pack settings, as last used.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackState {
    pub spec: PackSpec,
    pub naming: PackNaming,
}

/// A shot of a pack, by its media and first frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackKey {
    pub media_id: MediaId,
    pub shot: FrameIdx,
}

/// Shots of a pack picked (`include: true`), dropped (`false`) or handed
/// back to the filters (`null`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackPicks {
    pub shots: Vec<PackKey>,
    pub include: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackExportRequest {
    pub spec: PackSpec,
    pub naming: PackNaming,
    pub preset_id: String,
    /// The folder the files (or the character's folder) go to; the
    /// project's default export folder when `None`.
    pub out_dir: Option<String>,
    /// The preview's stamp: the export is refused if the pack changed since.
    pub stamp: String,
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
    /// A project's characters grouped again: the grouping got better since
    /// they were grouped (the user's decisions stay).
    Regroup,
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
