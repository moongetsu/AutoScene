//! The installed plugins: their files under `plugins/<id>/`, and a row for
//! each in `library.db`'s settings, `plugin.<id>`, which holds its
//! [`Record`]: the version, where its files are, its trust and pinned author
//! key, and whether it is on. The values the person changed in its settings
//! sit beside it in `plugin_settings.<id>`. Rows, not a table: a new library
//! schema would make every older AutoScene refuse the library, also for
//! people who never install a plugin.
//!
//! A package installs in two steps. [`Store::stage`] unpacks it into
//! `plugins/.staging/<uuid>` and checks it there, which gives a [`Ticket`]:
//! what the review dialog shows. [`Store::commit`] checks the files and the
//! signature again, renames the folder to `plugins/<id>/<version>+<8 hex>`
//! and writes the row; the version before goes only once the row names the
//! new one. What is thrown away goes through `plugins/.trash`, so it is gone
//! from its place at once. If AutoScene stops at any point, the plugin is as
//! it was or as it is now, and what lies about is moved out of the way at the
//! next open, and removed after it ([`Store::remove_leftovers`]).
//! A power cut can also undo the last rows written (library.db syncs
//! lazily): the plugin is then listed as missing its files. A developer's
//! folder is loaded where it is ([`Store::stage_folder`]) and is never
//! removed.
//!
//! An update never takes away the trust its plugin had: a plugin AutoScene
//! reviewed takes only reviewed versions, and one pinned to its author's key
//! only versions that key signed or AutoScene reviewed, which keep the pin
//! ([`trust::judge`]). An unreviewed one takes any, and one its author signed
//! pins the key. None goes back to an older version or changes its files
//! under the same one. A row this
//! AutoScene can't read (a later AutoScene's) is listed as such and never
//! written over, and so is a plugin that doesn't work with this AutoScene
//! (one put back to an older version).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use as_core::AppPaths;
use as_db::LibraryDb;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use parking_lot::Mutex;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{PluginError, PluginResult};
use crate::manifest::{
    self, Manifest, Parsed, Permission, PluginId, SettingKind, SettingSpec, SettingValue, current_platform, invalid,
    shown,
};
use crate::package::{self, FilesLock, LOCK, LOCK_VERSION, LockedFile, MANIFEST, MAX_LOCK_BYTES, Unpacked, file_in};
use crate::protocol::{ActionContext, AppContext, PluginContext, RunMessage, ToolPaths};
use crate::runtime::RunSpec;
use crate::trust::{self, AuthorKey, Keys, Trust, Verdict};

/// The [`Record`] version this AutoScene reads and writes. A row of another
/// is a later AutoScene's: listed as needing it, and never written over.
pub const RECORD_VERSION: u32 = 1;

const ROW: &str = "plugin.";
const SETTINGS_ROW: &str = "plugin_settings.";
const STAGING: &str = ".staging";
const TRASH: &str = ".trash";
/// Why a plugin whose folder is gone can't run.
pub const MISSING: &str = "Its files are missing. Install it again.";
const CHANGED: &str = "Its files were changed. Install it again.";
const NEWER: &str = "Needs a newer AutoScene.";
/// How long a rename waits for Windows: a virus scanner may still hold a
/// file it just unpacked.
const RENAME_WAIT: Duration = Duration::from_secs(1);

/// What the registry holds of an installed plugin (`plugin.<id>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// [`RECORD_VERSION`].
    pub v: u32,
    pub id: PluginId,
    pub version: String,
    pub source: Source,
    pub trust: Trust,
    /// The author key its updates must be signed with (see [`trust::judge`]).
    pub author_key: Option<AuthorKey>,
    /// What it said it does when the person installed it.
    pub permissions: Vec<Permission>,
    pub network_hosts: Vec<String>,
    pub enabled: bool,
    /// Its export actions that run after every export.
    pub after_export: Vec<String>,
    /// The SHA-256 of its `files.lock`; none for a loaded folder.
    pub lock_sha256: Option<String>,
    /// When this version was installed (Unix ms).
    pub installed_at: i64,
}

/// Where a plugin's files are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Installed from a package: the folder `<version>+<8 hex>` under
    /// `plugins/<id>/`.
    Package { folder: String },
    /// Loaded from a developer's folder (an absolute path), used where it is.
    Folder { path: String },
}

/// An installed plugin, as its row and its folder say.
#[derive(Clone, Debug)]
pub struct Installed {
    pub record: Record,
    /// Its manifest, unless it is broken.
    pub manifest: Option<Manifest>,
    pub warnings: Vec<String>,
    /// Its folder: the package's under `plugins/<id>/`, or the loaded one.
    pub dir: PathBuf,
    /// Why it can't run, as a sentence: its files are missing or were
    /// changed, a later AutoScene wrote its row, or it doesn't work with
    /// this AutoScene.
    pub broken: Option<String>,
}

/// A package (or a folder) checked and waiting for the person's yes: what
/// the review dialog shows, and what [`Store::commit`] installs.
#[derive(Debug)]
pub struct Ticket {
    pub id: Uuid,
    pub manifest: Manifest,
    pub warnings: Vec<String>,
    /// The trust it gets, and the author key it is pinned to after.
    pub verdict: Verdict,
    /// Its lock and the lock's exact bytes; none for a folder.
    pub lock: Option<FilesLock>,
    pub lock_bytes: Vec<u8>,
    /// Its `files.lock.minisig` and `author.pub`, for the check at commit.
    pub signature: Option<String>,
    pub author_pub: Option<String>,
    pub source: Source,
    /// The folder it was unpacked into, under `plugins/.staging`.
    pub staged: Option<PathBuf>,
    /// The record of the version it replaces.
    pub previous: Option<Record>,
    pub created: Instant,
}

/// The plugins installed in one app data folder.
pub struct Store {
    root: PathBuf,
    data: PathBuf,
    logs: PathBuf,
    db: Arc<LibraryDb>,
    keys: Keys,
    /// The version of this AutoScene, which each plugin must work with.
    app: Version,
    /// Held while a plugin's rows change, so each change starts from the
    /// rows the one before wrote.
    changing: Mutex<()>,
    /// What the open moved into `.trash`, for [`Store::remove_leftovers`].
    leftovers: Mutex<Vec<PathBuf>>,
}

impl Store {
    /// Opens the plugins of `paths` (registered in `db`, judged by `keys`)
    /// for the AutoScene of version `app`, and moves into `.trash` what an
    /// install or uninstall that never finished left: staged folders,
    /// versions a row no longer names, and folders of plugins without a
    /// row. A plugin whose row a later AutoScene wrote keeps its folder as
    /// it is. Moving is quick; removing them, and what `.trash` held, is
    /// [`Store::remove_leftovers`]'s, which a package's 4096 files can keep
    /// busy for a second.
    pub fn open(paths: &AppPaths, db: Arc<LibraryDb>, keys: Keys, app: Version) -> PluginResult<Self> {
        let store = Self {
            root: paths.plugins(),
            data: paths.plugin_data(),
            logs: paths.plugin_logs(),
            db,
            keys,
            app,
            changing: Mutex::default(),
            leftovers: Mutex::default(),
        };
        let (staging, trash) = (store.root.join(STAGING), store.root.join(TRASH));
        for dir in [&staging, &trash] {
            // The store's own folders: a file in their place goes.
            if fs::symlink_metadata(dir).is_ok_and(|f| !f.is_dir()) {
                remove_all(dir)?;
            }
            fs::create_dir_all(dir)?;
        }
        let mut leftovers = fs::read_dir(&trash)?.map(|entry| Ok(entry?.path())).collect::<io::Result<Vec<_>>>()?;
        for entry in fs::read_dir(&staging)? {
            leftovers.extend(store.move_away(&entry?.path()));
        }
        leftovers.extend(store.sweep()?);
        *store.leftovers.lock() = leftovers;
        Ok(store)
    }

    /// Removes what [`Store::open`] moved out of the way. Each lies in
    /// `.trash` under a name of its own, so what is staged, installed or
    /// thrown away meanwhile is never touched: this may run beside them.
    pub fn remove_leftovers(&self) {
        let leftovers = std::mem::take(&mut *self.leftovers.lock());
        for path in leftovers {
            if let Err(e) = remove_all(&path) {
                tracing::warn!(path = %as_core::paths::name(&path), error = %e, "a plugin's old files stay");
            }
        }
    }

    /// Moves into `.trash` each folder under `plugins/<id>/` its row doesn't
    /// name, and `plugins/<id>/` of a plugin without a row (or loaded from a
    /// folder), and gives where they went. Names that are no plugin id are
    /// left as they are.
    fn sweep(&self) -> PluginResult<Vec<PathBuf>> {
        let mut moved = Vec::new();
        let rows: BTreeMap<PluginId, String> = self.rows()?.into_iter().collect();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let Some(id) = entry.file_name().to_str().and_then(|name| PluginId::parse(name).ok()) else { continue };
            let keep = match rows.get(&id).map(|value| read_row(&id, value)) {
                Some(Some(Record { source: Source::Package { folder }, .. })) => folder,
                // A later AutoScene's row: so may its files be.
                Some(None) => continue,
                Some(Some(_)) | None => {
                    moved.extend(self.move_away(&entry.path()));
                    continue;
                }
            };
            // A file or a link in its folder's place goes.
            if !entry.file_type().is_ok_and(|f| f.is_dir()) {
                moved.extend(self.move_away(&entry.path()));
                continue;
            }
            // A folder that can't be read now may hold the version in use.
            let Ok(versions) = fs::read_dir(entry.path()) else { continue };
            for old in versions.flatten() {
                if old.file_name() != keep.as_str() {
                    moved.extend(self.move_away(&old.path()));
                }
            }
        }
        Ok(moved)
    }

    /// Every installed plugin, by id.
    pub fn list(&self) -> PluginResult<Vec<Installed>> {
        Ok(self.rows()?.into_iter().map(|(id, value)| self.installed(id, &value)).collect())
    }

    /// The installed plugin `id`, also when it is broken.
    pub fn get(&self, id: &PluginId) -> PluginResult<Installed> {
        let value = self.db.setting(&row_key(id))?.ok_or_else(|| not_found(id))?;
        Ok(self.installed(id.clone(), &value))
    }

    /// Unpacks the package `archive` and checks it for this AutoScene: its
    /// manifest, the files its command starts on this computer, its icon and
    /// its signature, and that it may replace the installed version. A
    /// package that fails leaves no folder.
    pub fn stage(&self, archive: &Path) -> PluginResult<Ticket> {
        let id = Uuid::now_v7();
        let staged = self.root.join(STAGING).join(id.simple().to_string());
        let unpacked = package::unpack(archive, &staged)?;
        let ticket = self.check_package(id, &staged, unpacked);
        if ticket.is_err() {
            self.throw_away(&staged);
        }
        ticket
    }

    fn check_package(&self, id: Uuid, staged: &Path, unpacked: Unpacked) -> PluginResult<Ticket> {
        let bytes = manifest::read_file(&staged.join(MANIFEST))?;
        let Parsed { manifest: m, warnings } = manifest::parse(&bytes, &self.app)?;
        package::check_launch(&unpacked.lock, &m.launch()?, current_platform())?;
        if let Some(icon) = &m.icon {
            package::check_icon(staged, &unpacked.lock, icon)?;
        }
        if let Some(banner) = &m.banner {
            package::check_banner(staged, &unpacked.lock, banner)?;
        }
        let previous = self.record(&m.id)?;
        let Unpacked { lock, lock_bytes, signature, author_key } = unpacked;
        let pin = previous.as_ref().and_then(|p| p.author_key.as_ref());
        let verdict = trust::judge(&self.keys, &lock_bytes, signature.as_deref(), author_key.as_deref(), &m, pin)?;
        if let Some(p) = &previous {
            check_update(p, &m, verdict.trust, Some(&package::sha256_hex(&lock_bytes)))?;
        }
        // The id's random end: two installs of one version get two folders.
        let folder = format!("{}+{}", m.version, &id.simple().to_string()[24..]);
        Ok(Ticket {
            id,
            manifest: m,
            warnings,
            verdict,
            lock: Some(lock),
            lock_bytes,
            signature,
            author_pub: author_key,
            source: Source::Package { folder },
            staged: Some(staged.to_path_buf()),
            previous,
            created: Instant::now(),
        })
    }

    /// Checks the plugin in the developer's folder `dir` for this AutoScene,
    /// to load it where it is: it has no lock, so it is Unreviewed, and its
    /// files are read as they are each time.
    pub fn stage_folder(&self, dir: &Path) -> PluginResult<Ticket> {
        let dir = std::path::absolute(dir)?;
        if !dir.is_dir() {
            return Err(invalid("That isn't a folder."));
        }
        // An installed version's folder would be thrown away by an update.
        if fs::canonicalize(&dir)?.starts_with(fs::canonicalize(&self.root)?) {
            return Err(invalid("That folder belongs to an installed plugin. Load the folder you work in."));
        }
        let path = dir.to_str().ok_or_else(|| invalid("AutoScene can't load a folder whose path isn't text."))?;
        let bytes = manifest::read_file(&dir.join(MANIFEST)).map_err(|e| match e.kind() {
            ErrorKind::NotFound => invalid("That folder has no plugin.json."),
            _ => e.into(),
        })?;
        let Parsed { manifest: m, warnings } = manifest::parse(&bytes, &self.app)?;
        for file in m.launch()?.files() {
            if !fs::metadata(file_in(&dir, &file)).is_ok_and(|f| f.is_file()) {
                return Err(invalid(format!(
                    "The plugin folder is missing {}, which its command needs.",
                    shown(&file)
                )));
            }
        }
        let previous = self.record(&m.id)?;
        let pin = previous.as_ref().and_then(|p| p.author_key.as_ref());
        let verdict = trust::judge(&self.keys, &[], None, None, &m, pin).map_err(|e| match e {
            // What judge says of an unsigned update: the folder isn't signed at all.
            PluginError::KeyChanged { .. } => {
                invalid("The installed version is signed by its author. Uninstall it to load a folder.")
            }
            e => e,
        })?;
        if let Some(p) = &previous {
            check_update(p, &m, verdict.trust, None)?;
        }
        Ok(Ticket {
            id: Uuid::now_v7(),
            manifest: m,
            warnings,
            verdict,
            lock: None,
            lock_bytes: Vec::new(),
            signature: None,
            author_pub: None,
            source: Source::Folder { path: path.to_owned() },
            staged: None,
            previous,
            created: Instant::now(),
        })
    }

    /// Installs the plugin of `t`. One AutoScene hasn't reviewed needs
    /// `allow_unreviewed` (the setting) and `accept_unreviewed` (the
    /// person's typed yes). Its files and signature are checked again, and
    /// what it replaces is the installed version now, which may have
    /// changed since it was staged. Whatever fails, the plugin stays as it
    /// was, and the staged folder is removed.
    pub fn commit(&self, t: Ticket, allow_unreviewed: bool, accept_unreviewed: bool) -> PluginResult<Installed> {
        let installed = self.install(&t, allow_unreviewed, accept_unreviewed);
        if installed.is_err() {
            self.discard(t);
        }
        installed
    }

    fn install(&self, t: &Ticket, allow_unreviewed: bool, accept_unreviewed: bool) -> PluginResult<Installed> {
        // Before the files are hashed again, when the answer is no already.
        allowed(t.verdict.trust, allow_unreviewed, accept_unreviewed)?;
        let m = &t.manifest;
        let package = match (&t.source, self.staged(t)) {
            (Source::Package { folder }, Some(staged)) => {
                self.check_staged(staged, &t.lock_bytes)?;
                Some((folder, staged))
            }
            (Source::Folder { .. }, None) => None,
            _ => return Err(changed_while_waiting()),
        };
        let _changing = self.changing.lock();
        let previous = self.record(&m.id)?;
        let pin = previous.as_ref().and_then(|p| p.author_key.as_ref());
        let verdict = trust::judge(&self.keys, &t.lock_bytes, t.signature.as_deref(), t.author_pub.as_deref(), m, pin)?;
        allowed(verdict.trust, allow_unreviewed, accept_unreviewed)?;
        let lock_sha256 = package.is_some().then(|| package::sha256_hex(&t.lock_bytes));
        if let Some(p) = &previous {
            check_update(p, m, verdict.trust, lock_sha256.as_deref())?;
        }

        let plugin_dir = self.root.join(m.id.as_str());
        let dir = match package {
            Some((folder, staged)) => {
                fs::create_dir_all(&plugin_dir)?;
                let dir = plugin_dir.join(folder);
                if let Err(e) = rename(staged, &dir) {
                    // Empty unless the version before lies in it.
                    let _ = fs::remove_dir(&plugin_dir);
                    return Err(e.into());
                }
                Some(dir)
            }
            None => None,
        };
        let actions: Vec<&str> = m.export_actions.iter().map(|a| a.id.as_str()).collect();
        let record = Record {
            v: RECORD_VERSION,
            id: m.id.clone(),
            version: m.version.clone(),
            source: t.source.clone(),
            trust: verdict.trust,
            author_key: verdict.author_key,
            permissions: m.permissions.clone(),
            network_hosts: m.network_hosts.clone(),
            enabled: previous.as_ref().is_none_or(|p| p.enabled),
            after_export: previous
                .as_ref()
                .map(|p| p.after_export.iter().filter(|a| actions.contains(&a.as_str())).cloned().collect())
                .unwrap_or_default(),
            lock_sha256,
            installed_at: as_api::now_ms(),
        };
        if let Err(e) = self.write_record(&record) {
            if let Some(dir) = &dir {
                self.throw_away(dir);
                // Empty unless the version before lies in it.
                let _ = fs::remove_dir(&plugin_dir);
            }
            return Err(e);
        }
        // The row names the new version: the one before can go.
        match (previous.map(|p| p.source), &dir) {
            (Some(Source::Package { .. }), None) => self.throw_away(&plugin_dir),
            (Some(Source::Package { folder }), Some(dir)) if plugin_dir.join(&folder) != *dir => {
                self.throw_away(&plugin_dir.join(folder));
            }
            _ => {}
        }
        Ok(self.load(record))
    }

    /// Drops `t`, and the folder it was staged in.
    pub fn discard(&self, t: Ticket) {
        if let Some(staged) = self.staged(&t) {
            self.throw_away(staged);
        }
    }

    /// The folder `t` was staged in, when it lies in `.staging`.
    fn staged<'a>(&self, t: &'a Ticket) -> Option<&'a Path> {
        t.staged.as_deref().filter(|staged| staged.parent() == Some(self.root.join(STAGING).as_path()))
    }

    /// Checks the staged folder `staged` before it is installed: it holds
    /// the lock `lock_bytes` it was staged with, and each file that lock
    /// lists as it was unpacked.
    fn check_staged(&self, staged: &Path, lock_bytes: &[u8]) -> PluginResult<()> {
        let mut on_disk = Vec::new();
        match File::open(staged.join(LOCK)) {
            Ok(file) => file.take(MAX_LOCK_BYTES + 1).read_to_end(&mut on_disk)?,
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(changed_while_waiting()),
            Err(e) => return Err(e.into()),
        };
        if on_disk != lock_bytes {
            return Err(changed_while_waiting());
        }
        package::verify_dir(staged, &package::read_lock(&on_disk)?)
    }

    /// Uninstalls `id` (also one whose row a later AutoScene wrote): its
    /// rows go in one transaction (its pinned author key with them, so
    /// installing it again is a first install), then its installed files. A
    /// loaded folder is the developer's and stays where it is. `delete_data`
    /// also removes what it kept ([`Store::data_dir`]).
    pub fn uninstall(&self, id: &PluginId, delete_data: bool) -> PluginResult<()> {
        let _changing = self.changing.lock();
        if self.db.setting(&row_key(id))?.is_none() {
            return Err(not_found(id));
        }
        self.db.write_settings(&[(&row_key(id), None), (&settings_key(id), None)])?;
        self.throw_away(&self.root.join(id.as_str()));
        if delete_data {
            self.throw_away(&self.data_dir(id));
        }
        Ok(())
    }

    /// Turns `id` on or off.
    pub fn set_enabled(&self, id: &PluginId, on: bool) -> PluginResult<Installed> {
        let _changing = self.changing.lock();
        let mut record = self.record(id)?.ok_or_else(|| not_found(id))?;
        record.enabled = on;
        self.write_record(&record)?;
        Ok(self.load(record))
    }

    /// Turns the export action `action` of `id` on or off for after every
    /// export.
    pub fn set_after_export(&self, id: &PluginId, action: &str, on: bool) -> PluginResult<Installed> {
        let _changing = self.changing.lock();
        let record = self.record(id)?.ok_or_else(|| not_found(id))?;
        let mut p = self.load(record);
        let m = manifest_of(&p)?;
        if !m.export_actions.iter().any(|a| a.id == action) {
            return Err(invalid(format!("The plugin has no export action {}.", shown(action))));
        }
        let after = &p.record.after_export;
        p.record.after_export = m
            .export_actions
            .iter()
            .filter(|a| if a.id == action { on } else { after.contains(&a.id) })
            .map(|a| a.id.clone())
            .collect();
        self.write_record(&p.record)?;
        Ok(p)
    }

    /// The value of each of the settings of `id`: the person's while it
    /// fits the setting, else its default (a setting without either has
    /// none).
    pub fn values(&self, id: &PluginId) -> PluginResult<BTreeMap<String, SettingValue>> {
        let p = self.get(id)?;
        Ok(merged(&manifest_of(&p)?.settings, &self.stored_values(id)?.unwrap_or_default()))
    }

    /// Sets the settings of `id` named in `values`, each checked against
    /// its setting; one that doesn't fit refuses them all, and a key that
    /// names no setting is dropped. Only values that differ from their
    /// default are kept, so a setting left alone follows the plugin's
    /// default. Gives every value, as [`Store::values`] does. Values a later
    /// AutoScene saved are never written over: a row this one can't read
    /// refuses the change, and the value of a setting it skipped (a secret, a
    /// type it doesn't know) stays as it is.
    pub fn set_values(
        &self,
        id: &PluginId,
        values: BTreeMap<String, SettingValue>,
    ) -> PluginResult<BTreeMap<String, SettingValue>> {
        let _changing = self.changing.lock();
        let p = self.get(id)?;
        let specs = &manifest_of(&p)?.settings;
        let spec = |key: &str| specs.iter().find(|s| s.key == key);
        let mut stored = self.stored_row(id)?.ok_or_else(|| {
            invalid("A newer AutoScene saved this plugin's settings, so this one can't change them. Update AutoScene, or uninstall the plugin.")
        })?;
        for (key, value) in values {
            if let Some(spec) = spec(&key) {
                check_value(spec, &value)?;
                stored.insert(key, serde_json::to_value(value).expect("a setting value serializes"));
            }
        }
        stored.retain(|key, value| match spec(key) {
            Some(s) => serde_json::from_value::<SettingValue>(value.clone())
                .is_ok_and(|v| check_value(s, &v).is_ok() && default_of(&s.kind) != Some(v)),
            None => true,
        });
        let json = serde_json::to_string(&stored).expect("setting values serialize");
        self.db.write_settings(&[(&settings_key(id), (!stored.is_empty()).then_some(json.as_str()))])?;
        Ok(merged(specs, &typed(stored)))
    }

    /// Where `id` keeps what it needs between runs (its runs start there).
    pub fn data_dir(&self, id: &PluginId) -> PathBuf {
        self.data.join(id.as_str())
    }

    /// Where the runs of `id` write their log.
    pub fn log_path(&self, id: &PluginId) -> PathBuf {
        self.logs.join(format!("{id}.log"))
    }

    /// A run of `action` of `p`, as [`crate::runtime::run`] takes it: its
    /// command from its folder, started in [`Store::data_dir`] and logged to
    /// [`Store::log_path`], and a run line with a new run id that names the
    /// plugin, this AutoScene, the app's FFmpeg (`ffmpeg` and `ffprobe`, of
    /// `ffmpeg_version`) and each of its settings, as [`Store::values`] gives
    /// them. The project and the export it runs on are the host's to add. A
    /// broken plugin can't run.
    pub fn run_spec(
        &self,
        p: &Installed,
        action: ActionContext,
        ffmpeg: &Path,
        ffprobe: &Path,
        ffmpeg_version: &str,
    ) -> PluginResult<RunSpec> {
        let m = manifest_of(p)?;
        let id = &p.record.id;
        let data_dir = self.data_dir(id);
        let message = RunMessage {
            protocol: manifest::PROTOCOL,
            run_id: Uuid::now_v7().to_string(),
            plugin: PluginContext {
                id: id.clone(),
                version: p.record.version.clone(),
                data_dir: data_dir.to_string_lossy().into_owned(),
            },
            app: AppContext { version: self.app.to_string(), platform: current_platform().into() },
            tools: ToolPaths {
                ffmpeg: ffmpeg.to_string_lossy().into_owned(),
                ffprobe: ffprobe.to_string_lossy().into_owned(),
                ffmpeg_version: ffmpeg_version.into(),
            },
            settings: merged(&m.settings, &self.stored_values(id)?.unwrap_or_default()),
            action,
            project: None,
            export: None,
        };
        Ok(RunSpec {
            name: m.name.clone(),
            dir: p.dir.clone(),
            launch: m.launch()?,
            data_dir,
            log_path: self.log_path(id),
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
            message,
        })
    }

    /// The icon of `p` as a `data:` URL, while it is the picture its
    /// package listed (a loaded folder's: a picture of an icon's size).
    pub fn icon_data_url(&self, p: &Installed) -> Option<String> {
        let icon = p.manifest.as_ref()?.icon.as_deref()?;
        let path = file_in(&p.dir, icon);
        let lock = match &p.record.lock_sha256 {
            Some(sha256) => installed_lock(&p.dir, sha256).ok()?,
            None => {
                let size = fs::symlink_metadata(&path).ok().filter(|f| f.is_file())?.len();
                let listed = LockedFile { path: icon.to_owned(), size, sha256: String::new(), exec: false };
                FilesLock { lock_version: LOCK_VERSION, files: vec![listed] }
            }
        };
        package::check_icon(&p.dir, &lock, icon).ok()?;
        let listed = lock.files.iter().find(|f| f.path == icon)?;
        let mut bytes = Vec::new();
        File::open(&path).ok()?.take(listed.size + 1).read_to_end(&mut bytes).ok()?;
        let hashed = p.record.lock_sha256.is_none() || package::sha256_hex(&bytes) == listed.sha256;
        if bytes.len() as u64 != listed.size || !hashed {
            return None;
        }
        let kind = if icon.to_ascii_lowercase().ends_with(".svg") { "image/svg+xml" } else { "image/png" };
        Some(format!("data:{kind};base64,{}", STANDARD.encode(&bytes)))
    }

    /// The banner of `p` as a `data:` URL, while it is the picture its
    /// package listed (a loaded folder's: a picture of a banner's size).
    pub fn banner_data_url(&self, p: &Installed) -> Option<String> {
        let banner = p.manifest.as_ref()?.banner.as_deref()?;
        let path = file_in(&p.dir, banner);
        let lock = match &p.record.lock_sha256 {
            Some(sha256) => installed_lock(&p.dir, sha256).ok()?,
            None => {
                let size = fs::symlink_metadata(&path).ok().filter(|f| f.is_file())?.len();
                let listed = LockedFile { path: banner.to_owned(), size, sha256: String::new(), exec: false };
                FilesLock { lock_version: LOCK_VERSION, files: vec![listed] }
            }
        };
        package::check_banner(&p.dir, &lock, banner).ok()?;
        let listed = lock.files.iter().find(|f| f.path == banner)?;
        let mut bytes = Vec::new();
        File::open(&path).ok()?.take(listed.size + 1).read_to_end(&mut bytes).ok()?;
        let hashed = p.record.lock_sha256.is_none() || package::sha256_hex(&bytes) == listed.sha256;
        if bytes.len() as u64 != listed.size || !hashed {
            return None;
        }
        let kind = if banner.to_ascii_lowercase().ends_with(".svg") { "image/svg+xml" } else { "image/png" };
        Some(format!("data:{kind};base64,{}", STANDARD.encode(&bytes)))
    }

    /// The registry's rows, by id. A key whose end is no plugin id is no
    /// plugin's.
    fn rows(&self) -> PluginResult<Vec<(PluginId, String)>> {
        let rows = self.db.settings_in(ROW)?;
        Ok(rows
            .into_iter()
            .filter_map(|(key, value)| Some((PluginId::parse(&key[ROW.len()..]).ok()?, value)))
            .collect())
    }

    /// The record of `id`, to change or replace: none when it isn't
    /// installed, and an error when a later AutoScene wrote its row.
    fn record(&self, id: &PluginId) -> PluginResult<Option<Record>> {
        let Some(value) = self.db.setting(&row_key(id))? else { return Ok(None) };
        read_row(id, &value).map(Some).ok_or_else(|| {
            invalid("A newer AutoScene installed this plugin, so this one can't change it. Update AutoScene, or uninstall the plugin.")
        })
    }

    fn write_record(&self, record: &Record) -> PluginResult<()> {
        let json = serde_json::to_string(record).expect("a plugin record serializes");
        Ok(self.db.write_settings(&[(&row_key(&record.id), Some(&json))])?)
    }

    /// The plugin of the row `value`.
    fn installed(&self, id: PluginId, value: &str) -> Installed {
        match read_row(&id, value) {
            Some(record) => self.load(record),
            None => Installed {
                dir: self.root.join(id.as_str()),
                record: unreadable(id, value),
                manifest: None,
                warnings: Vec::new(),
                broken: Some(NEWER.into()),
            },
        }
    }

    /// The plugin of `record`, with the manifest in its folder, while it
    /// works with this AutoScene.
    fn load(&self, record: Record) -> Installed {
        let dir = match &record.source {
            Source::Package { folder } => self.root.join(record.id.as_str()).join(folder),
            Source::Folder { path } => PathBuf::from(path),
        };
        // Checked again here: AutoScene may have been put back to an older version.
        let read = read_manifest(&record, &dir).and_then(|parsed| match parsed.manifest.requirement_met(&self.app) {
            Ok(()) => Ok(parsed),
            Err(PluginError::TooNew(_)) => Err(NEWER.into()),
            Err(e) => Err(e.to_string()),
        });
        let (manifest, warnings, broken) = match read {
            Ok(Parsed { manifest, warnings }) => (Some(manifest), warnings, None),
            Err(broken) => (None, Vec::new(), Some(broken)),
        };
        Installed { record, manifest, warnings, dir, broken }
    }

    /// Removes `path` (a file or folder), best-effort. It is moved into
    /// `.trash` first ([`Store::move_away`]), so it is gone from its place at
    /// once, also when a file in it can't be removed yet; the next open
    /// removes what stays there.
    fn throw_away(&self, path: &Path) {
        let Some(gone) = self.move_away(path) else { return };
        if let Err(e) = remove_all(&gone) {
            tracing::warn!(path = %as_core::paths::name(path), error = %e, "a plugin's old files stay");
        }
    }

    /// Moves `path` into `.trash` under a name of its own, and gives where
    /// it is to be removed: there, or where it is for a plugin's data on
    /// another drive. None when it is gone already, or can't be moved (a
    /// running program holds it on Windows): it stays whole where it is, for
    /// the next open to sweep.
    fn move_away(&self, path: &Path) -> Option<PathBuf> {
        let trash = self.root.join(TRASH);
        // Made again when something removed it, so the rename has a place.
        let _ = fs::create_dir_all(&trash);
        let trash = trash.join(Uuid::now_v7().simple().to_string());
        match fs::rename(path, &trash) {
            Ok(()) => Some(trash),
            // Gone already; not a trash folder that is missing.
            Err(_) if fs::symlink_metadata(path).is_err_and(|e| e.kind() == ErrorKind::NotFound) => None,
            Err(_) if !path.starts_with(&self.root) => Some(path.to_path_buf()),
            Err(e) => {
                tracing::warn!(path = %as_core::paths::name(path), error = %e, "a plugin's old files stay");
                None
            }
        }
    }

    /// The person's values of the settings of `id`, or none when their row
    /// doesn't read (a later AutoScene's). A value that doesn't read is left
    /// out.
    fn stored_values(&self, id: &PluginId) -> PluginResult<Option<BTreeMap<String, SettingValue>>> {
        Ok(self.stored_row(id)?.map(typed))
    }

    /// The person's values of the settings of `id` as their row holds them,
    /// or none when it doesn't read (a later AutoScene's).
    fn stored_row(&self, id: &PluginId) -> PluginResult<Option<BTreeMap<String, Value>>> {
        let Some(json) = self.db.setting(&settings_key(id))? else { return Ok(Some(BTreeMap::new())) };
        match serde_json::from_str(&json) {
            Ok(values) => Ok(Some(values)),
            Err(e) => {
                tracing::warn!(plugin = %id, error = %e, "a plugin's settings don't read");
                Ok(None)
            }
        }
    }
}

/// The values of a settings row that read as setting values.
fn typed(row: BTreeMap<String, Value>) -> BTreeMap<String, SettingValue> {
    row.into_iter().filter_map(|(key, value)| Some((key, serde_json::from_value(value).ok()?))).collect()
}

fn row_key(id: &PluginId) -> String {
    format!("{ROW}{id}")
}

fn settings_key(id: &PluginId) -> String {
    format!("{SETTINGS_ROW}{id}")
}

fn not_found(id: &PluginId) -> PluginError {
    PluginError::NotFound(id.to_string())
}

fn changed_while_waiting() -> PluginError {
    invalid("The plugin's files changed while it waited to be installed. Open it again.")
}

/// The record of the row `value` of `id`, when this AutoScene can read it:
/// of [`RECORD_VERSION`], for `id`, with a version, and with its files in
/// one folder under `plugins/<id>/` or a loaded folder's absolute path.
fn read_row(id: &PluginId, value: &str) -> Option<Record> {
    let record = match serde_json::from_str::<Record>(value) {
        Ok(record) => record,
        Err(e) => {
            tracing::debug!(plugin = %id, error = %e, "a plugin's row doesn't read");
            return None;
        }
    };
    let sound = match &record.source {
        Source::Package { folder } => {
            !folder.contains('/') && package::check_path(folder).is_ok() && record.lock_sha256.is_some()
        }
        Source::Folder { path } => Path::new(path).is_absolute() && record.lock_sha256.is_none(),
    };
    (record.v == RECORD_VERSION && record.id == *id && Version::parse(&record.version).is_ok() && sound)
        .then_some(record)
}

/// What can be shown of the row `value` this AutoScene can't read: its id
/// and version. It counts as off, and its files as unknown.
fn unreadable(id: PluginId, value: &str) -> Record {
    let row: Value = serde_json::from_str(value).unwrap_or_default();
    Record {
        v: row["v"].as_u64().and_then(|v| u32::try_from(v).ok()).unwrap_or_default(),
        id,
        version: row["version"].as_str().filter(|v| Version::parse(v).is_ok()).unwrap_or_default().to_owned(),
        source: Source::Package { folder: String::new() },
        trust: Trust::Unreviewed,
        author_key: None,
        permissions: Vec::new(),
        network_hosts: Vec::new(),
        enabled: false,
        after_export: Vec::new(),
        lock_sha256: None,
        installed_at: 0,
    }
}

/// The manifest in the folder `dir` of `record`, or why there is none to
/// go by: a package's must be the one its install checked (its lock is the
/// recorded one and lists it as it is), a loaded folder's must name the
/// same plugin.
fn read_manifest(record: &Record, dir: &Path) -> Result<Parsed, String> {
    if !dir.is_dir() {
        return Err(MISSING.into());
    }
    let bytes = manifest::read_file(&dir.join(MANIFEST));
    let bytes = match (&record.lock_sha256, bytes) {
        (Some(sha256), Ok(bytes)) => {
            let lock = installed_lock(dir, sha256)?;
            let listed = lock.files.iter().find(|f| f.path == MANIFEST);
            if listed.is_none_or(|f| f.sha256 != package::sha256_hex(&bytes)) {
                return Err(CHANGED.into());
            }
            bytes
        }
        (Some(_), Err(_)) => return Err(CHANGED.into()),
        (None, Ok(bytes)) => bytes,
        (None, Err(e)) if e.kind() == ErrorKind::NotFound => return Err(MISSING.into()),
        (None, Err(e)) => return Err(as_core::io_message(&e).into()),
    };
    let parsed = manifest::parse_for_any_app(&bytes).map_err(|e| e.to_string())?;
    if parsed.manifest.id != record.id {
        return Err("Its plugin.json names another plugin now.".into());
    }
    Ok(parsed)
}

/// The lock in an installed package's folder `dir`, when it is the one its
/// install recorded (of SHA-256 `sha256`).
fn installed_lock(dir: &Path, sha256: &str) -> Result<FilesLock, String> {
    let mut bytes = Vec::new();
    let read = File::open(dir.join(LOCK)).and_then(|file| file.take(MAX_LOCK_BYTES + 1).read_to_end(&mut bytes));
    if read.is_err() || package::sha256_hex(&bytes) != sha256 {
        return Err(CHANGED.into());
    }
    package::read_lock(&bytes).map_err(|_| CHANGED.into())
}

/// Whether a plugin of `trust` may be installed: one AutoScene hasn't
/// reviewed needs the setting that allows them, and the person's yes.
fn allowed(trust: Trust, allow_unreviewed: bool, accept_unreviewed: bool) -> PluginResult<()> {
    match trust {
        Trust::Reviewed => Ok(()),
        _ if !allow_unreviewed => Err(PluginError::NotAllowed),
        _ if !accept_unreviewed => Err(invalid("Confirm that you want to install it.")),
        _ => Ok(()),
    }
}

/// Whether the plugin of manifest `m`, of `trust` and with the lock of
/// SHA-256 `lock_sha256` (none for a folder), may replace the installed
/// `previous`: a reviewed plugin takes only reviewed versions, none goes
/// back to an older version, and a package's version installs again only
/// with the same files (a repair). The pin is [`trust::judge`]'s.
fn check_update(previous: &Record, m: &Manifest, trust: Trust, lock_sha256: Option<&str>) -> PluginResult<()> {
    if previous.trust == Trust::Reviewed && trust != Trust::Reviewed {
        return Err(invalid("AutoScene reviewed the installed version of this plugin, and not this one."));
    }
    let (Ok(installed), Ok(new)) = (Version::parse(&previous.version), Version::parse(&m.version)) else {
        return Err(invalid("The plugin's version isn't valid."));
    };
    match installed.cmp_precedence(&new) {
        Ordering::Greater => Err(invalid(format!(
            "Version {installed} is installed, which is newer than {new}. Uninstall it first to go back."
        ))),
        Ordering::Equal
            if matches!(previous.source, Source::Package { .. }) && previous.lock_sha256.as_deref() != lock_sha256 =>
        {
            Err(invalid(format!(
                "Version {installed} is installed already, with other files. A changed plugin needs a new version."
            )))
        }
        _ => Ok(()),
    }
}

/// The manifest of `p`, or why it has none.
fn manifest_of(p: &Installed) -> PluginResult<&Manifest> {
    p.manifest.as_ref().ok_or_else(|| invalid(p.broken.clone().unwrap_or_else(|| MISSING.into())))
}

/// The value of each setting of `specs`: the stored one while it fits, else
/// its default.
fn merged(specs: &[SettingSpec], stored: &BTreeMap<String, SettingValue>) -> BTreeMap<String, SettingValue> {
    let value = |s: &SettingSpec| {
        stored.get(&s.key).filter(|v| check_value(s, v).is_ok()).cloned().or_else(|| default_of(&s.kind))
    };
    specs.iter().filter_map(|s| Some((s.key.clone(), value(s)?))).collect()
}

fn default_of(kind: &SettingKind) -> Option<SettingValue> {
    match kind {
        SettingKind::String { default, .. } | SettingKind::Select { default, .. } => {
            default.clone().map(SettingValue::Text)
        }
        SettingKind::Number { default, .. } => default.map(SettingValue::Number),
        SettingKind::Boolean { default } => Some(SettingValue::Bool(*default)),
    }
}

/// Checks `value` against its setting `spec`: text of at most its length
/// without NUL, a finite number within its bounds (whole when it must be),
/// on or off, or one of a select's values.
fn check_value(spec: &SettingSpec, value: &SettingValue) -> PluginResult<()> {
    let refuse = |why: String| Err(invalid(format!("{} {why}.", shown(&spec.title))));
    match (&spec.kind, value) {
        (SettingKind::String { max_length, .. }, SettingValue::Text(text)) => {
            if text.contains('\0') {
                return refuse("can't hold a NUL character".into());
            }
            if text.chars().count() > *max_length as usize {
                return refuse(format!("can be at most {max_length} characters"));
            }
        }
        (SettingKind::String { .. }, _) => return refuse("must be text".into()),
        (SettingKind::Number { min, max, integer, .. }, SettingValue::Number(n)) => {
            if !n.is_finite() {
                return refuse("must be a number".into());
            }
            if *integer && n.fract() != 0.0 {
                return refuse("must be a whole number".into());
            }
            if let Some(min) = min.filter(|min| n < min) {
                return refuse(format!("must be at least {min}"));
            }
            if let Some(max) = max.filter(|max| n > max) {
                return refuse(format!("must be at most {max}"));
            }
        }
        (SettingKind::Number { .. }, _) => return refuse("must be a number".into()),
        (SettingKind::Boolean { .. }, SettingValue::Bool(_)) => {}
        (SettingKind::Boolean { .. }, _) => return refuse("must be on or off".into()),
        (SettingKind::Select { options, .. }, SettingValue::Text(text)) if options.iter().any(|o| o.value == *text) => {
        }
        (SettingKind::Select { .. }, _) => return refuse("must be one of its choices".into()),
    }
    Ok(())
}

/// Renames `from` to `to`, trying again for a moment while Windows refuses:
/// a virus scanner may still hold a file just unpacked there.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let start = Instant::now();
    loop {
        match fs::rename(from, to) {
            Err(e) if cfg!(windows) && e.kind() == ErrorKind::PermissionDenied && start.elapsed() < RENAME_WAIT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            renamed => return renamed,
        }
    }
}

/// Removes the file or folder `path`.
fn remove_all(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use minisign::KeyPair;
    use serde_json::json;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::package::{AUTHOR_KEY, SIG};

    const ID: &str = "dev.autoscene.hello-export";

    type Files = Vec<(String, Vec<u8>)>;

    fn app() -> Version {
        Version::parse("1.0.3-BETA").unwrap()
    }

    fn id() -> PluginId {
        PluginId::parse(ID).unwrap()
    }

    /// A test's app data folder and library.db, with room beside them for
    /// packages and a developer's folder.
    struct Place {
        tmp: tempfile::TempDir,
        paths: AppPaths,
        db: Arc<LibraryDb>,
    }

    impl Place {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let paths = AppPaths::at(tmp.path().join("data"));
            let db = Arc::new(LibraryDb::open(&paths.library_db()).unwrap());
            Self { tmp, paths, db }
        }

        fn store(&self) -> Store {
            self.store_with(Keys::new(&[]).unwrap())
        }

        fn store_with(&self, keys: Keys) -> Store {
            Store::open(&self.paths, Arc::clone(&self.db), keys, app()).unwrap()
        }

        /// What `plugins/` holds, two levels deep: paths with `/`, sorted.
        fn plugins(&self) -> Vec<String> {
            let mut out = Vec::new();
            for entry in fs::read_dir(self.paths.plugins()).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().unwrap().is_dir() {
                    for inner in fs::read_dir(entry.path()).unwrap() {
                        out.push(format!("{name}/{}", inner.unwrap().file_name().to_string_lossy()));
                    }
                }
                out.push(name);
            }
            out.sort();
            out
        }

        /// Makes library.db refuse to write the row of [`ID`], until
        /// [`Place::allow_writes`].
        fn refuse_writes(&self) {
            self.sql(&format!(
                "CREATE TRIGGER refuse_insert BEFORE INSERT ON settings WHEN NEW.key = 'plugin.{ID}'
                     BEGIN SELECT RAISE(ABORT, 'refused'); END;
                 CREATE TRIGGER refuse_update BEFORE UPDATE ON settings WHEN NEW.key = 'plugin.{ID}'
                     BEGIN SELECT RAISE(ABORT, 'refused'); END;"
            ));
        }

        fn allow_writes(&self) {
            self.sql("DROP TRIGGER refuse_insert; DROP TRIGGER refuse_update;");
        }

        fn sql(&self, sql: &str) {
            rusqlite::Connection::open(self.paths.library_db()).unwrap().execute_batch(sql).unwrap();
        }

        /// A package of `files` (`plugin.json` first) with their lock, as
        /// asp packs it: signed by `signer` if given, with its author.pub
        /// when `with_pub`.
        fn package(&self, files: &Files, signer: Option<&KeyPair>, with_pub: bool) -> PathBuf {
            let listed = files.iter().map(|(path, bytes)| LockedFile {
                path: path.clone(),
                size: bytes.len() as u64,
                sha256: package::sha256_hex(bytes),
                exec: false,
            });
            let lock = FilesLock { lock_version: LOCK_VERSION, files: listed.collect() };
            let lock_bytes = serde_json::to_vec(&lock).unwrap();
            let mut entries = files.clone();
            entries.push((LOCK.into(), lock_bytes.clone()));
            if let Some(pair) = signer {
                let m: Value = serde_json::from_slice(&files[0].1).unwrap();
                let (id, version) =
                    (PluginId::parse(m["id"].as_str().unwrap()).unwrap(), m["version"].as_str().unwrap());
                let comment = trust::trusted_comment(&id, version, &package::sha256_hex(&lock_bytes));
                let sig = minisign::sign(Some(&pair.pk), &pair.sk, &lock_bytes[..], Some(&comment), None).unwrap();
                entries.push((SIG.into(), sig.into_string().into_bytes()));
                if with_pub {
                    entries.push((AUTHOR_KEY.into(), pair.pk.to_base64().into_bytes()));
                }
            }
            let path = self.tmp.path().join(format!("{}.asp", Uuid::now_v7().simple()));
            let mut zip = ZipWriter::new(File::create(&path).unwrap());
            for (name, bytes) in &entries {
                zip.start_file(name.as_str(), SimpleFileOptions::default()).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            path
        }

        fn unsigned(&self, version: &str) -> PathBuf {
            self.package(&hello(version), None, false)
        }

        /// A developer's folder of `files`.
        fn folder(&self, name: &str, files: &Files) -> PathBuf {
            let dir = self.tmp.path().join(name);
            fs::create_dir(&dir).unwrap();
            for (path, bytes) in files {
                fs::write(dir.join(path), bytes).unwrap();
            }
            dir
        }
    }

    /// A plugin's manifest: two export actions and a setting of each kind.
    fn manifest(id: &str, version: &str) -> Value {
        json!({
            "manifestVersion": 1,
            "id": id,
            "version": version,
            "name": "Hello Export",
            "description": "Says hello after an export.",
            "author": {"name": "AutoScene"},
            "engines": {"autoscene": ">=1.0.0, <2", "protocol": 1},
            "runtime": {"type": "process", "command": {"*": ["uv", "run", "--script", "--locked", "main.py"]}},
            "permissions": ["exports:read"],
            "contributes": {
                "exportActions": [{"id": "hello", "title": "Say hello"}, {"id": "wave", "title": "Wave"}],
                "settings": [
                    {"key": "greeting", "type": "string", "title": "Greeting", "default": "Hi", "maxLength": 5},
                    {"key": "count", "type": "number", "title": "Count", "default": 3, "min": 1, "max": 10,
                     "integer": true},
                    {"key": "loud", "type": "boolean", "title": "Loud", "default": true},
                    {"key": "mode", "type": "select", "title": "Mode", "default": "a",
                     "options": [{"value": "a", "label": "A"}, {"value": "b", "label": "B"}]}
                ]
            }
        })
    }

    /// The files of the plugin of `manifest`: a uv script and its lock.
    fn files(manifest: &Value) -> Files {
        vec![
            ("plugin.json".into(), serde_json::to_vec_pretty(manifest).unwrap()),
            ("main.py".into(), b"print('hello')\n".to_vec()),
            ("main.py.lock".into(), b"version = 1\n".to_vec()),
        ]
    }

    fn hello(version: &str) -> Files {
        files(&manifest(ID, version))
    }

    /// Installs the package at `path`, with the setting on and the person's yes.
    fn install(store: &Store, path: &Path) -> PluginResult<Installed> {
        store.commit(store.stage(path)?, true, true)
    }

    /// What `plugins/` holds with only `p` installed.
    fn holding(p: &Installed) -> Vec<String> {
        let Source::Package { folder } = &p.record.source else { panic!("{:?}", p.record.source) };
        vec![".staging".into(), ".trash".into(), ID.into(), format!("{ID}/{folder}")]
    }

    fn key_pair() -> KeyPair {
        KeyPair::generate_unencrypted_keypair().unwrap()
    }

    fn values(pairs: &[(&str, SettingValue)]) -> BTreeMap<String, SettingValue> {
        pairs.iter().map(|(key, value)| (key.to_string(), value.clone())).collect()
    }

    #[test]
    fn an_install_is_all_or_nothing() {
        let place = Place::new();
        let store = place.store();
        let package = place.unsigned("1.0.0");
        place.refuse_writes();
        let t = store.stage(&package).unwrap();
        assert_eq!(place.plugins().len(), 3, "staged: {:?}", place.plugins());
        assert!(matches!(store.commit(t, true, true), Err(PluginError::Db(_))));
        assert!(store.list().unwrap().is_empty());
        assert_eq!(place.plugins(), [".staging", ".trash"]);

        place.allow_writes();
        let p = install(&store, &package).unwrap();
        assert_eq!((p.record.version.as_str(), p.broken.as_deref()), ("1.0.0", None));
        assert_eq!((p.record.enabled, p.record.trust), (true, Trust::Unreviewed));
        assert_eq!(place.plugins(), holding(&p));
        assert!(p.dir.join("main.py").is_file());
    }

    #[test]
    fn an_update_replaces_the_old_version_after_the_row_is_written() {
        let place = Place::new();
        let store = place.store();
        let old = install(&store, &place.unsigned("1.0.0")).unwrap();
        store.set_enabled(&id(), false).unwrap();
        store.set_after_export(&id(), "wave", true).unwrap();
        store.set_after_export(&id(), "hello", true).unwrap();
        // 1.1.0 has no wave any more.
        let mut m = manifest(ID, "1.1.0");
        m["contributes"]["exportActions"] = json!([{"id": "hello", "title": "Say hello"}]);
        let update = place.package(&files(&m), None, false);

        place.refuse_writes();
        assert!(install(&store, &update).is_err());
        let p = store.get(&id()).unwrap();
        assert_eq!(
            (p.record.version.as_str(), p.record.after_export.clone()),
            ("1.0.0", vec!["hello".into(), "wave".into()])
        );
        assert_eq!((p.dir.as_path(), p.broken.as_deref()), (old.dir.as_path(), None));
        assert_eq!(place.plugins(), holding(&old));

        place.allow_writes();
        let new = install(&store, &update).unwrap();
        assert_eq!(new.record.version, "1.1.0");
        assert!(!new.record.enabled, "a plugin that was off stays off");
        assert_eq!(new.record.after_export, ["hello"]);
        assert_eq!(store.get(&id()).unwrap().record, new.record);
        assert_eq!(place.plugins(), holding(&new));
    }

    #[test]
    fn leftovers_from_a_crash_are_swept_at_open() {
        let place = Place::new();
        let p = install(&place.store(), &place.unsigned("1.0.0")).unwrap();
        let make = |path: &str| {
            let path = place.paths.plugins().join(path);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("x"), "x").unwrap();
        };
        // A staged and a thrown-away package, a version an update left, a
        // plugin whose row went, and a later AutoScene's plugin.
        make(".staging/0190f0e8a3b47c1d");
        make(".trash/0190f0e8a3b47c1e");
        make(&format!("{ID}/0.9.0+0badf00d"));
        make("dev.autoscene.gone/1.0.0+12345678");
        make("dev.autoscene.later/2.0.0+12345678");
        let later = r#"{"v":2,"id":"dev.autoscene.later","version":"2.0.0"}"#;
        place.db.set_setting("plugin.dev.autoscene.later", later).unwrap();

        let store = place.store();
        let mut kept = holding(&p);
        kept.extend(["dev.autoscene.later".into(), "dev.autoscene.later/2.0.0+12345678".into()]);
        // Out of their places at once, all in the trash, whose old files stay
        // with them until they are removed.
        let trash = place.paths.plugins().join(TRASH);
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 4);
        // What is staged meanwhile isn't touched by their removal.
        let staged = store.stage(&place.unsigned("1.1.0")).unwrap();
        store.remove_leftovers();
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 0);
        assert!(staged.staged.as_ref().unwrap().join("plugin.json").is_file());
        store.discard(staged);
        assert_eq!(place.plugins(), kept);
        assert!(store.get(&id()).unwrap().broken.is_none());
        assert_eq!(place.db.setting("plugin.dev.autoscene.later").unwrap().as_deref(), Some(later));
    }

    #[test]
    fn a_row_whose_folder_is_gone_is_listed_as_broken() {
        let place = Place::new();
        let package = place.unsigned("1.0.0");
        let p = install(&place.store(), &package).unwrap();
        fs::remove_dir_all(&p.dir).unwrap();
        let store = place.store();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].broken.as_deref(), listed[0].manifest.is_none()), (Some(MISSING), true));
        assert!(store.values(&id()).is_err());

        // Installed again, it works again.
        let p = install(&store, &package).unwrap();
        assert!(p.broken.is_none());
        // A file of it changed is as good as gone.
        fs::write(p.dir.join("plugin.json"), b"{}").unwrap();
        assert_eq!(store.get(&id()).unwrap().broken.as_deref(), Some(CHANGED));
        // So is another files.lock, also one that lists the same files.
        let p = install(&store, &package).unwrap();
        let lock: Value = serde_json::from_slice(&fs::read(p.dir.join(LOCK)).unwrap()).unwrap();
        fs::write(p.dir.join(LOCK), serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
        assert_eq!(store.get(&id()).unwrap().broken.as_deref(), Some(CHANGED));
    }

    #[test]
    fn a_row_that_does_not_fit_its_plugin_is_listed_as_needing_a_newer_autoscene() {
        let place = Place::new();
        let store = place.store();
        install(&store, &place.unsigned("1.0.0")).unwrap();
        let row = format!("plugin.{ID}");
        let written: Value = serde_json::from_str(&place.db.setting(&row).unwrap().unwrap()).unwrap();
        let changes: [fn(&mut Value); 5] = [
            |r| r["id"] = json!("dev.autoscene.other"),
            |r| r["version"] = json!("one"),
            |r| r["source"]["folder"] = json!("1.0.0+0badf00d/x"),
            |r| r["lockSha256"] = Value::Null,
            // A loaded folder's path must be absolute.
            |r| {
                r["source"] = json!({"kind": "folder", "path": "hello-export"});
                r["lockSha256"] = Value::Null;
            },
        ];
        for change in changes {
            let mut r = written.clone();
            change(&mut r);
            place.db.set_setting(&row, &r.to_string()).unwrap();
            assert_eq!(store.get(&id()).unwrap().broken.as_deref(), Some(NEWER), "{r}");
        }
    }

    #[test]
    fn a_plugin_is_listed_as_broken_on_an_autoscene_it_does_not_work_with() {
        let place = Place::new();
        let p = install(&place.store(), &place.unsigned("1.0.0")).unwrap();
        // It works with ">=1.0.0, <2".
        let broken_on = |app: &str| {
            let keys = Keys::new(&[]).unwrap();
            let store = Store::open(&place.paths, Arc::clone(&place.db), keys, Version::parse(app).unwrap()).unwrap();
            store.get(&id()).unwrap().broken
        };
        assert_eq!(broken_on("0.9.0").as_deref(), Some(NEWER));
        assert_eq!(broken_on("2.0.0").as_deref(), Some("This plugin only works with an older AutoScene."));
        assert_eq!(broken_on("1.5.0"), None);
        assert_eq!(place.plugins(), holding(&p), "its files stay");
    }

    #[test]
    fn an_unreadable_row_is_kept_and_listed_as_needing_a_newer_autoscene() {
        let place = Place::new();
        let later = r#"{"v":2,"id":"dev.autoscene.later","version":"2.0.0","runtime":"wasm"}"#;
        place.db.set_setting("plugin.dev.autoscene.later", later).unwrap();
        place.db.set_setting("plugin.dev.autoscene.damaged", "not json").unwrap();
        let store = place.store();
        let listed = store.list().unwrap();
        let shown: Vec<_> =
            listed.iter().map(|p| (p.record.id.as_str(), p.record.version.as_str(), p.broken.as_deref())).collect();
        assert_eq!(shown, [("dev.autoscene.damaged", "", Some(NEWER)), ("dev.autoscene.later", "2.0.0", Some(NEWER))]);

        let later_id = PluginId::parse("dev.autoscene.later").unwrap();
        assert!(store.set_enabled(&later_id, true).is_err());
        assert!(store.set_after_export(&later_id, "hello", true).is_err());
        assert!(store.set_values(&later_id, values(&[("count", SettingValue::Number(4.0))])).is_err());
        let package = place.package(&files(&manifest("dev.autoscene.later", "3.0.0")), None, false);
        assert!(store.stage(&package).is_err());
        assert_eq!(place.db.setting("plugin.dev.autoscene.later").unwrap().as_deref(), Some(later));
        assert_eq!(place.db.setting("plugin_settings.dev.autoscene.later").unwrap(), None);
        assert_eq!(place.plugins(), [".staging", ".trash"]);

        // Uninstalling it is the way out.
        store.uninstall(&later_id, false).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);

        // A record this AutoScene could read, but of a later version.
        install(&store, &place.unsigned("1.0.0")).unwrap();
        let row = format!("plugin.{ID}");
        let v2 = place.db.setting(&row).unwrap().unwrap().replace(r#""v":1,"#, r#""v":2,"#);
        place.db.set_setting(&row, &v2).unwrap();
        assert_eq!(store.get(&id()).unwrap().broken.as_deref(), Some(NEWER));
        assert!(store.set_enabled(&id(), false).is_err());
        assert!(store.stage(&place.unsigned("1.1.0")).is_err());
        assert_eq!(place.db.setting(&row).unwrap(), Some(v2));
    }

    #[test]
    fn uninstalling_a_loaded_folder_never_deletes_it() {
        let place = Place::new();
        let store = place.store();
        let dev = place.folder("hello-export", &hello("1.0.0"));
        let t = store.stage_folder(&dev).unwrap();
        assert_eq!((t.verdict.trust, t.lock.is_none()), (Trust::Unreviewed, true));
        let p = store.commit(t, true, true).unwrap();
        assert_eq!(p.record.source, Source::Folder { path: dev.to_str().unwrap().into() });
        assert_eq!((p.dir.as_path(), p.broken.as_deref()), (dev.as_path(), None));
        fs::create_dir_all(store.data_dir(&id())).unwrap();
        fs::write(store.data_dir(&id()).join("state.json"), b"{}").unwrap();

        store.uninstall(&id(), true).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(!store.data_dir(&id()).exists(), "its data went, as asked");
        let left = |dir: &Path| {
            let mut names: Vec<String> =
                fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        };
        assert_eq!(left(&dev), ["main.py", "main.py.lock", "plugin.json"]);

        // Nor does a package that replaces it.
        store.commit(store.stage_folder(&dev).unwrap(), true, true).unwrap();
        let p = install(&store, &place.unsigned("1.1.0")).unwrap();
        assert_eq!(left(&dev), ["main.py", "main.py.lock", "plugin.json"]);
        // An installed version's folder isn't loaded: an update would throw it away.
        let e = store.stage_folder(&p.dir).unwrap_err();
        assert_eq!(e.to_string(), "That folder belongs to an installed plugin. Load the folder you work in.");
        assert_eq!(place.plugins(), holding(&p));
    }

    #[test]
    fn a_loaded_folder_replaces_a_package_and_leaves_no_files_in_plugins() {
        let place = Place::new();
        let store = place.store();
        install(&store, &place.unsigned("1.0.0")).unwrap();
        let dev = place.folder("hello-export", &hello("1.1.0"));
        let p = store.commit(store.stage_folder(&dev).unwrap(), true, true).unwrap();
        assert_eq!(store.list().unwrap().iter().map(|p| &p.record).collect::<Vec<_>>(), [&p.record]);
        assert_eq!(place.plugins(), [".staging", ".trash"]);
        // What lies in plugins/<id>/ of a loaded folder is no version of it.
        fs::create_dir_all(place.paths.plugins().join(ID).join("1.0.0+0badf00d")).unwrap();
        place.store().remove_leftovers();
        assert_eq!(place.plugins(), [".staging", ".trash"]);
        assert_eq!(fs::read_dir(&dev).unwrap().count(), 3);
    }

    #[test]
    fn a_loaded_folder_is_checked_as_it_is() {
        let place = Place::new();
        let store = place.store();
        let mut no_script = hello("1.0.0");
        no_script.retain(|(path, _)| path != "main.py");
        let e = store.stage_folder(&place.folder("no-script", &no_script)).unwrap_err();
        assert_eq!(e.to_string(), "The plugin folder is missing main.py, which its command needs.");

        let dev = place.folder("hello-export", &hello("1.0.0"));
        store.commit(store.stage_folder(&dev).unwrap(), true, true).unwrap();
        let other = serde_json::to_vec(&manifest("dev.autoscene.other", "1.0.0")).unwrap();
        fs::write(dev.join("plugin.json"), other).unwrap();
        assert_eq!(store.get(&id()).unwrap().broken.as_deref(), Some("Its plugin.json names another plugin now."));
    }

    #[test]
    fn an_unknown_plugin_or_action_is_refused() {
        let place = Place::new();
        let store = place.store();
        assert!(matches!(store.uninstall(&id(), false), Err(PluginError::NotFound(_))));
        install(&store, &place.unsigned("1.0.0")).unwrap();
        let e = store.set_after_export(&id(), "bow", true).unwrap_err();
        assert_eq!(e.to_string(), "The plugin has no export action bow.");
        assert!(store.get(&id()).unwrap().record.after_export.is_empty());
    }

    #[test]
    fn throwing_away_needs_no_trash_folder() {
        let place = Place::new();
        let store = place.store();
        let trash = place.paths.plugins().join(TRASH);
        install(&store, &place.unsigned("1.0.0")).unwrap();
        fs::create_dir_all(store.data_dir(&id())).unwrap();
        fs::remove_dir(&trash).unwrap();
        store.uninstall(&id(), true).unwrap();
        assert!(!store.data_dir(&id()).exists(), "its data went, as asked");
        assert_eq!(place.plugins(), [".staging", ".trash"]);

        // A file in its place: the data goes where it is, the plugin's
        // folder at the next open.
        install(&store, &place.unsigned("1.0.0")).unwrap();
        fs::create_dir_all(store.data_dir(&id())).unwrap();
        fs::remove_dir(&trash).unwrap();
        fs::write(&trash, b"x").unwrap();
        store.uninstall(&id(), true).unwrap();
        assert!(!store.data_dir(&id()).exists());
        place.store().remove_leftovers();
        assert_eq!(place.plugins(), [".staging", ".trash"]);
    }

    #[test]
    fn settings_values_are_checked_against_their_spec() {
        use SettingValue::{Bool, Number, Text};
        let place = Place::new();
        let store = place.store();
        install(&store, &place.unsigned("1.0.0")).unwrap();
        let with = |count: f64, loud: bool| {
            values(&[
                ("count", Number(count)),
                ("greeting", Text("Hi".into())),
                ("loud", Bool(loud)),
                ("mode", Text("a".into())),
            ])
        };
        assert_eq!(store.values(&id()).unwrap(), with(3.0, true));
        for (key, value, why) in [
            ("greeting", Text("Hello!".into()), "Greeting can be at most 5 characters."),
            ("greeting", Text("a\0b".into()), "Greeting can't hold a NUL character."),
            ("greeting", Number(1.0), "Greeting must be text."),
            ("count", Number(11.0), "Count must be at most 10."),
            ("count", Number(0.0), "Count must be at least 1."),
            ("count", Number(2.5), "Count must be a whole number."),
            ("count", Number(f64::NAN), "Count must be a number."),
            ("count", Text("4".into()), "Count must be a number."),
            ("loud", Text("yes".into()), "Loud must be on or off."),
            ("mode", Text("c".into()), "Mode must be one of its choices."),
        ] {
            // A value that fits is refused with one that doesn't.
            let e = store.set_values(&id(), values(&[("count", Number(4.0)), (key, value)])).unwrap_err();
            assert_eq!(e.to_string(), why);
        }
        assert_eq!(store.values(&id()).unwrap(), with(3.0, true));

        let set = values(&[("count", Number(4.0)), ("mode", Text("a".into())), ("colour", Text("red".into()))]);
        assert_eq!(store.set_values(&id(), set).unwrap(), with(4.0, true));
        // Only what differs from its default is kept.
        let row = format!("plugin_settings.{ID}");
        assert_eq!(place.db.setting(&row).unwrap().as_deref(), Some(r#"{"count":4.0}"#));

        // A value that doesn't fit any more (an update changed its setting)
        // gives the default.
        let later = r##"{"count":40,"mode":"z","loud":false,"later":{"secret":1},"tint":"#ff0000"}"##;
        place.db.set_setting(&row, later).unwrap();
        assert_eq!(store.values(&id()).unwrap(), with(3.0, false));
        // Back at their defaults, nothing of its settings is kept; the values
        // of settings this AutoScene skipped (a later one's) stay as they are.
        store.set_values(&id(), values(&[("loud", Bool(true))])).unwrap();
        assert_eq!(place.db.setting(&row).unwrap().as_deref(), Some(r##"{"later":{"secret":1},"tint":"#ff0000"}"##));
        store.set_values(&id(), values(&[("count", Number(4.0))])).unwrap();
        assert_eq!(
            place.db.setting(&row).unwrap().as_deref(),
            Some(r##"{"count":4.0,"later":{"secret":1},"tint":"#ff0000"}"##)
        );
        assert_eq!(store.values(&id()).unwrap(), with(4.0, true));
        place.db.set_setting(&row, r#"{"loud":false}"#).unwrap();
        store.set_values(&id(), values(&[("loud", Bool(true))])).unwrap();
        assert_eq!(place.db.setting(&row).unwrap(), None);

        // Values this AutoScene can't read (a later one's) give the defaults,
        // and are never written over.
        place.db.set_setting(&row, "[1]").unwrap();
        assert_eq!(store.values(&id()).unwrap(), with(3.0, true));
        assert!(store.set_values(&id(), values(&[("count", Number(4.0))])).is_err());
        assert_eq!(place.db.setting(&row).unwrap().as_deref(), Some("[1]"));
    }

    #[test]
    fn plugin_rows_never_match_plugin_settings_rows() {
        let place = Place::new();
        let store = place.store();
        install(&store, &place.unsigned("1.0.0")).unwrap();
        store.set_values(&id(), values(&[("count", SettingValue::Number(5.0))])).unwrap();
        // Rows beside the registry's, which a `LIKE 'plugin%'` would find.
        place.db.set_setting("plugin_settings.dev.autoscene.other", "{}").unwrap();
        place.db.set_setting("plugins.dev.autoscene.other", "{}").unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.iter().map(|p| p.record.id.as_str()).collect::<Vec<_>>(), [ID]);
        assert!(listed[0].broken.is_none());

        store.uninstall(&id(), false).unwrap();
        assert_eq!(place.db.setting(&format!("plugin_settings.{ID}")).unwrap(), None);
        assert!(store.list().unwrap().is_empty());
        assert_eq!(place.db.setting("plugin_settings.dev.autoscene.other").unwrap().as_deref(), Some("{}"));
    }

    #[test]
    fn unreviewed_needs_the_setting_and_the_consent() {
        let place = Place::new();
        let (review, author) = (key_pair(), key_pair());
        let store = place.store_with(Keys::new(&[&review.pk.to_base64()]).unwrap());
        for (package, trust) in [
            (place.unsigned("1.0.0"), Trust::Unreviewed),
            (place.package(&hello("1.0.0"), Some(&author), true), Trust::Pinned),
        ] {
            let t = store.stage(&package).unwrap();
            assert_eq!(t.verdict.trust, trust);
            assert!(matches!(store.commit(t, false, true), Err(PluginError::NotAllowed)));
            let t = store.stage(&package).unwrap();
            assert_eq!(store.commit(t, true, false).unwrap_err().to_string(), "Confirm that you want to install it.");
            assert!(store.list().unwrap().is_empty());
            assert_eq!(place.plugins(), [".staging", ".trash"], "a refused package leaves nothing");
            assert_eq!(install(&store, &package).unwrap().record.trust, trust);
            store.uninstall(&id(), false).unwrap();
        }
        let dev = place.folder("hello-export", &hello("1.0.0"));
        let t = store.stage_folder(&dev).unwrap();
        assert!(matches!(store.commit(t, false, true), Err(PluginError::NotAllowed)));

        // One AutoScene reviewed needs neither.
        let t = store.stage(&place.package(&hello("1.0.0"), Some(&review), false)).unwrap();
        assert_eq!(t.verdict.trust, Trust::Reviewed);
        assert_eq!(store.commit(t, false, false).unwrap().record.trust, Trust::Reviewed);
    }

    #[test]
    fn a_pinned_plugin_takes_a_reviewed_update_and_keeps_its_pin() {
        let place = Place::new();
        let (review, author) = (key_pair(), key_pair());
        let store = place.store_with(Keys::new(&[&review.pk.to_base64()]).unwrap());
        let p = install(&store, &place.package(&hello("1.0.0"), Some(&author), true)).unwrap();
        let pin = p.record.author_key.clone();
        assert!(pin.is_some());
        let p = install(&store, &place.package(&hello("1.1.0"), Some(&review), false)).unwrap();
        assert_eq!((p.record.trust, &p.record.author_key), (Trust::Reviewed, &pin));
        // Reviewed now, it takes no version its author alone signed.
        let e = store.stage(&place.package(&hello("1.2.0"), Some(&author), true)).unwrap_err();
        assert_eq!(e.to_string(), "AutoScene reviewed the installed version of this plugin, and not this one.");

        // An unreviewed plugin takes a signed update, which pins its key.
        store.uninstall(&id(), false).unwrap();
        install(&store, &place.unsigned("1.0.0")).unwrap();
        let p = install(&store, &place.package(&hello("1.1.0"), Some(&author), true)).unwrap();
        assert_eq!((p.record.trust, &p.record.author_key), (Trust::Pinned, &pin));
    }

    #[test]
    fn a_ticket_never_throws_away_a_folder_outside_staging() {
        let place = Place::new();
        let store = place.store();
        let dev = place.folder("hello-export", &hello("1.0.0"));
        // A loaded folder's ticket, made to name that folder as staged.
        let mut t = store.stage_folder(&dev).unwrap();
        t.staged = Some(dev.clone());
        store.discard(t);
        assert!(dev.join("plugin.json").is_file(), "discard kept the developer's folder");
        // A package's, whose commit fails.
        let mut t = store.stage(&place.unsigned("1.0.0")).unwrap();
        let staged = t.staged.replace(dev.clone()).unwrap();
        assert!(store.commit(t, true, true).is_err());
        assert!(dev.join("plugin.json").is_file(), "a failed commit kept the developer's folder");
        assert!(store.list().unwrap().is_empty());
        let _ = remove_all(&staged);
    }

    #[test]
    fn a_reviewed_plugin_takes_only_reviewed_updates() {
        let place = Place::new();
        let (review, author) = (key_pair(), key_pair());
        let store = place.store_with(Keys::new(&[&review.pk.to_base64()]).unwrap());
        let reviewed = |version: &str| place.package(&hello(version), Some(&review), false);
        store.commit(store.stage(&reviewed("1.0.0")).unwrap(), false, false).unwrap();
        for update in [place.unsigned("1.1.0"), place.package(&hello("1.1.0"), Some(&author), true)] {
            let e = store.stage(&update).unwrap_err();
            assert_eq!(e.to_string(), "AutoScene reviewed the installed version of this plugin, and not this one.");
        }
        let dev = place.folder("hello-export", &hello("1.1.0"));
        assert!(store.stage_folder(&dev).is_err());

        let p = store.commit(store.stage(&reviewed("1.1.0")).unwrap(), false, false).unwrap();
        assert_eq!((p.record.version.as_str(), p.record.trust), ("1.1.0", Trust::Reviewed));
    }

    #[test]
    fn an_older_version_does_not_replace_a_newer_one() {
        let place = Place::new();
        let store = place.store();
        install(&store, &place.unsigned("1.2.0")).unwrap();
        for older in ["1.1.9", "1.2.0-beta.1"] {
            let e = store.stage(&place.unsigned(older)).unwrap_err();
            let why =
                format!("Version 1.2.0 is installed, which is newer than {older}. Uninstall it first to go back.");
            assert_eq!(e.to_string(), why);
        }
        // Checked again as it is installed: a newer version installed meanwhile stays.
        let a = store.stage(&place.unsigned("1.3.0")).unwrap();
        let b = store.stage(&place.unsigned("1.4.0")).unwrap();
        let p = store.commit(b, true, true).unwrap();
        assert!(store.commit(a, true, true).is_err());
        assert_eq!(store.get(&id()).unwrap().record.version, "1.4.0");
        assert_eq!(place.plugins(), holding(&p));
    }

    #[test]
    fn the_same_version_installs_again_only_with_the_same_files() {
        let place = Place::new();
        let store = place.store();
        let package = place.unsigned("1.0.0");
        let first = install(&store, &package).unwrap();
        let again = install(&store, &package).unwrap();
        assert_ne!(first.dir, again.dir);
        assert_eq!(place.plugins(), holding(&again));

        let mut changed = hello("1.0.0");
        changed[1].1 = b"print('bye')\n".to_vec();
        let e = store.stage(&place.package(&changed, None, false)).unwrap_err();
        let why = "Version 1.0.0 is installed already, with other files. A changed plugin needs a new version.";
        assert_eq!(e.to_string(), why);
        // A build of its own is the same version: the message names the installed one.
        assert_eq!(store.stage(&place.unsigned("1.0.0+other")).unwrap_err().to_string(), why);
        assert_eq!(store.get(&id()).unwrap().record, again.record);
    }

    #[test]
    fn a_version_installed_while_a_ticket_waits_keeps_its_files() {
        let place = Place::new();
        let store = place.store();
        let mut changed = hello("1.0.0");
        changed[1].1 = b"print('bye')\n".to_vec();
        // Both staged before either is installed, so nothing to compare with yet.
        let a = store.stage(&place.unsigned("1.0.0")).unwrap();
        let b = store.stage(&place.package(&changed, None, false)).unwrap();
        let p = store.commit(a, true, true).unwrap();
        let e = store.commit(b, true, true).unwrap_err();
        let why = "Version 1.0.0 is installed already, with other files. A changed plugin needs a new version.";
        assert_eq!(e.to_string(), why);
        assert_eq!(store.get(&id()).unwrap().record, p.record);
        assert_eq!(place.plugins(), holding(&p));
    }

    #[test]
    fn a_package_without_what_its_command_needs_or_with_a_wrong_icon_is_refused() {
        let place = Place::new();
        let store = place.store();
        let mut no_lock = hello("1.0.0");
        no_lock.retain(|(path, _)| path != "main.py.lock");
        let mut m = manifest(ID, "1.0.0");
        m["icon"] = json!("icon.png");
        let mut wrong_icon = files(&m);
        wrong_icon.push(("icon.png".into(), b"GIF89a".to_vec()));
        for (files, why) in [
            (no_lock, "The plugin package is missing main.py.lock, which its command needs."),
            (wrong_icon, "The plugin's icon icon.png isn't a PNG image."),
        ] {
            let e = store.stage(&place.package(&files, None, false)).unwrap_err();
            assert_eq!(e.to_string(), why);
            assert_eq!(place.plugins(), [".staging", ".trash"]);
        }
    }

    #[test]
    fn an_update_keeps_its_authors_pin() {
        let place = Place::new();
        let store = place.store();
        let (author, other) = (key_pair(), key_pair());
        let pin = AuthorKey { base64: author.pk.to_base64(), key_id: trust::key_id(&author.pk.to_base64()).unwrap() };
        let p = install(&store, &place.package(&hello("1.0.0"), Some(&author), true)).unwrap();
        assert_eq!((p.record.trust, p.record.author_key.as_ref()), (Trust::Pinned, Some(&pin)));
        // Signed by the pinned key, without author.pub: the pin checks it, and stays.
        let p = install(&store, &place.package(&hello("1.1.0"), Some(&author), false)).unwrap();
        assert_eq!((p.record.trust, p.record.author_key.as_ref()), (Trust::Pinned, Some(&pin)));
        for update in [place.package(&hello("1.2.0"), Some(&other), true), place.unsigned("1.2.0")] {
            assert!(matches!(store.stage(&update), Err(PluginError::KeyChanged { .. })));
        }
        let dev = place.folder("hello-export", &hello("1.2.0"));
        let e = store.stage_folder(&dev).unwrap_err();
        assert_eq!(e.to_string(), "The installed version is signed by its author. Uninstall it to load a folder.");
        assert_eq!(store.get(&id()).unwrap().record.author_key, Some(pin));
    }

    #[test]
    fn a_pin_set_while_a_ticket_waits_refuses_it() {
        let place = Place::new();
        let store = place.store();
        let (author, other) = (key_pair(), key_pair());
        // Staged while nothing is installed: pinned to the key it brings.
        let t = store.stage(&place.package(&hello("1.1.0"), Some(&other), true)).unwrap();
        assert_eq!(t.verdict.trust, Trust::Pinned);
        // Meanwhile the author's version is installed, and pins the author's key.
        let p = install(&store, &place.package(&hello("1.0.0"), Some(&author), true)).unwrap();
        assert!(matches!(store.commit(t, true, true), Err(PluginError::KeyChanged { .. })));
        assert_eq!(store.get(&id()).unwrap().record, p.record);
        assert_eq!(place.plugins(), holding(&p));
    }

    #[test]
    fn uninstalling_forgets_the_authors_key() {
        let place = Place::new();
        let store = place.store();
        let (author, other) = (key_pair(), key_pair());
        install(&store, &place.package(&hello("1.0.0"), Some(&author), true)).unwrap();
        store.uninstall(&id(), false).unwrap();
        // Installed again, it is a first install: another author's key pins it.
        let p = install(&store, &place.package(&hello("1.0.0"), Some(&other), true)).unwrap();
        assert_eq!(p.record.author_key.map(|k| k.base64), Some(other.pk.to_base64()));
    }

    #[test]
    fn a_package_changed_while_it_waits_is_not_installed() {
        let place = Place::new();
        let store = place.store();
        let package = place.unsigned("1.0.0");
        let t = store.stage(&package).unwrap();
        fs::write(t.staged.as_ref().unwrap().join("main.py"), b"import os\n").unwrap();
        let e = store.commit(t, true, true).unwrap_err();
        assert_eq!(e.to_string(), "The plugin's main.py doesn't match its files.lock.");
        let t = store.stage(&package).unwrap();
        fs::write(t.staged.as_ref().unwrap().join(LOCK), br#"{"lockVersion":1,"files":[]}"#).unwrap();
        let e = store.commit(t, true, true).unwrap_err();
        assert_eq!(e.to_string(), "The plugin's files changed while it waited to be installed. Open it again.");
        assert!(store.list().unwrap().is_empty());
        assert_eq!(place.plugins(), [".staging", ".trash"]);
    }

    #[test]
    fn an_icon_is_shown_while_it_is_the_listed_picture() {
        let place = Place::new();
        let store = place.store();
        let mut m = manifest(ID, "1.0.0");
        m["icon"] = json!("icon.png");
        let mut with_icon = files(&m);
        with_icon.push(("icon.png".into(), b"\x89PNG\r\n\x1a\nrest".to_vec()));
        let p = install(&store, &place.package(&with_icon, None, false)).unwrap();
        assert_eq!(store.icon_data_url(&p).as_deref(), Some("data:image/png;base64,iVBORw0KGgpyZXN0"));
        fs::write(p.dir.join("icon.png"), b"\x89PNG\r\n\x1a\nRest").unwrap();
        assert_eq!(store.icon_data_url(&p), None);

        // A loaded folder's, as it is.
        store.uninstall(&id(), false).unwrap();
        m["icon"] = json!("icon.svg");
        let mut with_icon = files(&m);
        with_icon.push(("icon.svg".into(), b"<svg/>".to_vec()));
        let dev = place.folder("hello-export", &with_icon);
        let p = store.commit(store.stage_folder(&dev).unwrap(), true, true).unwrap();
        assert_eq!(store.icon_data_url(&p).as_deref(), Some("data:image/svg+xml;base64,PHN2Zy8+"));
        fs::write(dev.join("icon.svg"), b"not a picture").unwrap();
        assert_eq!(store.icon_data_url(&p), None);
    }
}
