//! `fixtures/out/truth.json` — what the fixture generator rendered.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    /// A real shot; its first frame after a hard cut is a cut point.
    Shot,
    /// Gradual transition frames (dissolve, fade, wipe…), belonging to no shot.
    Transition,
    /// Pure black gap.
    Black,
}

/// A contiguous run of master frames `[start, end)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Element {
    pub kind: ElementKind,
    pub start: u32,
    pub end: u32,
    /// Barcode element id.
    pub id: u8,
    pub label: String,
    /// ffmpeg xfade transition name for `Transition` elements.
    pub transition: Option<String>,
    /// Absolute master frames inside this shot that are white flash frames.
    pub flashes: Vec<u32>,
    /// Animation is held on threes (anime-style "on 3s").
    pub held: bool,
    /// Audio tone for the element; `None` = silence.
    pub tone_hz: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Variant {
    pub file: String,
    pub description: String,
    pub video_codec: String,
    /// Nominal frame rate, e.g. "24000/1001".
    pub fps: String,
    pub vfr: bool,
    /// Presentation time of the first frame, seconds.
    pub start_sec: f64,
    /// Per-frame presentation time (relative to the first frame) for VFR variants.
    pub frame_times_sec: Option<Vec<f64>>,
    pub audio_tracks: u32,
    /// Time of the first audio sample when it isn't the video's start.
    #[serde(default)]
    pub audio_start_sec: f64,
    /// Interlaced content (motion between the two fields of a frame), whether
    /// or not the file says so.
    #[serde(default)]
    pub interlaced: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Truth {
    /// Master frame rate (integer fps).
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub frame_count: u32,
    pub audio_sample_rate: u32,
    pub elements: Vec<Element>,
    pub variants: Vec<Variant>,
}

impl Truth {
    /// First frames of shots that follow a hard cut (no transition between).
    pub fn hard_cuts(&self) -> Vec<u32> {
        self.elements
            .windows(2)
            .filter(|w| w[0].kind != ElementKind::Transition && w[1].kind != ElementKind::Transition)
            .map(|w| w[1].start)
            .collect()
    }

    pub fn shots(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| e.kind == ElementKind::Shot)
    }

    pub fn transitions(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| e.kind == ElementKind::Transition)
    }

    pub fn element_at(&self, frame: u32) -> Option<&Element> {
        self.elements.iter().find(|e| frame >= e.start && frame < e.end)
    }

    pub fn variant(&self, file: &str) -> Option<&Variant> {
        self.variants.iter().find(|v| v.file == file)
    }
}
