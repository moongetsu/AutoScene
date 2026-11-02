//! Plugins (Settings → Plugins): what is installed, installing it after a
//! review, uninstalling, turning plugins and their after-export actions on
//! or off, their settings, and running their actions. Each change emits
//! `plugins:changed`; runs report as jobs.

use std::collections::BTreeMap;
use std::path::Path;

use as_core::{ExportId, JobId, ProjectId};
use as_plugin::{ActionKind, PluginError, PluginId, SettingValue, Trigger};
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::AppResult;
use crate::events::PluginsChanged;
use crate::paths;
use crate::plugins::{self, PluginInfo, PluginReview, host, info, review};
use crate::state::{AppState, blocking};

/// Every installed plugin, by id (also the broken ones).
#[tauri::command]
#[specta::specta]
pub async fn plugins_list(state: State<'_, AppState>) -> AppResult<Vec<PluginInfo>> {
    let (host, lib) = (host(&state)?, state.library.clone());
    blocking(move || {
        let allow = lib.allows_unreviewed_plugins()?;
        Ok(host.store.list()?.iter().map(|p| info(p, &host.store, allow)).collect())
    })
    .await
}

/// Unpack and check the plugin package at `path`, for the review dialog,
/// which answers with `plugin_install` or `plugin_discard`.
#[tauri::command]
#[specta::specta]
pub async fn plugin_inspect(state: State<'_, AppState>, path: String) -> AppResult<PluginReview> {
    paths::check(&path)?;
    let (host, lib) = (host(&state)?, state.library.clone());
    blocking(move || {
        let allow = lib.allows_unreviewed_plugins()?;
        let ticket = host.store.stage(Path::new(&path))?;
        let review = review(&ticket, &host.store, allow);
        host.keep(ticket);
        Ok(review)
    })
    .await
}

/// Check the plugin in the folder `path` to load it where it is (a
/// developer's), for the review dialog. Only while plugins AutoScene hasn't
/// reviewed are allowed: a folder is never reviewed.
#[tauri::command]
#[specta::specta]
pub async fn plugin_inspect_folder(state: State<'_, AppState>, path: String) -> AppResult<PluginReview> {
    paths::check(&path)?;
    let (host, lib) = (host(&state)?, state.library.clone());
    blocking(move || {
        if !lib.allows_unreviewed_plugins()? {
            return Err(PluginError::NotAllowed.into());
        }
        let ticket = host.store.stage_folder(Path::new(&path))?;
        let review = review(&ticket, &host.store, true);
        host.keep(ticket);
        Ok(review)
    })
    .await
}

/// Install the plugin the review `ticket_id` showed. One AutoScene hasn't
/// reviewed needs the setting, and `accept_unreviewed`: the person typed its
/// name. Refused while the version it replaces runs.
#[tauri::command]
#[specta::specta]
pub async fn plugin_install(
    app: AppHandle,
    state: State<'_, AppState>,
    ticket_id: String,
    accept_unreviewed: bool,
) -> AppResult<PluginInfo> {
    let (host, lib) = (host(&state)?, state.library.clone());
    let installed = blocking(move || {
        let allow = lib.allows_unreviewed_plugins()?;
        let ticket = host.take(&ticket_id)?;
        if host.is_running(&ticket.manifest.id) {
            let busy = PluginError::Busy(ticket.manifest.name.clone());
            host.keep(ticket);
            return Err(busy.into());
        }
        let p = host.store.commit(ticket, allow, accept_unreviewed)?;
        Ok(info(&p, &host.store, allow))
    })
    .await?;
    changed(&app, Some(installed.id.clone()));
    Ok(installed)
}

/// Drop the review `ticket_id` (the dialog was closed), and the files it
/// unpacked. One that is gone already is no error.
#[tauri::command]
#[specta::specta]
pub async fn plugin_discard(state: State<'_, AppState>, ticket_id: String) -> AppResult<()> {
    let host = host(&state)?;
    blocking(move || {
        if let Ok(ticket) = host.take(&ticket_id) {
            host.store.discard(ticket);
        }
        Ok(())
    })
    .await
}

/// Uninstall a plugin, with what it kept when `delete_data`. A folder it was
/// loaded from stays. Refused while it runs; its runs still waiting go.
#[tauri::command]
#[specta::specta]
pub async fn plugin_uninstall(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    delete_data: bool,
) -> AppResult<()> {
    let id = PluginId::parse(&plugin_id)?;
    let (host, jobs) = (host(&state)?, state.jobs.clone());
    blocking(move || {
        if host.is_running(&id) {
            let name = host.store.get(&id).ok().and_then(|p| p.manifest.map(|m| m.name));
            return Err(PluginError::Busy(name.unwrap_or_else(|| id.to_string())).into());
        }
        host.store.uninstall(&id, delete_data)?;
        host.stop(&jobs, &id);
        Ok(())
    })
    .await?;
    changed(&app, Some(plugin_id));
    Ok(())
}

/// Turn a plugin on or off. Off, its runs stop, and those still waiting go.
#[tauri::command]
#[specta::specta]
pub async fn plugin_set_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    enabled: bool,
) -> AppResult<PluginInfo> {
    let id = PluginId::parse(&plugin_id)?;
    let (host, lib, jobs) = (host(&state)?, state.library.clone(), state.jobs.clone());
    let p = blocking(move || {
        let p = host.store.set_enabled(&id, enabled)?;
        if !enabled {
            host.stop(&jobs, &id);
        }
        Ok(info(&p, &host.store, lib.allows_unreviewed_plugins()?))
    })
    .await?;
    changed(&app, Some(p.id.clone()));
    Ok(p)
}

/// Turn a plugin's export action on or off for after every export.
#[tauri::command]
#[specta::specta]
pub async fn plugin_set_after_export(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    action_id: String,
    on: bool,
) -> AppResult<PluginInfo> {
    let id = PluginId::parse(&plugin_id)?;
    let (host, lib) = (host(&state)?, state.library.clone());
    let p = blocking(move || {
        let p = host.store.set_after_export(&id, &action_id, on)?;
        Ok(info(&p, &host.store, lib.allows_unreviewed_plugins()?))
    })
    .await?;
    changed(&app, Some(p.id.clone()));
    Ok(p)
}

/// A plugin's settings: the person's value of each, else its default.
#[tauri::command]
#[specta::specta]
pub async fn plugin_settings_get(
    state: State<'_, AppState>,
    plugin_id: String,
) -> AppResult<BTreeMap<String, SettingValue>> {
    let id = PluginId::parse(&plugin_id)?;
    let host = host(&state)?;
    blocking(move || Ok(host.store.values(&id)?)).await
}

/// Change some of a plugin's settings, each checked against what it may
/// be; gives all of them as they are now.
#[tauri::command]
#[specta::specta]
pub async fn plugin_settings_set(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    values: BTreeMap<String, SettingValue>,
) -> AppResult<BTreeMap<String, SettingValue>> {
    let id = PluginId::parse(&plugin_id)?;
    let host = host(&state)?;
    let values = blocking(move || Ok(host.store.set_values(&id, values)?)).await?;
    changed(&app, Some(plugin_id));
    Ok(values)
}

/// Run a plugin's export action on an export (from its menu), or its
/// command (from the palette), in the project when it needs one. `None`
/// when that run waits or runs already. It reports as a job.
#[tauri::command]
#[specta::specta]
pub async fn plugin_run(
    app: AppHandle,
    plugin_id: String,
    action_id: String,
    kind: ActionKind,
    project_id: Option<ProjectId>,
    export_id: Option<ExportId>,
) -> AppResult<Option<JobId>> {
    let id = PluginId::parse(&plugin_id)?;
    let trigger = match kind {
        ActionKind::ExportAction => Trigger::Menu,
        ActionKind::Command => Trigger::Palette,
    };
    blocking(move || plugins::queue_run(&app, &id, kind, &action_id, trigger, project_id, export_id)).await
}

fn changed(app: &AppHandle, plugin_id: Option<String>) {
    let _ = PluginsChanged { plugin_id }.emit(app);
}
