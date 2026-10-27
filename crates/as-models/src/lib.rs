//! AI models: the pinned manifest (`models/manifest.toml`) and a store in the
//! app data folder that downloads, resumes and verifies them.
//!
//! A model counts as installed only when its file has the pinned size and a
//! `<file>.verified` marker records the pinned SHA256 — the hash is checked
//! once after download, not on every launch. Downloads go to `<file>.part`
//! and resume with an HTTP range request after an interruption. A model that
//! is one file of a zip archive (`member`) is fetched alone, by the byte
//! ranges the archive's directory gives (see [`zip`]), and unpacked.

mod zip;

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
    /// The file at the pinned source isn't the pinned one any more.
    #[error("An AI model changed where AutoScene downloads it from. Update AutoScene to get it.")]
    Changed(String),
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
            Self::Changed(why) => format!("model source changed: {why}"),
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
    /// The model is this file of the zip archive at `url`; `sha256` and
    /// `size` are the file's, unpacked.
    #[serde(default)]
    pub member: Option<String>,
    pub sha256: String,
    pub size: u64,
    pub license: String,
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    /// Files of models a better one replaced.
    #[serde(default)]
    retired: Vec<String>,
    model: Vec<Model>,
}

static MANIFEST: LazyLock<Manifest> = LazyLock::new(|| {
    toml::from_str::<Manifest>(include_str!("../../../models/manifest.toml")).expect("models/manifest.toml is valid")
});

/// Every model AutoScene knows about.
pub fn manifest() -> &'static [Model] {
    &MANIFEST.model
}

/// Files of models AutoScene no longer uses (see [`ModelStore::prune`]).
pub fn retired() -> &'static [String] {
    &MANIFEST.retired
}

pub fn model(id: &str) -> ModelResult<&'static Model> {
    manifest().iter().find(|m| m.id == id).ok_or_else(|| ModelError::Unknown(id.into()))
}

/// Ids, so code can't misspell them.
pub mod ids {
    pub const TRANSNET: &str = "transnetv2";
    pub const SCRFD: &str = "scrfd-10g";
    /// The face embedder. Media whose faces another one found are found
    /// again (`Project::refresh_outdated_faces`).
    pub const ARCFACE: &str = "arcface-glintr100";
    /// What the face pass needs.
    pub const FACES: [&str; 2] = [SCRFD, ARCFACE];
    /// Fast Scan's detector and embedder (the Scener app's).
    pub const YUNET: &str = "yunet-2023mar";
    pub const SFACE: &str = "sface-2021dec";
    /// What the Fast Scan face pass needs.
    pub const FAST_FACES: [&str; 2] = [YUNET, SFACE];
    /// People (their whole bodies), and how they look (clothes, build).
    pub const PERSONS: &str = "person-ppyoloe";
    pub const PERSON_REID: &str = "person-reid-0277";
    /// What finding people without a face needs, on top of [`FACES`].
    pub const BODIES: [&str; 2] = [PERSONS, PERSON_REID];
}

/// Name lookup, connecting and the response headers each get this long.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// A download that receives nothing for this long fails (and resumes later).
const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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
    pub fn install(&self, id: &str, cancel: &CancelToken, progress: impl FnMut(u64, u64)) -> ModelResult<PathBuf> {
        if let Some(p) = self.installed(id)? {
            return Ok(p);
        }
        self.fetch(model(id)?, cancel, progress)
    }

    fn fetch(&self, m: &Model, cancel: &CancelToken, mut progress: impl FnMut(u64, u64)) -> ModelResult<PathBuf> {
        let id = m.id.as_str();
        fs::create_dir_all(&self.dir)?;
        let part = self.dir.join(format!("{}.part", m.file));
        let agent = agent();
        // The file as it will be kept, and its hash.
        let (ready, got) = match &m.member {
            None => {
                self.download(&agent, m, &part, cancel, &mut progress)?;
                (part.clone(), sha256(&part)?)
            }
            Some(member) => self.download_member(&agent, m, member, &part, cancel, &mut progress)?,
        };
        if got != m.sha256 {
            let _ = fs::remove_file(&part);
            let _ = fs::remove_file(&ready);
            return Err(ModelError::Checksum { id: id.into(), got });
        }
        let path = self.file(m);
        fs::rename(&ready, &path)?;
        if ready != part {
            let _ = fs::remove_file(&part);
        }
        fs::write(self.marker(m), &m.sha256)?;
        tracing::info!(model = id, "model installed");
        Ok(path)
    }

    /// Download (or resume) the whole file of `m` into `part`.
    fn download(
        &self,
        agent: &ureq::Agent,
        m: &Model,
        part: &Path,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(u64, u64),
    ) -> ModelResult<()> {
        let mut have = fs::metadata(part).map(|md| md.len()).unwrap_or(0);
        if have > m.size {
            fs::remove_file(part)?;
            have = 0;
        }
        if have == m.size {
            return Ok(());
        }
        tracing::info!(model = %m.id, resume_from = have, "downloading model");
        let mut req = agent.get(&m.url);
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let resp = req.call().map_err(|e| ModelError::Download(e.to_string()))?;
        // A server that ignores the range sends everything again.
        if have > 0 && resp.status() != 206 {
            have = 0;
        }
        stream(resp, part, have, m.size, &m.id, cancel, progress)
    }

    /// Fetch `member` of the zip archive at `m.url` alone: the archive's
    /// directory (at its end), the member's local header, then its packed
    /// bytes into `part` (resumable), unpacked next to where the model goes.
    /// Returns the unpacked file and its hash.
    fn download_member(
        &self,
        agent: &ureq::Agent,
        m: &Model,
        member: &str,
        part: &Path,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(u64, u64),
    ) -> ModelResult<(PathBuf, String)> {
        let fail = |e: String| ModelError::Download(format!("{} in {}: {e}", m.id, m.url));
        let total = remote_len(agent, &m.url)?;
        let tail_start = total - zip::TAIL.min(total);
        let tail = get_range(agent, &m.url, tail_start, total - tail_start)?;
        let (offset, size) = zip::directory(&tail).map_err(fail)?;
        let dir = match offset.checked_sub(tail_start) {
            Some(at) if offset + size <= total => tail[at as usize..(at + size) as usize].to_vec(),
            _ => get_range(agent, &m.url, offset, size)?,
        };
        let entry = zip::find(&dir, member).map_err(fail)?;
        if entry.size != m.size {
            return Err(ModelError::Changed(format!(
                "{}: {member} in {} is {} bytes, the pinned file {}",
                m.id, m.url, entry.size, m.size
            )));
        }
        // Nothing much bigger than the file is downloaded for it.
        if entry.compressed > zip::packed_at_most(m.size) {
            return Err(ModelError::Changed(format!(
                "{}: {member} in {} takes {} bytes for {}",
                m.id, m.url, entry.compressed, m.size
            )));
        }
        let local = get_range(agent, &m.url, entry.header, zip::LOCAL_LEN)?;
        let data = entry.header + zip::data_offset(&local).map_err(fail)?;

        let mut have = fs::metadata(part).map(|md| md.len()).unwrap_or(0);
        if have > entry.compressed {
            fs::remove_file(part)?;
            have = 0;
        }
        if have < entry.compressed {
            tracing::info!(model = %m.id, member, resume_from = have, "downloading model");
            let (from, to) = (data + have, data + entry.compressed - 1);
            let resp = agent
                .get(&m.url)
                .header("Range", &format!("bytes={from}-{to}"))
                .call()
                .map_err(|e| ModelError::Download(e.to_string()))?;
            if resp.status() != 206 {
                return Err(fail(format!("the server sent {} instead of the member's bytes", resp.status())));
            }
            stream(resp, part, have, entry.compressed, &m.id, cancel, progress)?;
        }
        // Unpacked and hashed in one go — never more than the file and a
        // byte, whatever the data claims; an archive replaced meanwhile (a
        // resumed download of another one) fails the hash and starts over.
        let unpacked = self.dir.join(format!("{}.unpacked", m.file));
        let mut out = Hashing { file: File::create(&unpacked)?, hash: Sha256::new() };
        let packed = std::io::BufReader::new(File::open(part)?.take(entry.compressed));
        let n = match zip::unpack(entry.method, packed, &mut out, m.size + 1) {
            Ok(n) => n,
            // Bytes that don't unpack are damaged: they go, like a wrong hash.
            Err(e) => {
                drop(out);
                let _ = fs::remove_file(part);
                let _ = fs::remove_file(&unpacked);
                return Err(ModelError::Checksum { id: m.id.clone(), got: format!("{member} doesn't unpack: {e}") });
            }
        };
        out.file.sync_all()?;
        let got = if n == m.size { hex(&out.hash.finalize()) } else { format!("{n} bytes unpacked") };
        Ok((unpacked, got))
    }

    /// Delete the files of models a better one replaced (the manifest's
    /// `retired` list), with their `.verified` markers and unfinished
    /// `.part` downloads — nothing else in the folder. Returns the bytes
    /// freed.
    pub fn prune(&self) -> ModelResult<u64> {
        let retired: Vec<&str> = retired().iter().map(String::as_str).collect();
        self.prune_files(&retired)
    }

    fn prune_files(&self, retired: &[&str]) -> ModelResult<u64> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let mut freed = 0;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let file =
                [".verified", ".part", ".unpacked"].iter().find_map(|suffix| name.strip_suffix(suffix)).unwrap_or(name);
            if !retired.contains(&file) || manifest().iter().any(|m| m.file == file) || !entry.file_type()?.is_file() {
                continue;
            }
            let len = entry.metadata().map_or(0, |m| m.len());
            match fs::remove_file(entry.path()) {
                Ok(()) => {
                    freed += len;
                    tracing::info!(file = name, "removed a model that is no longer used");
                }
                Err(e) => tracing::warn!(file = name, "removing a model that is no longer used: {e}"),
            }
        }
        Ok(freed)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_resolve(Some(CONNECT_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_send_request(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(CONNECT_TIMEOUT))
        .build()
        .into()
}

/// Write the body of `resp` to `part` — after its first `have` bytes, which
/// it keeps — until it holds `total`. `progress(done, total)` as bytes come.
fn stream(
    resp: ureq::http::Response<ureq::Body>,
    part: &Path,
    have: u64,
    total: u64,
    id: &str,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(u64, u64),
) -> ModelResult<()> {
    let mut out = OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(part)?;
    // The body is read on its own thread, so cancelling (or a stalled
    // connection) never waits on a socket that has gone quiet.
    let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<Vec<u8>>>(16);
    let mut body = resp.into_body().into_reader();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let r = body.read(&mut buf).map(|n| buf[..n].to_vec());
            let end = !matches!(&r, Ok(b) if !b.is_empty());
            if tx.send(r).is_err() || end {
                return;
            }
        }
    });
    let mut done = have;
    let mut last_data = std::time::Instant::now();
    progress(done, total);
    loop {
        if cancel.is_cancelled() {
            out.flush()?;
            return Err(ModelError::Cancelled);
        }
        let chunk = match rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(r) => r.map_err(|e| ModelError::Download(e.to_string()))?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if last_data.elapsed() > STALL_TIMEOUT {
                    out.flush()?;
                    return Err(ModelError::Download(format!("{id}: no data for {STALL_TIMEOUT:?}")));
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Vec::new(),
        };
        if chunk.is_empty() {
            break;
        }
        last_data = std::time::Instant::now();
        out.write_all(&chunk)?;
        done += chunk.len() as u64;
        progress(done.min(total), total);
    }
    out.sync_all()?;
    if done != total {
        return Err(ModelError::Download(format!("{id}: got {done} of {total} bytes")));
    }
    Ok(())
}

/// Bytes `from..from + len` of the file at `url` (`len` > 0): only those,
/// or an error.
fn get_range(agent: &ureq::Agent, url: &str, from: u64, len: u64) -> ModelResult<Vec<u8>> {
    let failed = |e: String| ModelError::Download(format!("{url} bytes {from}+{len}: {e}"));
    let resp = agent
        .get(url)
        .header("Range", &format!("bytes={from}-{}", from + len.max(1) - 1))
        .call()
        .map_err(|e| failed(e.to_string()))?;
    if resp.status() != 206 {
        return Err(failed(format!("the server answered {} to a range request", resp.status())));
    }
    // One byte more than asked for: the reader errs on reaching its limit
    // before it sees the end.
    let body = resp.into_body().into_with_config().limit(len + 1).read_to_vec().map_err(|e| failed(e.to_string()))?;
    if body.len() as u64 != len {
        return Err(failed(format!("got {} bytes", body.len())));
    }
    Ok(body)
}

/// How big the file at `url` is: the total its server names for its first byte.
fn remote_len(agent: &ureq::Agent, url: &str) -> ModelResult<u64> {
    let failed = |e: String| ModelError::Download(format!("{url}: {e}"));
    let resp = agent.get(url).header("Range", "bytes=0-0").call().map_err(|e| failed(e.to_string()))?;
    let total = resp
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|t| t.trim().parse::<u64>().ok());
    match (resp.status().as_u16(), total) {
        (206, Some(total)) if total > 0 => Ok(total),
        (status, _) => Err(failed(format!("no size in the answer ({status}) to a range request"))),
    }
}

/// A file being written and hashed at once.
struct Hashing {
    file: File,
    hash: Sha256,
}

impl Write for Hashing {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
    Ok(hex(&h.finalize()))
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
        for id in [ids::TRANSNET, ids::SCRFD, ids::ARCFACE, ids::PERSONS, ids::PERSON_REID] {
            assert!(model(id).is_ok(), "{id}");
        }
        assert!(matches!(model("nope"), Err(ModelError::Unknown(_))));
        // Files of one archive land as the files they are.
        for id in ids::BODIES {
            let m = model(id).unwrap();
            assert_eq!(m.member.as_deref(), Some(m.file.as_str()), "{id}");
        }
    }

    /// A server for one file that answers byte ranges, counting what it sent.
    fn serve(data: Vec<u8>) -> (String, std::sync::Arc<std::sync::atomic::AtomicU64>) {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/cheetah.zip", listener.local_addr().unwrap());
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counted = sent.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { return };
                let mut reader = std::io::BufReader::new(conn.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    if let Some(r) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        let (a, b) = r.trim().split_once('-').unwrap();
                        let a: u64 = a.parse().unwrap();
                        let b: u64 = if b.is_empty() { data.len() as u64 - 1 } else { b.parse().unwrap() };
                        range = Some((a, b.min(data.len() as u64 - 1)));
                    }
                }
                let (status, body, extra) = match range {
                    Some((a, b)) => (
                        "206 Partial Content",
                        &data[a as usize..=b as usize],
                        format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()),
                    ),
                    None => ("200 OK", &data[..], String::new()),
                };
                counted.fetch_add(body.len() as u64, std::sync::atomic::Ordering::Relaxed);
                let head =
                    format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", body.len());
                let _ = conn.write_all(head.as_bytes()).and_then(|()| conn.write_all(body));
            }
        });
        (url, sent)
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[test]
    fn a_model_inside_a_zip_is_fetched_alone() {
        // An archive with a big file around the model: only the model's bytes travel.
        let model: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let other = vec![7u8; 2_000_000];
        let zip = zip::tests::archive(
            &[("w600k_r50.onnx", &other, false), ("reid_0265.onnx", &model, true), ("MODEL.LICENSE", b"{}", false)],
            b"",
        );
        let (url, sent) = serve(zip.clone());
        let m = Model {
            id: "test-reid".into(),
            file: "reid_0265.onnx".into(),
            purpose: String::new(),
            url,
            member: Some("reid_0265.onnx".into()),
            sha256: sha(&model),
            size: model.len() as u64,
            license: String::new(),
            source: String::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        // Half of it was there from an interrupted try.
        let entry = {
            let tail = &zip[zip.len() - (zip::TAIL as usize).min(zip.len())..];
            let (offset, size) = zip::directory(tail).unwrap();
            zip::find(&zip[offset as usize..(offset + size) as usize], "reid_0265.onnx").unwrap()
        };
        let data = (entry.header + zip::data_offset(&zip[entry.header as usize..]).unwrap()) as usize;
        let half = entry.compressed as usize / 2;
        fs::write(dir.path().join("reid_0265.onnx.part"), &zip[data..data + half]).unwrap();
        let mut last = (0, 0);
        let path = store.fetch(&m, &CancelToken::new(), |d, t| last = (d, t)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), model);
        assert_eq!(fs::read_to_string(dir.path().join("reid_0265.onnx.verified")).unwrap(), m.sha256);
        assert_eq!(last, (entry.compressed, entry.compressed), "progress counts the packed bytes");
        assert!(
            !dir.path().join("reid_0265.onnx.part").exists() && !dir.path().join("reid_0265.onnx.unpacked").exists()
        );
        let travelled = sent.load(std::sync::atomic::Ordering::Relaxed);
        assert!(
            travelled < entry.compressed + zip::TAIL + 1024,
            "{travelled} bytes for a {} byte member",
            entry.compressed
        );

        // A part left from another archive fails the hash, goes, and the next try works.
        let again = tempfile::tempdir().unwrap();
        let store = ModelStore::new(again.path());
        fs::write(again.path().join("reid_0265.onnx.part"), vec![0u8; half]).unwrap();
        assert!(matches!(store.fetch(&m, &CancelToken::new(), |_, _| {}), Err(ModelError::Checksum { .. })));
        assert!(!again.path().join("reid_0265.onnx.part").exists());
        assert_eq!(fs::read(store.fetch(&m, &CancelToken::new(), |_, _| {}).unwrap()).unwrap(), model);

        // A member that isn't the pinned size: the source changed, nothing is downloaded.
        let changed = Model { size: m.size + 1, ..m };
        let other_dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            ModelStore::new(other_dir.path()).fetch(&changed, &CancelToken::new(), |_, _| {}),
            Err(ModelError::Changed(_))
        ));
    }

    #[test]
    fn prune_removes_only_retired_models() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let current = &model(ids::TRANSNET).unwrap().file;
        // A model of someone's own (a bench export) is not ours to delete.
        let keep = [
            current.clone(),
            format!("{current}.verified"),
            format!("{current}.part"),
            "notes.txt".into(),
            "bench_export.onnx".into(),
        ];
        let gone = ["old_model.onnx", "old_model.onnx.verified", "older.onnx.part"];
        for f in keep.iter().map(String::as_str).chain(gone) {
            fs::write(dir.path().join(f), b"1234").unwrap();
        }
        fs::create_dir(dir.path().join("folder.onnx")).unwrap();
        assert_eq!(store.prune_files(&["old_model.onnx", "older.onnx", "folder.onnx"]).unwrap(), 12);
        for f in &keep {
            assert!(dir.path().join(f).exists(), "{f} kept");
        }
        for f in gone {
            assert!(!dir.path().join(f).exists(), "{f} removed");
        }
        assert!(dir.path().join("folder.onnx").is_dir());
        // A store that was never used has nothing to prune.
        assert_eq!(ModelStore::new(dir.path().join("missing")).prune().unwrap(), 0);
    }

    /// The pinned files from their real source (network: `cargo test -p
    /// as-models -- --ignored`).
    #[test]
    #[ignore = "downloads from the internet"]
    fn the_pinned_body_models_download() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        for id in ids::BODIES {
            let path = store.install(id, &CancelToken::new(), |_, _| {}).unwrap();
            assert_eq!(fs::metadata(&path).unwrap().len(), model(id).unwrap().size, "{id}");
            assert_eq!(store.installed(id).unwrap(), Some(path));
        }
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
