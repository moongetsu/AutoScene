use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use as_api::{AppSettings, ImportCandidate, NewProject, ProjectKind, ProjectSummary, now_ms};
use as_core::{AppPaths, MediaId, ProjectId};
use as_db::{LibraryDb, NewMedia};
use parking_lot::Mutex;

use crate::naming::{episode_cmp, is_video, parse_episode, safe_component};
use crate::project::{PROJECT_DB, Project};
use crate::{ProjectError, ProjectResult};

const KEY_LIBRARY_ROOT: &str = "library_root";
const KEY_EXPORT_PRESET: &str = "export_preset_id";
const DEFAULT_EXPORT_PRESET: &str = "prores-hq";
const KEY_AUDIO_LANGUAGE: &str = "audio_language";
const KEY_FIND_BODIES: &str = "find_bodies";
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
    /// Whether people are followed by their bodies too (a setting every
    /// open project reads).
    find_bodies: Arc<AtomicBool>,
}

impl Library {
    pub fn open(paths: &AppPaths) -> ProjectResult<Self> {
        std::fs::create_dir_all(&paths.data)?;
        let db = LibraryDb::open(&paths.library_db())?;
        let find_bodies = Arc::new(AtomicBool::new(db.setting(KEY_FIND_BODIES)?.as_deref() == Some("1")));
        Ok(Self { db, open: Mutex::new(HashMap::new()), find_bodies })
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
        })
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
        };
        let project = Arc::new(Project::create(&summary, self.find_bodies.clone())?);
        self.db.insert_project(&summary)?;
        self.open.lock().insert(summary.id, project.clone());
        let added = self.add_media(&project, &req.files, req.kind)?;
        Ok((self.refresh_stats(&project)?, project, added))
    }

    /// Open (or return the cached) project.
    ///
    /// The cache stays locked while opening: `Project::open` closes out stale
    /// exports and deletes unused preview files, which a second, racing open
    /// would otherwise do to work the first instance had already started.
    pub fn project(&self, id: ProjectId) -> ProjectResult<Arc<Project>> {
        let mut open = self.open.lock();
        if let Some(p) = open.get(&id) {
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

    /// Remove a project from the library. With `delete_files`, also delete its
    /// `.autoscene` folder (database, posters, proxies). Source media and
    /// exported clips are never touched. Cancel the project's jobs first.
    pub fn delete_project(&self, id: ProjectId, delete_files: bool) -> ProjectResult<()> {
        let summary = self.summary(id)?;
        // Drop our handle so the database file is closed before deleting the folder.
        self.open.lock().remove(&id);
        self.db.delete_project(id)?;
        if delete_files {
            let dir = PathBuf::from(&summary.dir);
            // Only ever delete what is recognisably a project folder — a
            // direct child of the library; a forged `dir` must not reach
            // outside it.
            let ours = self.library_root().is_ok_and(|root| dir.parent() == Some(root.as_path()));
            if ours && dir.join(PROJECT_DB).is_file() {
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

    /// Add files to a project (skipping ones already there). Returns the new media ids.
    pub fn add_media(&self, project: &Project, files: &[String], kind: ProjectKind) -> ProjectResult<Vec<MediaId>> {
        let candidates = scan_paths(files, kind, &HashSet::new());
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
            let file_name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            let parent = f.parent().and_then(|d| d.file_name()).and_then(|d| d.to_str());
            let (season, episode) =
                if kind == ProjectKind::Series { parse_episode(&file_name, parent) } else { (None, None) };
            let path = f.display().to_string();
            ImportCandidate {
                duplicate: known.contains(&path),
                size_bytes: std::fs::metadata(&f).map(|m| m.len() as f64).unwrap_or(0.0),
                label: as_db::episode_label(season, episode),
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
    use as_api::ContentKind;
    use as_db::ProjectDb;

    fn lib() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&AppPaths::at(dir.path().join("data"))).unwrap();
        lib.update_settings(&AppSettings {
            library_root: dir.path().join("Library").display().to_string(),
            export_preset_id: "h264-edit".into(),
            audio_language: "German".into(),
            find_bodies: false,
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
    fn rejects_relative_library_roots() {
        let (_dir, lib) = lib();
        let bad = AppSettings {
            library_root: "relative/path".into(),
            export_preset_id: "h264-edit".into(),
            audio_language: "eng".into(),
            find_bodies: false,
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
