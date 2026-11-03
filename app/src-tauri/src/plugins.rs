//! The plugin host: the installed plugins ([`Store`]) as the app shows them
//! ([`PluginInfo`]), the packages checked and waiting for the person's yes
//! (tickets), and plugin runs as jobs ([`queue_run`]).
//!
//! A run is a job of its own (`JobKind::Plugin`) on `Class::Plugin`, or on
//! `Class::Analysis` when the plugin uses the GPU, which the engine needs
//! too. It never carries a media id: the episode lists would show it as that
//! episode's work. An export action's run carries its export's id. A plugin
//! runs only while it is active ([`active`]): turned on, not broken, and
//! reviewed by AutoScene unless the setting lets the others run. That is
//! checked when a run is queued and again when it starts, so a run queued
//! before the plugin (or the setting) went off fails instead; one running
//! or still waiting then is stopped ([`PluginHost::stop`],
//! [`PluginHost::allow_changed`]). A loaded folder is never reviewed,
//! whatever its row says. A plugin runs once at a time, and a run that
//! starts while the plugin is installed, updated or uninstalled waits for
//! that ([`PluginHost::hold`]); a change is refused while a run of it runs
//! ([`PluginHost::change`]).
//!
//! An export that finished whole queues each export action that active
//! plugins turned on for after every export ([`queue_after_export`]), off
//! the export's job: a plugin never fails or holds up an export. Queued runs
//! aren't kept anywhere: one still waiting at quit is lost.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use as_api::{ExportSummary, JobKind};
use as_core::{AppPaths, ExportId, JobId, ProjectId};
use as_jobs::{Class, JobCtx, JobError, JobResult, JobRunner, JobSpec};
use as_plugin::protocol::{ClipContext, ExportContext, ProjectContext, RunMessage};
use as_plugin::store::RECORD_VERSION;
use as_plugin::{
    ActionContext, ActionKind, CommandSpec, Installed, Keys, Launch, Manifest, Permission, PluginError, PluginId,
    PluginResult, Record, SettingSpec, Source, Store, Ticket, Trigger, Trust, package, runtime,
};
use as_project::{Library, Project, ProjectError, ProjectResult};
use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::AppState;
use crate::work::job_error;

/// How long a checked package waits for the person's yes. Its unpacked
/// files go once it has waited longer, at the next package checked.
pub const TICKET_TTL: Duration = Duration::from_secs(600);

/// How often a run waiting for its turn ([`PluginHost::hold`]) looks
/// whether it was cancelled.
const WAIT_TICK: Duration = Duration::from_millis(50);

/// The plugins of this app, and what they are doing.
pub struct PluginHost {
    pub store: Store,
    /// Packages and folders checked and waiting for the person's yes, by
    /// ticket id.
    tickets: Mutex<HashMap<String, Ticket>>,
    /// The runs [`queue_run`] queued that may not have ended yet, by job:
    /// what each runs, so the same run isn't queued twice, and a plugin
    /// turned off loses those still waiting too. Taken before the job
    /// runner's lock, never inside it (see [`PluginHost::submit`]).
    runs: Mutex<HashMap<JobId, RunKey>>,
    /// What runs, and what is being changed ([`PluginHost::hold`],
    /// [`PluginHost::change`]). Never held while the job runner is called.
    busy: Mutex<Busy>,
    /// Told whenever a run or a change of a plugin ends.
    freed: Condvar,
}

/// The plugins in use: by a run, or by an install, update or uninstall.
#[derive(Default)]
struct Busy {
    /// The plugin each running plugin job runs. Its folder stays as it is
    /// meanwhile: an update or uninstall would throw it away under the run.
    running: HashMap<JobId, PluginId>,
    /// The plugins being installed, updated or uninstalled, whose runs wait.
    changing: HashSet<PluginId>,
}

/// What a run runs, and on what: two runs alike are the same run, whatever
/// started them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RunKey {
    plugin: PluginId,
    kind: ActionKind,
    /// The action's id in the plugin's `plugin.json`.
    action: String,
    project_id: Option<ProjectId>,
    export_id: Option<ExportId>,
}

/// An installed plugin, as Settings → Plugins shows it.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// Its author's name.
    pub author: String,
    pub license: Option<String>,
    /// An `https://` link.
    pub repository: Option<String>,
    pub trust: Trust,
    /// The key its updates must be signed with, as minisign shows it.
    pub author_key_id: Option<String>,
    pub source: PluginSource,
    /// Its folder: its package's, or the one it was loaded from.
    pub folder: String,
    pub enabled: bool,
    /// Turned on, not broken, and allowed to run: its actions are offered.
    pub active: bool,
    /// Why it can't run while it is turned on (the setting keeps plugins
    /// AutoScene hasn't reviewed from running).
    pub blocked_reason: Option<String>,
    /// Why it can't run at all: its files are missing or changed, or it
    /// needs another AutoScene.
    pub broken: Option<String>,
    /// What it says it does: declared by its author, not enforced.
    pub permissions: Vec<Permission>,
    pub network_hosts: Vec<String>,
    /// It uses the GPU, so its runs wait for the analysis.
    pub gpu: bool,
    /// It is a Python script that uv runs here: on its first run, uv
    /// downloads Python (when this computer has none) and the packages its
    /// lock names.
    pub uv_script: bool,
    pub export_actions: Vec<PluginExportAction>,
    pub commands: Vec<CommandSpec>,
    pub settings: Vec<SettingSpec>,
    /// A `data:` URL of its icon (PNG or SVG), when it has a sound one.
    pub icon: Option<String>,
    /// An optional hex brand/accent color (e.g. `#2a2a2a`).
    pub color: Option<String>,
    /// Where its runs write their log.
    pub log_path: String,
    /// What was skipped or changed in its `plugin.json`.
    pub warnings: Vec<String>,
}

/// Where a plugin's files are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PluginSource {
    /// Installed from a package.
    Package,
    /// Loaded from a developer's folder, used where it is.
    Folder,
}

/// An entry a plugin adds to each export's menu.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginExportAction {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    /// It runs after every export that finishes.
    pub after_export: bool,
}

/// A package (or folder) checked for install, for the person to say yes
/// (`plugin_install`) or no (`plugin_discard`).
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginReview {
    pub ticket_id: String,
    /// The plugin as it would be once installed.
    pub plugin: PluginInfo,
    /// What changes from the installed version, when it replaces one.
    pub update: Option<PluginUpdate>,
    /// AutoScene hasn't reviewed it: it needs the setting, and the person's
    /// typed yes.
    pub needs_unreviewed: bool,
    /// It may be installed now (reviewed, or the setting is on).
    pub allowed: bool,
    /// It is a uv script, and uv isn't installed: it won't run until it is.
    pub uv_missing: bool,
}

/// What an update changes.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PluginUpdate {
    pub from_version: String,
    pub trust_before: Trust,
    /// Where the installed version's files are: a package replacing a
    /// loaded folder leaves that folder as it is.
    pub source_before: PluginSource,
    /// Permissions it says it needs that the installed version didn't.
    pub added_permissions: Vec<Permission>,
    pub added_hosts: Vec<String>,
}

/// The plugin host of this app's data folder, or none when its store
/// can't open (the plugin commands then say so).
pub fn open(app: &AppHandle, library: &Library, paths: &AppPaths) -> Option<Arc<PluginHost>> {
    let version = app.package_info().version.clone();
    match Store::open(paths, library.shared_db(), Keys::builtin(), version) {
        Ok(store) => Some(Arc::new(PluginHost::new(store))),
        Err(e) => {
            log::warn!("plugins couldn't start: {}", e.detail());
            None
        }
    }
}

/// The plugin host, for a command.
pub fn host(state: &AppState) -> AppResult<Arc<PluginHost>> {
    state
        .plugins
        .clone()
        .ok_or_else(|| AppError::new(ErrorCode::NotReady, "Plugins couldn't start. Details are in the log."))
}

impl PluginHost {
    pub fn new(store: Store) -> Self {
        Self { store, tickets: Mutex::default(), runs: Mutex::default(), busy: Mutex::default(), freed: Condvar::new() }
    }

    /// Keeps `t` for [`PluginHost::take`] under its id, and drops the
    /// tickets that waited longer than [`TICKET_TTL`], with their files.
    pub fn keep(&self, t: Ticket) {
        let expired: Vec<Ticket> = {
            let mut tickets = self.tickets.lock();
            let old: Vec<String> =
                tickets.iter().filter(|(_, t)| t.created.elapsed() > TICKET_TTL).map(|(id, _)| id.clone()).collect();
            let expired = old.iter().filter_map(|id| tickets.remove(id)).collect();
            tickets.insert(t.id.to_string(), t);
            expired
        };
        for t in expired {
            self.store.discard(t);
        }
    }

    /// The ticket `id`, unless it was taken already (or its review was
    /// closed), or it waited longer than [`TICKET_TTL`] (it is dropped then).
    pub fn take(&self, id: &str) -> AppResult<Ticket> {
        let t = self
            .tickets
            .lock()
            .remove(id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "This review was closed. Open the plugin again."))?;
        if t.created.elapsed() > TICKET_TTL {
            self.store.discard(t);
            return Err(AppError::new(
                ErrorCode::NotFound,
                "This plugin waited too long to be installed. Open it again.",
            ));
        }
        Ok(t)
    }

    /// The developers' folders plugins are loaded from.
    pub fn loaded_folders(&self) -> Vec<PathBuf> {
        let plugins = self.store.list().unwrap_or_default();
        plugins.into_iter().filter(|p| source(&p.record) == PluginSource::Folder).map(|p| p.dir).collect()
    }

    /// Whether a run of `id` is running.
    #[cfg(test)]
    fn is_running(&self, id: &PluginId) -> bool {
        self.busy.lock().running.values().any(|p| p == id)
    }

    /// Marks `id` (named `name`) as being installed, updated or uninstalled
    /// until the guard drops: a run of it that starts meanwhile waits, and
    /// then finds the plugin as the change left it. Refused while a run of
    /// it runs, from the folder the change would throw away. Looking and
    /// marking are one step, so no run starts between them. Another change
    /// of it goes first.
    pub fn change(&self, id: &PluginId, name: &str) -> PluginResult<Changing<'_>> {
        let mut busy = self.busy.lock();
        while busy.changing.contains(id) {
            self.freed.wait(&mut busy);
        }
        if busy.running.values().any(|p| p == id) {
            return Err(PluginError::Busy(name.to_owned()));
        }
        busy.changing.insert(id.clone());
        Ok(Changing { host: self, id: id.clone() })
    }

    /// Stops the runs of `id`, running or still waiting (it was turned off,
    /// or uninstalled).
    pub fn stop(&self, jobs: &JobRunner, id: &PluginId) {
        let stop: Vec<JobId> = self.runs().into_iter().filter(|(_, p)| p == id).map(|(job, _)| job).collect();
        if !stop.is_empty() {
            jobs.cancel_where(|j| stop.contains(&j.id));
        }
    }

    /// The setting that lets plugins AutoScene hasn't reviewed run went
    /// from `before` to `after`: when it went off, their runs stop, running
    /// or still waiting.
    pub fn allow_changed(&self, jobs: &JobRunner, before: bool, after: bool) {
        if !before || after {
            return;
        }
        let stop = unreviewed_runs(&self.runs(), |id| self.store.get(id).is_ok_and(|p| reviewed(&p)));
        if !stop.is_empty() {
            jobs.cancel_where(|j| stop.contains(&j.id));
        }
    }

    /// The plugin jobs and their plugins, queued or running, as they are
    /// now: what is done with them happens after the locks are let go,
    /// since a job ending takes one too. A job may be listed twice, and one
    /// that ended meanwhile too.
    fn runs(&self) -> Vec<(JobId, PluginId)> {
        let mut runs: Vec<_> = self.runs.lock().iter().map(|(job, key)| (*job, key.plugin.clone())).collect();
        runs.extend(self.busy.lock().running.iter().map(|(job, id)| (*job, id.clone())));
        runs
    }

    /// Queues `work` as the run `key`, unless the same run waits or runs
    /// already (`None`, and `work` is dropped). Two callers can't both find
    /// none: the second waits for the first to note its job. The runs it
    /// compares with are copied before the job runner is called, so the
    /// runner's lock never waits for this one.
    fn submit(
        &self,
        jobs: &JobRunner,
        key: RunKey,
        spec: JobSpec,
        work: impl FnOnce(&JobCtx) -> JobResult + Send + 'static,
    ) -> Option<JobId> {
        let mut runs = self.runs.lock();
        let live: HashSet<JobId> = jobs.jobs().into_iter().filter(|j| !j.state.is_finished()).map(|j| j.id).collect();
        runs.retain(|job, _| live.contains(job));
        let same: Vec<JobId> = runs.iter().filter(|(_, k)| **k == key).map(|(job, _)| *job).collect();
        let job = jobs.submit_unless(|j| same.contains(&j.id), spec, work)?;
        runs.insert(job, key);
        Some(job)
    }

    /// Holds the job of `ctx` as the run of `id` until the guard drops, once
    /// no other run of `id` runs and no change of it is on its way: a
    /// plugin runs once at a time (a loaded folder that moved to or from the
    /// GPU runs on the other class), and never from files an update or an
    /// uninstall is replacing. A job cancelled while it waits ends so.
    fn hold(&self, ctx: &JobCtx, id: &PluginId) -> Result<Running<'_>, JobError> {
        let mut said = false;
        let mut busy = self.busy.lock();
        loop {
            let changing = busy.changing.contains(id);
            if !changing && !busy.running.values().any(|p| p == id) {
                busy.running.insert(ctx.id, id.clone());
                return Ok(Running { host: self, job: ctx.id });
            }
            if ctx.cancel.is_cancelled() {
                return Err(JobError::Cancelled);
            }
            if !said {
                // The job runner is told without the lock.
                drop(busy);
                ctx.stage(if changing { "Waiting while it is updated" } else { "Waiting for its other run" });
                said = true;
                busy = self.busy.lock();
                continue;
            }
            self.freed.wait_for(&mut busy, WAIT_TICK);
        }
    }
}

/// A run's place in [`Busy::running`].
struct Running<'a> {
    host: &'a PluginHost,
    job: JobId,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.host.busy.lock().running.remove(&self.job);
        self.host.freed.notify_all();
    }
}

/// A plugin's place in [`Busy::changing`] (see [`PluginHost::change`]).
pub struct Changing<'a> {
    host: &'a PluginHost,
    id: PluginId,
}

impl Drop for Changing<'_> {
    fn drop(&mut self) {
        self.host.busy.lock().changing.remove(&self.id);
        self.host.freed.notify_all();
    }
}

/// Whether `p` may run now: turned on, not broken, and reviewed by
/// AutoScene unless `allow_unreviewed` (the setting) lets the others run.
pub fn active(p: &Installed, allow_unreviewed: bool) -> bool {
    runnable(p, allow_unreviewed).is_ok()
}

/// The manifest of `p` while it may run (see [`active`]), else why it may
/// not.
fn runnable(p: &Installed, allow_unreviewed: bool) -> PluginResult<&Manifest> {
    let m = match (&p.manifest, &p.broken) {
        (Some(m), None) => m,
        (_, Some(broken)) => return Err(PluginError::Invalid(broken.clone())),
        (None, None) => return Err(PluginError::Invalid(as_plugin::store::MISSING.into())),
    };
    if !p.record.enabled {
        return Err(PluginError::Invalid(format!("{} is turned off.", m.name)));
    }
    blocked(p, allow_unreviewed).map_or(Ok(m), Err)
}

/// Why `p` may not run while it is turned on: AutoScene hasn't reviewed it,
/// and the setting keeps such plugins from running.
fn blocked(p: &Installed, allow_unreviewed: bool) -> Option<PluginError> {
    (!reviewed(p) && !allow_unreviewed).then_some(PluginError::NotAllowed)
}

/// Whether AutoScene reviewed `p` (see [`trust`]).
fn reviewed(p: &Installed) -> bool {
    trust(&p.record) == Trust::Reviewed
}

/// How far the plugin of `r` is trusted: as its row says, but a plugin
/// loaded from a folder is never reviewed or signed, whatever its row says
/// (another program could have written it).
fn trust(r: &Record) -> Trust {
    match r.source {
        Source::Package { .. } => r.trust,
        Source::Folder { .. } => Trust::Unreviewed,
    }
}

/// Where the files of the plugin of `r` are.
fn source(r: &Record) -> PluginSource {
    match r.source {
        Source::Package { .. } => PluginSource::Package,
        Source::Folder { .. } => PluginSource::Folder,
    }
}

/// The name of `p`, or its id when it is broken.
fn name(p: &Installed) -> &str {
    p.manifest.as_ref().map_or(p.record.id.as_str(), |m| m.name.as_str())
}

/// `p` as the app shows it, with `allow_unreviewed` the setting.
pub fn info(p: &Installed, store: &Store, allow_unreviewed: bool) -> PluginInfo {
    let m = p.manifest.as_ref();
    let id = &p.record.id;
    let actions = m.map_or(&[][..], |m| &m.export_actions);
    PluginInfo {
        id: id.to_string(),
        name: name(p).to_owned(),
        version: p.record.version.clone(),
        description: m.map(|m| m.description.clone()).unwrap_or_default(),
        author: m.map(|m| m.author.name.clone()).unwrap_or_default(),
        license: m.and_then(|m| m.license.clone()),
        repository: m.and_then(|m| m.repository.clone()),
        trust: trust(&p.record),
        author_key_id: p.record.author_key.as_ref().map(|k| k.key_id.clone()),
        source: source(&p.record),
        folder: p.dir.to_string_lossy().into_owned(),
        enabled: p.record.enabled,
        active: active(p, allow_unreviewed),
        blocked_reason: blocked(p, allow_unreviewed).filter(|_| p.broken.is_none()).map(|e| e.to_string()),
        broken: p.broken.clone(),
        // What its plugin.json says now: a loaded folder's may have changed.
        permissions: m.map_or_else(|| p.record.permissions.clone(), |m| m.permissions.clone()),
        network_hosts: m.map_or_else(|| p.record.network_hosts.clone(), |m| m.network_hosts.clone()),
        gpu: m.is_some_and(|m| m.runtime.gpu),
        uv_script: m.is_some_and(|m| matches!(m.launch(), Ok(Launch::UvScript { .. }))),
        export_actions: actions
            .iter()
            .map(|a| PluginExportAction {
                id: a.id.clone(),
                title: a.title.clone(),
                description: a.description.clone(),
                after_export: p.record.after_export.contains(&a.id),
            })
            .collect(),
        commands: m.map(|m| m.commands.clone()).unwrap_or_default(),
        settings: m.map(|m| m.settings.clone()).unwrap_or_default(),
        icon: store.icon_data_url(p),
        color: m.and_then(|m| m.color.clone()),
        log_path: store.log_path(id).to_string_lossy().into_owned(),
        warnings: p.warnings.clone(),
    }
}

/// `t` for the review dialog, with `allow_unreviewed` the setting.
pub fn review(t: &Ticket, store: &Store, allow_unreviewed: bool) -> PluginReview {
    let needs_unreviewed = t.verdict.trust != Trust::Reviewed;
    let plugin = info(&as_installed(t), store, allow_unreviewed);
    PluginReview {
        ticket_id: t.id.to_string(),
        update: t.previous.as_ref().map(|previous| update(previous, &t.manifest)),
        needs_unreviewed,
        allowed: !needs_unreviewed || allow_unreviewed,
        uv_missing: plugin.uv_script && runtime::find_uv().is_none(),
        plugin,
    }
}

/// The plugin of `t` as it would be once installed, in the folder it was
/// checked in (as [`Store::commit`] keeps what the version before had).
fn as_installed(t: &Ticket) -> Installed {
    let m = &t.manifest;
    let previous = t.previous.as_ref();
    let after_export = previous.map_or_else(Vec::new, |p| {
        p.after_export.iter().filter(|a| m.export_actions.iter().any(|e| e.id == **a)).cloned().collect()
    });
    let dir = match (&t.staged, &t.source) {
        (Some(staged), _) => staged.clone(),
        (None, Source::Folder { path }) => PathBuf::from(path),
        (None, Source::Package { .. }) => PathBuf::new(),
    };
    Installed {
        record: Record {
            v: RECORD_VERSION,
            id: m.id.clone(),
            version: m.version.clone(),
            source: t.source.clone(),
            trust: t.verdict.trust,
            author_key: t.verdict.author_key.clone(),
            permissions: m.permissions.clone(),
            network_hosts: m.network_hosts.clone(),
            enabled: previous.is_none_or(|p| p.enabled),
            after_export,
            lock_sha256: t.lock.is_some().then(|| package::sha256_hex(&t.lock_bytes)),
            installed_at: 0,
        },
        manifest: Some(m.clone()),
        warnings: t.warnings.clone(),
        dir,
        broken: None,
    }
}

/// What the plugin of manifest `m` changes from the installed `previous`.
fn update(previous: &Record, m: &Manifest) -> PluginUpdate {
    PluginUpdate {
        from_version: previous.version.clone(),
        trust_before: trust(previous),
        source_before: source(previous),
        added_permissions: m.permissions.iter().filter(|p| !previous.permissions.contains(p)).copied().collect(),
        added_hosts: m.network_hosts.iter().filter(|h| !previous.network_hosts.contains(h)).cloned().collect(),
    }
}

/// Queues a run of `action` (an action of `kind`) of the plugin `id`,
/// started by `trigger`: an export action runs on the export `export_id`
/// of the project `project_id`, a command in the project when it needs
/// one. None when the same run waits or runs already. A plugin that isn't
/// active is refused.
pub fn queue_run(
    app: &AppHandle,
    id: &PluginId,
    kind: ActionKind,
    action: &str,
    trigger: Trigger,
    project_id: Option<ProjectId>,
    export_id: Option<ExportId>,
) -> AppResult<Option<JobId>> {
    let state = app.state::<AppState>();
    let host = host(&state)?;
    let p = host.store.get(id)?;
    let m = runnable(&p, state.library.allows_unreviewed_plugins()?)?;
    let title = action_title(m, kind, action, project_id, export_id)?;
    let spec = job_spec(m, &title, project_id, export_id);
    let key = RunKey { plugin: id.clone(), kind, action: action.to_owned(), project_id, export_id };
    let work = {
        let (app, host, key, gpu) = (app.clone(), host.clone(), key.clone(), m.runtime.gpu);
        move |ctx: &JobCtx| run(&app, &host, ctx, &key, trigger, gpu)
    };
    Ok(host.submit(&state.jobs, key, spec, work))
}

/// The title of the action `action` of `kind` in `m`, when it may run with
/// what it is given: an export action needs an export (with its project),
/// a command never runs on one, and one that needs a project needs it.
fn action_title(
    m: &Manifest,
    kind: ActionKind,
    action: &str,
    project_id: Option<ProjectId>,
    export_id: Option<ExportId>,
) -> PluginResult<String> {
    let refused = |why: &str| Err(PluginError::Invalid(why.into()));
    let no_such = || PluginError::Invalid(format!("{} has no such action.", m.name));
    match kind {
        ActionKind::ExportAction => {
            let a = m.export_actions.iter().find(|a| a.id == action).ok_or_else(no_such)?;
            if export_id.is_none() || project_id.is_none() {
                return refused("Choose an export to run it on.");
            }
            Ok(a.title.clone())
        }
        ActionKind::Command => {
            let c = m.commands.iter().find(|c| c.id == action).ok_or_else(no_such)?;
            if export_id.is_some() {
                return refused("A command doesn't run on an export.");
            }
            if c.needs_project && project_id.is_none() {
                return refused("Open a project to run it in.");
            }
            Ok(c.title.clone())
        }
    }
}

/// The job of a run titled `title` of the plugin of `m`: a plugin job, on
/// the analysis' class when the plugin uses the GPU, in its project and on
/// its export. Never of a media: the episode lists would show it as that
/// episode's work.
fn job_spec(m: &Manifest, title: &str, project_id: Option<ProjectId>, export_id: Option<ExportId>) -> JobSpec {
    JobSpec {
        kind: JobKind::Plugin,
        class: if m.runtime.gpu { Class::Analysis } else { Class::Plugin },
        // Cleaned and cut when its plugin.json was read, as its name is.
        title: title.to_owned(),
        subtitle: Some(m.name.clone()),
        project_id,
        media_id: None,
        export_id,
    }
}

/// The queued run `key`, started by `trigger`, now it starts; `gpu` is
/// whether it waited as analysis.
fn run(app: &AppHandle, host: &PluginHost, ctx: &JobCtx, key: &RunKey, trigger: Trigger, gpu: bool) -> JobResult {
    // Held before it looks whether it may run: a change to the plugin or
    // the setting from here on finds it and stops it.
    let _running = host.hold(ctx, &key.plugin)?;
    let state = app.state::<AppState>();
    let p = startable(&host.store, &state.library, key, gpu)?;
    let tools = state.tools().map_err(|e| JobError::Failed(e.message))?;
    let action = ActionContext { kind: key.kind, id: key.action.clone(), trigger };
    let mut spec =
        host.store.run_spec(&p, action, &tools.ffmpeg, &tools.ffprobe, &tools.version).map_err(plugin_job_error)?;
    add_place(&mut spec.message, &state.library, key).map_err(job_error)?;
    ctx.stage(&format!("Starting {}", spec.name));
    if let Some(message) =
        runtime::run(spec, &ctx.cancel, &|stage, f| ctx.report(stage, f)).map_err(plugin_job_error)?
    {
        ctx.message(&message);
    }
    Ok(())
}

/// The plugin of the queued run `key` now it starts, unless it may no
/// longer run (see [`active`]) or no longer runs `key` as it was queued: an
/// update, or a loaded folder's changed `plugin.json`, may have taken the
/// action away, changed what it needs, or moved the plugin to or from the
/// GPU (`gpu`: it waited as analysis).
fn startable(store: &Store, lib: &Library, key: &RunKey, gpu: bool) -> Result<Installed, JobError> {
    let refused = |e: PluginError| JobError::Failed(e.to_string());
    let p = store.get(&key.plugin).map_err(plugin_job_error)?;
    let allow = lib.allows_unreviewed_plugins().map_err(job_error)?;
    let m = runnable(&p, allow).map_err(refused)?;
    action_title(m, key.kind, &key.action, key.project_id, key.export_id).map_err(refused)?;
    if m.runtime.gpu != gpu {
        return Err(JobError::Failed(format!("{} changed since this run was queued. Run it again.", m.name)));
    }
    Ok(p)
}

/// Gives `message` the project the run `key` runs in and the export it
/// runs on, as they are now.
fn add_place(message: &mut RunMessage, lib: &Library, key: &RunKey) -> ProjectResult<()> {
    let Some(project_id) = key.project_id else { return Ok(()) };
    let project = lib.project(project_id)?;
    message.project = Some(project_context(&project)?);
    if let Some(export_id) = key.export_id {
        message.export = Some(export_context(&project, &project.export(export_id)?)?);
    }
    Ok(())
}

/// `e` as its job ends with it. The plugin's own error line is its answer,
/// given before any cancel (else the run says Cancelled): it stays the
/// job's error when a cancel comes after it.
fn plugin_job_error(e: PluginError) -> JobError {
    match e {
        PluginError::Cancelled => JobError::Cancelled,
        e @ PluginError::Failed { .. } => {
            log::warn!("{}", e.detail());
            JobError::FailedFirst(e.to_string())
        }
        e => {
            log::warn!("{}", e.detail());
            JobError::Failed(e.to_string())
        }
    }
}

fn project_context(project: &Project) -> ProjectResult<ProjectContext> {
    Ok(ProjectContext {
        id: project.id.to_string(),
        title: project.title()?,
        folder: project.dir.to_string_lossy().into_owned(),
    })
}

/// `export` as a run gets it: each clip with the episode it shows, as the
/// project has it now. An episode removed since has no file or name.
fn export_context(project: &Project, export: &ExportSummary) -> ProjectResult<ExportContext> {
    let mut episodes = HashMap::new();
    let mut clips = Vec::with_capacity(export.items.len());
    for item in &export.items {
        let media = match episodes.entry(item.media_id) {
            Entry::Occupied(known) => known.into_mut(),
            Entry::Vacant(new) => new.insert(match project.media(item.media_id) {
                Ok(m) => Some(m),
                Err(ProjectError::NotFound(_)) => None,
                Err(e) => return Err(e),
            }),
        };
        let media = media.as_ref();
        clips.push(ClipContext {
            name: item.name.clone(),
            path: item.path.clone(),
            status: serde_name(&item.status),
            media_id: item.media_id.to_string(),
            source_path: media.map(|m| m.path.clone()).unwrap_or_default(),
            episode: media
                .map(|m| if m.label.is_empty() { m.file_name.clone() } else { m.label.clone() })
                .unwrap_or_default(),
            start: item.start.into(),
            end: item.end.into(),
            fps: media.and_then(|m| m.fps).map(|fps| fps.to_string()),
        });
    }
    Ok(ExportContext {
        id: export.id.to_string(),
        label: export.label.clone(),
        preset: export.preset.id.clone(),
        out_dir: export.out_dir.clone(),
        created_at: export.created_at,
        merged_file: export.merged.as_ref().and_then(|f| f.path.clone()),
        clips,
    })
}

/// The name `value` (a unit enum) has in JSON: what the UI calls it too.
fn serde_name(value: &impl Serialize) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Queues, for the export `summary` of `project` that finished whole, the
/// export actions that active plugins turned on for after every export.
/// Off the export's job, which it never fails or holds up: what goes wrong
/// here is only logged.
pub fn queue_after_export(app: &AppHandle, project: &Project, summary: &ExportSummary) {
    let (app, project_id, export_id) = (app.clone(), project.id, summary.id);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let Some(host) = state.plugins.clone() else { return };
        let plugins = host.store.list().map_err(|e| e.detail());
        let allow = state.library.allows_unreviewed_plugins().map_err(|e| e.detail());
        let (plugins, allow) = match (plugins, allow) {
            (Ok(plugins), Ok(allow)) => (plugins, allow),
            (Err(e), _) | (_, Err(e)) => {
                log::warn!("plugins after export {export_id}: {e}");
                return;
            }
        };
        for (id, action) in after_export_runs(&plugins, allow) {
            let queued = queue_run(
                &app,
                id,
                ActionKind::ExportAction,
                action,
                Trigger::AfterExport,
                Some(project_id),
                Some(export_id),
            );
            match queued {
                Ok(Some(_)) => {}
                Ok(None) => log::info!("{id} {action} after export {export_id}: waits or runs already"),
                Err(e) => log::warn!("running {id} after export {export_id}: {}", e.message),
            }
        }
    });
}

/// The export actions to run after an export: of each active plugin, those
/// it turned on for after every export, in the order its plugin.json lists
/// them.
fn after_export_runs(plugins: &[Installed], allow_unreviewed: bool) -> Vec<(&PluginId, &str)> {
    plugins
        .iter()
        .filter(|p| active(p, allow_unreviewed))
        .flat_map(|p| {
            let actions = p.manifest.as_ref().map_or(&[][..], |m| &m.export_actions);
            actions.iter().filter(|a| p.record.after_export.contains(&a.id)).map(|a| (&p.record.id, a.id.as_str()))
        })
        .collect()
}

/// Of the plugin jobs `runs`, those to stop once plugins AutoScene hasn't
/// reviewed may no longer run: each whose plugin isn't `reviewed`, also
/// when it can't be read now.
fn unreviewed_runs(runs: &[(JobId, PluginId)], reviewed: impl Fn(&PluginId) -> bool) -> Vec<JobId> {
    runs.iter().filter(|(_, id)| !reviewed(id)).map(|(job, _)| *job).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    use as_api::{AppSettings, ContentKind, ExportItem, ExportStatus, JobState, NewProject, ProjectKind};

    use super::*;

    const UV: [&str; 5] = ["uv", "run", "--script", "--locked", "main.py"];

    /// The `plugin.json` of a plugin `id` with the export actions `post` and
    /// `tag`.
    fn manifest(id: &str, gpu: bool) -> serde_json::Value {
        serde_json::json!({
            "manifestVersion": 1,
            "id": id,
            "version": "1.0.0",
            "name": format!("Plugin {id}"),
            "description": "Does a thing.",
            "author": {"name": "Someone"},
            "engines": {"autoscene": ">=1.0.0, <2", "protocol": 1},
            "runtime": {"type": "process", "command": {"*": UV}, "gpu": gpu},
            "contributes": {
                "exportActions": [{"id": "post", "title": "Post"}, {"id": "tag", "title": "Tag"}],
                "commands": [
                    {"id": "hello", "title": "Say hello"},
                    {"id": "count", "title": "Count clips", "needsProject": true}
                ]
            }
        })
    }

    /// An installed plugin `id` of `trust` (see [`manifest`]).
    fn plugin(id: &str, trust: Trust, gpu: bool) -> Installed {
        let m = as_plugin::manifest::parse_for_any_app(manifest(id, gpu).to_string().as_bytes()).unwrap().manifest;
        Installed {
            record: Record {
                v: RECORD_VERSION,
                id: m.id.clone(),
                version: m.version.clone(),
                source: Source::Package { folder: "1.0.0+0123abcd".into() },
                trust,
                author_key: None,
                permissions: Vec::new(),
                network_hosts: Vec::new(),
                enabled: true,
                after_export: Vec::new(),
                lock_sha256: Some("0".repeat(64)),
                installed_at: 0,
            },
            manifest: Some(m),
            warnings: Vec::new(),
            dir: PathBuf::new(),
            broken: None,
        }
    }

    fn after(mut p: Installed, actions: &[&str]) -> Installed {
        p.record.after_export = actions.iter().map(|a| a.to_string()).collect();
        p
    }

    /// `p` as if loaded from a folder.
    fn loaded(mut p: Installed) -> Installed {
        p.record.source = Source::Folder { path: "/plugins/dev".into() };
        p.record.lock_sha256 = None;
        p
    }

    /// A plugin host of a library of its own in `tmp`.
    fn host_in(tmp: &Path) -> (Library, Arc<PluginHost>) {
        let paths = AppPaths::at(tmp.join("data"));
        let lib = Library::open(&paths).unwrap();
        let root = tmp.join("library").display().to_string();
        lib.update_settings(&AppSettings { library_root: root, ..lib.settings().unwrap() }).unwrap();
        let app = semver::Version::parse("1.0.3-BETA").unwrap();
        let store = Store::open(&paths, lib.shared_db(), Keys::new(&[]).unwrap(), app).unwrap();
        (lib, Arc::new(PluginHost::new(store)))
    }

    /// Writes the plugin of `manifest` into its folder in `tmp`, and checks
    /// it for loading.
    fn folder(host: &PluginHost, tmp: &Path, manifest: &serde_json::Value) -> Ticket {
        let dir = tmp.join(manifest["id"].as_str().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        for file in ["main.py", "main.py.lock"] {
            std::fs::write(dir.join(file), b"").unwrap();
        }
        std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
        host.store.stage_folder(&dir).unwrap()
    }

    /// The run of `action` of `kind` of the plugin `id`, in `project` and on
    /// `export`.
    fn key(
        id: &PluginId,
        kind: ActionKind,
        action: &str,
        project: Option<ProjectId>,
        export: Option<ExportId>,
    ) -> RunKey {
        RunKey { plugin: id.clone(), kind, action: action.into(), project_id: project, export_id: export }
    }

    #[test]
    fn after_export_runs_only_turned_on_actions_of_active_plugins() {
        let reviewed = after(plugin("dev.a.reviewed", Trust::Reviewed, false), &["tag", "post", "gone"]);
        let pinned = after(plugin("dev.a.pinned", Trust::Pinned, false), &["post"]);
        let unreviewed = after(plugin("dev.a.unreviewed", Trust::Unreviewed, false), &["tag"]);
        let mut off = after(plugin("dev.a.off", Trust::Reviewed, false), &["post"]);
        off.record.enabled = false;
        let mut broken = after(plugin("dev.a.broken", Trust::Reviewed, false), &["post"]);
        broken.broken = Some("Its files are missing. Install it again.".into());
        let none = plugin("dev.a.none", Trust::Reviewed, false);
        let plugins = [reviewed, pinned, unreviewed, off, broken, none];
        let runs = |allow| {
            after_export_runs(&plugins, allow).into_iter().map(|(id, a)| format!("{id} {a}")).collect::<Vec<_>>()
        };

        // In the order the plugin lists them; an action it no longer has is left out.
        assert_eq!(runs(false), ["dev.a.reviewed post", "dev.a.reviewed tag"]);
        // With the setting on, those AutoScene hasn't reviewed run too; never one turned off or broken.
        assert_eq!(
            runs(true),
            ["dev.a.reviewed post", "dev.a.reviewed tag", "dev.a.pinned post", "dev.a.unreviewed tag"]
        );
    }

    #[test]
    fn unreviewed_plugins_stop_when_the_setting_goes_off() {
        let plugins = [
            plugin("dev.a.reviewed", Trust::Reviewed, false),
            plugin("dev.a.pinned", Trust::Pinned, false),
            plugin("dev.a.unreviewed", Trust::Unreviewed, false),
            // Its row says reviewed, but a folder never is.
            loaded(plugin("dev.a.folder", Trust::Reviewed, false)),
        ];
        let id = |s: &str| PluginId::parse(s).unwrap();
        let jobs: Vec<JobId> = (0..5).map(|_| JobId::new()).collect();
        let runs = [
            (jobs[0], id("dev.a.reviewed")),
            (jobs[1], id("dev.a.pinned")),
            (jobs[2], id("dev.a.unreviewed")),
            (jobs[3], id("dev.a.folder")),
            (jobs[4], id("dev.a.gone")),
        ];
        let known = |p: &PluginId| plugins.iter().find(|i| i.record.id == *p).is_some_and(reviewed);
        // Signed by its author is still not reviewed; one that can't be read now stops too.
        assert_eq!(unreviewed_runs(&runs, known), &jobs[1..]);

        // And none of them runs again: each is refused, and says why.
        let allow = false;
        assert!(active(&plugins[0], allow));
        for p in &plugins[1..] {
            assert!(!active(p, allow) && active(p, true));
            assert_eq!(
                runnable(p, allow).unwrap_err().to_string(),
                "Plugins that AutoScene has not reviewed are turned off in Settings."
            );
        }
        // Settings shows the folder as what it is.
        assert_eq!(trust(&plugins[3].record), Trust::Unreviewed);
    }

    #[test]
    fn plugin_jobs_never_carry_a_media_id() {
        let (project, export) = (Some(ProjectId::new()), Some(ExportId::new()));
        let p = plugin("dev.a.post", Trust::Reviewed, false);
        let m = p.manifest.as_ref().unwrap();
        let title = action_title(m, ActionKind::ExportAction, "post", project, export).unwrap();
        let spec = job_spec(m, &title, project, export);
        assert_eq!((spec.kind, spec.class, spec.media_id), (JobKind::Plugin, Class::Plugin, None));
        assert_eq!((spec.title.as_str(), spec.subtitle.as_deref()), ("Post", Some("Plugin dev.a.post")));
        assert_eq!((spec.project_id, spec.export_id), (project, export));
        // One that uses the GPU waits its turn with the analysis.
        let gpu = plugin("dev.a.gpu", Trust::Reviewed, true);
        assert_eq!(job_spec(gpu.manifest.as_ref().unwrap(), "Post", project, export).class, Class::Analysis);
        // A palette command runs without a project, unless it needs one.
        let spec = job_spec(m, &action_title(m, ActionKind::Command, "hello", None, None).unwrap(), None, None);
        assert_eq!((spec.project_id, spec.media_id, spec.export_id), (None, None, None));

        let refused = |kind, action, project, export| action_title(m, kind, action, project, export).unwrap_err();
        let said = |e: PluginError| e.to_string();
        assert_eq!(said(refused(ActionKind::Command, "count", None, None)), "Open a project to run it in.");
        assert_eq!(said(refused(ActionKind::ExportAction, "post", project, None)), "Choose an export to run it on.");
        assert_eq!(said(refused(ActionKind::ExportAction, "post", None, export)), "Choose an export to run it on.");
        assert_eq!(said(refused(ActionKind::Command, "hello", project, export)), "A command doesn't run on an export.");
        assert_eq!(said(refused(ActionKind::Command, "post", None, None)), "Plugin dev.a.post has no such action.");
    }

    #[test]
    fn a_plugins_own_error_stays_its_error_after_a_cancel() {
        let failed = PluginError::Failed { message: "No webhook is set.".into(), detail: None };
        assert!(matches!(plugin_job_error(failed), JobError::FailedFirst(m) if m == "No webhook is set."));
        // What the host says of a run a cancel may have caused gives way to the cancel.
        let crashed = PluginError::Crashed { name: "Poster".into(), detail: "exit code: 1".into() };
        assert!(matches!(plugin_job_error(crashed), JobError::Failed(m) if m == "Poster stopped unexpectedly."));
        assert!(matches!(plugin_job_error(PluginError::Cancelled), JobError::Cancelled));
    }

    #[test]
    fn an_update_shows_what_it_adds() {
        let mut previous = plugin("dev.a.post", Trust::Pinned, false).record;
        previous.permissions = vec![Permission::ExportsRead];
        previous.network_hosts = vec!["discord.com".into()];
        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "id": "dev.a.post",
            "version": "1.1.0",
            "name": "Poster",
            "description": "Posts clips.",
            "author": {"name": "Someone"},
            "engines": {"autoscene": ">=1.0.0", "protocol": 1},
            "runtime": {"type": "process", "command": {"*": UV}},
            "permissions": ["exports:read", "network"],
            "network": {"hosts": ["discord.com", "example.com"]}
        });
        let m = as_plugin::manifest::parse_for_any_app(manifest.to_string().as_bytes()).unwrap().manifest;
        let u = update(&previous, &m);
        assert_eq!(
            (u.from_version.as_str(), u.trust_before, u.source_before),
            ("1.0.0", Trust::Pinned, PluginSource::Package)
        );
        assert_eq!(u.added_permissions, [Permission::Network]);
        assert_eq!(u.added_hosts, ["example.com"]);
        // A package replacing a loaded folder says so: that folder was never reviewed.
        let folder = loaded(plugin("dev.a.post", Trust::Reviewed, false)).record;
        let u = update(&folder, &m);
        assert_eq!((u.trust_before, u.source_before), (Trust::Unreviewed, PluginSource::Folder));
    }

    #[test]
    fn turning_a_plugin_or_the_setting_off_stops_its_runs() {
        let tmp = std::env::temp_dir().join(format!("autoscene-plugin-host-{}", std::process::id()));
        let (lib, host) = host_in(&tmp);
        // Two plugins loaded from their folders, so neither is reviewed.
        let ids = ["dev.a.one", "dev.a.two"].map(|id| {
            let t = folder(&host, &tmp, &manifest(id, false));
            let shown = review(&t, &host.store, false);
            assert!(shown.needs_unreviewed && !shown.allowed && shown.update.is_none());
            // A uv script: the review says what uv fetches, and whether uv is there.
            assert!(shown.plugin.uv_script);
            assert_eq!(shown.uv_missing, runtime::find_uv().is_none());
            assert_eq!(
                (shown.plugin.source, shown.plugin.folder.as_str()),
                (PluginSource::Folder, tmp.join(id).to_str().unwrap())
            );
            assert_eq!(shown.plugin.blocked_reason.as_deref(), Some(PluginError::NotAllowed.to_string().as_str()));
            host.store.commit(t, true, true).unwrap().record.id
        });
        // Their folders may be shown.
        assert_eq!(host.loaded_folders(), ids.each_ref().map(|id| tmp.join(id.as_str())));

        let jobs = JobRunner::new(&[(Class::Plugin, 3)], |_| {});
        let (tx, started) = mpsc::channel();
        let run = |id: &str| {
            let (host, tx, id) = (host.clone(), tx.clone(), PluginId::parse(id).unwrap());
            let p = plugin(id.as_str(), Trust::Unreviewed, false);
            jobs.submit(job_spec(p.manifest.as_ref().unwrap(), "Post", None, None), move |ctx| {
                let _running = host.hold(ctx, &id)?;
                tx.send(()).unwrap();
                while !ctx.cancel.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(JobError::Cancelled)
            })
        };
        // The third plugin was uninstalled meanwhile.
        let runs = [run("dev.a.one"), run("dev.a.two"), run("dev.a.gone")];
        for _ in runs {
            started.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        let states = || runs.map(|run| jobs.jobs().into_iter().find(|j| j.id == run).unwrap().state);
        let wait_for = |want: [JobState; 3]| {
            let start = Instant::now();
            while states() != want {
                assert!(start.elapsed() < Duration::from_secs(10), "{:?} never became {want:?}", states());
                thread::sleep(Duration::from_millis(5));
            }
        };
        assert!(host.is_running(&ids[0]) && host.is_running(&ids[1]));

        // Turned off, its run stops, and only its.
        host.store.set_enabled(&ids[0], false).unwrap();
        host.stop(&jobs, &ids[0]);
        wait_for([JobState::Cancelled, JobState::Running, JobState::Running]);
        // The setting off: every plugin AutoScene hasn't reviewed stops, and
        // one that can't be read now with them.
        host.allow_changed(&jobs, true, false);
        wait_for([JobState::Cancelled; 3]);
        let start = Instant::now();
        while host.is_running(&ids[1]) {
            assert!(start.elapsed() < Duration::from_secs(10), "a stopped run stays in running");
            thread::sleep(Duration::from_millis(5));
        }
        drop((host, lib));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_plugin_runs_once_at_a_time_and_never_while_it_changes() {
        let tmp = std::env::temp_dir().join(format!("autoscene-plugin-busy-{}", std::process::id()));
        let (lib, host) = host_in(&tmp);
        let id = PluginId::parse("dev.a.one").unwrap();
        // A loaded folder's plugin.json moved it to the GPU while a run of it ran.
        let jobs = JobRunner::new(&[(Class::Plugin, 1), (Class::Analysis, 1)], |_| {});
        let (tx, held) = mpsc::channel();
        let run = |class, done: Arc<AtomicBool>| {
            let (host, tx, id) = (host.clone(), tx.clone(), id.clone());
            let spec = JobSpec {
                class,
                ..job_spec(plugin(id.as_str(), Trust::Unreviewed, false).manifest.as_ref().unwrap(), "Post", None, None)
            };
            jobs.submit(spec, move |ctx| {
                let _running = host.hold(ctx, &id)?;
                tx.send(ctx.id).unwrap();
                while !done.load(Ordering::SeqCst) && !ctx.cancel.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                Ok(())
            })
        };
        let job = |id: JobId| jobs.jobs().into_iter().find(|j| j.id == id).unwrap();
        let flag = || Arc::new(AtomicBool::new(false));
        let release = |done: &Arc<AtomicBool>| done.store(true, Ordering::SeqCst);
        let wait = Duration::from_millis(300);

        let first_done = flag();
        let first = run(Class::Plugin, first_done.clone());
        assert_eq!(held.recv_timeout(Duration::from_secs(10)).unwrap(), first);
        // No update or uninstall while it runs.
        let busy = host.change(&id, "One").err().unwrap();
        assert_eq!(busy.to_string(), "One is running. Wait for it to finish, or stop it in Activity.");
        // A second run on the other class waits for the first.
        let second_done = flag();
        let second = run(Class::Analysis, second_done.clone());
        assert!(held.recv_timeout(wait).is_err());
        assert_eq!(job(second).stage.as_deref(), Some("Waiting for its other run"));
        release(&first_done);
        assert_eq!(held.recv_timeout(Duration::from_secs(10)).unwrap(), second);
        release(&second_done);
        while host.is_running(&id) {
            thread::sleep(Duration::from_millis(5));
        }

        // A run that starts while the plugin changes waits for the change.
        let changing = host.change(&id, "One").unwrap();
        let third = run(Class::Plugin, flag());
        assert!(held.recv_timeout(wait).is_err());
        assert_eq!(job(third).stage.as_deref(), Some("Waiting while it is updated"));
        drop(changing);
        assert_eq!(held.recv_timeout(Duration::from_secs(10)).unwrap(), third);
        jobs.cancel(third);
        // Cancelled while it waits, it ends so.
        while host.is_running(&id) {
            thread::sleep(Duration::from_millis(5));
        }
        let changing = host.change(&id, "One").unwrap();
        let fourth = run(Class::Analysis, flag());
        while job(fourth).stage.is_none() {
            thread::sleep(Duration::from_millis(5));
        }
        jobs.cancel(fourth);
        let start = Instant::now();
        while job(fourth).state != JobState::Cancelled {
            assert!(start.elapsed() < Duration::from_secs(10), "a waiting run never stopped");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!host.is_running(&id));
        drop(changing);
        drop((host, lib));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn the_same_run_is_queued_once_and_what_waits_stops_too() {
        let tmp = std::env::temp_dir().join(format!("autoscene-plugin-runs-{}", std::process::id()));
        let (lib, host) = host_in(&tmp);
        let ids = ["dev.a.one", "dev.a.two"]
            .map(|id| host.store.commit(folder(&host, &tmp, &manifest(id, false)), true, true).unwrap().record.id);
        let jobs = JobRunner::new(&[(Class::Plugin, 1)], |_| {});
        let (tx, started) = mpsc::channel();
        let (project, export, other) = (Some(ProjectId::new()), Some(ExportId::new()), Some(ExportId::new()));
        // Every run looks alike in Activity: one plugin is the other's fork,
        // with its name and the same action.
        let submit = |key: RunKey| {
            let tx = tx.clone();
            let spec = JobSpec {
                kind: JobKind::Plugin,
                class: Class::Plugin,
                title: "Post".into(),
                subtitle: Some("Discord".into()),
                project_id: key.project_id,
                media_id: None,
                export_id: key.export_id,
            };
            host.submit(&jobs, key, spec, move |ctx| {
                tx.send(ctx.id).unwrap();
                while !ctx.cancel.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(JobError::Cancelled)
            })
        };
        let post = key(&ids[0], ActionKind::ExportAction, "post", project, export);
        let first = submit(post.clone()).unwrap();
        assert_eq!(started.recv_timeout(Duration::from_secs(10)).unwrap(), first);
        // The same run again isn't queued; on another export, or of the other plugin, it is.
        assert_eq!(submit(post.clone()), None);
        let elsewhere = submit(key(&ids[0], ActionKind::ExportAction, "post", project, other)).unwrap();
        let fork = submit(key(&ids[1], ActionKind::ExportAction, "post", project, export)).unwrap();
        let state = |id: JobId| jobs.jobs().into_iter().find(|j| j.id == id).unwrap().state;
        assert_eq!([state(elsewhere), state(fork)], [JobState::Queued; 2]);

        // The setting going on, or staying on, stops nothing.
        host.allow_changed(&jobs, false, true);
        host.allow_changed(&jobs, true, true);
        assert_eq!(
            [state(first), state(elsewhere), state(fork)],
            [JobState::Running, JobState::Queued, JobState::Queued]
        );
        // Turned off, a plugin's runs stop, also the one still waiting; the fork's runs on.
        host.stop(&jobs, &ids[0]);
        assert_eq!(state(elsewhere), JobState::Cancelled);
        assert_eq!(started.recv_timeout(Duration::from_secs(10)).unwrap(), fork);
        assert_eq!(state(first), JobState::Cancelled);
        // One that ended doesn't keep the same run from being queued again.
        let again = submit(post).unwrap();
        // The setting off: what waits stops with what runs.
        host.allow_changed(&jobs, true, false);
        assert_eq!(state(again), JobState::Cancelled);
        let start = Instant::now();
        while state(fork) != JobState::Cancelled {
            assert!(start.elapsed() < Duration::from_secs(10), "the fork's run never stopped");
            thread::sleep(Duration::from_millis(5));
        }
        drop((host, lib));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_run_starts_only_as_it_was_queued() {
        let tmp = std::env::temp_dir().join(format!("autoscene-plugin-start-{}", std::process::id()));
        let (lib, host) = host_in(&tmp);
        let id = host.store.commit(folder(&host, &tmp, &manifest("dev.a.one", false)), true, true).unwrap().record.id;
        let allow = |on| lib.set_allows_unreviewed_plugins(on).unwrap();
        let said = |key: &RunKey, gpu| match startable(&host.store, &lib, key, gpu) {
            Ok(_) => "starts".to_owned(),
            Err(JobError::Failed(why) | JobError::FailedFirst(why)) => why,
            Err(JobError::Cancelled) => "cancelled".to_owned(),
        };
        let (project, export) = (Some(ProjectId::new()), Some(ExportId::new()));
        let post = key(&id, ActionKind::ExportAction, "post", project, export);
        let tag = key(&id, ActionKind::ExportAction, "tag", project, export);
        let hello = key(&id, ActionKind::Command, "hello", None, None);

        // Loaded from a folder, it is never reviewed: the setting decides.
        assert_eq!(said(&post, false), "Plugins that AutoScene has not reviewed are turned off in Settings.");
        allow(true);
        assert_eq!([said(&post, false), said(&hello, false)], ["starts", "starts"]);
        // Its plugin.json changed while the run waited.
        assert_eq!(said(&post, true), "Plugin dev.a.one changed since this run was queued. Run it again.");
        let mut changed = manifest("dev.a.one", false);
        changed["contributes"]["exportActions"] = serde_json::json!([{"id": "tag", "title": "Tag"}]);
        changed["contributes"]["commands"][0]["needsProject"] = serde_json::json!(true);
        std::fs::write(tmp.join("dev.a.one").join("plugin.json"), changed.to_string()).unwrap();
        assert_eq!(said(&post, false), "Plugin dev.a.one has no such action.");
        assert_eq!(said(&hello, false), "Open a project to run it in.");
        assert_eq!(said(&tag, false), "starts");
        // Turned off, or gone.
        host.store.set_enabled(&id, false).unwrap();
        assert_eq!(said(&tag, false), "Plugin dev.a.one is turned off.");
        host.store.uninstall(&id, false).unwrap();
        assert_eq!(said(&tag, false), "dev.a.one isn't installed.");
        drop((host, lib));
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn a_run_gets_its_project_and_export_as_they_are_now() {
        let tmp = std::env::temp_dir().join(format!("autoscene-plugin-place-{}", std::process::id()));
        let (lib, host) = host_in(&tmp);
        let files = ["Show S01E02.mkv", "Pilot.mkv", "Gone.mkv"].map(|name| {
            let file = tmp.join("footage").join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"x").unwrap();
            file.display().to_string()
        });
        let req = NewProject {
            title: "Show".into(),
            kind: ProjectKind::Series,
            content: ContentKind::Live,
            files: files.to_vec(),
        };
        let (summary, project, _) = lib.create_project(&req).unwrap();
        let episodes = project.media_list_uncounted().unwrap();
        let [show, pilot, gone] = ["Show S01E02.mkv", "Pilot.mkv", "Gone.mkv"]
            .map(|name| episodes.iter().find(|m| m.file_name == name).unwrap().id);
        project.remove_media(gone).unwrap();
        let clip = |media_id, name: &str, start, end| ExportItem {
            id: 0,
            media_id,
            start,
            end,
            name: name.into(),
            status: ExportStatus::Done,
            path: Some(format!("/out/{name}.mp4")),
            verified: true,
            encoder: None,
            bytes: None,
            error: None,
        };
        let export = ExportSummary {
            id: ExportId::new(),
            project_id: summary.id,
            created_at: 7,
            preset: as_export::builtin_presets()[0].clone(),
            out_dir: "/out".into(),
            status: ExportStatus::Done,
            items: vec![
                clip(show, "One", 10, 20),
                clip(pilot, "Two", 0, 5),
                clip(gone, "Three", 3, 4),
                clip(show, "Four", 30, 40),
            ],
            label: Some("Walter · scenepack".into()),
            merged: None,
        };

        let got = export_context(&project, &export).unwrap();
        assert_eq!((got.id, got.label.as_deref()), (export.id.to_string(), Some("Walter · scenepack")));
        assert_eq!((got.preset, got.out_dir.as_str(), got.created_at), (export.preset.id.clone(), "/out", 7));
        let clips: Vec<_> = got
            .clips
            .iter()
            .map(|c| (c.name.as_str(), c.episode.as_str(), c.source_path.as_str(), c.start, c.end))
            .collect();
        // An episode is named by its number, else its file; one removed since has neither.
        assert_eq!(
            clips,
            [
                ("One", "S01E02", files[0].as_str(), 10, 20),
                ("Two", "Pilot.mkv", files[1].as_str(), 0, 5),
                ("Three", "", "", 3, 4),
                ("Four", "S01E02", files[0].as_str(), 30, 40),
            ]
        );
        let c = &got.clips[0];
        assert_eq!(
            (c.status.as_str(), c.path.as_deref(), c.media_id.clone()),
            ("done", Some("/out/One.mp4"), show.to_string())
        );

        // A command in the project gets the project, and no export.
        let p = host.store.commit(folder(&host, &tmp, &manifest("dev.a.one", false)), true, true).unwrap();
        let action = ActionContext { kind: ActionKind::Command, id: "count".into(), trigger: Trigger::Palette };
        let mut message =
            host.store.run_spec(&p, action, Path::new("ffmpeg"), Path::new("ffprobe"), "9.0").unwrap().message;
        let count = key(&p.record.id, ActionKind::Command, "count", Some(summary.id), None);
        add_place(&mut message, &lib, &count).unwrap();
        let place = message.project.as_ref().unwrap();
        assert_eq!(
            (place.id.clone(), place.title.as_str(), place.folder.clone()),
            (summary.id.to_string(), "Show", summary.dir)
        );
        assert!(message.export.is_none());
        // An export removed since stops the run.
        let removed = key(&p.record.id, ActionKind::ExportAction, "post", Some(summary.id), Some(ExportId::new()));
        assert!(matches!(add_place(&mut message, &lib, &removed), Err(ProjectError::NotFound(_))));
        drop((host, project, lib));
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
