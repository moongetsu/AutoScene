//! AI models: the pinned manifest (`models/manifest.toml`) and a store in the
//! app data folder that downloads, resumes and verifies them.
//!
//! A model counts as installed when its file has the pinned size, a
//! `<file>.verified` marker records the pinned SHA256 (written once its
//! download was checked), and the file still hashes to it: a process hashes
//! a model the first time it asks for it, and again whenever the file
//! changed since. A file that no longer matches loses its marker and is
//! downloaded again. That repairs silent damage (a failing disk, a sync
//! tool, an antivirus restore); it is no lock against a local user who means
//! harm, who could as well replace the runtime libraries the app loads from
//! the same folder tree.
//!
//! Downloads go to `<file>.part` and resume with an HTTP range request after
//! an interruption. One install of a model runs at a time, across processes
//! (`<file>.lock`), and only when its drive has room for it. A model that is
//! one file of a zip archive (`member`) is fetched alone, by the byte ranges
//! the archive's directory gives (see [`zip`]), and unpacked.

mod zip;

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

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
    /// The drive of the models folder hasn't room for a download.
    #[error(
        "There isn't enough free space for the AI model. Free up {} MB on the drive of AutoScene's data folder and try again.",
        missing_mb(.need, .free)
    )]
    NoSpace { dir: PathBuf, need: u64, free: u64 },
    #[error("Cancelled.")]
    Cancelled,
    #[error("{}", as_core::io_message(.0))]
    Io(#[from] std::io::Error),
}

impl ModelError {
    /// Technical description for logs.
    pub fn detail(&self) -> String {
        match self {
            Self::Download(why) => format!("model download failed: {why}"),
            Self::Checksum { id, got } => format!("{id}: sha256 {got} doesn't match the manifest"),
            Self::Changed(why) => format!("model source changed: {why}"),
            Self::NoSpace { dir, need, free } => {
                format!("{}: a model download needs {need} bytes free, the drive has {free}", dir.display())
            }
            Self::Io(e) => format!("model files: {e}"),
            e => e.to_string(),
        }
    }
}

/// Megabytes (rounded up) that `free` lacks of `need`.
fn missing_mb(need: &u64, free: &u64) -> u64 {
    need.saturating_sub(*free).div_ceil(1_000_000)
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
    /// Its licence allows non-commercial use only (the app says so where it
    /// lists the models and where a setting decides which ones it fetches).
    #[serde(default)]
    pub non_commercial: bool,
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
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// A download that receives nothing for this long fails (and resumes later).
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Bytes left free on the drive besides a download: the library's database
/// and the logs live there too.
const SPARE: u64 = 64 << 20;

/// A file's size and time of change, which a hash taken of it holds for.
type Stamp = (u64, Option<SystemTime>);

#[derive(Clone, Debug)]
pub struct ModelStore {
    dir: PathBuf,
    /// Models this process hashed, with the stamp of their file then: they
    /// aren't hashed again until it changes. Shared by the store's clones.
    checked: Arc<Mutex<HashMap<String, Stamp>>>,
    /// How long a download may receive nothing before it fails.
    stall: Duration,
    /// The free bytes on the drive of a folder.
    free_space: fn(&Path) -> std::io::Result<u64>,
}

impl ModelStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), checked: Arc::default(), stall: STALL_TIMEOUT, free_space }
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

    fn checked(&self) -> MutexGuard<'_, HashMap<String, Stamp>> {
        self.checked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Path of an installed model, `None` when it has to be downloaded —
    /// again, when its file no longer hashes to the pinned SHA256 (see the
    /// module doc). The first call for a model in a process reads its file.
    pub fn installed(&self, id: &str) -> ModelResult<Option<PathBuf>> {
        self.ready(model(id)?)
    }

    /// Path of a model whose download finished and was verified, without
    /// hashing its file again: for listing the models, not for loading one.
    pub fn downloaded(&self, id: &str) -> ModelResult<Option<PathBuf>> {
        Ok(self.marked(model(id)?))
    }

    /// The file of `m` when it has the pinned size and its marker the pinned hash.
    fn marked(&self, m: &Model) -> Option<PathBuf> {
        let path = self.file(m);
        let size_ok = fs::metadata(&path).is_ok_and(|md| md.len() == m.size);
        let verified = fs::read_to_string(self.marker(m)).is_ok_and(|s| s.trim() == m.sha256);
        (size_ok && verified).then_some(path)
    }

    /// The file of `m` when it is installed and still the pinned one.
    fn ready(&self, m: &Model) -> ModelResult<Option<PathBuf>> {
        let Some(path) = self.marked(m) else { return Ok(None) };
        let now = stamp(&path)?;
        if self.checked().get(&m.id) == Some(&now) {
            return Ok(Some(path));
        }
        let got = sha256(&path)?;
        if got != m.sha256 {
            let damaged = ModelError::Checksum { id: m.id.clone(), got };
            tracing::warn!(model = %m.id, error = %damaged.detail(), "an installed model is damaged; it is downloaded again");
            // Without its marker, the next install fetches it over the damaged file.
            let _ = fs::remove_file(self.marker(m));
            return Ok(None);
        }
        self.checked().insert(m.id.clone(), now);
        Ok(Some(path))
    }

    /// Download (or resume) `id` and verify it. `progress(done, total)` runs
    /// as bytes arrive. Returns the model's path. Waits while another
    /// install of the same model runs (in this process or another one).
    pub fn install(&self, id: &str, cancel: &CancelToken, progress: impl FnMut(u64, u64)) -> ModelResult<PathBuf> {
        self.install_model(model(id)?, cancel, progress)
    }

    fn install_model(&self, m: &Model, cancel: &CancelToken, progress: impl FnMut(u64, u64)) -> ModelResult<PathBuf> {
        if let Some(p) = self.ready(m)? {
            return Ok(p);
        }
        fs::create_dir_all(&self.dir)?;
        let _lock = self.lock(m, cancel)?;
        // The install that held the lock may have brought the model.
        if let Some(p) = self.ready(m)? {
            return Ok(p);
        }
        self.fetch(m, cancel, progress)
    }

    /// Hold `<file>.lock` of `m` (until the file is dropped), waiting while
    /// another install holds it. Two installs of one model would write the
    /// same `.part` and `.unpacked` files, and an unpacked file the other one
    /// truncated could even pass the hash, which covers what each wrote. The
    /// lock file stays: removing it would let a third install lock a new one
    /// while the second still waits on the old.
    fn lock(&self, m: &Model, cancel: &CancelToken) -> ModelResult<File> {
        let path = self.dir.join(format!("{}.lock", m.file));
        let file = OpenOptions::new().create(true).write(true).truncate(false).open(&path)?;
        let mut waiting = false;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) => {
                    if !waiting {
                        tracing::info!(model = %m.id, "another install of the model runs; waiting for it");
                        waiting = true;
                    }
                    if cancel.is_cancelled() {
                        return Err(ModelError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                // A file system that can't lock (some network or FUSE
                // mounts): installs there go on as before, unguarded.
                Err(TryLockError::Error(e)) => {
                    tracing::warn!(model = %m.id, error = %e, "can't lock the model's install; installing without");
                    return Ok(file);
                }
            }
        }
    }

    fn fetch(&self, m: &Model, cancel: &CancelToken, mut progress: impl FnMut(u64, u64)) -> ModelResult<PathBuf> {
        let id = m.id.as_str();
        let watch = Watch { cancel, stall: self.stall };
        watch.check()?;
        fs::create_dir_all(&self.dir)?;
        let part = self.dir.join(format!("{}.part", m.file));
        let agent = agent(&m.url);
        // The file as it will be kept, and its hash.
        let (ready, got) = match &m.member {
            None => {
                self.download(&agent, m, &part, watch, &mut progress)?;
                (part.clone(), sha256(&part)?)
            }
            Some(member) => self.download_member(&agent, m, member, &part, watch, &mut progress)?,
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
        // Just hashed: not again in this process unless the file changes.
        if let Ok(now) = stamp(&path) {
            self.checked().insert(m.id.clone(), now);
        }
        tracing::info!(model = id, "model installed");
        Ok(path)
    }

    /// Fail before `bytes` are downloaded when the models folder's drive
    /// hasn't room for them (and [`SPARE`]). A drive whose free space can't
    /// be read is let through: a full one then fails the write, which says so.
    fn room_for(&self, bytes: u64) -> ModelResult<()> {
        let need = bytes.saturating_add(SPARE);
        match (self.free_space)(&self.dir) {
            Ok(free) if free < need => Err(ModelError::NoSpace { dir: self.dir.clone(), need, free }),
            Ok(_) => Ok(()),
            Err(e) => {
                tracing::warn!(dir = %self.dir.display(), "free space unknown: {e}");
                Ok(())
            }
        }
    }

    /// Download (or resume) the whole file of `m` into `part`.
    fn download(
        &self,
        agent: &ureq::Agent,
        m: &Model,
        part: &Path,
        watch: Watch,
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
        self.room_for(m.size - have)?;
        tracing::info!(model = %m.id, resume_from = have, "downloading model");
        let mut req = agent.get(&m.url);
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        watch.check()?;
        let resp = req.call().map_err(|e| ModelError::Download(e.to_string()))?;
        // A server that ignores the range sends everything again.
        if have > 0 && resp.status() != 206 {
            have = 0;
        }
        stream(resp, part, have, m.size, &m.id, watch, progress)
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
        watch: Watch,
        progress: &mut dyn FnMut(u64, u64),
    ) -> ModelResult<(PathBuf, String)> {
        let fail = |e: String| ModelError::Download(format!("{} in {}: {e}", m.id, m.url));
        // Room for the packed bytes still to come and the unpacked file
        // beside them, before anything travels: as many as a member of the
        // pinned size can take (the directory only tells later how many).
        let have = fs::metadata(part).map(|md| md.len()).unwrap_or(0);
        self.room_for(zip::packed_at_most(m.size).saturating_sub(have).saturating_add(m.size))?;
        let total = remote_len(agent, &m.url, watch)?;
        let tail_start = total - zip::TAIL.min(total);
        let tail = get_range(agent, &m.url, tail_start, total - tail_start, watch)?;
        let (offset, size) = zip::directory(&tail).map_err(fail)?;
        let dir = match offset.checked_sub(tail_start) {
            Some(at) if offset + size <= total => tail[at as usize..(at + size) as usize].to_vec(),
            _ => get_range(agent, &m.url, offset, size, watch)?,
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
        let local = get_range(agent, &m.url, entry.header, zip::LOCAL_LEN, watch)?;
        let data = entry.header + zip::data_offset(&local).map_err(fail)?;

        let mut have = fs::metadata(part).map(|md| md.len()).unwrap_or(0);
        if have > entry.compressed {
            fs::remove_file(part)?;
            have = 0;
        }
        if have < entry.compressed {
            tracing::info!(model = %m.id, member, resume_from = have, "downloading model");
            let (from, to) = (data + have, data + entry.compressed - 1);
            watch.check()?;
            let resp = agent
                .get(&m.url)
                .header("Range", &format!("bytes={from}-{to}"))
                .call()
                .map_err(|e| ModelError::Download(e.to_string()))?;
            if resp.status() != 206 {
                return Err(fail(format!("the server sent {} instead of the member's bytes", resp.status())));
            }
            stream(resp, part, have, entry.compressed, &m.id, watch, progress)?;
        }
        // Unpacked and hashed in one go — never more than the file and a
        // byte, whatever the data claims; an archive replaced meanwhile (a
        // resumed download of another one) fails the hash and starts over.
        // The hash covers what this install wrote, which is what the file
        // holds because no other install runs beside it (`lock`).
        let unpacked = self.dir.join(format!("{}.unpacked", m.file));
        let mut out = Hashing::new(File::create(&unpacked)?);
        let n = match unpack_member(&entry, m, member, part, &mut out) {
            Ok(n) => n,
            Err(e) => {
                drop(out);
                let _ = fs::remove_file(&unpacked);
                return Err(e);
            }
        };
        out.file.sync_all()?;
        let got = if n == m.size { hex(&out.hash.finalize()) } else { format!("{n} bytes unpacked") };
        Ok((unpacked, got))
    }

    /// Delete the files of models a better one replaced (the manifest's
    /// `retired` list), with their `.verified` markers, `.lock` files and
    /// unfinished `.part` downloads — nothing else in the folder. Returns
    /// the bytes freed.
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
            let file = [".verified", ".part", ".unpacked", ".lock"]
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
                .unwrap_or(name);
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

/// The size and time of change of the file at `path`.
fn stamp(path: &Path) -> std::io::Result<Stamp> {
    let md = fs::metadata(path)?;
    Ok((md.len(), md.modified().ok()))
}

/// Unpack `entry` (`member` of `m`'s archive), whose packed bytes are in
/// `part`, into `out`. Bytes that don't unpack are damaged: they go, like a
/// wrong hash. A write that fails is the disk's (full, most likely), not
/// the download's: its bytes stay for the next try.
fn unpack_member<W: Write>(
    entry: &zip::Entry,
    m: &Model,
    member: &str,
    part: &Path,
    out: &mut Hashing<W>,
) -> ModelResult<u64> {
    let packed = std::io::BufReader::new(File::open(part)?.take(entry.compressed));
    match zip::unpack(entry.method, packed, out, m.size + 1) {
        Ok(n) => Ok(n),
        Err(e) if out.write_failed => Err(ModelError::Io(e)),
        Err(e) => {
            let _ = fs::remove_file(part);
            Err(ModelError::Checksum { id: m.id.clone(), got: format!("{member} doesn't unpack: {e}") })
        }
    }
}

/// The agent for a model's requests. It never follows a model on https to
/// a plain http address: the hash would still catch a forged file, but
/// nothing of it has to travel in the clear.
fn agent(url: &str) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(url.starts_with("https://"))
        .timeout_resolve(Some(CONNECT_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_send_request(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(CONNECT_TIMEOUT))
        .build()
        .into()
}

/// How long a small request (a file's size, a few bytes of an archive) may
/// take in all, its body included: a minute, and a second per 16 KiB. Never
/// set on the agent: a whole model takes as long as the line needs.
fn small_request_budget(len: u64) -> Duration {
    Duration::from_secs(60 + len / 16_384)
}

/// What ends a transfer early: a cancel, or a connection quiet for `stall`.
#[derive(Clone, Copy)]
struct Watch<'a> {
    cancel: &'a CancelToken,
    stall: Duration,
}

impl Watch<'_> {
    /// `Cancelled` once cancelled; checked before every request.
    fn check(&self) -> ModelResult<()> {
        if self.cancel.is_cancelled() { Err(ModelError::Cancelled) } else { Ok(()) }
    }
}

/// Hand the body of `resp` to `sink` as it comes. The body is read on its
/// own thread, so cancelling (or a stalled connection) never waits on a
/// socket that has gone quiet. `what` names the transfer in errors.
fn pump(
    resp: ureq::http::Response<ureq::Body>,
    what: &str,
    watch: Watch,
    mut sink: impl FnMut(&[u8]) -> ModelResult<()>,
) -> ModelResult<()> {
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
    let mut last_data = Instant::now();
    loop {
        watch.check()?;
        let chunk = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(r) => r.map_err(|e| ModelError::Download(format!("{what}: {e}")))?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if last_data.elapsed() > watch.stall {
                    return Err(ModelError::Download(format!("{what}: no data for {:?}", watch.stall)));
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        if chunk.is_empty() {
            return Ok(());
        }
        last_data = Instant::now();
        sink(&chunk)?;
    }
}

/// Write the body of `resp` to `part` — after its first `have` bytes, which
/// it keeps — until it holds `total`. `progress(done, total)` as bytes come.
fn stream(
    resp: ureq::http::Response<ureq::Body>,
    part: &Path,
    have: u64,
    total: u64,
    id: &str,
    watch: Watch,
    progress: &mut dyn FnMut(u64, u64),
) -> ModelResult<()> {
    let mut out = OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(part)?;
    let mut done = have;
    progress(done, total);
    pump(resp, id, watch, |chunk| {
        out.write_all(chunk)?;
        done += chunk.len() as u64;
        progress(done.min(total), total);
        Ok(())
    })?;
    out.sync_all()?;
    if done != total {
        return Err(ModelError::Download(format!("{id}: got {done} of {total} bytes")));
    }
    Ok(())
}

/// Bytes `from..from + len` of the file at `url` (`len` > 0): only those,
/// or an error.
fn get_range(agent: &ureq::Agent, url: &str, from: u64, len: u64, watch: Watch) -> ModelResult<Vec<u8>> {
    let what = format!("{url} bytes {from}+{len}");
    let failed = |e: String| ModelError::Download(format!("{what}: {e}"));
    watch.check()?;
    let resp = agent
        .get(url)
        .header("Range", &format!("bytes={from}-{}", from + len.max(1) - 1))
        .config()
        .timeout_global(Some(small_request_budget(len)))
        .build()
        .call()
        .map_err(|e| failed(e.to_string()))?;
    if resp.status() != 206 {
        return Err(failed(format!("the server answered {} to a range request", resp.status())));
    }
    let mut body = Vec::new();
    pump(resp, &what, watch, |chunk| {
        if body.len() as u64 + chunk.len() as u64 > len {
            return Err(failed(format!("more than {len} bytes")));
        }
        body.extend_from_slice(chunk);
        Ok(())
    })?;
    if body.len() as u64 != len {
        return Err(failed(format!("got {} bytes", body.len())));
    }
    Ok(body)
}

/// How big the file at `url` is: the total its server names for its first byte.
fn remote_len(agent: &ureq::Agent, url: &str, watch: Watch) -> ModelResult<u64> {
    let failed = |e: String| ModelError::Download(format!("{url}: {e}"));
    watch.check()?;
    let resp = agent
        .get(url)
        .header("Range", "bytes=0-0")
        .config()
        .timeout_global(Some(small_request_budget(1)))
        .build()
        .call()
        .map_err(|e| failed(e.to_string()))?;
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
struct Hashing<W = File> {
    file: W,
    hash: Sha256,
    /// A write failed: the trouble is the disk's, not the data's.
    write_failed: bool,
}

impl<W> Hashing<W> {
    fn new(file: W) -> Self {
        Self { file, hash: Sha256::new(), write_failed: false }
    }
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(buf).inspect_err(|_| self.write_failed = true)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// The bytes the current user may still write on the drive of `dir`.
#[cfg(windows)]
fn free_space(dir: &Path) -> std::io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    // SAFETY: a NUL-terminated wide path and a valid out pointer; the totals
    // it isn't asked for may be null.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(free) }
}

/// The bytes an unprivileged user may still write on the drive of `dir`.
#[cfg(unix)]
#[allow(clippy::useless_conversion)] // the fields are 32 bits wide on some systems, 64 on others
fn free_space(dir: &Path) -> std::io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: a NUL-terminated path and a buffer statvfs fills on success.
    if unsafe { libc::statvfs(path.as_ptr(), st.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: statvfs succeeded, so it filled the buffer.
    let st = unsafe { st.assume_init() };
    Ok(u64::from(st.f_bavail).saturating_mul(u64::from(st.f_frsize)))
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
    use std::io::BufRead;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[test]
    fn manifest_is_complete() {
        assert!(!manifest().is_empty());
        for m in manifest() {
            assert_eq!(m.sha256.len(), 64, "{}: sha256", m.id);
            assert!(m.url.starts_with("https://"), "{}: url", m.id);
            assert!(m.size > 0, "{}: size", m.id);
            assert!(!m.license.is_empty(), "{}: license", m.id);
            assert_eq!(
                m.non_commercial,
                m.license.to_lowercase().contains("non-commercial"),
                "{}: non_commercial says what the licence says",
                m.id
            );
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
        // The full face pass and the people are InsightFace's, for
        // non-commercial use; the shots and Fast Scan aren't restricted.
        for id in ids::FACES.into_iter().chain(ids::BODIES) {
            assert!(model(id).unwrap().non_commercial, "{id}");
        }
        for id in ids::FAST_FACES.into_iter().chain([ids::TRANSNET]) {
            assert!(!model(id).unwrap().non_commercial, "{id}");
        }
    }

    /// The range a request's headers ask for, within a file of `len` bytes
    /// (`None`: the whole file).
    fn read_range(reader: &mut impl BufRead, len: u64) -> Option<(u64, u64)> {
        let mut range = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                return range;
            }
            if let Some(r) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                let (a, b) = r.trim().split_once('-').unwrap();
                let a: u64 = a.parse().unwrap();
                let b: u64 = if b.is_empty() { len - 1 } else { b.parse().unwrap() };
                range = Some((a, b.min(len - 1)));
            }
        }
    }

    /// A server for one file that answers byte ranges, counting what it sent.
    fn serve(data: Vec<u8>) -> (String, Arc<AtomicU64>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/cheetah.zip", listener.local_addr().unwrap());
        let sent = Arc::new(AtomicU64::new(0));
        let counted = sent.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { return };
                let mut reader = std::io::BufReader::new(conn.try_clone().unwrap());
                let (status, body, extra) = match read_range(&mut reader, data.len() as u64) {
                    Some((a, b)) => (
                        "206 Partial Content",
                        &data[a as usize..=b as usize],
                        format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()),
                    ),
                    None => ("200 OK", &data[..], String::new()),
                };
                counted.fetch_add(body.len() as u64, Ordering::Relaxed);
                let head =
                    format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n", body.len());
                let _ = conn.write_all(head.as_bytes()).and_then(|()| conn.write_all(body));
            }
        });
        (url, sent)
    }

    /// A server that answers every range request with its headers and half
    /// its bytes, then holds the connection without a word more.
    fn serve_stalling(data: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/cheetah.zip", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { return };
                let mut reader = std::io::BufReader::new(conn.try_clone().unwrap());
                let (a, b) = read_range(&mut reader, data.len() as u64).unwrap_or((0, data.len() as u64 - 1));
                let body = &data[a as usize..=b as usize];
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\n\r\n",
                    body.len(),
                    data.len()
                );
                let _ = conn.write_all(head.as_bytes()).and_then(|()| conn.write_all(&body[..body.len() / 2]));
                held.push(conn);
            }
        });
        url
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    /// Bytes that look like a model's (nothing a zip can shrink much).
    fn model_bytes(len: u32) -> Vec<u8> {
        (0..len).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect()
    }

    /// A model of `bytes` at `url`, kept as `file`.
    fn test_model(file: &str, url: String, member: Option<&str>, bytes: &[u8]) -> Model {
        Model {
            id: format!("test-{file}"),
            file: file.into(),
            purpose: String::new(),
            url,
            member: member.map(Into::into),
            sha256: sha(bytes),
            size: bytes.len() as u64,
            license: String::new(),
            non_commercial: false,
            source: String::new(),
        }
    }

    /// An archive with a big file around `model` (`reid_0265.onnx`).
    fn archive_around(model: &[u8]) -> Vec<u8> {
        let other = vec![7u8; 2_000_000];
        zip::tests::archive(
            &[("w600k_r50.onnx", &other, false), ("reid_0265.onnx", model, true), ("MODEL.LICENSE", b"{}", false)],
            b"",
        )
    }

    #[test]
    fn a_model_inside_a_zip_is_fetched_alone() {
        // An archive with a big file around the model: only the model's bytes travel.
        let model = model_bytes(300_000);
        let zip = archive_around(&model);
        let (url, sent) = serve(zip.clone());
        let m = test_model("reid_0265.onnx", url, Some("reid_0265.onnx"), &model);
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        // Cancelled before it starts: nothing travels.
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert!(matches!(store.fetch(&m, &cancelled, |_, _| {}), Err(ModelError::Cancelled)));
        assert_eq!(sent.load(Ordering::Relaxed), 0);
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
        let travelled = sent.load(Ordering::Relaxed);
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
    fn a_quiet_server_fails_the_download_and_a_cancel_ends_it_at_once() {
        // The archive's end comes halfway, then nothing.
        let model = model_bytes(300_000);
        let m = test_model("reid_0265.onnx", serve_stalling(archive_around(&model)), Some("reid_0265.onnx"), &model);
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore { stall: Duration::from_millis(300), ..ModelStore::new(dir.path()) };
        let started = Instant::now();
        let r = store.fetch(&m, &CancelToken::new(), |_, _| {});
        assert!(matches!(&r, Err(ModelError::Download(why)) if why.contains("no data")), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());

        // A cancel doesn't wait for the stall.
        let store = ModelStore::new(dir.path());
        let cancel = CancelToken::new();
        let later = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            later.cancel();
        });
        let started = Instant::now();
        assert!(matches!(store.fetch(&m, &cancel, |_, _| {}), Err(ModelError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }

    #[test]
    fn a_model_on_https_is_never_fetched_in_the_clear() {
        assert!(agent("https://huggingface.co/immich-app/buffalo_l/resolve/x/model.onnx").config().https_only());
        // Only a test server speaks plain http.
        assert!(!agent("http://127.0.0.1:1/cheetah.zip").config().https_only());
    }

    #[test]
    fn a_download_needs_room_on_the_drive() {
        fn one_mb(_: &Path) -> std::io::Result<u64> {
            Ok(1 << 20)
        }
        fn unknown(_: &Path) -> std::io::Result<u64> {
            Err(std::io::Error::other("no such call"))
        }
        let model = model_bytes(300_000);
        let (url, sent) = serve(model.clone());
        let m = test_model("room.onnx", url, None, &model);
        let dir = tempfile::tempdir().unwrap();
        let full = ModelStore { free_space: one_mb, ..ModelStore::new(dir.path()) };
        let e = full.fetch(&m, &CancelToken::new(), |_, _| {}).unwrap_err();
        assert!(
            matches!(e, ModelError::NoSpace { need, free, .. } if need == m.size + SPARE && free == 1 << 20),
            "{e:?}"
        );
        assert_eq!(
            e.to_string(),
            "There isn't enough free space for the AI model. Free up 67 MB on the drive of AutoScene's data folder and try again."
        );
        assert_eq!(sent.load(Ordering::Relaxed), 0, "nothing travels");

        // A member needs room for its packed bytes and the file, before anything travels.
        let (url, sent) = serve(archive_around(&model));
        let member = test_model("reid_0265.onnx", url, Some("reid_0265.onnx"), &model);
        let e = full.fetch(&member, &CancelToken::new(), |_, _| {}).unwrap_err();
        let need = zip::packed_at_most(member.size) + member.size + SPARE;
        assert!(matches!(e, ModelError::NoSpace { need: n, .. } if n == need), "{e:?}");
        assert_eq!(sent.load(Ordering::Relaxed), 0, "nothing travels");

        // A drive whose free space can't be read lets the download through.
        let unread = ModelStore { free_space: unknown, ..ModelStore::new(dir.path()) };
        assert_eq!(fs::read(unread.fetch(&m, &CancelToken::new(), |_, _| {}).unwrap()).unwrap(), model);
        // This drive has room.
        assert!(free_space(dir.path()).unwrap() > 0);
    }

    /// A writer on a full disk.
    struct FullDisk;

    impl Write for FullDisk {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::StorageFull.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_full_disk_says_so_and_keeps_the_download() {
        let e = ModelError::from(std::io::Error::from(std::io::ErrorKind::StorageFull));
        assert_eq!(e.to_string(), "The disk is full.");
        assert!(e.detail().starts_with("model files: "), "{}", e.detail());

        // Unpacking onto a full disk: the downloaded bytes stay for the next try.
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("full.onnx.part");
        let model = model_bytes(1000);
        fs::write(&part, &model).unwrap();
        let m = test_model("full.onnx", String::new(), Some("full.onnx"), &model);
        let stored = zip::Entry { method: zip::Method::Stored, compressed: 1000, size: 1000, header: 0 };
        let r = unpack_member(&stored, &m, "full.onnx", &part, &mut Hashing::new(FullDisk));
        assert!(matches!(&r, Err(ModelError::Io(e)) if e.kind() == std::io::ErrorKind::StorageFull), "{r:?}");
        assert!(part.exists());
        // Bytes that don't unpack are damaged, and go.
        fs::write(&part, [0xFFu8; 1000]).unwrap();
        let deflated = zip::Entry { method: zip::Method::Deflated, ..stored };
        let r = unpack_member(&deflated, &m, "full.onnx", &part, &mut Hashing::new(Vec::new()));
        assert!(matches!(r, Err(ModelError::Checksum { .. })), "{r:?}");
        assert!(!part.exists());
    }

    #[test]
    fn one_install_of_a_model_runs_at_a_time() {
        let model = model_bytes(300_000);
        let (url, sent) = serve(model.clone());
        let m = test_model("lock.onnx", url, None, &model);
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        // Another install (another process, say) holds the model's lock.
        let held = File::create(dir.path().join("lock.onnx.lock")).unwrap();
        held.lock().unwrap();
        let waiting = {
            let (store, m) = (store.clone(), m.clone());
            std::thread::spawn(move || store.install_model(&m, &CancelToken::new(), |_, _| {}))
        };
        std::thread::sleep(Duration::from_millis(400));
        assert!(!waiting.is_finished(), "it waits");
        assert_eq!(sent.load(Ordering::Relaxed), 0, "nothing travels while the other install runs");
        // The other install brings the model and lets go: this one finds it.
        fs::write(dir.path().join("lock.onnx"), &model).unwrap();
        fs::write(dir.path().join("lock.onnx.verified"), &m.sha256).unwrap();
        held.unlock().unwrap();
        assert_eq!(waiting.join().unwrap().unwrap(), dir.path().join("lock.onnx"));
        assert_eq!(sent.load(Ordering::Relaxed), 0, "and downloads nothing");
        assert!(dir.path().join("lock.onnx.lock").exists(), "the lock file stays");

        // An install that waits can be cancelled.
        let other = test_model("other.onnx", m.url.clone(), None, &model);
        let held = File::create(dir.path().join("other.onnx.lock")).unwrap();
        held.lock().unwrap();
        let cancel = CancelToken::new();
        let waiting = {
            let (store, cancel) = (store.clone(), cancel.clone());
            std::thread::spawn(move || store.install_model(&other, &cancel, |_, _| {}))
        };
        std::thread::sleep(Duration::from_millis(200));
        cancel.cancel();
        assert!(matches!(waiting.join().unwrap(), Err(ModelError::Cancelled)));
        assert_eq!(sent.load(Ordering::Relaxed), 0);
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
            format!("{current}.lock"),
            "notes.txt".into(),
            "bench_export.onnx".into(),
        ];
        let gone = ["old_model.onnx", "old_model.onnx.verified", "old_model.onnx.lock", "older.onnx.part"];
        for f in keep.iter().map(String::as_str).chain(gone) {
            fs::write(dir.path().join(f), b"1234").unwrap();
        }
        fs::create_dir(dir.path().join("folder.onnx")).unwrap();
        assert_eq!(store.prune_files(&["old_model.onnx", "older.onnx", "folder.onnx"]).unwrap(), 16);
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
    fn needs_size_marker_and_content_to_count_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let m = model(ids::TRANSNET).unwrap();
        let marker = dir.path().join(format!("{}.verified", m.file));
        assert_eq!(store.installed(ids::TRANSNET).unwrap(), None);
        // Right size but never verified: not installed.
        File::create(dir.path().join(&m.file)).unwrap().set_len(m.size).unwrap();
        assert_eq!(store.installed(ids::TRANSNET).unwrap(), None);
        // A marker beside content that isn't the pinned one: not installed
        // either, and the marker goes, so the next install fetches it again.
        fs::write(&marker, &m.sha256).unwrap();
        assert_eq!(
            store.downloaded(ids::TRANSNET).unwrap(),
            Some(dir.path().join(&m.file)),
            "the listing doesn't hash"
        );
        assert_eq!(store.installed(ids::TRANSNET).unwrap(), None);
        assert!(!marker.exists());
    }

    #[test]
    fn a_process_hashes_a_model_once_and_again_when_it_changes() {
        let model = model_bytes(100_000);
        let m = test_model("check.onnx", String::new(), None, &model);
        let dir = tempfile::tempdir().unwrap();
        let (path, marker) = (dir.path().join("check.onnx"), dir.path().join("check.onnx.verified"));
        let install = || {
            fs::write(&path, &model).unwrap();
            fs::write(&marker, &m.sha256).unwrap();
        };
        let touch = |at: SystemTime| File::options().write(true).open(&path).unwrap().set_modified(at).unwrap();
        install();
        let store = ModelStore::new(dir.path());
        assert_eq!(store.ready(&m).unwrap(), Some(path.clone()));
        // Damaged in place, its size and time of change kept: this process
        // hashed the file already and doesn't again…
        let when = fs::metadata(&path).unwrap().modified().unwrap();
        let mut damaged = model.clone();
        damaged[500] ^= 1;
        fs::write(&path, &damaged).unwrap();
        touch(when);
        assert_eq!(store.clone().ready(&m).unwrap(), Some(path.clone()));
        // …but a new one does (the engine's next start, the app's next
        // launch): the marker goes, and the model is downloaded again.
        assert_eq!(ModelStore::new(dir.path()).ready(&m).unwrap(), None);
        assert!(!marker.exists());

        // A file that changed is hashed again in any process.
        install();
        touch(when + Duration::from_secs(10));
        assert_eq!(store.ready(&m).unwrap(), Some(path.clone()));
        fs::write(&path, &damaged).unwrap();
        touch(when + Duration::from_secs(20));
        assert_eq!(store.ready(&m).unwrap(), None);
        assert!(!marker.exists());
    }
}
