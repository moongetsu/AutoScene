use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use as_api::{
    ClipRequest, ExportItem, ExportPlan, ExportRequest, ExportStatus, ExportSummary, FrameMap, ImportCandidate,
    MediaItem, MediaStatus, MergedFile, PresetSize, ProjectKind, ProjectSummary, ProxyState, Relink, RelinkSummary,
};
use as_core::{CancelToken, ExportId, FrameIndex, FrameRange, MediaId, ProjectId};
use as_db::{IngestRecord, NewExportItem, ProjectDb, Relinked};
use as_export::LosslessCodec;
use as_export::clip::PARTIAL_DIR;
use as_export::preset::VideoCodec;
use as_export::{
    ClipJob, EstimateClip, ExportPreset, JoinJob, JoinPart, MergeSource, Part, estimate_bytes, exact_format,
    export_clip, join_parts, lossless_merge_blocker, plan_lossless_merge, plan_merge,
};
use as_media::poster::{StillSize, extract_frame_jpegs};
use as_media::{Caps, MediaInfo, MediaResult, Source, Tools, build_frame_index, idet, probe, proxy};
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

/// When this process first opened (or made) a project. Nothing of this
/// process wrote into a project or an export folder before it, and one
/// AutoScene runs at a time: a work file last written before it (an
/// unfinished clip or poster, a scratch file) is a crash's leftover.
static FIRST_OPEN: LazyLock<SystemTime> = LazyLock::new(SystemTime::now);

/// A file last written before this process opened a project ([`FIRST_OPEN`]):
/// nothing is writing it any more.
fn left_over(e: &std::fs::DirEntry) -> bool {
    e.metadata().is_ok_and(|m| m.is_file() && m.modified().is_ok_and(|t| t < *FIRST_OPEN))
}

/// Progress callback: `(stage, fraction 0..1)`; `None`: how far it is isn't
/// known for now (a clip failed and its share is skipped: the pace is
/// measured anew from the next fraction).
pub type Progress<'a> = &'a dyn Fn(&str, Option<f64>);

/// Share of a merged export's progress that rendering its clips takes; the
/// join (a copy, and the sound encoded once) takes the rest.
const RENDER_SHARE: f64 = 0.95;

/// What rendering a clip costs besides its frames (starting ffmpeg, seeking
/// into the episode, checking the clip), in frames' work: measured 0.6 s a
/// clip against 6 ms a frame with NVENC, about 100 frames; slower encoders
/// spend less beside their frames (about 25 at 25 ms a frame). Between the
/// two, the time left of a 124-clip export was told within about 5 %; by
/// clip count, 17 %.
const CLIP_WORK: f64 = 48.0;

/// How far along an export of clips is, by the work each one takes.
struct ClipShares(Vec<f64>);

impl ClipShares {
    fn new(frames: impl IntoIterator<Item = u32>) -> Self {
        let mut starts = vec![0.0];
        for f in frames {
            starts.push(starts.last().expect("starts at 0") + f64::from(f) + CLIP_WORK);
        }
        let all = starts.last().copied().unwrap_or_default().max(f64::MIN_POSITIVE);
        Self(starts.into_iter().map(|s| s / all).collect())
    }

    /// The export when clip `i` is `p` done (a clip done is exactly where
    /// the next one starts).
    fn at(&self, i: usize, p: f64) -> f64 {
        let (from, to) = (self.0[i], self.0[i + 1]);
        if p >= 1.0 { to } else { from + (to - from) * p.max(0.0) }
    }
}

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

/// What a folder search for missing media found ([`Project::relink_folder`]).
#[derive(Debug, Default)]
pub struct RelinkedFolder {
    pub summary: RelinkSummary,
    /// Another copy of the file: to index again.
    pub reindex: Vec<MediaId>,
    /// The file as it was: everything kept.
    pub same: Vec<MediaId>,
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
    /// The library's setting: people are followed by their bodies too.
    find_bodies: Arc<AtomicBool>,
    /// Its faces are found with Fast Scan (meta [`SCAN`]).
    fast_scan: AtomicBool,
}

/// How a project's faces are found (`fast`: Fast Scan; unset: in full).
const SCAN: &str = "scan";
const FAST: &str = "fast";

impl Project {
    pub(crate) fn create(s: &ProjectSummary, find_bodies: Arc<AtomicBool>) -> ProjectResult<Self> {
        LazyLock::force(&FIRST_OPEN);
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
        if s.fast_scan {
            db.set_meta(SCAN, FAST)?;
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
            find_bodies,
            fast_scan: AtomicBool::new(s.fast_scan),
        };
        p.ensure_dirs()?;
        Ok(p)
    }

    pub(crate) fn open(id: ProjectId, dir: PathBuf, find_bodies: Arc<AtomicBool>) -> ProjectResult<Self> {
        LazyLock::force(&FIRST_OPEN);
        let db = ProjectDb::open(&dir.join(PROJECT_DB))?;
        let fast_scan = AtomicBool::new(db.meta(SCAN)?.as_deref() == Some(FAST));
        let p = Self {
            id,
            dir,
            db,
            indexes: Mutex::default(),
            proxy_builds: Mutex::default(),
            clustering: Mutex::default(),
            posters: Mutex::default(),
            portraits: Mutex::default(),
            find_bodies,
            fast_scan,
        };
        p.ensure_dirs()?;
        // Exports can't resume mid-way (M1): close them out. No export runs
        // yet, so any parts of a file still on disk (a crash, a folder that
        // couldn't be removed) go.
        p.db.close_stale_exports()?;
        for (id, out_dir) in p.db.merged_exports()? {
            let work = merge_work_dir(Path::new(&out_dir), id);
            if Path::new(&out_dir).is_absolute() && work.is_dir() {
                drop(WorkDir(work));
            }
        }
        p.clean_proxies();
        p.sweep_leftovers();
        // Characters aren't looked for by their bodies any more, but the
        // setting changed while this project was closed (or its faces
        // couldn't be placed again then): what the bodies said goes now.
        if !p.finds_bodies()
            && p.db.has_body_names()?
            && let Err(e) = p.reassign()
        {
            tracing::warn!(project = %p.id, error = %e, "placing the faces again without their bodies");
        }
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

    /// Whether characters are looked for by their bodies too (the library's
    /// setting): their bodies are found with their faces, and named after
    /// every clustering. Never with Fast Scan, which looks for faces alone
    /// (as the Scener app does).
    pub fn finds_bodies(&self) -> bool {
        self.find_bodies.load(Ordering::Relaxed) && !self.fast_scan()
    }

    /// Whether this project's faces are found with Fast Scan (the way the
    /// Scener app finds them; see `as_analysis::fast`).
    pub fn fast_scan(&self) -> bool {
        self.fast_scan.load(Ordering::Relaxed)
    }

    /// Say how this project's faces are found. Faces found the other way are
    /// found again (see [`Project::faces_by_other_model`]), the user's
    /// decisions on them kept. Returns whether it changed; then the cast is
    /// grouped again by the next upkeep.
    pub fn set_fast_scan(&self, on: bool) -> ProjectResult<bool> {
        if self.fast_scan() == on {
            return Ok(false);
        }
        // Not while the cast is being grouped (as for the footage).
        let _one = self.clustering.lock();
        if on {
            self.db.set_meta(SCAN, FAST)?;
        } else {
            self.db.delete_meta(SCAN)?;
        }
        self.fast_scan.store(on, Ordering::Relaxed);
        self.cast_grouped_for_other_footage()?;
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn set_finds_bodies(&self, on: bool) {
        self.find_bodies.store(on, Ordering::Relaxed);
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
        Ok(found(self.db.media_list()?))
    }

    /// [`Self::media_list`] without each media's shots and faces counted
    /// (`shot_count` and `face_count` are `None`): a pass over every track
    /// that only what shows the counts needs.
    pub fn media_list_uncounted(&self) -> ProjectResult<Vec<MediaItem>> {
        Ok(found(self.db.media_list_uncounted()?))
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
    /// video is refused unless `force`. A file showing another picture at the
    /// poster's frame is another video, even at the very same size.
    pub fn relink(&self, tools: &Tools, id: MediaId, path: &Path, force: bool) -> ProjectResult<Relink> {
        let m = self.db.media(id)?.ok_or_else(|| ProjectError::NotFound("This episode was removed.".into()))?;
        if !path.is_file() {
            return Err(ProjectError::NotFound("That file can't be found.".into()));
        }
        let new = path.display().to_string();
        if same_path(&m.path, &new) && Path::new(&m.path).is_file() {
            return Ok(Relink::Same);
        }
        // Another path to a file in the project keeps its name (a mapped
        // drive and its network path, a link to the folder): only media of
        // that name are looked up on disk, not every episode of a long series
        // for every file a folder search tries.
        let lower_name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Some(other) = self
            .db
            .media_list_uncounted()?
            .into_iter()
            .find(|x| x.id != id && x.file_name.to_lowercase() == lower_name && same_path(&x.path, &new))
        {
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
        // The same bytes keep everything only when the frame index on disk
        // was made from them (an indexing that never finished left an older
        // one).
        let indexed = m.status == MediaStatus::Ready && m.info.is_some();
        let same_bytes = same.is_ok() && indexed && meta.len() == m.size_bytes as u64;
        // Does it show the same picture as the poster, at the poster's
        // frame? Two episodes can be just as long, and another video can
        // have the very same size, so this is asked of the same bytes too.
        // Without a poster or an index nobody can tell (`None`).
        let same_picture = match same.is_ok() && m.info.is_some() {
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
        let changed_key = crate::cast::source_changed_key(id);
        let how = if same_bytes && same_picture != Some(false) {
            Relinked::Same
        } else if same_picture == Some(true) {
            Relinked::Copy
        } else {
            // Not shown to be the same picture: its analysis is made again,
            // and the decisions wait for faces that look like theirs.
            Relinked::Changed { changed_key: &changed_key }
        };
        // All of it or nothing: never the new file on the old one's index.
        if !self.db.relink_media(id, &new, &file_name, meta.len(), mtime, how)? {
            return Err(ProjectError::NotFound("This episode was removed.".into()));
        }
        tracing::info!(media = %id, file = %file_name, ?same_picture, "found again");
        if how == Relinked::Same {
            return Ok(Relink::Same);
        }
        self.indexes.lock().remove(&id);
        Ok(Relink::Reindex)
    }

    /// Whether the file at `path` (probed as `info`) shows what the media's
    /// poster shows, at the same frame; `None` when that can't be told.
    fn same_picture(&self, tools: &Tools, m: &MediaItem, path: &Path, info: &MediaInfo) -> Option<bool> {
        let poster = m.poster.as_deref().map(Path::new).filter(|p| p.is_file())?;
        let index = self.frame_index(m.id).ok()?;
        let probe = self.tmp_dir().join(format!("relink-{}.jpg", m.id));
        // Deinterlaced like the poster was: the field order indexing found
        // (the stream may not say), on what `same_source` showed is the same stream.
        let mut info = info.clone();
        if info.video.field_order.is_none() {
            info.video.field_order = m.info.as_ref().and_then(|i| i.video.field_order.clone());
        }
        let src = Source { path, info: &info, index: &index };
        let made = extract_frame_jpegs(tools, src, index.len() / 10, &[(POSTER_SMALL, &probe)], &CancelToken::new());
        let score = made.ok().and_then(|()| as_media::poster::picture_similarity(tools, poster, &probe).ok());
        let _ = std::fs::remove_file(&probe);
        let score = score?;
        tracing::info!(media = %m.id, similarity = score, "compared the picture of a file found again");
        Some(score >= SAME_PICTURE)
    }

    /// Look for every missing media in `dir` (and its folders): by file name,
    /// then size, then episode number — each candidate only if it is the
    /// same video ([`Self::relink`]).
    pub fn relink_folder(&self, tools: &Tools, dir: &Path) -> ProjectResult<RelinkedFolder> {
        let missing: Vec<MediaItem> =
            self.media_list_uncounted()?.into_iter().filter(|m| m.status == MediaStatus::Missing).collect();
        let known = self.db.media_paths()?;
        let found = crate::library::scan_paths(&[dir.display().to_string()], ProjectKind::Series, &known);
        let mut used: HashSet<String> = HashSet::new();
        let mut out = RelinkedFolder::default();
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
                Some(Relink::Same) => {
                    out.summary.same += 1;
                    out.same.push(m.id);
                }
                Some(_) => {
                    out.summary.reindex += 1;
                    out.reindex.push(m.id);
                }
                None => out.summary.missing += 1,
            }
        }
        Ok(out)
    }

    pub fn remove_media(&self, id: MediaId) -> ProjectResult<()> {
        let m = self.media(id)?;
        self.db.remove_media(id)?;
        let _ = self.db.delete_meta(&crate::cast::source_changed_key(id));
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
        let posters = self.media_list_uncounted()?.iter().filter(|m| self.posters_outdated(m)).map(|m| m.id).collect();
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
        progress("Reading file", Some(0.0));
        let mut info = probe(tools, &path)?;
        cancel.check().map_err(|_| ProjectError::Cancelled)?;
        if info.video.field_order.is_none()
            && let Some(order) = idet::detect_field_order(tools, &path, &info)?
        {
            tracing::info!(file = %m.file_name, order, "interlaced content detected");
            info.video.field_order = Some(order.to_string());
        }

        let index = build_frame_index(tools, &path, &info, &self.tmp_dir(), cancel, |p| {
            progress("Indexing frames", Some(p * 0.95))
        })?;

        progress("Creating poster", Some(0.96));
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
        let result = proxy::build_proxy(tools, src, audio, &out, cancel, |p| progress("Preparing preview", Some(p)));
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
        let Ok(media) = self.db.media_list_uncounted() else { return };
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

    /// Remove what a crash (or a delete that failed) left behind: pictures of
    /// media that are gone (`faces/<id>/`, `thumbs/<id>-*.jpg`), unfinished
    /// posters, scratch files, and this project's unfinished clips in export
    /// folders. Only names this app gives are touched, and nothing being
    /// written (nothing of this project runs before it is open; a tool
    /// opening it beside the app leaves the app's files alone).
    fn sweep_leftovers(&self) {
        // Listed before the media are read: a media's row always exists
        // before its files do.
        let faces: Vec<_> = std::fs::read_dir(self.faces_dir()).into_iter().flatten().flatten().collect();
        let thumbs: Vec<_> = std::fs::read_dir(self.thumbs_dir()).into_iter().flatten().flatten().collect();
        let Ok(media) = self.db.media_list_uncounted() else { return };
        let known: HashSet<MediaId> = media.iter().map(|m| m.id).collect();
        let gone = |name: &str| name.parse::<MediaId>().is_ok_and(|id| !known.contains(&id));
        for e in faces {
            if gone(&e.file_name().to_string_lossy()) {
                tracing::debug!(dir = %e.path().display(), "removing the faces of a removed media");
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        for e in thumbs {
            let name = e.file_name().to_string_lossy().into_owned();
            // `<id>-<size>.jpg` (`<id>.jpg` from before the sizes).
            let poster = name.get(..36).filter(|_| name[36..].starts_with(['-', '.']));
            let unfinished = |id: &str| id.parse::<MediaId>().is_ok() && name.ends_with(".part.jpg") && left_over(&e);
            if poster.is_some_and(|id| gone(id) || unfinished(id)) {
                tracing::debug!(file = %e.path().display(), "removing a poster nothing uses");
                let _ = std::fs::remove_file(e.path());
            }
        }
        for e in std::fs::read_dir(self.tmp_dir()).into_iter().flatten().flatten() {
            if left_over(&e) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        self.clean_partial_clips();
    }

    /// Unfinished clips a crash left in their export folder's work folder:
    /// this project's clips that never finished (`<name>.*`), last written
    /// before this process opened a project — another project may be
    /// exporting a clip of the same name there now. (A merged export's
    /// parts are in a folder of its own, removed by `open`.)
    fn clean_partial_clips(&self) {
        let Ok(exports) = self.db.exports(self.id) else { return };
        let mut unfinished: HashMap<&str, HashSet<&str>> = HashMap::new();
        for e in exports.iter().filter(|e| e.merged.is_none() && Path::new(&e.out_dir).is_absolute()) {
            for item in e.items.iter().filter(|i| i.status != ExportStatus::Done) {
                unfinished.entry(e.out_dir.as_str()).or_default().insert(item.name.as_str());
            }
        }
        for (out_dir, names) in unfinished {
            let work = Path::new(out_dir).join(PARTIAL_DIR);
            let mut removed = false;
            for e in std::fs::read_dir(&work).into_iter().flatten().flatten() {
                let file = e.file_name().to_string_lossy().into_owned();
                let ours = names.iter().any(|n| file.strip_prefix(n).is_some_and(|rest| rest.starts_with('.')));
                if ours && left_over(&e) {
                    tracing::debug!(file = %e.path().display(), "removing an unfinished clip");
                    removed |= std::fs::remove_file(e.path()).is_ok();
                }
            }
            // Only when something went: another export may have just made
            // it for its clip.
            if removed {
                let _ = std::fs::remove_dir(&work);
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
        check_export_dir(out_dir)?;
        let title = self.title()?;
        let ext = preset.container.ext();
        // Names already promised to exports that haven't written their files
        // yet: their clips', or the one file they join them into. Compared
        // case-insensitively, like Windows and macOS file names.
        let out_str = out_dir.display().to_string();
        let mut promised: HashSet<String> = HashSet::new();
        for e in self.exports()?.into_iter().filter(|e| e.out_dir == out_str && e.preset.container == preset.container)
        {
            let live = |s: ExportStatus| matches!(s, ExportStatus::Queued | ExportStatus::Running);
            match &e.merged {
                Some(file) if live(e.status) => {
                    promised.insert(file.name.to_lowercase());
                }
                Some(_) => {}
                None => promised.extend(e.items.iter().filter(|i| live(i.status)).map(|i| i.name.to_lowercase())),
            }
        }
        // A Bit-Perfect clip is a .mov, or an .mkv where FFV1 keeps it.
        let exts: &[&str] = if preset.video == VideoCodec::Lossless { &["mov", "mkv"] } else { &[ext] };
        let taken = |n: &str, promised: &HashSet<String>| {
            promised.contains(&n.to_lowercase()) || exts.iter().any(|e| out_dir.join(format!("{n}.{e}")).exists())
        };
        let apart = req.merge.is_some() && self.apart_reason(&preset, &req.clips)?.is_some();
        let merged = match req.merge.as_deref().map(str::trim).filter(|_| !apart) {
            Some("") => return Err(ProjectError::Invalid("The file needs a name.".into())),
            Some(name) => {
                let name = unique_name(&safe_component(name, " ", 120), |n| taken(n, &promised));
                Some(MergedFile { name, path: None, verified: false, bytes: None, error: None })
            }
            None => None,
        };
        // The clips of one file only name its chapters: they only need to
        // differ from each other.
        let mut used: HashSet<String> = if merged.is_some() { HashSet::new() } else { promised.clone() };
        // Each episode once, however many clips it gives.
        let mut sources: HashMap<MediaId, (MediaItem, MediaInfo, Arc<FrameIndex>)> = HashMap::new();
        let mut items = Vec::with_capacity(req.clips.len());
        for c in &req.clips {
            if let std::collections::hash_map::Entry::Vacant(e) = sources.entry(c.media_id) {
                e.insert(self.ready_source(c.media_id)?);
            }
            let (m, _, index) = &sources[&c.media_id];
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
                None => default_clip_name(&title, m, index, c.start),
            };
            // Windows and macOS file names ignore case.
            let name = unique_name(&base, |n| match merged {
                Some(_) => used.contains(&n.to_lowercase()),
                None => taken(n, &used),
            });
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
        self.db.insert_export(id, &preset, &out_str, req.label.as_deref(), merged.as_ref(), &items)?;
        self.export(id)
    }

    /// What an export of `clips` would make: the names its files get (as
    /// [`Project::create_export`] names them, before telling them apart from
    /// files already in the folder) and its size with each of `presets`, with
    /// the encoders of `caps`.
    pub fn export_plan(
        &self,
        clips: &[ClipRequest],
        presets: &[ExportPreset],
        caps: &Caps,
    ) -> ProjectResult<ExportPlan> {
        let title = self.title()?;
        let mut sources: HashMap<MediaId, (MediaItem, MediaInfo, Arc<FrameIndex>)> = HashMap::new();
        let mut used: HashSet<String> = HashSet::new();
        let mut names = Vec::with_capacity(clips.len());
        for c in clips {
            if let std::collections::hash_map::Entry::Vacant(e) = sources.entry(c.media_id) {
                e.insert(self.ready_source(c.media_id)?);
            }
            let (m, _, index) = &sources[&c.media_id];
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
        }
        let sized: Vec<EstimateClip> = clips
            .iter()
            .map(|c| {
                let (m, info, index) = &sources[&c.media_id];
                EstimateClip {
                    info,
                    audio_ordinal: m.audio_ordinal,
                    frames: u64::from(c.end - c.start),
                    seconds: index.t(c.end) - index.t(c.start),
                    fps: index.nominal_fps.to_f64(),
                }
            })
            .collect();
        let sizes = presets
            .iter()
            .map(|p| {
                let (ext, note) = lossless_note(p, sized.iter().map(|c| c.info));
                PresetSize {
                    preset_id: p.id.clone(),
                    bytes: estimate_bytes(p, caps, &sized, false),
                    apart: None,
                    ext,
                    note,
                }
            })
            .collect();
        Ok(ExportPlan { names, sizes, folder: None, file: None })
    }

    /// Why a Bit-Perfect file of `clips` is made a file per clip instead
    /// (their episodes' pictures or sound differ); `None` for other presets,
    /// or when they can share one.
    pub(crate) fn apart_reason(&self, preset: &ExportPreset, clips: &[ClipRequest]) -> ProjectResult<Option<String>> {
        if preset.video != VideoCodec::Lossless {
            return Ok(None);
        }
        let mut sources: HashMap<MediaId, (MediaItem, MediaInfo, Arc<FrameIndex>)> = HashMap::new();
        for c in clips {
            if let std::collections::hash_map::Entry::Vacant(e) = sources.entry(c.media_id) {
                e.insert(self.ready_source(c.media_id)?);
            }
        }
        let merge: Vec<MergeSource> = clips
            .iter()
            .filter(|c| c.start < c.end)
            .map(|c| {
                let (m, info, index) = &sources[&c.media_id];
                MergeSource { info, index, range: FrameRange::new(c.start, c.end), audio_ordinal: m.audio_ordinal }
            })
            .collect();
        Ok(lossless_merge_blocker(&merge).map(|why| apart_sentence(&why)))
    }

    /// Mark an export failed before it could start (e.g. ffmpeg missing).
    pub fn fail_export(&self, id: ExportId, message: &str) -> ProjectResult<()> {
        let ex = self.export(id)?;
        for item in &ex.items {
            self.db.update_export_item(item.id, ExportStatus::Failed, None, Some(message))?;
        }
        if let Some(file) = ex.merged {
            self.db.set_export_merged(id, &MergedFile { error: Some(message.into()), ..file }, &[])?;
        }
        Ok(self.db.set_export_status(id, ExportStatus::Failed)?)
    }

    pub fn export(&self, id: ExportId) -> ProjectResult<ExportSummary> {
        self.db.export(self.id, id)?.ok_or_else(|| ProjectError::NotFound("This export no longer exists.".into()))
    }

    pub fn exports(&self) -> ProjectResult<Vec<ExportSummary>> {
        Ok(self.db.exports(self.id)?)
    }

    /// Render every clip of an export (and join them into one file, if it
    /// asks for that). A failing clip doesn't stop the others; cancelling
    /// marks the remaining clips cancelled. `changed` runs whenever a clip
    /// starts or finishes, so the UI can follow along.
    pub fn run_export(
        &self,
        tools: &Tools,
        id: ExportId,
        cancel: &CancelToken,
        progress: Progress,
        changed: &dyn Fn(),
    ) -> ProjectResult<ExportSummary> {
        let ex = self.export(id)?;
        check_export_dir(Path::new(&ex.out_dir))?;
        self.db.set_export_status(id, ExportStatus::Running)?;
        if let Some(file) = ex.merged.clone() {
            return self.run_merged(tools, &ex, file, cancel, progress, changed);
        }
        let shares = ClipShares::new(ex.items.iter().map(|i| i.end - i.start));
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
                    out_stem: out_dir.join(stored_name(&item.name)),
                    title: Some(item.name.clone()),
                    part: None,
                };
                Ok(export_clip(tools, &job, cancel, |p| progress(&stage, Some(shares.at(i, p))))?)
            })();
            match result {
                Ok(outcome) => self.db.update_export_item(item.id, ExportStatus::Done, Some(&outcome), None)?,
                Err(ProjectError::Cancelled) => break,
                Err(e) => {
                    failed = true;
                    tracing::warn!(clip = %item.name, error = %e, detail = %e.detail(), "clip export failed");
                    self.db.update_export_item(item.id, ExportStatus::Failed, None, Some(&e.to_string()))?;
                    // Its share is skipped, not made: no pace to learn from.
                    progress(&stage, None);
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

    /// Render every clip of `ex` as a part of one file, then join them, with
    /// a chapter per clip. A clip that fails is left out of the file; the
    /// parts go once the file is made (or not).
    fn run_merged(
        &self,
        tools: &Tools,
        ex: &ExportSummary,
        file: MergedFile,
        cancel: &CancelToken,
        progress: Progress,
        changed: &dyn Fn(),
    ) -> ProjectResult<ExportSummary> {
        let id = ex.id;
        let out_dir = PathBuf::from(&ex.out_dir);
        let work = WorkDir(merge_work_dir(&out_dir, id));
        let fail = |item: &ExportItem, e: &ProjectError| -> ProjectResult<()> {
            tracing::warn!(clip = %item.name, error = %e, detail = %e.detail(), "clip export failed");
            Ok(self.db.update_export_part(item.id, ExportStatus::Failed, None, Some(&e.to_string()))?)
        };
        let finish = |file: MergedFile, in_file: &[i64]| -> ProjectResult<ExportSummary> {
            let failed = file.error.is_some() || self.export(id)?.items.iter().any(|i| i.error.is_some());
            self.db.set_export_merged(id, &file, in_file)?;
            self.db.set_export_status(id, if failed { ExportStatus::Failed } else { ExportStatus::Done })?;
            changed();
            self.export(id)
        };
        let none_left = || MergedFile { error: Some("None of the clips could be exported.".into()), ..file.clone() };
        if let Err(e) = std::fs::create_dir_all(&work.0) {
            let e = ProjectError::Invalid(format!("Can't write to {}: {e}", out_dir.display()));
            for item in &ex.items {
                fail(item, &e)?;
            }
            return finish(MergedFile { error: Some(e.to_string()), ..file }, &[]);
        }

        // Each episode once, however many clips it gives (one that can't be
        // cut from is asked again for each of its clips, which all fail).
        let mut episodes: HashMap<MediaId, Rc<(PathBuf, MediaInfo, Arc<FrameIndex>)>> = HashMap::new();
        let mut sources = Vec::with_capacity(ex.items.len());
        // Clips were checked when the export was made, but an episode can be
        // indexed again while it waits (relinked to a shorter file): a clip
        // past its end is left out, before the plan would look it up.
        let outside = |item: &ExportItem, (path, _, index): &(PathBuf, MediaInfo, Arc<FrameIndex>)| {
            let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into());
            let (start, end, n) = (item.start, item.end, index.len());
            ProjectError::Invalid(format!("Frames {start}–{end} are outside {name} (0–{n})."))
        };
        for item in &ex.items {
            let episode = match episodes.get(&item.media_id) {
                Some(e) => Ok(e.clone()),
                None => self.ready_source(item.media_id).map(|(m, info, index)| {
                    let e = Rc::new((PathBuf::from(&m.path), info, index));
                    episodes.insert(item.media_id, e.clone());
                    e
                }),
            };
            match episode.and_then(|e| Ok((e, self.db.export_item_audio(item.id)?))) {
                Ok((e, _)) if item.start >= item.end || item.end > e.2.len() => fail(item, &outside(item, &e))?,
                Ok((e, audio)) => sources.push((item, e, audio)),
                Err(e) => fail(item, &e)?,
            }
        }
        if sources.is_empty() {
            return finish(none_left(), &[]);
        }
        let merge_sources: Vec<MergeSource> = sources
            .iter()
            .map(|(item, episode, audio)| MergeSource {
                info: &episode.1,
                index: &episode.2,
                range: FrameRange::new(item.start, item.end),
                audio_ordinal: *audio,
            })
            .collect();
        let planned = if ex.preset.video == VideoCodec::Lossless {
            // What every episode decodes to, exactly: only alike ones join.
            sources
                .iter()
                .map(|(_, e, _)| exact_format(tools, &e.0, e.1.video.index))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|exact| plan_lossless_merge(tools.caps(), &merge_sources, &exact))
        } else {
            plan_merge(tools.caps(), &ex.preset, &merge_sources)
        };
        let plan = match planned {
            Ok(plan) => plan,
            Err(e) => {
                let e = ProjectError::from(e);
                for (item, ..) in &sources {
                    fail(item, &e)?;
                }
                return finish(MergedFile { error: Some(e.to_string()), ..file }, &[]);
            }
        };

        let n = sources.len();
        let shares = ClipShares::new(sources.iter().map(|(item, ..)| item.end - item.start));
        let mut parts = Vec::with_capacity(n);
        let mut in_file = Vec::with_capacity(n);
        let mut verified = true;
        for (i, ((item, episode, audio), timing)) in sources.iter().zip(&plan.parts).enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            let (path, info, index) = &**episode;
            let stage = if n > 1 { format!("Clip {} of {n}", i + 1) } else { "Rendering".into() };
            self.db.update_export_part(item.id, ExportStatus::Running, None, None)?;
            changed();
            let job = ClipJob {
                src: Source { path, info, index },
                range: FrameRange::new(item.start, item.end),
                audio_ordinal: *audio,
                preset: &ex.preset,
                out_stem: work.0.join(format!("part-{:05}", i + 1)),
                title: None,
                part: Some(Part { shape: &plan.shape, samples: timing.samples }),
            };
            let share = |p: f64| shares.at(i, p) * RENDER_SHARE;
            match export_clip(tools, &job, cancel, |p| progress(&stage, Some(share(p)))).map_err(ProjectError::from) {
                Ok(outcome) => {
                    self.db.update_export_part(item.id, ExportStatus::Done, Some(&outcome), None)?;
                    verified &= outcome.verified;
                    in_file.push(item.id);
                    parts.push(JoinPart {
                        path: PathBuf::from(&outcome.path),
                        frames: outcome.frames,
                        timing: *timing,
                        title: item.name.clone(),
                    });
                }
                Err(ProjectError::Cancelled) => break,
                Err(e) => {
                    fail(item, &e)?;
                    // Its share is skipped, not made: no pace to learn from.
                    progress(&stage, None);
                }
            }
            changed();
        }
        if cancel.is_cancelled() {
            return self.cancel_merged(id);
        }
        if parts.is_empty() {
            return finish(none_left(), &[]);
        }

        let stage = "Joining into one file";
        progress(stage, Some(RENDER_SHARE));
        let job = JoinJob {
            parts: &parts,
            shape: &plan.shape,
            preset: &ex.preset,
            out_stem: out_dir.join(stored_name(&file.name)),
            work_dir: &work.0,
            title: ex.label.clone(),
        };
        match join_parts(tools, &job, cancel, |p| progress(stage, Some(RENDER_SHARE + (1.0 - RENDER_SHARE) * p)))
            .map_err(ProjectError::from)
        {
            Ok(outcome) => {
                // A name taken meanwhile got a `_2`.
                let name = Path::new(&outcome.path).file_stem().map(|s| s.to_string_lossy().into_owned());
                let done = MergedFile {
                    name: name.unwrap_or(file.name),
                    verified: verified && outcome.verified,
                    bytes: Some(outcome.bytes),
                    path: Some(outcome.path),
                    error: None,
                };
                finish(done, &in_file)
            }
            Err(ProjectError::Cancelled) => self.cancel_merged(id),
            Err(e) => {
                tracing::warn!(error = %e, detail = %e.detail(), "joining the clips failed");
                for item in ex.items.iter().filter(|i| in_file.contains(&i.id)) {
                    let message = "It couldn't be joined into the file.";
                    self.db.update_export_part(item.id, ExportStatus::Failed, None, Some(message))?;
                }
                finish(MergedFile { error: Some(e.to_string()), ..file }, &[])
            }
        }
    }

    /// An export that stopped on an error of its own (the database, the
    /// disk): what hadn't finished failed, so nothing stays "exporting".
    pub fn abandon_export(&self, id: ExportId, message: &str) -> ProjectResult<()> {
        let ex = self.export(id)?;
        for item in &ex.items {
            let live = matches!(item.status, ExportStatus::Queued | ExportStatus::Running);
            match &ex.merged {
                // Rendered clips of a file that wasn't made are in no file.
                Some(file) if live || (item.status == ExportStatus::Done && file.path.is_none()) => {
                    self.db.update_export_part(item.id, ExportStatus::Failed, None, Some(message))?
                }
                None if live => self.db.update_export_item(item.id, ExportStatus::Failed, None, Some(message))?,
                _ => {}
            }
        }
        if let Some(file) = ex.merged.filter(|f| f.path.is_none() && f.error.is_none()) {
            self.db.set_export_merged(id, &MergedFile { error: Some(message.into()), ..file }, &[])?;
        }
        Ok(self.db.set_export_status(id, ExportStatus::Failed)?)
    }

    /// A merged export stopped: none of its clips is in a file.
    fn cancel_merged(&self, id: ExportId) -> ProjectResult<ExportSummary> {
        for item in self.export(id)?.items.iter().filter(|i| i.status != ExportStatus::Failed) {
            self.db.update_export_part(item.id, ExportStatus::Cancelled, None, None)?;
        }
        self.db.set_export_status(id, ExportStatus::Cancelled)?;
        Err(ProjectError::Cancelled)
    }
}

/// Where a merged export renders its parts: a folder of its own in the
/// destination's hidden work folder (on the same drive as the file, so it
/// is moved into place, not copied).
/// For a Bit-Perfect export of clips of `infos`, the extension when it
/// isn't the preset's (`mkv`: every clip FFV1) and a note when FFV1 keeps
/// some: After Effects can't open those.
pub(crate) fn lossless_note<'a>(
    preset: &ExportPreset,
    infos: impl Iterator<Item = &'a MediaInfo>,
) -> (Option<String>, Option<String>) {
    if preset.video != VideoCodec::Lossless {
        return (None, None);
    }
    let codecs: Vec<LosslessCodec> =
        infos.map(|i| as_export::lossless::codec_for(&i.video.pix_fmt, (i.video.width, i.video.height))).collect();
    let ffv1 = codecs.iter().filter(|c| **c == LosslessCodec::Ffv1).count();
    if ffv1 == 0 {
        return (None, None);
    }
    let all = ffv1 == codecs.len();
    let ext = all.then(|| "mkv".to_string());
    let whose = if all { "These pictures" } else { "Some of these pictures" };
    let note = format!(
        "{whose} can't be stored exactly as HEVC, so they're kept as FFV1 in an MKV: DaVinci Resolve and VLC open it, After Effects doesn't."
    );
    (ext, Some(note))
}

/// What the export dialog says when Bit-Perfect makes a file per clip.
pub(crate) fn apart_sentence(why: &str) -> String {
    format!("Bit-Perfect makes a file per shot here: {why}.")
}

fn merge_work_dir(out_dir: &Path, id: ExportId) -> PathBuf {
    out_dir.join(PARTIAL_DIR).join(id.to_string())
}

/// Exports go to a full folder path: a relative one would land wherever the
/// app happens to run from.
fn check_export_dir(out_dir: &Path) -> ProjectResult<()> {
    match out_dir.is_absolute() {
        true => Ok(()),
        false => Err(ProjectError::Invalid("Choose a full folder path for the export.".into())),
    }
}

/// Longer than any name an export is given (up to 120 characters, and a
/// `_2` that tells it apart), so those come out as they are.
const STORED_NAME_CHARS: usize = 200;

/// The file name an export's database row gives, made safe once more right
/// before something is written under it: one path component, never a way out
/// of the export's folder or a device name. Names are made safe when the
/// export is made, so they come out the same; the database isn't trusted
/// with where files go.
fn stored_name(name: &str) -> String {
    let safe = safe_component(name, " ", STORED_NAME_CHARS);
    if safe != name {
        tracing::warn!(name, safe, "an export's stored file name wasn't safe to write");
    }
    safe
}

/// A merged export's work folder, removed with everything in it however the
/// export ends; the shared hidden folder goes too once nothing else uses it.
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        // A virus scanner or the search indexer may still hold a new part
        // open for a moment: try again before giving up (until the next
        // launch, which looks again).
        for attempt in 1..=5 {
            match std::fs::remove_dir_all(&self.0) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound && attempt < 5 => {
                    std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
                }
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    tracing::warn!(dir = %self.0.display(), error = %e, "couldn't remove an export's work folder");
                }
                _ => break,
            }
        }
        if let Some(shared) = self.0.parent() {
            let _ = std::fs::remove_dir(shared);
        }
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

/// `list` with the media whose files disappeared as `missing`.
fn found(mut list: Vec<MediaItem>) -> Vec<MediaItem> {
    for m in &mut list {
        if !Path::new(&m.path).is_file() {
            m.status = MediaStatus::Missing;
        }
    }
    list
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

/// Two paths name the same file: spelled alike (on Windows whatever the case
/// and slashes), or, when both files are there, leading to the same file
/// once `.` and `..`, links, mapped drives (and their network paths), `\\?\`
/// and short 8.3 names are resolved.
fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| {
        let p = p.replace('\\', "/");
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    norm(a) == norm(b) || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
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
                &Caps::default(),
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
        assert!(project.export_plan(&[clip(2390, 2500, None)], &presets, &Caps::default()).is_err());
    }

    #[test]
    fn an_export_moves_on_by_the_work_its_clips_take() {
        // 148 and 348 frames' work of 496: the long clip takes most of the bar.
        let s = ClipShares::new([100, 300]);
        let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-12, "{a} vs {b}");
        assert_eq!(s.at(0, 0.0), 0.0);
        near(s.at(0, 1.0), 148.0 / 496.0);
        assert_eq!(s.at(0, 1.0), s.at(1, 0.0));
        near(s.at(1, 0.5), (148.0 + 174.0) / 496.0);
        assert_eq!(s.at(1, 1.0), 1.0);
        assert_eq!(s.at(1, 2.0), 1.0, "past the end stays at the end");
        assert_eq!(ClipShares::new([]).0, [0.0]);
    }

    #[test]
    fn what_a_crash_left_goes_when_the_project_opens() {
        let (dir, project, m) = crate::analysis::tests::project_with_media(240);
        let gone = MediaId::new();
        let write = |p: &Path| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
            p.to_path_buf()
        };
        // Written before this process opened a project (as by a run that crashed).
        let old = |p: &Path| {
            let p = write(p);
            let hour_ago = SystemTime::now() - std::time::Duration::from_secs(3600);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(hour_ago).unwrap();
            p
        };
        let (faces, thumbs) = (project.faces_dir(), project.thumbs_dir());
        let stray_faces = write(&faces.join(gone.to_string()).join("1.jpg"));
        let stray_poster = write(&thumbs.join(format!("{gone}-720.jpg")));
        let stray_legacy = write(&thumbs.join(format!("{gone}.jpg")));
        let half_poster = old(&thumbs.join(format!("{m}-720.part.jpg")));
        let scratch = old(&project.tmp_dir().join("index-1-1.txt"));
        let kept = [
            write(&faces.join(m.to_string()).join("1.jpg")),
            write(&thumbs.join(format!("{m}-720.jpg"))),
            write(&faces.join("notes").join("mine.txt")),
            write(&thumbs.join("cover.jpg")),
            // Being written by an app beside it (not this process).
            write(&project.tmp_dir().join("index-2-1.txt")),
        ];

        // A clip of an export cut short, and one that finished.
        let out = dir.path().join("Exports");
        let preset = as_export::builtin_presets().remove(0);
        let item =
            |name: &str| NewExportItem { media_id: m, start: 0, end: 24, name: name.into(), audio_ordinal: None };
        let ex = ExportId::new();
        let out_str = out.display().to_string();
        project.db().insert_export(ex, &preset, &out_str, None, None, &[item("Clip 1"), item("Clip 2")]).unwrap();
        let done = project.export(ex).unwrap().items[1].id;
        project.db().update_export_item(done, ExportStatus::Done, None, None).unwrap();
        let work = out.join(PARTIAL_DIR);
        let cut_short = [old(&work.join("Clip 1.mov")), old(&work.join("Clip 1.stats.txt"))];
        // The same name being exported now (by another project), someone
        // else's, and a finished clip's name: never touched.
        let live = write(&work.join("Clip 1.mkv"));
        let others = [old(&work.join("Clip 10.mov")), old(&work.join("Clip 2.mov"))];

        let (id, project_dir) = (project.id, project.dir.clone());
        drop(project);
        let project = Project::open(id, project_dir, Arc::new(AtomicBool::new(false))).unwrap();
        for p in [&stray_faces, &stray_poster, &stray_legacy, &half_poster, &scratch].into_iter().chain(&cut_short) {
            assert!(!p.exists(), "{} is left over", p.display());
        }
        assert!(!faces.join(gone.to_string()).exists());
        for p in kept.iter().chain(&others).chain([&live]) {
            assert!(p.exists(), "{} was removed", p.display());
        }
        assert_eq!(project.media(m).unwrap().id, m);
    }

    #[test]
    fn unique_names() {
        let taken = ["a", "a_2"];
        assert_eq!(unique_name("a", |n| taken.contains(&n)), "a_3");
        assert_eq!(unique_name("b", |n| taken.contains(&n)), "b");
    }

    #[test]
    fn the_same_file_by_another_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Show - 01.mkv");
        std::fs::write(&file, b"x").unwrap();
        std::fs::create_dir(dir.path().join("Season 1")).unwrap();
        let s = |p: &Path| p.display().to_string();
        let plain = s(&file);
        assert!(same_path(&plain, &plain));
        // Spelled otherwise, leading to the same file.
        assert!(same_path(&plain, &s(&dir.path().join(".").join("Show - 01.mkv"))));
        assert!(same_path(&s(&dir.path().join("Season 1").join("..").join("Show - 01.mkv")), &plain));
        #[cfg(windows)]
        {
            assert!(same_path(&plain, &format!(r"\\?\{plain}")));
            assert!(same_path(&plain, &plain.to_uppercase()));
            assert!(same_path(&plain, &plain.replace('\\', "/")));
        }
        #[cfg(unix)]
        {
            let link = dir.path().join("Linked");
            std::os::unix::fs::symlink(dir.path(), &link).unwrap();
            assert!(same_path(&plain, &s(&link.join("Show - 01.mkv"))));
        }
        // Another file is another file.
        let other = dir.path().join("Show - 02.mkv");
        std::fs::write(&other, b"x").unwrap();
        assert!(!same_path(&plain, &s(&other)));
        // A file that isn't there can only be told by its spelling.
        let gone = s(&dir.path().join("Gone.mkv"));
        assert!(same_path(&gone, &gone));
        assert!(!same_path(&gone, &s(&dir.path().join(".").join("Gone.mkv"))));
    }

    #[test]
    fn exports_are_written_where_their_names_say_and_nowhere_else() {
        // Names exports are given come back as they are.
        for raw in ["Opening: shot", "  What?  If  ", "Frieren – 03", "con", "Con.Air", "...", &"x".repeat(300)] {
            let made = safe_component(raw, " ", 120);
            for name in [made.clone(), format!("{made}_2"), format!("{made}_123")] {
                assert_eq!(stored_name(&name), name);
            }
        }
        let rendered = default_clip_name(
            "Show: The Series",
            &media(MediaStatus::Ready, None, None),
            &FrameIndex::cfr(Rational::new(1, 24), Rational::new(24, 1), 0, 240),
            48,
        );
        assert_eq!(stored_name(&rendered), rendered);
        // Anything else a database says is one name in the export's folder.
        assert_eq!(stored_name("../../Windows/evil"), ".. .. Windows evil");
        assert_eq!(stored_name(r"C:\Users\me\x"), "C Users me x");
        assert_eq!(stored_name(".."), "_");
        assert_eq!(stored_name("nul.x"), "_nul.x");
        // And the folder is a full path.
        assert!(matches!(check_export_dir(Path::new("relative/out")), Err(ProjectError::Invalid(_))));
        assert!(check_export_dir(&std::env::temp_dir()).is_ok());
    }

    #[test]
    fn an_export_to_a_relative_folder_is_refused() {
        let (_dir, project, m) = crate::analysis::tests::project_with_media(240);
        let req = ExportRequest {
            preset_id: "h264-edit".into(),
            out_dir: None,
            clips: vec![ClipRequest { media_id: m, start: 0, end: 24, name: None }],
            label: None,
            merge: None,
        };
        let preset = as_export::builtin_presets().into_iter().find(|p| p.id == "h264-edit").unwrap();
        let e = project.create_export(&req, preset, Path::new("Exports")).unwrap_err();
        assert!(matches!(&e, ProjectError::Invalid(s) if s == "Choose a full folder path for the export."), "{e:?}");
        assert!(project.exports().unwrap().is_empty());
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
            bodies: as_api::StageState::Pending,
            bodies_error: None,
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
