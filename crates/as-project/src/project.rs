use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use as_api::{
    ExportRequest, ExportStatus, ExportSummary, FrameMap, MediaItem, MediaStatus, ProjectSummary, ProxyState,
};
use as_core::{CancelToken, ExportId, FrameIndex, FrameRange, MediaId, ProjectId, Rational, Timecode};
use as_db::{IngestRecord, NewExportItem, ProjectDb};
use as_export::{ClipJob, ExportPreset, export_clip};
use as_media::{MediaInfo, Source, Tools, build_frame_index, idet, poster, probe, proxy};
use parking_lot::Mutex;

use crate::naming::safe_component;
use crate::{ProjectError, ProjectResult};

pub(crate) const PROJECT_DB: &str = "project.db";
const POSTER_HEIGHT: u32 = 360;

/// Progress callback: `(stage, fraction 0..1)`.
pub type Progress<'a> = &'a dyn Fn(&str, f64);

/// An open project.
pub struct Project {
    pub id: ProjectId,
    pub dir: PathBuf,
    db: ProjectDb,
    indexes: Mutex<HashMap<MediaId, Arc<FrameIndex>>>,
    /// Latest proxy build per media. A replaced build that finishes (or
    /// fails) late must not overwrite the state of the one that replaced it.
    proxy_builds: Mutex<HashMap<MediaId, u64>>,
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
        let p =
            Self { id: s.id, dir, db, indexes: Mutex::new(HashMap::new()), proxy_builds: Mutex::new(HashMap::new()) };
        p.ensure_dirs()?;
        Ok(p)
    }

    pub(crate) fn open(id: ProjectId, dir: PathBuf) -> ProjectResult<Self> {
        let db = ProjectDb::open(&dir.join(PROJECT_DB))?;
        let p = Self { id, dir, db, indexes: Mutex::new(HashMap::new()), proxy_builds: Mutex::new(HashMap::new()) };
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

    pub fn remove_media(&self, id: MediaId) -> ProjectResult<()> {
        let m = self.media(id)?;
        self.db.remove_media(id)?;
        self.indexes.lock().remove(&id);
        if let Some(p) = m.poster {
            let _ = std::fs::remove_file(p);
        }
        self.remove_proxies(id, None);
        Ok(())
    }

    /// Media interrupted mid-ingest by a quit or crash go back to `queued`;
    /// returns them (and media still waiting for a proxy) so the app can re-queue the work.
    pub fn recover(&self) -> ProjectResult<(Vec<MediaId>, Vec<MediaId>)> {
        Ok((self.db.requeue_interrupted()?, self.db.pending_proxies()?))
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
        let idx = Arc::new(FrameIndex::from_bytes(&blob).map_err(|e| ProjectError::Invalid(e.to_string()))?);
        self.indexes.lock().insert(id, idx.clone());
        Ok(idx)
    }

    fn ready_source(&self, id: MediaId) -> ProjectResult<(MediaItem, MediaInfo, Arc<FrameIndex>)> {
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
        let poster_path = self.thumbs_dir().join(format!("{id}.jpg"));
        let poster_frame = index.len() / 10;
        let src = Source { path: &path, info: &info, index: &index };
        let poster = match poster::extract_frame_jpeg(tools, src, poster_frame, POSTER_HEIGHT, &poster_path) {
            Ok(()) => Some(poster_path.display().to_string()),
            Err(e) => {
                tracing::warn!(file = %m.file_name, error = %e, "poster failed");
                None
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
            index_blob: index.to_bytes(),
            info,
        };
        self.db.record_ingest(id, &rec)?;
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
        self.db.insert_export(id, &preset, &out_str, &items)?;
        self.export(id)
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

/// The webview plays the file itself, frame-accurately, with the chosen audio track.
fn plays_direct(info: &MediaInfo, index: &FrameIndex, audio_ordinal: Option<u32>) -> bool {
    let default_audio = info.default_audio().map(|a| a.ordinal);
    proxy::playable_direct(info, index) && (audio_ordinal.is_none() || audio_ordinal == default_audio)
}

fn is_drop_frame(fps: Rational) -> bool {
    let base = fps.to_f64().round() as i64;
    fps.den == 1001 && (base == 30 || base == 60)
}

/// `Title_S01E03_00-12-03-05`: project, episode (or file name), source timecode.
fn default_clip_name(title: &str, m: &MediaItem, index: &FrameIndex, start: u32) -> String {
    let fps = index.nominal_fps;
    let tc = Timecode::from_frame(start as u64, fps, is_drop_frame(fps)).to_string().replace([':', ';'], "-");
    let what = if m.label.is_empty() {
        Path::new(&m.file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("clip").to_string()
    } else {
        m.label.clone()
    };
    safe_component(&format!("{title} {what} {tc}"), "_", 120)
}

fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}_{i}")).find(|n| !taken(n)).expect("unbounded")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
