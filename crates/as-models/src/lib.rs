//! AI models: the pinned manifest (`models/manifest.toml`) and a store in the
//! app data folder that downloads, resumes and verifies them.
//!
//! A model counts as installed only when its file has the pinned size and a
//! `<file>.verified` marker records the pinned SHA256 — the hash is checked
//! once after download, not on every launch. Downloads go to `<file>.part`
//! and resume with an HTTP range request after an interruption.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use as_core::CancelToken;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub type ModelResult<T> = Result<T, ModelError>;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("Unknown model {0}.")]
    Unknown(String),
    #[error("The AI model couldn't be downloaded. Check the internet connection and try again.")]
    Download(String),
    #[error("The downloaded AI model is damaged. It will be downloaded again.")]
    Checksum { id: String, got: String },
    #[error("Cancelled.")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ModelError {
    /// Technical description for logs.
    pub fn detail(&self) -> String {
        match self {
            Self::Download(why) => format!("model download failed: {why}"),
            Self::Checksum { id, got } => format!("{id}: sha256 {got} doesn't match the manifest"),
            e => e.to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Model {
    pub id: String,
    pub file: String,
    pub purpose: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub license: String,
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    model: Vec<Model>,
}

static MANIFEST: LazyLock<Vec<Model>> = LazyLock::new(|| {
    toml::from_str::<Manifest>(include_str!("../../../models/manifest.toml"))
        .expect("models/manifest.toml is valid")
        .model
});

/// Every model AutoScene knows about.
pub fn manifest() -> &'static [Model] {
    &MANIFEST
}

pub fn model(id: &str) -> ModelResult<&'static Model> {
    manifest().iter().find(|m| m.id == id).ok_or_else(|| ModelError::Unknown(id.into()))
}

/// Ids, so code can't misspell them.
pub mod ids {
    pub const TRANSNET: &str = "transnetv2";
}

#[derive(Clone, Debug)]
pub struct ModelStore {
    dir: PathBuf,
}

impl ModelStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, m: &Model) -> PathBuf {
        self.dir.join(&m.file)
    }

    fn marker(&self, m: &Model) -> PathBuf {
        self.dir.join(format!("{}.verified", m.file))
    }

    /// Path of an installed model, `None` when it still has to be downloaded.
    pub fn installed(&self, id: &str) -> ModelResult<Option<PathBuf>> {
        let m = model(id)?;
        let path = self.file(m);
        let size_ok = fs::metadata(&path).is_ok_and(|md| md.len() == m.size);
        let verified = fs::read_to_string(self.marker(m)).is_ok_and(|s| s.trim() == m.sha256);
        Ok((size_ok && verified).then_some(path))
    }

    /// Download (or resume) `id` and verify it. `progress(done, total)` runs
    /// as bytes arrive. Returns the model's path.
    pub fn install(&self, id: &str, cancel: &CancelToken, mut progress: impl FnMut(u64, u64)) -> ModelResult<PathBuf> {
        if let Some(p) = self.installed(id)? {
            return Ok(p);
        }
        let m = model(id)?;
        fs::create_dir_all(&self.dir)?;
        let part = self.dir.join(format!("{}.part", m.file));
        let mut have = fs::metadata(&part).map(|md| md.len()).unwrap_or(0);
        if have > m.size {
            fs::remove_file(&part)?;
            have = 0;
        }
        if have < m.size {
            tracing::info!(model = id, resume_from = have, "downloading model");
            let mut req = ureq::get(&m.url);
            if have > 0 {
                req = req.header("Range", &format!("bytes={have}-"));
            }
            let resp = req.call().map_err(|e| ModelError::Download(e.to_string()))?;
            // A server that ignores the range sends everything again.
            if have > 0 && resp.status() != 206 {
                have = 0;
            }
            let mut out =
                OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(&part)?;
            let mut body = resp.into_body().into_reader();
            let mut buf = vec![0u8; 1 << 16];
            let mut done = have;
            progress(done, m.size);
            loop {
                if cancel.is_cancelled() {
                    out.flush()?;
                    return Err(ModelError::Cancelled);
                }
                let n = body.read(&mut buf).map_err(|e| ModelError::Download(e.to_string()))?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])?;
                done += n as u64;
                progress(done.min(m.size), m.size);
            }
            out.sync_all()?;
            if done != m.size {
                return Err(ModelError::Download(format!("{id}: got {done} of {} bytes", m.size)));
            }
        }
        let got = sha256(&part)?;
        if got != m.sha256 {
            let _ = fs::remove_file(&part);
            return Err(ModelError::Checksum { id: id.into(), got });
        }
        let path = self.file(m);
        fs::rename(&part, &path)?;
        fs::write(self.marker(m), &m.sha256)?;
        tracing::info!(model = id, "model installed");
        Ok(path)
    }
}

fn sha256(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_complete() {
        assert!(!manifest().is_empty());
        for m in manifest() {
            assert_eq!(m.sha256.len(), 64, "{}: sha256", m.id);
            assert!(m.url.starts_with("https://"), "{}: url", m.id);
            assert!(m.size > 0, "{}: size", m.id);
        }
        assert!(model(ids::TRANSNET).is_ok());
        assert!(matches!(model("nope"), Err(ModelError::Unknown(_))));
    }

    #[test]
    fn needs_size_and_marker_to_count_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let m = model(ids::TRANSNET).unwrap();
        assert_eq!(store.installed(ids::TRANSNET).unwrap(), None);
        // Right size but never verified: not installed.
        File::create(dir.path().join(&m.file)).unwrap().set_len(m.size).unwrap();
        assert_eq!(store.installed(ids::TRANSNET).unwrap(), None);
        fs::write(dir.path().join(format!("{}.verified", m.file)), &m.sha256).unwrap();
        assert_eq!(store.installed(ids::TRANSNET).unwrap(), Some(dir.path().join(&m.file)));
    }
}
