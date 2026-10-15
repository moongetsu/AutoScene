use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use as_api::{
    ClipRequest, ExportPlan, ExportRequest, ExportStatus, ExportSummary, FrameMap, ImportCandidate, MediaItem,
    MediaStatus, PresetSize, ProjectKind, ProjectSummary, ProxyState, Relink, RelinkSummary,
};
use as_core::{CancelToken, ExportId, FrameIndex, FrameRange, MediaId, ProjectId};
use as_db::{IngestRecord, NewExportItem, ProjectDb};
use as_export::{ClipJob, ExportPreset, estimate_bytes, export_clip};
use as_media::poster::{StillSize, extract_frame_jpegs};
use as_media::{MediaInfo, MediaResult, Source, Tools, build_frame_index, idet, probe, proxy};
use parking_lot::Mutex;

use crate::naming::{CLIP_TEMPLATE, NameParts, clip_timecode, is_drop_frame, render_name, safe_component};
use crate::{ProjectError, ProjectResult};

pub(crate) const PROJECT_DB: &str = "project.db";
/// Posters: 720p for full-bleed artwork, and a small one that covers a
/// 480×270 tile (sharp in a 300 px card at 150 % display scaling).
const POSTER: StillSize = StillSize::Height(720);
const POSTER_SMALL: StillSize = StillSize::Cover(480, 270);
/// How alike a file found again must look at the poster's frame to keep its
/// analysis (SSIM: the same frame scores about 0.95, another picture far less).
const SAME_PICTURE: f64 = 0.85;

/// Progress callback: `(stage, fraction 0..1)`.
pub type Progress<'a> = &'a dyn Fn(&str, f64);

/// Work a project still owes after a quit, a crash or an update.
#[derive(Debug, Default)]
pub struct Recovery {
    /// Interrupted mid-ingest; back in `queued`.
    pub ingest: Vec<MediaId>,
    /// Still waiting for a playback proxy.
    pub proxies: Vec<MediaId>,
    /// Ready, with posters from before the current sizes.
    pub posters: Vec<MediaId>,
}

/// An open project.
pub struct Project {
    pub id: ProjectId,
    pub dir: PathBuf,
    db: ProjectDb,
    indexes: Mutex<HashMap<MediaId, Arc<FrameIndex>>>,
    /// Latest proxy build per media. A replaced build that finishes (or
    /// fails) late must not overwrite the state of the one that replaced it.
    proxy_builds: Mutex<HashMap<MediaId, u64>>,
    /// One clustering of the cast at a time.
    pub(crate) clustering: Mutex<()>,
    /// One poster decode at a time: an ingest and the poster upkeep of the
    /// same media would write the same files.
    posters: Mutex<()>,
    /// Portraits being made and ones that failed (see `cast::PortraitWork`).
    pub(crate) portraits: Mutex<crate::cast::PortraitWork>,
}

impl Project {
    pub(crate) fn create(s: &ProjectSummary) -> ProjectResult<Self> {
        let dir = PathBuf::from(&s.dir);
        let db = ProjectDb::open(&dir.join(PROJECT_DB))?;
        // The folder describes itself, so it can be re-opened without library.db.
        let name = |v: serde_json::Value| v.as_str().unwrap_or_default().to_string();
        for (k, v) in [
            ("id", s.id.to_string()),
            ("title", s.title.clone()),
            ("kind", name(serde_json::json!(s.kind))),
            ("content", name(serde_json::json!(s.content))),
            ("created_at", s.created_at.to_string()),
        ] {
            db.set_meta(k, &v)?;
        }
        let p = Self {
            id: s.id,
            dir,
            db,
            indexes: Mutex::default(),
            proxy_builds: Mutex::default(),
            clustering: Mutex::default(),
            posters: Mutex::default(),
            portraits: Mutex::default(),
        };
        p.ensure_dirs()?;
        Ok(p)
    }

    pub(crate) fn open(id: ProjectId, dir: PathBuf) -> ProjectResult<Self> {
        let db = ProjectDb::open(&dir.join(PROJECT_DB))?;
        let p = Self {
            id,
            dir,
            db,
            indexes: Mutex::default(),
            proxy_builds: Mutex::default(),
            clustering: Mutex::default(),
            posters: Mutex::default(),
            portraits: Mutex::default(),
        };
        p.ensure_dirs()?;
        // Exports can't resume mid-way (M1): close them out.
        p.db.close_stale_exports()?;
        p.clean_proxies();
        Ok(p)
    }

    fn ensure_dirs(&self) -> ProjectResult<()> {
        for d in [self.thumbs_dir(), self.proxies_dir(), self.tmp_dir()] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    pub(crate) fn db(&self) -> &ProjectDb {
        &self.db
    }

    pub fn thumbs_dir(&self) -> PathBuf {
        self.dir.join("thumbs")
    }
    pub fn proxies_dir(&self) -> PathBuf {
        self.dir.join("proxies")
    }
    pub fn tmp_dir(&self) -> PathBuf {
        self.dir.join("tmp")
    }

    /// Where a media's posters go: named after their sizes, so posters made
    /// at new sizes get new names and the webview never shows a cached old one.
    fn poster_paths(&self, id: MediaId) -> (PathBuf, PathBuf) {
        let name = |size: StillSize| match size {
            StillSize::Height(h) => format!("{id}-{h}.jpg"),
            StillSize::Cover(w, h) | StillSize::Region(_, w, h) => format!("{id}-{w}x{h}.jpg"),
        };
        (self.thumbs_dir().join(name(POSTER)), self.thumbs_dir().join(name(POSTER_SMALL)))
    }

    /// Write both posters of a media (a tenth of the way in) from one decode.
    fn write_posters(&self, tools: &Tools, id: MediaId, src: Source, cancel: &CancelToken) -> MediaResult<()> {
        let (big, small) = self.poster_paths(id);
        let outs = [(POSTER, big.as_path()), (POSTER_SMALL, small.as_path())];
        let _one = self.posters.lock();
        extract_frame_jpegs(tools, src, src.index.len() / 10, &outs, cancel)
    }

    pub fn title(&self) -> ProjectResult<String> {
        Ok(self.db.meta("title")?.unwrap_or_default())
    }

    pub(crate) fn set_title(&self, title: &str) -> ProjectResult<()> {
        Ok(self.db.set_meta("title", title)?)
    }

    // -- media --------------------------------------------------------------

    /// All media in episode order. Files that disappeared show as `missing`.
    pub fn media_list(&self) -> ProjectResult<Vec<MediaItem>> {
        let mut list = self.db.media_list()?;
        for m in &mut list {
            if !Path::new(&m.path).is_file() {
                m.status = MediaStatus::Missing;
            }
        }
        Ok(list)
    }

    pub fn media(&self, id: MediaId) -> ProjectResult<MediaItem> {
        let mut m = self.db.media(id)?.ok_or_else(|| ProjectError::NotFound("This episode was removed.".into()))?;
        if !Path::new(&m.path).is_file() {
            m.status = MediaStatus::Missing;
        }
        Ok(m)
    }

    /// Point a media whose file moved at `path` (see [`Relink`]): the same
    /// file keeps everything; another copy is indexed again (the caller
    /// queues it) and keeps its analysis if its frames are the same; another
    /// video is refused unless `force`.
    pub fn relink(&self, tools: &Tools, id: MediaId, path: &Path, force: bool) -> ProjectResult<Relink> {
        let m = self.db.media(id)?.ok_or_else(|| ProjectError::NotFound("This episode was removed.".into()))?;
        if !path.is_file() {
            return Err(ProjectError::NotFound("That file can't be found.".into()));
        }
        let new = path.display().to_string();
        if same_path(&m.path, &new) && Path::new(&m.path).is_file() {
            return Ok(Relink::Same);
        }
        if let Some(other) = self.db.media_list()?.into_iter().find(|x| x.id != id && same_path(&x.path, &new)) {
            let name = if other.label.is_empty() { other.file_name } else { other.label };
            return Err(ProjectError::Invalid(format!("That file is already in the project, as {name}.")));
        }
        let meta = std::fs::metadata(path)?;
        let found = as_media::probe(tools, path)?;
        // Never indexed: nothing to compare, nothing to lose.
        let same = m.info.as_ref().map_or(Ok(()), |old| as_media::same_source(old, &found));
        if let (Err(reason), false) = (&same, force) {
            return Ok(Relink::Different { reason: reason.clone() });
        }
        let indexed = m.status == MediaStatus::Ready && m.info.is_some();
        let same_bytes = same.is_ok() && indexed && meta.len() == m.size_bytes as u64;
        // Another copy (another size): does it show the same picture? Two
        // episodes can be just as long.
        let same_picture = match same.is_ok() && indexed && !same_bytes {
            true => self.same_picture(tools, &m, path, &found),
            false => None,
        };
        if same_picture == Some(false) && !force {
            return Ok(Relink::Different { reason: "At the same moment it shows another picture.".into() });
        }
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        self.db.relink_media(id, &new, &file_name, meta.len(), mtime)?;
        // What failed while the file was gone can run now.
        self.db.retry_failed_analysis(id)?;
        tracing::info!(media = %id, file = %file_name, ?same_picture, "found again");
        if same_bytes {
            return Ok(Relink::Same);
        }
        // Not shown to be the same picture: its analysis is made again (the
        // decisions wait for faces that look like theirs).
        if same_picture != Some(true) {
            self.db.reset_shots(id)?;
        }
        // Not playable or cut on the old frame index meanwhile.
        self.db.set_media_status(id, MediaStatus::Queued, None)?;
        self.indexes.lock().remove(&id);
        Ok(Relink::Reindex)
    }

    /// Whether the file at `path` (probed as `info`) shows what the media's
    /// poster shows, at the same frame; `None` when that can't be told.
    fn same_picture(&self, tools: &Tools, m: &MediaItem, path: &Path, info: &MediaInfo) -> Option<bool> {
        let poster = m.poster.as_deref().map(Path::new).filter(|p| p.is_file())?;
        let index = self.frame_index(m.id).ok()?;
        let probe = self.tmp_dir().join(format!("relink-{}.jpg", m.id));
        let src = Source { path, info, index: &index };
        let made = extract_frame_jpegs(tools, src, index.len() / 10, &[(POSTER_SMALL, &probe)], &CancelToken::new());
        let score = made.ok().and_then(|()| as_media::poster::picture_similarity(tools, poster, &probe).ok());
        let _ = std::fs::remove_file(&probe);
        let score = score?;
        tracing::info!(media = %m.id, similarity = score, "compared the picture of a file found again");
        Some(score >= SAME_PICTURE)
    }

    /// Look for every missing media in `dir` (and its folders): by file name,
    /// then size, then episode number — each candidate only if it is the
    /// same video ([`Self::relink`]). Returns the summary and the media to
    /// index again.
    pub fn relink_folder(&self, tools: &Tools, dir: &Path) -> ProjectResult<(RelinkSummary, Vec<MediaId>)> {
        let missing: Vec<MediaItem> =
            self.media_list()?.into_iter().filter(|m| m.status == MediaStatus::Missing).collect();
        let known = self.db.media_paths()?;
        let found = crate::library::scan_paths(&[dir.display().to_string()], ProjectKind::Series, &known);
        let mut used: HashSet<String> = HashSet::new();
        let mut summary = RelinkSummary::default();
        let mut reindex = Vec::new();
        for m in &missing {
            // By file name, then size, then episode number; a few at most.
            let rank = |c: &ImportCandidate| {
                if c.file_name.eq_ignore_ascii_case(&m.file_name) {
                    Some(0)
                } else if m.size_bytes > 0.0 && c.size_bytes == m.size_bytes {
                    Some(1)
                } else if m.episode.is_some() && (c.season, c.episode) == (m.season, m.episode) {
                    Some(2)
                } else {
                    None
                }
            };
            // Files in the project already (another episode's) aren't candidates.
            let mut tried: Vec<(u8, &ImportCandidate)> = found
                .iter()
                .filter(|c| !c.duplicate && !used.contains(&c.path))
                .filter_map(|c| Some((rank(c)?, c)))
                .collect();
            tried.sort_by_key(|(r, _)| *r);
            let mut outcome = None;
            for (_, c) in tried.into_iter().take(3) {
                match self.relink(tools, m.id, Path::new(&c.path), false) {
                    Ok(Relink::Different { .. }) => continue,
                    Ok(r) => {
                        used.insert(c.path.clone());
                        outcome = Some(r);
                        break;
                    }
                    // A file that can't be read (half copied, not a video): the next one.
                    Err(e) => tracing::warn!(file = %c.path, error = %e.detail(), "a candidate couldn't be used"),
                }
            }
            match outcome {
                Some(Relink::Same) => summary.same += 1,
                Some(_) => {
                    summary.reindex += 1;
                    reindex.push(m.id);
                }
                None => summary.missing += 1,
            }
        }
        Ok((summary, reindex))
    }

    pub fn remove_media(&self, id: MediaId) -> ProjectResult<()> {
        let m = self.media(id)?;
        self.db.remove_media(id)?;
        self.indexes.lock().remove(&id);
        // Its pictures go: the ones it points at, and ones made under its
        // names that it doesn't point at yet (a poster upkeep it raced).
        self.remove_stale_posters(&m, [None, None]);
        let (big, small) = self.poster_paths(id);
        let _ = std::fs::remove_file(big);
        let _ = std::fs::remove_file(small);
        self.remove_proxies(id, None);
        let _ = std::fs::remove_dir_all(self.faces_dir().join(id.to_string()));
        Ok(())
    }

    /// Media interrupted mid-ingest by a quit or crash go back to `queued`;
    /// returns them, media still waiting for a proxy and media whose posters
    /// are outdated, so the app can re-queue the work.
    pub fn recover(&self) -> ProjectResult<Recovery> {
        let ingest = self.db.requeue_interrupted()?;
        let proxies = self.db.pending_proxies()?;
        let posters = self.media_list()?.iter().filter(|m| self.posters_outdated(m)).map(|m| m.id).collect();
        Ok(Recovery { ingest, proxies, posters })
    }

    fn posters_outdated(&self, m: &MediaItem) -> bool {
        let (big, small) = self.poster_paths(m.id);
        posters_outdated(m, &big, &small)
    }

    /// Make a ready media's posters again at the current sizes (the old
    /// files go). `Ok(false)`: nothing to do (up to date, no longer ready,
    /// or removed meanwhile).
    pub fn refresh_posters(&self, tools: &Tools, id: MediaId, cancel: &CancelToken) -> ProjectResult<bool> {
        let m = match self.media(id) {
            Ok(m) if self.posters_outdated(&m) => m,
            Ok(_) | Err(ProjectError::NotFound(_)) => return Ok(false),
            Err(e) => return Err(e),
        };
        let (_, info, index) = self.ready_source(id)?;
        let path = PathBuf::from(&m.path);
        self.write_posters(tools, id, Source { path: &path, info: &info, index: &index }, cancel)?;
        let (big, small) = self.poster_paths(id);
        let (big_s, small_s) = (big.display().to_string(), small.display().to_string());
        if !self.db.set_posters(id, &big_s, &small_s)? {
            // Removed while we worked.
            let _ = std::fs::remove_file(&big);
            let _ = std::fs::remove_file(&small);
            return Ok(false);
        }
        self.remove_stale_posters(&m, [Some(&big_s), Some(&small_s)]);
        Ok(true)
    }

    /// Delete the poster files `before` pointed at that the media no longer
    /// uses (`now`: its poster and small poster).
    fn remove_stale_posters(&self, before: &MediaItem, now: [Option<&str>; 2]) {
        let thumbs = self.thumbs_dir();
        for p in stale_posters([before.poster.as_deref(), before.poster_small.as_deref()], now, &thumbs) {
            let _ = std::fs::remove_file(p);
        }
    }

    pub fn set_media_failed(&self, id: MediaId, message: &str) -> ProjectResult<()> {
        Ok(self.db.set_media_status(id, MediaStatus::Failed, Some(message))?)
    }

    pub fn set_media_queued(&self, id: MediaId) -> ProjectResult<()> {
        Ok(self.db.set_media_status(id, MediaStatus::Queued, None)?)
    }

    /// The decoded-frame index of a ready media (cached).
    pub fn frame_index(&self, id: MediaId) -> ProjectResult<Arc<FrameIndex>> {
        if let Some(i) = self.indexes.lock().get(&id) {
            return Ok(i.clone());
        }
        let blob = self.db.frame_index_blob(id)?.ok_or_else(|| ProjectError::NotReady("Still indexing.".into()))?;
        let idx = Arc::new(FrameIndex::from_bytes(&blob).map_err(|e| {
            tracing::warn!(media = %id, error = %e, "stored frame index is damaged");
            // Failed, so its menu and the player offer to index it again.
            let message = "Its frame index is damaged. Index it again.";
            if let Err(e) = self.db.set_media_status(id, MediaStatus::Failed, Some(message)) {
                tracing::warn!(media = %id, error = %e, "marking a damaged index failed");
            }
            ProjectError::Invalid(message.into())
        })?);
        self.indexes.lock().insert(id, idx.clone());
        Ok(idx)
    }

    pub(crate) fn ready_source(&self, id: MediaId) -> ProjectResult<(MediaItem, MediaInfo, Arc<FrameIndex>)> {
        let m = self.media(id)?;
        match m.status {
            MediaStatus::Ready => {}
            MediaStatus::Missing => {
                return Err(ProjectError::NotFound(format!("{} can't be found. Was it moved?", m.file_name)));
            }
            _ => return Err(ProjectError::NotReady(format!("{} isn't indexed yet.", m.file_name))),
        }
        let info = m.info.clone().ok_or_else(|| ProjectError::NotReady("Still indexing.".into()))?;
        let index = self.frame_index(id)?;
        Ok((m, info, index))
    }

    /// Probe, detect interlacing, build the frame index and poster, pick the
    /// audio track (an earlier choice stays; otherwise the first track in
    /// `audio_language`, else the file's default) and decide whether a
    /// playback proxy is needed. Marks the media `ready` (or `failed` with a
    /// readable reason).
    pub fn ingest(
        &self,
        tools: &Tools,
        id: MediaId,
        audio_language: &str,
        cancel: &CancelToken,
        progress: Progress,
    ) -> ProjectResult<MediaItem> {
        let result = self.ingest_inner(tools, id, audio_language, cancel, progress);
        match &result {
            Ok(_) => {}
            // Back in the queue: a quit resumes it on the next launch. The app
            // marks it cancelled when a person stopped it.
            Err(ProjectError::Cancelled) => self.set_media_queued(id)?,
            Err(e) => self.set_media_failed(id, &e.to_string())?,
        }
        result
    }

    fn ingest_inner(
        &self,
        tools: &Tools,
        id: MediaId,
        audio_language: &str,
        cancel: &CancelToken,
        progress: Progress,
    ) -> ProjectResult<MediaItem> {
        let m = self.media(id)?;
        let path = PathBuf::from(&m.path);
        if !path.is_file() {
            return Err(ProjectError::NotFound(format!("{} can't be found.", m.file_name)));
        }
        self.db.set_media_status(id, MediaStatus::Indexing, None)?;
        progress("Reading file", 0.0);
        let mut info = probe(tools, &path)?;
        cancel.check().map_err(|_| ProjectError::Cancelled)?;
        if info.video.field_order.is_none()
            && let Some(order) = idet::detect_field_order(tools, &path, &info)?
        {
            tracing::info!(file = %m.file_name, order, "interlaced content detected");
            info.video.field_order = Some(order.to_string());
        }

        let index =
            build_frame_index(tools, &path, &info, &self.tmp_dir(), cancel, |p| progress("Indexing frames", p * 0.95))?;

        progress("Creating poster", 0.96);
        let src = Source { path: &path, info: &info, index: &index };
        let (poster, poster_small) = match self.write_posters(tools, id, src, cancel) {
            Ok(()) => {
                let (big, small) = self.poster_paths(id);
                (Some(big.display().to_string()), Some(small.display().to_string()))
            }
            Err(e) if e.is_cancelled() => return Err(ProjectError::Cancelled),
            Err(e) => {
                tracing::warn!(file = %m.file_name, error = %e.detail(), "poster failed");
                (None, None)
            }
        };

        let audio_ordinal = m
            .audio_ordinal
            .filter(|o| info.audio.iter().any(|a| a.ordinal == *o))
            .or_else(|| info.preferred_audio(audio_language).map(|a| a.ordinal));
        let proxy =
            if plays_direct(&info, &index, audio_ordinal) { ProxyState::NotNeeded } else { ProxyState::Pending };
        let rec = IngestRecord {
            audio_ordinal,
            frames: index.len(),
            duration_sec: index.duration(),
            fps: index.nominal_fps,
            vfr: index.is_vfr(),
            interlaced: info.is_interlaced(),
            proxy,
            poster,
            poster_small,
            index_blob: index.to_bytes(),
            info,
        };
        self.db.record_ingest(id, &rec)?;
        // Indexed again (a file that was missing): pictures it no longer
        // uses go, also when no poster could be made this time.
        self.remove_stale_posters(&m, [rec.poster.as_deref(), rec.poster_small.as_deref()]);
        self.indexes.lock().insert(id, Arc::new(index));
        self.media(id)
    }

    /// Choose the audio track used for playback and export. Returns `true`
    /// when the change needs a (new) playback proxy.
    pub fn set_audio(&self, id: MediaId, ordinal: Option<u32>) -> ProjectResult<bool> {
        let m = self.media(id)?;
        if let Some(o) = ordinal
            && !m.info.as_ref().is_some_and(|i| i.audio.iter().any(|a| a.ordinal == o))
        {
            return Err(ProjectError::Invalid("That audio track doesn't exist.".into()));
        }
        self.db.set_audio(id, ordinal)?;
        if m.status != MediaStatus::Ready {
            return Ok(false);
        }
        let (_, info, index) = self.ready_source(id)?;
        if plays_direct(&info, &index, ordinal) {
            // The old preview may still be on screen; `frame_map` removes it
            // once the player has moved to the source.
            self.db.set_proxy(id, ProxyState::NotNeeded, None, None)?;
            Ok(false)
        } else {
            self.db.set_proxy(id, ProxyState::Pending, None, None)?;
            Ok(true)
        }
    }

    /// Render the playback proxy for the media's current audio track.
    pub fn build_proxy(
        &self,
        tools: &Tools,
        id: MediaId,
        cancel: &CancelToken,
        progress: Progress,
    ) -> ProjectResult<()> {
        let (m, info, index) = self.ready_source(id)?;
        let audio = m.audio_ordinal.or_else(|| info.default_audio().map(|a| a.ordinal));
        let out = self.proxies_dir().join(format!("{id}-a{}.mp4", audio.map_or("x".into(), |a| a.to_string())));
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.proxy_builds.lock().insert(id, generation);
        let latest = || self.proxy_builds.lock().get(&id) == Some(&generation);
        self.db.set_proxy(id, ProxyState::Building, None, None)?;
        let path = PathBuf::from(&m.path);
        let src = Source { path: &path, info: &info, index: &index };
        let result = proxy::build_proxy(tools, src, audio, &out, cancel, |p| progress("Preparing preview", p));
        // The audio track may have changed, or a newer build started, while we
        // worked; the newer state wins then.
        let superseded = !latest() || self.media(id)?.audio_ordinal != m.audio_ordinal;
        match result {
            Ok(_) if superseded => {
                let _ = std::fs::remove_file(&out);
                Err(ProjectError::Cancelled)
            }
            Ok(r) => {
                self.db.set_proxy(id, ProxyState::Ready, Some(&out.display().to_string()), Some(r.t0))?;
                self.remove_proxies(id, Some(&out));
                Ok(())
            }
            Err(e) => {
                if !superseded {
                    // A cancelled build stays pending, so it is offered again (and resumed on next launch).
                    let state = if e.is_cancelled() { ProxyState::Pending } else { ProxyState::Failed };
                    self.db.set_proxy(id, state, None, None)?;
                }
                Err(e.into())
            }
        }
    }

    /// Remove proxy files nothing points at: half-written `.part` files and
    /// previews for an audio track that is no longer selected.
    fn clean_proxies(&self) {
        let Ok(media) = self.db.media_list() else { return };
        let Ok(entries) = std::fs::read_dir(self.proxies_dir()) else { return };
        let mut current = HashSet::new();
        for m in &media {
            if let Ok(Some((p, _))) = self.db.proxy(m.id) {
                current.insert(PathBuf::from(p));
            }
        }
        for e in entries.filter_map(Result::ok) {
            let p = e.path();
            if !current.contains(&p) {
                tracing::debug!(file = %p.display(), "removing unused preview file");
                let _ = std::fs::remove_file(p);
            }
        }
    }

    /// Delete finished preview files of `id` except `keep`. Unfinished
    /// `.part` files may belong to a build that is still running; the next
    /// project open removes leftovers (`clean_proxies`).
    fn remove_proxies(&self, id: MediaId, keep: Option<&Path>) {
        let prefix = format!("{id}-");
        let Ok(entries) = std::fs::read_dir(self.proxies_dir()) else { return };
        for e in entries.filter_map(Result::ok) {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && !name.ends_with(".part.mp4") && Some(p.as_path()) != keep {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    /// Mapping between `<video>` time and source frames for the player.
    pub fn frame_map(&self, id: MediaId) -> ProjectResult<FrameMap> {
        let (m, _, index) = self.ready_source(id)?;
        let (src, is_proxy, offset) = match m.proxy {
            ProxyState::NotNeeded => {
                // A preview from an earlier audio choice is no longer needed.
                self.remove_proxies(id, None);
                (m.path.clone(), false, index.t0())
            }
            ProxyState::Ready => {
                let (p, t0) = self.db.proxy(id)?.ok_or_else(|| ProjectError::NotReady("Preparing preview…".into()))?;
                if !Path::new(&p).is_file() {
                    // The app queues a rebuild for media left `pending`.
                    self.db.set_proxy(id, ProxyState::Pending, None, None)?;
                    return Err(ProjectError::NotReady("The preview file is missing. Preparing it again…".into()));
                }
                (p, true, t0)
            }
            ProxyState::Failed => {
                return Err(ProjectError::NotReady("The preview couldn't be prepared for this file.".into()));
            }
            _ => {
                return Err(ProjectError::NotReady(
                    "This format needs a preview copy for frame-accurate playback.".into(),
                ));
            }
        };
        let n = index.len();
        let t0 = index.t0();
        let fps = index.nominal_fps;
        let (step, times) = if index.is_vfr() {
            (None, Some((0..=n).map(|f| index.t(f) - t0).collect()))
        } else {
            (Some(index.duration() / n as f64), None)
        };
        Ok(FrameMap {
            media_id: id,
            src,
            is_proxy,
            frames: n,
            fps,
            drop_frame: is_drop_frame(fps),
            offset,
            step,
            times,
        })
    }

    // -- exports ------------------------------------------------------------

    /// Validate and record an export (status `queued`); run it with [`Project::run_export`].
    pub fn create_export(
        &self,
        req: &ExportRequest,
        preset: ExportPreset,
        out_dir: &Path,
    ) -> ProjectResult<ExportSummary> {
        if req.clips.is_empty() {
            return Err(ProjectError::Invalid("Nothing to export.".into()));
        }
        let title = self.title()?;
        let ext = preset.container.ext();
        // Names already promised to exports that haven't written their files yet.
        let out_str = out_dir.display().to_string();
        let mut used: HashSet<String> = self
            .exports()?
            .into_iter()
            .filter(|e| e.out_dir == out_str && e.preset.container == preset.container)
            .flat_map(|e| e.items)
            .filter(|i| matches!(i.status, ExportStatus::Queued | ExportStatus::Running))
            // Compared case-insensitively, like Windows and macOS file names.
            .map(|i| i.name.to_lowercase())
            .collect();
        let mut items = Vec::with_capacity(req.clips.len());
        for c in &req.clips {
            let (m, _, index) = self.ready_source(c.media_id)?;
            if c.start >= c.end || c.end > index.len() {
                return Err(ProjectError::Invalid(format!(
                    "Frames {}–{} are outside {} (0–{}).",
                    c.start,
                    c.end,
                    m.file_name,
                    index.len()
                )));
            }
            let base = match c.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => safe_component(n, " ", 120),
                None => default_clip_name(&title, &m, &index, c.start),
            };
            // Windows and macOS file names ignore case.
            let name =
                unique_name(&base, |n| used.contains(&n.to_lowercase()) || out_dir.join(format!("{n}.{ext}")).exists());
            used.insert(name.to_lowercase());
            items.push(NewExportItem {
                media_id: c.media_id,
                start: c.start,
                end: c.end,
                name,
                audio_ordinal: m.audio_ordinal,
            });
        }
        std::fs::create_dir_all(out_dir)
            .map_err(|e| ProjectError::Invalid(format!("Can't write to {}: {e}", out_dir.display())))?;
        let id = ExportId::new();
        self.db.insert_export(id, &preset, &out_str, req.label.as_deref(), &items)?;
        self.export(id)
    }

    /// What an export of `clips` would make: the names its files get (as
    /// [`Project::create_export`] names them, before telling them apart from
    /// files already in the folder) and its size with each of `presets`.
    pub fn export_plan(&self, clips: &[ClipRequest], presets: &[ExportPreset]) -> ProjectResult<ExportPlan> {
        let title = self.title()?;
        let mut sources: HashMap<MediaId, (MediaItem, MediaInfo, Arc<FrameIndex>)> = HashMap::new();
        let mut used: HashSet<String> = HashSet::new();
        let mut names = Vec::with_capacity(clips.len());
        let mut sizes = vec![0.0; presets.len()];
        for c in clips {
            if let std::collections::hash_map::Entry::Vacant(e) = sources.entry(c.media_id) {
                e.insert(self.ready_source(c.media_id)?);
            }
            let (m, info, index) = &sources[&c.media_id];
            if c.start >= c.end || c.end > index.len() {
                return Err(ProjectError::Invalid(format!(
                    "Frames {}–{} are outside {}.",
                    c.start, c.end, m.file_name
                )));
            }
            let base = match c.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => safe_component(n, " ", 120),
                None => default_clip_name(&title, m, index, c.start),
            };
            let name = unique_name(&base, |n| used.contains(&n.to_lowercase()));
            used.insert(name.to_lowercase());
            names.push(name);
            let seconds = index.t(c.end) - index.t(c.start);
            for (size, preset) in sizes.iter_mut().zip(presets) {
                *size += estimate_bytes(preset, info, m.audio_ordinal, u64::from(c.end - c.start), seconds);
            }
        }
        let sizes = presets.iter().zip(sizes).map(|(p, bytes)| PresetSize { preset_id: p.id.clone(), bytes }).collect();
        Ok(ExportPlan { names, sizes, folder: None })
    }

    /// Mark an export failed before it could start (e.g. ffmpeg missing).
    pub fn fail_export(&self, id: ExportId, message: &str) -> ProjectResult<()> {
        for item in self.export(id)?.items {
            self.db.update_export_item(item.id, ExportStatus::Failed, None, Some(message))?;
        }
        Ok(self.db.set_export_status(id, ExportStatus::Failed)?)
    }

    pub fn export(&self, id: ExportId) -> ProjectResult<ExportSummary> {
        self.db.export(self.id, id)?.ok_or_else(|| ProjectError::NotFound("This export no longer exists.".into()))
    }

    pub fn exports(&self) -> ProjectResult<Vec<ExportSummary>> {
        Ok(self.db.exports(self.id)?)
    }

    /// Render every clip of an export. A failing clip doesn't stop the others;
    /// cancelling marks the remaining clips cancelled. `changed` runs whenever
    /// a clip starts or finishes, so the UI can follow along.
    pub fn run_export(
        &self,
        tools: &Tools,
        id: ExportId,
        cancel: &CancelToken,
        progress: Progress,
        changed: &dyn Fn(),
    ) -> ProjectResult<ExportSummary> {
        let ex = self.export(id)?;
        self.db.set_export_status(id, ExportStatus::Running)?;
        let n = ex.items.len().max(1) as f64;
        let out_dir = PathBuf::from(&ex.out_dir);
        let mut failed = false;
        for (i, item) in ex.items.iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            let stage =
                if ex.items.len() > 1 { format!("Clip {} of {}", i + 1, ex.items.len()) } else { "Rendering".into() };
            self.db.update_export_item(item.id, ExportStatus::Running, None, None)?;
            changed();
            let result = (|| -> ProjectResult<as_api::ClipOutcome> {
                let (m, info, index) = self.ready_source(item.media_id)?;
                let path = PathBuf::from(&m.path);
                let job = ClipJob {
                    src: Source { path: &path, info: &info, index: &index },
                    range: FrameRange::new(item.start, item.end),
                    audio_ordinal: self.db.export_item_audio(item.id)?,
                    preset: &ex.preset,
                    out_stem: out_dir.join(&item.name),
                    title: Some(item.name.clone()),
                };
                Ok(export_clip(tools, &job, cancel, |p| progress(&stage, (i as f64 + p) / n))?)
            })();
            match result {
                Ok(outcome) => self.db.update_export_item(item.id, ExportStatus::Done, Some(&outcome), None)?,
                Err(ProjectError::Cancelled) => break,
                Err(e) => {
                    failed = true;
                    tracing::warn!(clip = %item.name, error = %e, detail = %e.detail(), "clip export failed");
                    self.db.update_export_item(item.id, ExportStatus::Failed, None, Some(&e.to_string()))?;
                }
            }
            changed();
        }
        if cancel.is_cancelled() {
            for item in self
                .export(id)?
                .items
                .iter()
                .filter(|i| matches!(i.status, ExportStatus::Queued | ExportStatus::Running))
            {
                self.db.update_export_item(item.id, ExportStatus::Cancelled, None, None)?;
            }
            self.db.set_export_status(id, ExportStatus::Cancelled)?;
            return Err(ProjectError::Cancelled);
        }
        self.db.set_export_status(id, if failed { ExportStatus::Failed } else { ExportStatus::Done })?;
        self.export(id)
    }
}

/// Does a media need its posters made again? A ready media does when either
/// poster isn't the one of the current sizes (`big`, `small`) or its file is
/// gone: the 360p posters from before the full-bleed artwork had other names
/// and no small one. A media without a poster had it fail at ingest; that
/// isn't retried on every launch.
fn posters_outdated(m: &MediaItem, big: &Path, small: &Path) -> bool {
    let current = |have: Option<&str>, want: &Path| have.is_some_and(|p| Path::new(p) == want && want.is_file());
    m.status == MediaStatus::Ready
        && m.poster.is_some()
        && !(current(m.poster.as_deref(), big) && current(m.poster_small.as_deref(), small))
}

/// Poster files a media pointed at (`before`) that it no longer does (`now`):
/// they can go. Only files in the project's `thumbs` folder: a stale path
/// (say, of a project folder that was copied) never touches another copy.
fn stale_posters<'a>(before: [Option<&'a str>; 2], now: [Option<&str>; 2], thumbs: &Path) -> Vec<&'a Path> {
    let kept = |p: &Path| now.iter().flatten().any(|n| Path::new(n) == p);
    before.into_iter().flatten().map(Path::new).filter(|p| p.parent() == Some(thumbs) && !kept(p)).collect()
}

/// The webview plays the file itself, frame-accurately, with the chosen audio track.
fn plays_direct(info: &MediaInfo, index: &FrameIndex, audio_ordinal: Option<u32>) -> bool {
    let default_audio = info.default_audio().map(|a| a.ordinal);
    proxy::playable_direct(info, index) && (audio_ordinal.is_none() || audio_ordinal == default_audio)
}

/// `Title_S01E03_00-12-03-05`: project, episode (or file name), source timecode.
fn default_clip_name(title: &str, m: &MediaItem, index: &FrameIndex, start: u32) -> String {
    let (tc, episode) = (clip_timecode(start, index.nominal_fps), episode_name(m));
    let parts = NameParts { series: title, character: None, episode: &episode, index: (1, 1), tc: &tc, frame: start };
    render_name(CLIP_TEMPLATE, &parts).expect("the built-in template is valid")
}

/// An episode as clip names show it: its label (`S01E03`), or the file's
/// name without extension.
pub(crate) fn episode_name(m: &MediaItem) -> String {
    if m.label.is_empty() {
        Path::new(&m.file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("clip").to_string()
    } else {
        m.label.clone()
    }
}

fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}_{i}")).find(|n| !taken(n)).expect("unbounded")
}

/// Two paths name the same file (on Windows whatever the case and slashes).
fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| {
        let p = p.replace('\\', "/");
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use as_core::Rational;

    use super::*;

    #[test]
    fn a_damaged_frame_index_asks_to_index_again() {
        let (_dir, project, m) = crate::analysis::tests::project_with_media(240);
        let rec = as_db::IngestRecord {
            info: crate::analysis::tests::info(),
            frames: 240,
            duration_sec: 10.0,
            fps: Rational::new(24, 1),
            vfr: false,
            interlaced: false,
            proxy: ProxyState::NotNeeded,
            poster: None,
            poster_small: None,
            index_blob: b"not an index".to_vec(),
            audio_ordinal: None,
        };
        project.db().record_ingest(m, &rec).unwrap();

        let e = project.frame_map(m).unwrap_err();
        assert!(matches!(e, ProjectError::Invalid(_)), "{e:?}");
        assert!(e.to_string().contains("Index it again"), "{e}");
        let media = project.media(m).unwrap();
        assert_eq!(media.status, MediaStatus::Failed);
        assert_eq!(media.error.as_deref(), Some(e.to_string().as_str()));
    }

    #[test]
    fn an_export_plan_names_like_the_export_and_sizes_each_preset() {
        let (_dir, project, m) = crate::analysis::tests::project_with_media(2400);
        let clip = |start, end, name: Option<&str>| ClipRequest { media_id: m, start, end, name: name.map(Into::into) };
        let presets = as_export::builtin_presets();
        let plan = project
            .export_plan(
                &[clip(48, 96, None), clip(48, 96, None), clip(240, 480, Some("Opening: shot")), clip(0, 24, None)],
                &presets,
            )
            .unwrap();
        // The default name; the same twice gets told apart; a given name is kept, made safe.
        assert_eq!(
            plan.names,
            ["Show_E01_00-00-02-00", "Show_E01_00-00-02-00_2", "Opening shot", "Show_E01_00-00-00-00"]
        );
        assert_eq!(plan.sizes.len(), presets.len());
        let size = |id: &str| plan.sizes.iter().find(|s| s.preset_id == id).unwrap().bytes;
        assert!(size("prores-hq") > size("h264-edit") && size("h264-edit") > 0.0);
        // Out of range: refused, like the export itself would.
        assert!(project.export_plan(&[clip(2390, 2500, None)], &presets).is_err());
    }

    #[test]
    fn unique_names() {
        let taken = ["a", "a_2"];
        assert_eq!(unique_name("a", |n| taken.contains(&n)), "a_3");
        assert_eq!(unique_name("b", |n| taken.contains(&n)), "b");
    }

    #[test]
    fn drop_frame_rates() {
        assert!(is_drop_frame(Rational::new(30000, 1001)));
        assert!(is_drop_frame(Rational::new(60000, 1001)));
        assert!(!is_drop_frame(Rational::new(24000, 1001)));
        assert!(!is_drop_frame(Rational::new(30, 1)));
    }

    fn media(status: MediaStatus, poster: Option<&Path>, poster_small: Option<&Path>) -> MediaItem {
        let path = |p: Option<&Path>| p.map(|p| p.display().to_string());
        MediaItem {
            id: MediaId::new(),
            path: "/s/Show - 01.mkv".into(),
            file_name: "Show - 01.mkv".into(),
            season: None,
            episode: Some(1),
            label: "E01".into(),
            position: 0,
            size_bytes: 1.0,
            status,
            error: None,
            info: None,
            frames: Some(240),
            duration_sec: Some(10.0),
            fps: Some(Rational::new(24, 1)),
            vfr: false,
            interlaced: false,
            audio_ordinal: None,
            proxy: ProxyState::NotNeeded,
            poster: path(poster),
            poster_small: path(poster_small),
            added_at: 0,
            shots: as_api::StageState::Pending,
            shots_error: None,
            shot_count: None,
            faces: as_api::StageState::Pending,
            faces_error: None,
            face_count: None,
        }
    }

    #[test]
    fn posters_are_made_again_when_older_than_the_current_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, b"jpeg").unwrap();
            p
        };
        let (old, big, small) = (file("e1.jpg"), file("e1-720.jpg"), file("e1-480x270.jpg"));
        let outdated = |m: &MediaItem| posters_outdated(m, &big, &small);
        let ready = MediaStatus::Ready;

        // A 360p poster from before the full-bleed artwork: no small one.
        assert!(outdated(&media(ready, Some(&old), None)));
        // An old poster next to a current small one is still old.
        assert!(outdated(&media(ready, Some(&old), Some(&small))));
        // Made at the current sizes.
        assert!(!outdated(&media(ready, Some(&big), Some(&small))));

        // Files that are gone are made again.
        std::fs::remove_file(&small).unwrap();
        assert!(outdated(&media(ready, Some(&big), Some(&small))));

        // Only ready media: a missing source can't be decoded, and media
        // being indexed get new posters anyway.
        for status in [MediaStatus::Queued, MediaStatus::Indexing, MediaStatus::Failed, MediaStatus::Missing] {
            assert!(!outdated(&media(status, Some(&old), None)), "{status:?}");
        }
        // A poster that failed at ingest isn't retried on every launch.
        assert!(!outdated(&media(ready, None, None)));
    }

    #[test]
    fn only_posters_no_longer_used_in_this_projects_folder_go() {
        let thumbs = Path::new("/lib/Show.autoscene/thumbs");
        let at = |name: &str| thumbs.join(name).display().to_string();
        let (old, big, small) = (at("e1.jpg"), at("e1-720.jpg"), at("e1-480x270.jpg"));
        let (old, big, small) = (old.as_str(), big.as_str(), small.as_str());
        let stale = |before: [Option<&str>; 2], now: [Option<&str>; 2]| -> Vec<String> {
            stale_posters(before, now, thumbs).iter().map(|p| p.display().to_string()).collect()
        };

        // Made again at the current sizes: the old one goes.
        assert_eq!(stale([Some(old), None], [Some(big), Some(small)]), [old]);
        // Same names (the same picture, indexed again): nothing goes.
        assert!(stale([Some(big), Some(small)], [Some(big), Some(small)]).is_empty());
        // No poster this time, or the media is removed: all of its pictures go.
        assert_eq!(stale([Some(big), Some(small)], [None, None]), [big, small]);
        assert!(stale([None, None], [None, None]).is_empty());

        // A copied project still pointing into the original's folder, or a
        // path outside any thumbs folder: never touched.
        let other = Path::new("/lib/Show (copy).autoscene/thumbs/e1.jpg").display().to_string();
        assert!(stale([Some(&other), None], [Some(big), Some(small)]).is_empty());
        assert!(stale([Some("/home/me/Pictures/e1.jpg"), None], [None, None]).is_empty());

        // Stored paths mix separators (the project folder joined with `\`).
        #[cfg(windows)]
        {
            let thumbs = Path::new("C:/Lib/Show.autoscene/thumbs");
            let mixed = r"C:/Lib/Show.autoscene\thumbs\e1.jpg";
            assert_eq!(stale_posters([Some(mixed), None], [None, None], thumbs), [Path::new(mixed)]);
            let same = r"C:\Lib\Show.autoscene\thumbs\e1.jpg";
            assert!(stale_posters([Some(mixed), None], [Some(same), None], thumbs).is_empty());
        }
    }
}
