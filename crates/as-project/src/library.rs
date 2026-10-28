use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use as_api::{AppSettings, ContentKind, ImportCandidate, NewProject, ProjectKind, ProjectSummary, now_ms};
use as_core::{AppPaths, MediaId, ProjectId};
use as_db::{LibraryDb, NewMedia, ProjectDb};
use parking_lot::Mutex;

use crate::naming::{episode_cmp, is_video, parse_episode, safe_component};
use crate::project::{PROJECT_DB, Project};
use crate::{ProjectError, ProjectResult};

const KEY_LIBRARY_ROOT: &str = "library_root";
const KEY_EXPORT_PRESET: &str = "export_preset_id";
const DEFAULT_EXPORT_PRESET: &str = "prores-hq";
const KEY_AUDIO_LANGUAGE: &str = "audio_language";
const KEY_FIND_BODIES: &str = "find_bodies";
const KEY_FAST_SCAN: &str = "fast_scan";
/// Most series and films people cut are watched in their original English
/// track; dubbed tracks are often flagged "default" too, so a language
/// preference beats the file's flag.
const DEFAULT_AUDIO_LANGUAGE: &str = "eng";
/// How deep folder imports look (`Show/Season 1/Disc 2/file.mkv`).
const SCAN_DEPTH: usize = 4;

/// The project list, settings, and the cache of open projects.
pub struct Library {
    db: LibraryDb,
    open: Mutex<HashMap<ProjectId, Arc<Project>>>,
    /// One lock per project that was opened here: a project is opened once
    /// at a time, while other projects open, and open ones are handed out,
    /// meanwhile (see [`Library::project`]).
    opening: Mutex<HashMap<ProjectId, Arc<Mutex<()>>>>,
    /// Whether people are followed by their bodies too (a setting every
    /// open project reads).
    find_bodies: Arc<AtomicBool>,
}

impl Library {
    pub fn open(paths: &AppPaths) -> ProjectResult<Self> {
        std::fs::create_dir_all(&paths.data)?;
        let db = LibraryDb::open(&paths.library_db())?;
        let find_bodies = Arc::new(AtomicBool::new(db.setting(KEY_FIND_BODIES)?.as_deref() == Some("1")));
        Ok(Self { db, open: Mutex::default(), opening: Mutex::default(), find_bodies })
    }

    /// Whether characters are looked for by their bodies too.
    pub fn finds_bodies(&self) -> bool {
        self.find_bodies.load(Ordering::Relaxed)
    }

    // -- settings -----------------------------------------------------------

    pub fn settings(&self) -> ProjectResult<AppSettings> {
        Ok(AppSettings {
            library_root: self.library_root()?.display().to_string(),
            export_preset_id: self.db.setting(KEY_EXPORT_PRESET)?.unwrap_or_else(|| DEFAULT_EXPORT_PRESET.into()),
            audio_language: self.audio_language()?,
            find_bodies: self.finds_bodies(),
            fast_scan: self.fast_scan()?,
        })
    }

    /// Whether new projects find their faces with Fast Scan.
    pub fn fast_scan(&self) -> ProjectResult<bool> {
        Ok(self.db.setting(KEY_FAST_SCAN)?.as_deref() == Some("1"))
    }

    pub fn update_settings(&self, s: &AppSettings) -> ProjectResult<AppSettings> {
        let root = PathBuf::from(s.library_root.trim());
        if !root.is_absolute() {
            return Err(ProjectError::Invalid("Choose a full folder path for the library.".into()));
        }
        std::fs::create_dir_all(&root)
            .map_err(|e| ProjectError::Invalid(format!("Can't use {} as the library: {e}", root.display())))?;
        self.db.set_setting(KEY_LIBRARY_ROOT, &root.display().to_string())?;
        if as_export::builtin_presets().iter().any(|p| p.id == s.export_preset_id) {
            self.db.set_setting(KEY_EXPORT_PRESET, &s.export_preset_id)?;
        }
        let language = as_media::lang::canonical(&s.audio_language).unwrap_or_default();
        self.db.set_setting(KEY_AUDIO_LANGUAGE, &language)?;
        self.db.set_setting(KEY_FIND_BODIES, if s.find_bodies { "1" } else { "0" })?;
        self.find_bodies.store(s.find_bodies, Ordering::Relaxed);
        self.db.set_setting(KEY_FAST_SCAN, if s.fast_scan { "1" } else { "0" })?;
        self.settings()
    }

    /// Audio language chosen automatically for new episodes (`""` = file default).
    pub fn audio_language(&self) -> ProjectResult<String> {
        Ok(self.db.setting(KEY_AUDIO_LANGUAGE)?.unwrap_or_else(|| DEFAULT_AUDIO_LANGUAGE.into()))
    }

    /// Where new project folders go.
    pub fn library_root(&self) -> ProjectResult<PathBuf> {
        Ok(self.db.setting(KEY_LIBRARY_ROOT)?.map(PathBuf::from).unwrap_or_else(AppPaths::default_library_root))
    }

    /// Default destination for a project's exports: `<library>/Exports/<Title>`.
    pub fn default_export_dir(&self, project: &ProjectSummary) -> ProjectResult<PathBuf> {
        Ok(self.library_root()?.join("Exports").join(safe_component(&project.title, " ", 80)))
    }

    // -- projects -----------------------------------------------------------

    pub fn projects(&self) -> ProjectResult<Vec<ProjectSummary>> {
        Ok(self.db.projects()?)
    }

    pub fn summary(&self, id: ProjectId) -> ProjectResult<ProjectSummary> {
        self.db.project(id)?.ok_or_else(|| ProjectError::NotFound("This project no longer exists.".into()))
    }

    /// Create the project folder and database and add `req.files` (status
    /// `queued`). Returns the new project and the media to ingest.
    pub fn create_project(&self, req: &NewProject) -> ProjectResult<(ProjectSummary, Arc<Project>, Vec<MediaId>)> {
        let title = req.title.trim();
        if title.is_empty() {
            return Err(ProjectError::Invalid("Give the project a name.".into()));
        }
        // Before anything is made: a file that can't be imported stops it.
        let found = importable(&req.files, req.kind)?;
        let root = self.library_root()?;
        std::fs::create_dir_all(&root)?;
        let dir = unique_dir(&root, &safe_component(title, " ", 80));
        std::fs::create_dir_all(&dir)?;

        let now = now_ms();
        let summary = ProjectSummary {
            id: ProjectId::new(),
            title: title.to_string(),
            kind: req.kind,
            content: req.content,
            dir: dir.display().to_string(),
            created_at: now,
            updated_at: now,
            media_count: 0,
            ready_count: 0,
            duration_sec: 0.0,
            poster: None,
            poster_small: None,
            // Anime finds no faces at all.
            fast_scan: self.fast_scan()? && req.content != ContentKind::Anime,
        };
        let project = Arc::new(Project::create(&summary, self.find_bodies.clone())?);
        self.db.insert_project(&summary)?;
        self.open.lock().insert(summary.id, project.clone());
        let added = self.add_found(&project, found)?;
        Ok((self.refresh_stats(&project)?, project, added))
    }

    /// Open (or return the cached) project.
    ///
    /// A project is opened once at a time: `Project::open` closes out stale
    /// exports and deletes unused preview files, which a second, racing open
    /// would otherwise do to work the first instance had already started.
    /// Only that project waits for it, though: opening one can take a while
    /// (a database upgrade, its faces placed again), and every other project,
    /// open or not, is handed out meanwhile.
    pub fn project(&self, id: ProjectId) -> ProjectResult<Arc<Project>> {
        if let Some(p) = self.open.lock().get(&id) {
            return Ok(p.clone());
        }
        let one = self.opening.lock().entry(id).or_default().clone();
        let _one_at_a_time = one.lock();
        // Opened while this waited.
        if let Some(p) = self.open.lock().get(&id) {
            return Ok(p.clone());
        }
        let summary = self.summary(id)?;
        let dir = PathBuf::from(&summary.dir);
        if !dir.join(PROJECT_DB).is_file() {
            return Err(ProjectError::NotFound(format!(
                "The project folder {} is missing. Was it moved or deleted?",
                dir.display()
            )));
        }
        let project = Arc::new(Project::open(id, dir, self.find_bodies.clone())?);
        let mut open = self.open.lock();
        // Removed from the library while it opened: not kept open.
        if self.db.project(id)?.is_none() {
            return Err(ProjectError::NotFound("This project no longer exists.".into()));
        }
        open.insert(id, project.clone());
        Ok(project)
    }

    pub fn rename_project(&self, id: ProjectId, title: &str) -> ProjectResult<ProjectSummary> {
        let title = title.trim();
        if title.is_empty() {
            return Err(ProjectError::Invalid("Give the project a name.".into()));
        }
        self.db.rename_project(id, title)?;
        if let Ok(p) = self.project(id) {
            p.set_title(title)?;
        }
        self.summary(id)
    }

    /// Say what a project's footage is (see [`Project::set_content`]).
    /// Returns the summary and whether the cast must be grouped again (also
    /// when an earlier attempt already changed it).
    pub fn set_project_content(&self, id: ProjectId, content: ContentKind) -> ProjectResult<(ProjectSummary, bool)> {
        let project = self.project(id)?;
        let before = self.summary(id)?.content;
        self.db.set_project_content(id, content)?;
        if let Err(e) = project.set_content(content) {
            // The library shows what the project's cast is grouped for.
            let _ = self.db.set_project_content(id, before);
            return Err(e);
        }
        Ok((self.summary(id)?, project.grouped_by_older_version()?))
    }

    /// Say whether a project's faces are found with Fast Scan (see
    /// [`Project::set_fast_scan`]). Returns the summary and whether its
    /// faces are to be found again.
    pub fn set_project_fast_scan(&self, id: ProjectId, on: bool) -> ProjectResult<(ProjectSummary, bool)> {
        let project = self.project(id)?;
        if on && project.content()? == ContentKind::Anime {
            return Err(ProjectError::Invalid("Anime has no faces to scan.".into()));
        }
        // The library first: it is what shows, and the project's own word
        // is the one its faces follow.
        let before = self.summary(id)?.fast_scan;
        self.db.set_project_fast_scan(id, on)?;
        let changed = project.set_fast_scan(on).inspect_err(|_| {
            let _ = self.db.set_project_fast_scan(id, before);
        })?;
        Ok((self.summary(id)?, changed))
    }

    /// Remove a project from the library. With `delete_files`, also delete its
    /// `.autoscene` folder (database, posters, proxies). Source media and
    /// exported clips are never touched. Stop the project's jobs first.
    pub fn delete_project(&self, id: ProjectId, delete_files: bool) -> ProjectResult<()> {
        let summary = self.summary(id)?;
        // The row goes while the cache is locked: a lookup can't find the
        // project in between and open its database again (which would keep
        // it open for the rest of the session).
        let project = {
            let mut open = self.open.lock();
            self.db.delete_project(id)?;
            open.remove(&id)
        };
        if delete_files {
            // Whatever still holds the project (a command finishing, work
            // letting go) closes its database before the folder goes.
            if let Some(project) = project {
                let held = Arc::downgrade(&project);
                drop(project);
                let until = std::time::Instant::now() + RELEASE_WAIT;
                while held.strong_count() > 0 && std::time::Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
            let dir = PathBuf::from(&summary.dir);
            // Only ever delete what is recognisably this project's folder.
            if is_project_dir(&dir, id) {
                remove_dir_with_retry(&dir).map_err(|e| {
                    ProjectError::Io(std::io::Error::new(
                        e.kind(),
                        format!("The project was removed, but its folder couldn't be deleted: {e}"),
                    ))
                })?;
            }
        }
        Ok(())
    }

    /// Recompute the denormalised stats shown on the Library grid.
    pub fn refresh_stats(&self, project: &Project) -> ProjectResult<ProjectSummary> {
        self.db.update_stats(project.id, &project.db().stats()?)?;
        self.summary(project.id)
    }

    /// Show a project's current posters on the Library grid, without moving it
    /// to the front. Returns whether they changed.
    pub fn refresh_posters(&self, project: &Project) -> ProjectResult<bool> {
        let s = project.db().stats()?;
        Ok(self.db.set_posters(project.id, s.poster.as_deref(), s.poster_small.as_deref())?)
    }

    // -- import -------------------------------------------------------------

    /// Expand files and folders into importable videos, in episode order.
    /// `existing` marks candidates already in a project as duplicates.
    pub fn scan(
        &self,
        paths: &[String],
        kind: ProjectKind,
        existing: Option<&Project>,
    ) -> ProjectResult<Vec<ImportCandidate>> {
        let known = match existing {
            Some(p) => p.db().media_paths()?,
            None => HashSet::new(),
        };
        Ok(scan_paths(paths, kind, &known))
    }

    /// Add files to a project (skipping ones already there). Returns the new
    /// media ids. Nothing is added when one of them can't be imported.
    pub fn add_media(&self, project: &Project, files: &[String], kind: ProjectKind) -> ProjectResult<Vec<MediaId>> {
        self.add_found(project, importable(files, kind)?)
    }

    fn add_found(&self, project: &Project, candidates: Vec<ImportCandidate>) -> ProjectResult<Vec<MediaId>> {
        let new: Vec<NewMedia> = candidates
            .into_iter()
            .map(|c| {
                let mtime = std::fs::metadata(&c.path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64);
                NewMedia {
                    id: MediaId::new(),
                    path: c.path,
                    file_name: c.file_name,
                    size_bytes: c.size_bytes as u64,
                    mtime,
                    season: c.season,
                    episode: c.episode,
                }
            })
            .collect();
        let added = project.db().add_media(&new)?;
        self.refresh_stats(project)?;
        Ok(added)
    }

    /// Projects currently open (for shutdown and recovery).
    pub fn open_projects(&self) -> Vec<Arc<Project>> {
        self.open.lock().values().cloned().collect()
    }
}

fn unique_dir(root: &Path, name: &str) -> PathBuf {
    let mut dir = root.join(format!("{name}.autoscene"));
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{name} {n}.autoscene"));
        n += 1;
    }
    dir
}

/// Whether `dir` is recognisably project `id`'s folder, to be deleted with
/// everything in it: an `.autoscene` folder whose database says it is that
/// project. The library's record alone isn't trusted with a whole folder.
/// Where the folder is doesn't matter: the library folder can be changed,
/// and projects made before stay where they are.
fn is_project_dir(dir: &Path, id: ProjectId) -> bool {
    let db = dir.join(PROJECT_DB);
    if !db.is_file() {
        return false;
    }
    let named = dir.extension().is_some_and(|e| e.eq_ignore_ascii_case("autoscene"));
    let says = ProjectDb::stored_id(&db);
    let theirs = named && matches!(&says, Ok(Some(s)) if *s == id.to_string());
    if !theirs {
        tracing::warn!(dir = %as_core::paths::name(dir), ?says, "not deleting a folder that isn't recognisably the project's");
    }
    theirs
}

/// How long deleting a project's folder waits for others still holding the
/// project to let go of it (its jobs have stopped by then).
const RELEASE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Windows keeps files locked for a moment after handles close (antivirus,
/// the indexer, a just-closed SQLite WAL); retry briefly before giving up.
fn remove_dir_with_retry(dir: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..5 {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(100 * (attempt + 1)));
    }
    Err(last.expect("loop ran"))
}

/// Why a file whose path isn't valid Unicode can't be imported: media keep
/// their path as text, and such a path, turned into text, names no file.
const NOT_UNICODE: &str = "Its name, or the name of a folder it's in, isn't valid Unicode. Rename it to import it.";

/// The videos `files` (files and folders) hold, in episode order — none
/// when one of them can't be imported, with an error that names it.
fn importable(files: &[String], kind: ProjectKind) -> ProjectResult<Vec<ImportCandidate>> {
    let found = scan_paths(files, kind, &HashSet::new());
    let refused: Vec<&str> = found.iter().filter(|c| c.refused.is_some()).map(|c| c.file_name.as_str()).collect();
    let why = match refused[..] {
        [] => return Ok(found),
        [one] => format!(
            "{one} can't be imported: its name, or the name of a folder it's in, isn't valid Unicode. \
             Rename it and add it again."
        ),
        [first, ref rest @ ..] => format!(
            "{first} and {} more can't be imported: their names, or the names of folders they're in, aren't \
             valid Unicode. Rename them and add them again.",
            rest.len()
        ),
    };
    Err(ProjectError::Invalid(why))
}

pub(crate) fn scan_paths(paths: &[String], kind: ProjectKind, known: &HashSet<String>) -> Vec<ImportCandidate> {
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        let p = PathBuf::from(p);
        if p.is_dir() {
            walk(&p, SCAN_DEPTH, &mut files);
        } else if p.is_file() && is_video(&p) {
            files.push(p);
        }
    }
    let mut seen = HashSet::new();
    let mut out: Vec<ImportCandidate> = files
        .into_iter()
        .filter(|f| seen.insert(f.clone()))
        .map(|f| {
            let file_name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let parent = f.parent().and_then(|d| d.file_name()).and_then(|d| d.to_str());
            let (season, episode) =
                if kind == ProjectKind::Series { parse_episode(&file_name, parent) } else { (None, None) };
            let path = f.display().to_string();
            ImportCandidate {
                duplicate: known.contains(&path),
                size_bytes: std::fs::metadata(&f).map(|m| m.len() as f64).unwrap_or(0.0),
                label: as_db::episode_label(season, episode),
                refused: f.to_str().is_none().then(|| NOT_UNICODE.into()),
                path,
                file_name,
                season,
                episode,
            }
        })
        .collect();
    out.sort_by(|a, b| episode_cmp((a.season, a.episode, &a.file_name), (b.season, b.episode, &b.file_name)));
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        // Hidden files, macOS resource forks, scene-release sample folders, our own project folders.
        if name.starts_with('.') || name.eq_ignore_ascii_case("sample") || name.eq_ignore_ascii_case("samples") {
            continue;
        }
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if depth > 0 && !name.ends_with(".autoscene") {
                walk(&path, depth - 1, out);
            }
        } else if is_video(&path) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&AppPaths::at(dir.path().join("data"))).unwrap();
        lib.update_settings(&AppSettings {
            library_root: dir.path().join("Library").display().to_string(),
            export_preset_id: "h264-edit".into(),
            audio_language: "German".into(),
            find_bodies: false,
            fast_scan: false,
        })
        .unwrap();
        assert_eq!(lib.audio_language().unwrap(), "deu", "stored as an ISO 639-2 code");
        (dir, lib)
    }

    fn touch(p: &Path) -> String {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
        p.display().to_string()
    }

    #[test]
    fn scans_folders_in_episode_order() {
        let (dir, lib) = lib();
        let show = dir.path().join("Show");
        touch(&show.join("Season 1/Show - 10.mkv"));
        touch(&show.join("Season 1/Show - 2.mkv"));
        touch(&show.join("Season 1/Show - 1.mkv"));
        touch(&show.join("Season 1/notes.txt"));
        touch(&show.join("Season 1/Sample/Show - 1 sample.mkv"));
        touch(&show.join(".hidden.mkv"));
        touch(&show.join("Season 2/Show S02E01.mkv"));
        let found = lib.scan(&[show.display().to_string()], ProjectKind::Series, None).unwrap();
        let labels: Vec<_> = found.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["S01E01", "S01E02", "S01E10", "S02E01"]);

        // Movies ignore numbers in names.
        let movie = touch(&dir.path().join("Blade Runner 2049.mkv"));
        let found = lib.scan(&[movie], ProjectKind::Movie, None).unwrap();
        assert_eq!((found[0].season, found[0].episode), (None, None));
    }

    /// `before`, a character no Unicode text holds (an unpaired surrogate on
    /// Windows, a stray byte elsewhere), then `after`.
    fn not_unicode(dir: &Path, before: &str, after: &str) -> PathBuf {
        #[cfg(windows)]
        let name = {
            use std::os::windows::ffi::OsStringExt;
            let wide: Vec<u16> = before.encode_utf16().chain([0xD800]).chain(after.encode_utf16()).collect();
            std::ffi::OsString::from_wide(&wide)
        };
        #[cfg(unix)]
        let name = {
            use std::os::unix::ffi::OsStringExt;
            std::ffi::OsString::from_vec([before.as_bytes(), b"\xff", after.as_bytes()].concat())
        };
        dir.join(name)
    }

    #[test]
    fn a_file_whose_name_isnt_unicode_is_refused_with_a_reason() {
        let (dir, lib) = lib();
        let show = dir.path().join("Show");
        let good = touch(&show.join("Show - 01.mkv"));
        let bad = not_unicode(&show, "Show", " - 02.mkv");
        if std::fs::write(&bad, b"x").is_err() {
            return; // a file system that never has such names (APFS)
        }
        // Listed with why, not as a file that is missing later.
        let found = lib.scan(&[show.display().to_string()], ProjectKind::Series, None).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].refused, None);
        assert!(found[1].refused.as_deref().is_some_and(|r| r.contains("Unicode")), "{:?}", found[1]);
        assert_eq!((found[1].episode, found[1].label.as_str()), (Some(2), "E02"), "named for the list");

        // Nothing is made or added while one is among them.
        let req = NewProject {
            title: "Show".into(),
            kind: ProjectKind::Series,
            content: ContentKind::Live,
            files: vec![show.display().to_string()],
        };
        let Err(ProjectError::Invalid(why)) = lib.create_project(&req) else { panic!("created") };
        assert!(why.starts_with("Show\u{FFFD} - 02.mkv can't") && why.contains("Rename it"), "{why}");
        assert!(lib.projects().unwrap().is_empty());
        let (_, project, added) = lib.create_project(&NewProject { files: vec![good], ..req.clone() }).unwrap();
        assert_eq!(added.len(), 1);
        std::fs::write(not_unicode(&show, "Show", " - 03.mkv"), b"x").unwrap();
        let more = lib.add_media(&project, &[show.display().to_string()], ProjectKind::Series);
        assert!(matches!(&more, Err(ProjectError::Invalid(why)) if why.contains("and 1 more")), "{more:?}");
        assert_eq!(project.media_list_uncounted().unwrap().len(), 1);
    }

    #[test]
    fn creates_renames_and_deletes_projects() {
        let (dir, lib) = lib();
        let a = touch(&dir.path().join("src/Show - 01.mkv"));
        let b = touch(&dir.path().join("src/Show - 02.mkv"));
        let req = NewProject {
            title: "Show: The Series".into(),
            kind: ProjectKind::Series,
            content: ContentKind::Anime,
            files: vec![b.clone(), a.clone(), a.clone()],
        };
        let (summary, project, added) = lib.create_project(&req).unwrap();
        assert_eq!(added.len(), 2);
        assert_eq!(summary.media_count, 2);
        assert!(summary.dir.ends_with("Show The Series.autoscene"), "{}", summary.dir);
        // A second project with the same title gets its own folder.
        let (second, ..) = lib.create_project(&NewProject { files: vec![], ..req.clone() }).unwrap();
        assert!(second.dir.ends_with("Show The Series 2.autoscene"), "{}", second.dir);

        // Re-adding the same file is a no-op; duplicates are flagged in scans.
        assert!(lib.add_media(&project, std::slice::from_ref(&a), ProjectKind::Series).unwrap().is_empty());
        let scan = lib.scan(std::slice::from_ref(&a), ProjectKind::Series, Some(&project)).unwrap();
        assert!(scan[0].duplicate);

        lib.rename_project(summary.id, "Renamed").unwrap();
        assert_eq!(lib.summary(summary.id).unwrap().title, "Renamed");
        assert_eq!(project.title().unwrap(), "Renamed");

        drop(project);
        lib.delete_project(summary.id, true).unwrap();
        assert!(!Path::new(&summary.dir).exists());
        assert!(Path::new(&a).exists(), "source media are never deleted");
        assert_eq!(lib.projects().unwrap().len(), 1);
        assert!(matches!(lib.project(summary.id), Err(ProjectError::NotFound(_))));
    }

    #[test]
    fn only_the_projects_own_folder_is_deleted_with_it() {
        let (dir, lib) = lib();
        let new = |title: &str| NewProject {
            title: title.into(),
            kind: ProjectKind::Movie,
            content: ContentKind::Live,
            files: vec![],
        };
        let (a, ..) = lib.create_project(&new("A")).unwrap();
        let (b, ..) = lib.create_project(&new("B")).unwrap();
        let (c, ..) = lib.create_project(&new("C")).unwrap();
        // What the library says a project's folder is (as a damaged or
        // edited library.db could).
        let point = |id: ProjectId, to: &Path| {
            rusqlite::Connection::open(AppPaths::at(dir.path().join("data")).library_db())
                .unwrap()
                .execute("UPDATE projects SET dir = ?2 WHERE id = ?1", [id.to_string(), to.display().to_string()])
                .unwrap();
        };
        // A folder with a project database that says it is `id`'s.
        let says = |to: &Path, id: ProjectId| {
            std::fs::create_dir_all(to).unwrap();
            ProjectDb::open(&to.join(PROJECT_DB)).unwrap().set_meta("id", &id.to_string()).unwrap();
            std::fs::write(to.join("keep.txt"), b"mine").unwrap();
        };

        // Another project's folder (a copy of B's): kept.
        let b_copy = dir.path().join("B copy.autoscene");
        says(&b_copy, b.id);
        point(a.id, &b_copy);
        lib.delete_project(a.id, true).unwrap();
        assert!(b_copy.join("keep.txt").is_file());
        assert!(matches!(lib.summary(a.id), Err(ProjectError::NotFound(_))), "the project itself is removed");

        // A folder that isn't a project folder, even with the project's database in it: kept.
        let stuff = dir.path().join("Stuff");
        says(&stuff, b.id);
        point(b.id, &stuff);
        lib.delete_project(b.id, true).unwrap();
        assert!(stuff.join("keep.txt").is_file());

        // Its own folder goes, also when the library has moved since.
        let moved =
            AppSettings { library_root: dir.path().join("Elsewhere").display().to_string(), ..lib.settings().unwrap() };
        lib.update_settings(&moved).unwrap();
        lib.delete_project(c.id, true).unwrap();
        assert!(!Path::new(&c.dir).exists());
    }

    #[test]
    fn a_project_opens_once_while_others_are_handed_out() {
        let (dir, lib) = lib();
        let new = |title: &str| NewProject {
            title: title.into(),
            kind: ProjectKind::Movie,
            content: ContentKind::Live,
            files: vec![],
        };
        let (a, ..) = lib.create_project(&new("A")).unwrap();
        let (b, ..) = lib.create_project(&new("B")).unwrap();
        let (c, ..) = lib.create_project(&new("C")).unwrap();
        drop(lib);
        // A launch: nothing is open yet.
        let lib = Library::open(&AppPaths::at(dir.path().join("data"))).unwrap();
        let open_a = lib.project(a.id).unwrap();
        let wait = std::time::Duration::from_secs(20);

        std::thread::scope(|s| {
            // B is being opened (say, its database upgraded) for as long as this is held.
            let b_opening = lib.opening.lock().entry(b.id).or_default().clone();
            let held = b_opening.lock();
            let (tx, rx) = std::sync::mpsc::channel();
            let lib = &lib;
            let tx_b = tx.clone();
            s.spawn(move || tx_b.send(("b", lib.project(b.id).unwrap())).unwrap());
            // Meanwhile an open project is handed out, and another one opens.
            let tx_a = tx.clone();
            s.spawn(move || tx_a.send(("a", lib.project(a.id).unwrap())).unwrap());
            s.spawn(move || tx.send(("c", lib.project(c.id).unwrap())).unwrap());
            let mut done: Vec<_> = (0..2).map(|_| rx.recv_timeout(wait).expect("not held up by B")).collect();
            done.sort_by_key(|(name, _)| *name);
            assert_eq!(done.iter().map(|(name, _)| *name).collect::<Vec<_>>(), ["a", "c"]);
            assert!(Arc::ptr_eq(&done[0].1, &open_a));
            // B waits for its own opening only.
            assert!(rx.recv_timeout(std::time::Duration::from_millis(200)).is_err());
            drop(held);
            let (name, open_b) = rx.recv_timeout(wait).unwrap();
            assert_eq!(name, "b");
            assert!(Arc::ptr_eq(&open_b, &lib.project(b.id).unwrap()));
        });

        // Asked for by many at once: opened once, the same for all.
        drop(lib);
        let lib = Library::open(&AppPaths::at(dir.path().join("data"))).unwrap();
        let all: Vec<Arc<Project>> = std::thread::scope(|s| {
            let asks: Vec<_> = (0..8).map(|_| s.spawn(|| lib.project(c.id).unwrap())).collect();
            asks.into_iter().map(|t| t.join().unwrap()).collect()
        });
        assert!(all.iter().all(|p| Arc::ptr_eq(p, &all[0])));
    }

    #[test]
    fn a_deleted_project_is_let_go_of_before_its_folder_goes() {
        let (_dir, lib) = lib();
        let req =
            NewProject { title: "Held".into(), kind: ProjectKind::Movie, content: ContentKind::Live, files: vec![] };
        let (summary, project, _) = lib.create_project(&req).unwrap();
        // Work still finishing with it (its database open) when it is deleted.
        let released = Arc::new(AtomicBool::new(false));
        let holder = {
            let released = released.clone();
            std::thread::spawn(move || {
                // Longer than removing the folder keeps trying.
                std::thread::sleep(std::time::Duration::from_millis(1800));
                released.store(true, Ordering::SeqCst);
                drop(project);
            })
        };
        lib.delete_project(summary.id, true).unwrap();
        assert!(released.load(Ordering::SeqCst), "the folder went while the project was still held");
        assert!(!Path::new(&summary.dir).exists());
        assert!(lib.open_projects().is_empty());
        assert!(matches!(lib.project(summary.id), Err(ProjectError::NotFound(_))));
        holder.join().unwrap();
    }

    #[test]
    fn says_what_the_footage_is_after_import() {
        let (_dir, lib) = lib();
        let new = |content| NewProject { title: "Film".into(), kind: ProjectKind::Movie, content, files: vec![] };
        let (film, ..) = lib.create_project(&new(ContentKind::Live)).unwrap();
        let project = lib.project(film.id).unwrap();
        project.db().set_meta(crate::cast::GROUPED_WITH, &as_identity::VERSION.to_string()).unwrap();

        let (summary, regroup) = lib.set_project_content(film.id, ContentKind::Animation).unwrap();
        assert!(!regroup, "no faces yet: nothing to group");
        assert_eq!(summary.content, ContentKind::Animation);
        assert_eq!(project.content().unwrap(), ContentKind::Animation);
        // The cast is grouped again for it (by the next upkeep, also after a restart).
        assert_eq!(project.db().meta(crate::cast::GROUPED_WITH).unwrap().as_deref(), Some("0"));
        // The same again changes nothing.
        assert!(!lib.set_project_content(film.id, ContentKind::Animation).unwrap().1);

        // Never to or from anime: it is analysed differently from the start.
        assert!(matches!(lib.set_project_content(film.id, ContentKind::Anime), Err(ProjectError::Invalid(_))));
        assert_eq!(lib.summary(film.id).unwrap().content, ContentKind::Animation);
        let (anime, ..) = lib.create_project(&new(ContentKind::Anime)).unwrap();
        assert!(matches!(lib.set_project_content(anime.id, ContentKind::Live), Err(ProjectError::Invalid(_))));
        assert_eq!(lib.summary(anime.id).unwrap().content, ContentKind::Anime);
    }

    #[test]
    fn fast_scan_is_for_projects_made_while_it_is_on() {
        let (dir, lib) = lib();
        let new =
            |title: &str, content| NewProject { title: title.into(), kind: ProjectKind::Movie, content, files: vec![] };
        // Off by default.
        assert!(!lib.settings().unwrap().fast_scan);
        let (before, ..) = lib.create_project(&new("Before", ContentKind::Live)).unwrap();
        assert!(!before.fast_scan && !lib.project(before.id).unwrap().fast_scan());

        let on = AppSettings { fast_scan: true, ..lib.settings().unwrap() };
        assert!(lib.update_settings(&on).unwrap().fast_scan);
        let (after, ..) = lib.create_project(&new("After", ContentKind::Animation)).unwrap();
        assert!(after.fast_scan && lib.project(after.id).unwrap().fast_scan());
        // Projects made before keep how they were made; anime finds no faces.
        assert!(!lib.summary(before.id).unwrap().fast_scan);
        let (anime, ..) = lib.create_project(&new("Anime", ContentKind::Anime)).unwrap();
        assert!(!anime.fast_scan);
        assert!(matches!(lib.set_project_fast_scan(anime.id, true), Err(ProjectError::Invalid(_))));

        // Each project can be switched on its own ("Edit details…").
        let (summary, changed) = lib.set_project_fast_scan(before.id, true).unwrap();
        assert!(summary.fast_scan && changed && lib.project(before.id).unwrap().fast_scan());
        assert!(!lib.set_project_fast_scan(before.id, true).unwrap().1);
        let (summary, changed) = lib.set_project_fast_scan(after.id, false).unwrap();
        assert!(!summary.fast_scan && changed);
        // The setting itself is kept.
        drop(lib);
        let reopened = Library::open(&AppPaths::at(dir.path().join("data"))).unwrap();
        assert!(reopened.settings().unwrap().fast_scan);
        assert!(reopened.summary(before.id).unwrap().fast_scan && !reopened.summary(after.id).unwrap().fast_scan);
    }

    #[test]
    fn rejects_relative_library_roots() {
        let (_dir, lib) = lib();
        let bad = AppSettings {
            library_root: "relative/path".into(),
            export_preset_id: "h264-edit".into(),
            audio_language: "eng".into(),
            find_bodies: false,
            fast_scan: false,
        };
        assert!(matches!(lib.update_settings(&bad), Err(ProjectError::Invalid(_))));
        assert_eq!(lib.settings().unwrap().export_preset_id, "h264-edit");
    }

    #[test]
    fn project_db_open_is_shared() {
        let (_dir, lib) = lib();
        let (s, ..) = lib
            .create_project(&NewProject {
                title: "X".into(),
                kind: ProjectKind::Movie,
                content: ContentKind::Live,
                files: vec![],
            })
            .unwrap();
        let a = lib.project(s.id).unwrap();
        let b = lib.project(s.id).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(ProjectDb::open(&PathBuf::from(&s.dir).join(PROJECT_DB)).is_ok());
    }
}
