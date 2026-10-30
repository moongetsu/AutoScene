//! Engine wire protocol: one JSON object per line over the engine's stdin
//! (requests) and stdout (replies). Binary payloads travel as base64.

use std::path::PathBuf;

use as_media::MediaInfo;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bytes that serialise as a base64 string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blob(pub Vec<u8>);

impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD.decode(s).map(Blob).map_err(serde::de::Error::custom)
    }
}

/// App → engine.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Measure the shot signals of a media from `resume` on.
    Shots(Box<ShotsJob>),
    /// Find, embed and track the faces of a media's shots from `resume` on.
    Faces(Box<FacesJob>),
    /// Embed a still image (a character portrait, say) with the anime
    /// embedder (`as_vision::ccip`) for matching against grouped
    /// characters; `Reply::Embedding` then `Done`.
    Embed(Box<EmbedJob>),
    /// Stop job `id` as soon as possible (it then fails with `cancelled`).
    Cancel { id: u64 },
    /// Finish the current job's cancellation and exit.
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShotsJob {
    pub id: u64,
    pub path: PathBuf,
    pub info: MediaInfo,
    /// `FrameIndex::to_bytes`.
    pub index: Blob,
    /// First frame to measure; a multiple of `chunk`.
    pub resume: u32,
    /// Frames per checkpoint chunk; a multiple of TransNet's step.
    pub chunk: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FacesJob {
    pub id: u64,
    pub path: PathBuf,
    pub info: MediaInfo,
    /// `FrameIndex::to_bytes`.
    pub index: Blob,
    /// Shot ranges `(start, end)`, sorted; faces are looked for only there.
    pub shots: Vec<(u32, u32)>,
    /// First frame to look at: 0 or the end of a chunk handed out earlier.
    pub resume: u32,
    /// Frames per checkpoint chunk (chunks end at the next shot start).
    pub chunk: u32,
    /// Look for people too, tied to the faces (`FaceChunk::bodies`).
    #[serde(default)]
    pub bodies: bool,
    /// Faces found before (`as_analysis::tracks::encode_known`): only people
    /// are looked for, and tied to these.
    #[serde(default)]
    pub known: Option<Blob>,
    /// 3D animation: faces are searched as `FaceParams::animated` does.
    #[serde(default)]
    pub animated: bool,
    /// Fast Scan: faces are found as `as_analysis::fast` does (YuNet,
    /// SFace, a few frames per shot) whatever the footage; nobody is looked
    /// for by their body.
    #[serde(default)]
    pub fast: bool,
    /// Anime footage: heads are found and embedded as `as_analysis::anime`
    /// does (anime head/person detectors, CCIP); `bodies`, `known`,
    /// `animated` and `fast` don't apply.
    #[serde(default)]
    pub anime: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbedJob {
    pub id: u64,
    /// The still image file to embed (any format ffmpeg reads).
    pub path: PathBuf,
}

/// A shot's sprite (`as_analysis::ShotSprite`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sprite {
    pub shot: u32,
    pub end: u32,
    pub cells: u32,
    pub jpeg: Blob,
}

impl Request {
    pub fn id(&self) -> Option<u64> {
        match self {
            Self::Shots(j) => Some(j.id),
            Self::Faces(j) => Some(j.id),
            Self::Embed(j) => Some(j.id),
            Self::Cancel { .. } | Self::Shutdown => None,
        }
    }
}

/// A model a job uses and the device it runs on (`as_infer::Device::label`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelDevice {
    pub model: String,
    pub device: String,
}

/// Engine → app.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Reply {
    /// The runtime is loaded; models run on `device` where they can (each
    /// falls back to the CPU on its own, which every job reports: `Devices`).
    Ready {
        device: String,
    },
    /// Where job `id`'s models run, once they are loaded (before its first
    /// progress): on `preferred` each one it can take, on the CPU the rest.
    Devices {
        id: u64,
        preferred: String,
        models: Vec<ModelDevice>,
    },
    Progress {
        id: u64,
        fraction: f64,
    },
    /// Finished signals of frames `start..start + len` (`as_analysis::signals::encode`).
    ShotChunk {
        id: u64,
        start: u32,
        len: u32,
        signals: Blob,
    },
    /// Face tracks of frames `start..end` (`as_analysis::tracks::encode`),
    /// the sprites of its shots, and the people found there
    /// (`as_analysis::bodies::encode`; empty when none were looked for).
    FaceChunk {
        id: u64,
        start: u32,
        end: u32,
        tracks: Blob,
        #[serde(default)]
        sprites: Vec<Sprite>,
        #[serde(default)]
        bodies: Blob,
    },
    /// The vector an `Embed` job got from the still, unit-normalised (768
    /// floats); `Done` follows it.
    Embedding {
        id: u64,
        embedding: Vec<f32>,
    },
    Done {
        id: u64,
    },
    Failed {
        id: u64,
        message: String,
        detail: String,
        cancelled: bool,
    },
    /// The engine can't work at all (runtime or ffmpeg missing) and exits.
    Fatal {
        message: String,
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_travel_as_base64() {
        let r = Reply::ShotChunk { id: 7, start: 2400, len: 2400, signals: Blob(vec![0, 1, 2, 255]) };
        let line = serde_json::to_string(&r).unwrap();
        assert_eq!(line, r#"{"event":"shot_chunk","id":7,"start":2400,"len":2400,"signals":"AAEC/w=="}"#);
        match serde_json::from_str::<Reply>(&line).unwrap() {
            Reply::ShotChunk { signals, .. } => assert_eq!(signals.0, [0, 1, 2, 255]),
            other => panic!("{other:?}"),
        }
        let c: Request = serde_json::from_str(r#"{"op":"cancel","id":3}"#).unwrap();
        assert!(matches!(c, Request::Cancel { id: 3 }));
    }

    #[test]
    fn devices_travel_by_model() {
        let r = Reply::Devices {
            id: 2,
            preferred: "GPU (DirectML)".into(),
            models: vec![ModelDevice { model: "transnetv2".into(), device: "CPU".into() }],
        };
        let line = serde_json::to_string(&r).unwrap();
        assert_eq!(
            line,
            r#"{"event":"devices","id":2,"preferred":"GPU (DirectML)","models":[{"model":"transnetv2","device":"CPU"}]}"#
        );
        assert!(
            matches!(serde_json::from_str::<Reply>(&line).unwrap(), Reply::Devices { models, .. } if models.len() == 1)
        );
    }

    #[test]
    fn people_are_optional_on_the_wire() {
        // A chunk from an engine that looks for no one.
        let chunk: Reply =
            serde_json::from_str(r#"{"event":"face_chunk","id":1,"start":0,"end":9,"tracks":"AQ=="}"#).unwrap();
        assert!(
            matches!(chunk, Reply::FaceChunk { ref bodies, ref sprites, .. } if bodies.0.is_empty() && sprites.is_empty())
        );
        let r = Reply::FaceChunk {
            id: 1,
            start: 0,
            end: 9,
            tracks: Blob(vec![1]),
            sprites: vec![],
            bodies: Blob(vec![1, 0]),
        };
        let back: Reply = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(matches!(back, Reply::FaceChunk { bodies, .. } if bodies.0 == [1, 0]));
    }

    fn info() -> MediaInfo {
        serde_json::from_value(serde_json::json!({
            "container": "matroska,webm", "durationSec": null, "startSec": 0.0, "sizeBytes": null, "bitRate": null,
            "video": {
                "index": 0, "codec": "h264", "profile": null, "pixFmt": "yuv420p", "bitDepth": 8,
                "width": 1920, "height": 1080, "sar": as_core::Rational::new(1, 1),
                "rFrameRate": as_core::Rational::new(24, 1), "avgFrameRate": null,
                "timeBase": as_core::Rational::new(1, 1000), "colorPrimaries": null,
                "colorTransfer": null, "colorSpace": null, "colorRange": null, "fieldOrder": null,
                "rotation": 0, "hdr": "none"
            },
            "audio": [], "subtitles": []
        }))
        .unwrap()
    }

    #[test]
    fn the_anime_flag_defaults_off_and_travels() {
        // An app that predates the flags sends none: every one defaults off.
        let plain = serde_json::json!({
            "op": "faces", "id": 4, "path": "/show/01.mkv",
            "info": serde_json::to_value(info()).unwrap(),
            "index": "AQI=", "shots": [[0, 100]], "resume": 0, "chunk": 100
        });
        match serde_json::from_value::<Request>(plain).unwrap() {
            Request::Faces(j) => {
                assert!(!j.anime && !j.fast && !j.animated && !j.bodies && j.known.is_none())
            }
            other => panic!("{other:?}"),
        }
        let job = FacesJob {
            id: 4,
            path: PathBuf::from("/show/01.mkv"),
            info: info(),
            index: Blob(vec![1, 2]),
            shots: vec![(0, 100)],
            resume: 0,
            chunk: 100,
            bodies: false,
            known: None,
            animated: false,
            fast: false,
            anime: true,
        };
        let line = serde_json::to_string(&Request::Faces(Box::new(job))).unwrap();
        assert!(line.contains(r#""anime":true"#), "{line}");
        match serde_json::from_str::<Request>(&line).unwrap() {
            Request::Faces(j) => assert!(j.anime && j.id == 4),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn embed_jobs_and_their_vectors_round_trip() {
        let r = Request::Embed(Box::new(EmbedJob { id: 9, path: PathBuf::from("portrait.jpg") }));
        let line = serde_json::to_string(&r).unwrap();
        assert_eq!(line, r#"{"op":"embed","id":9,"path":"portrait.jpg"}"#);
        assert_eq!(r.id(), Some(9));
        match serde_json::from_str::<Request>(&line).unwrap() {
            Request::Embed(j) => assert_eq!(j.id, 9),
            other => panic!("{other:?}"),
        }
        let r = Reply::Embedding { id: 9, embedding: vec![0.5, -1.0] };
        let line = serde_json::to_string(&r).unwrap();
        assert_eq!(line, r#"{"event":"embedding","id":9,"embedding":[0.5,-1.0]}"#);
        match serde_json::from_str::<Reply>(&line).unwrap() {
            Reply::Embedding { id: 9, embedding } => assert_eq!(embedding, [0.5, -1.0]),
            other => panic!("{other:?}"),
        }
    }
}
