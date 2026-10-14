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

/// Engine → app.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Reply {
    /// The runtime is loaded; jobs run on `device`.
    Ready {
        device: String,
    },
    Progress {
        id: u64,
        fraction: f64,
    },
    /// Finished signals of frames `start..` (`as_analysis::signals::encode`).
    ShotChunk {
        id: u64,
        start: u32,
        signals: Blob,
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
        let r = Reply::ShotChunk { id: 7, start: 2400, signals: Blob(vec![0, 1, 2, 255]) };
        let line = serde_json::to_string(&r).unwrap();
        assert_eq!(line, r#"{"event":"shot_chunk","id":7,"start":2400,"signals":"AAEC/w=="}"#);
        match serde_json::from_str::<Reply>(&line).unwrap() {
            Reply::ShotChunk { signals, .. } => assert_eq!(signals.0, [0, 1, 2, 255]),
            other => panic!("{other:?}"),
        }
        let c: Request = serde_json::from_str(r#"{"op":"cancel","id":3}"#).unwrap();
        assert!(matches!(c, Request::Cancel { id: 3 }));
    }
}
