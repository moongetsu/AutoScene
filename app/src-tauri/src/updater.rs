//! In-app updates for macOS and Windows.
//!
//! Wraps `tauri-plugin-updater` in three commands the webview drives:
//! `update_check`, `update_install`, `update_relaunch`. Keeping the logic in
//! Rust means the webview needs no updater capability, and the local override
//! file below can fake an update without a release server.
//!
//! ## Release side
//!
//! `bundle.createUpdaterArtifacts` makes `tauri build` emit signed updater
//! bundles. Publish them next to a `latest.json` manifest at
//! `plugins.updater.endpoints`. Signing needs `TAURI_SIGNING_PRIVATE_KEY`; the
//! matching public key is `plugins.updater.pubkey`.
//!
//! ## Local override — `~/.autoscene/updates.json`
//!
//! ```json
//! {
//!   "endpoint": "http://127.0.0.1:8000/latest.json",
//!   "simulate": { "version": "0.3.0", "notes": "What's new…", "date": "2026-10-01" }
//! }
//! ```
//!
//! Both keys are optional. `endpoint` repoints the check (staging). `simulate`
//! short-circuits it: `update_check` reports that version as available and
//! `update_install` runs the progress arc without touching the installed app.
//! A file is used rather than an env var because an app launched from Finder
//! or the Start menu inherits no shell environment.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};

/// Simulated downloads pretend to be this big, over this long.
const FAKE_BYTES: u64 = 48 * 1024 * 1024;
const FAKE_STEPS: u64 = 40;
const FAKE_STEP_MS: u64 = 60;

#[derive(Debug, Default, Deserialize)]
struct LocalConfig {
    endpoint: Option<String>,
    simulate: Option<Simulated>,
}

#[derive(Debug, Clone, Deserialize)]
struct Simulated {
    version: Option<String>,
    notes: Option<String>,
    date: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum UpdateState {
    Available,
    Current,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub state: UpdateState,
    /// Version running right now.
    pub current_version: String,
    /// Version on offer. Only set when `state == "available"`.
    pub version: Option<String>,
    /// Release notes, as written in the manifest.
    pub notes: Option<String>,
    /// Publish date, `YYYY-MM-DD`.
    pub date: Option<String>,
    /// True when the answer came from the local override, not a real feed.
    pub simulated: bool,
}

/// Download progress. `downloaded == 0 && total == None` after the download
/// finishes means "installing".
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "update:progress")]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

fn config_path() -> std::path::PathBuf {
    as_core::AppPaths::legacy_dot_dir().join("updates.json")
}

/// Missing or malformed file means "no override" — never a failed check.
fn local_config() -> LocalConfig {
    let path = config_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return LocalConfig::default();
    };
    serde_json::from_str::<LocalConfig>(&raw).unwrap_or_else(|e| {
        log::warn!("Ignoring {}: {e}", path.display());
        LocalConfig::default()
    })
}

fn update_err(e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::Update, e.to_string())
}

fn updater_for(app: &AppHandle, cfg: &LocalConfig) -> AppResult<tauri_plugin_updater::Updater> {
    // On Windows the installer starts and the plugin calls `exit(0)` without
    // a RunEvent::Exit: stop our jobs first (this replaces the plugin's
    // default hook, so run its cleanup too).
    let handle = app.clone();
    let mut builder = app.updater_builder().on_before_exit(move || {
        crate::state::shutdown_work(&handle);
        handle.cleanup_before_exit();
    });
    if let Some(endpoint) = &cfg.endpoint {
        let url = endpoint.parse().map_err(|e| update_err(format!("Bad endpoint in updates.json: {e}")))?;
        builder = builder.endpoints(vec![url]).map_err(update_err)?;
    }
    builder.build().map_err(update_err)
}

fn simulated_status(sim: &Simulated, current: String) -> UpdateStatus {
    UpdateStatus {
        state: UpdateState::Available,
        current_version: current,
        version: Some(sim.version.clone().unwrap_or_else(|| "0.3.0".into())),
        notes: Some(sim.notes.clone().unwrap_or_else(|| {
            "Simulated release notes. Set `simulate.notes` in ~/.autoscene/updates.json to change this text.".into()
        })),
        date: sim.date.clone(),
        simulated: true,
    }
}

/// Ask the configured feed whether a newer build exists. Errors are surfaced
/// so a manual check can say *why* it failed; the launch check stays quiet.
#[tauri::command]
#[specta::specta]
pub async fn update_check(app: AppHandle) -> AppResult<UpdateStatus> {
    let current = app.package_info().version.to_string();
    let cfg = local_config();
    if let Some(sim) = &cfg.simulate {
        log::info!("Update check answered from ~/.autoscene/updates.json (simulate)");
        return Ok(simulated_status(sim, current));
    }
    let status = match updater_for(&app, &cfg)?.check().await.map_err(update_err)? {
        Some(update) => UpdateStatus {
            state: UpdateState::Available,
            current_version: current,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            date: update.date.map(|d| d.date().to_string()),
            simulated: false,
        },
        None => UpdateStatus {
            state: UpdateState::Current,
            current_version: current,
            version: None,
            notes: None,
            date: None,
            simulated: false,
        },
    };
    Ok(status)
}

/// Download and install the pending update, emitting [`UpdateProgress`].
/// Re-checks the feed rather than caching a handle so it is safe to retry.
#[tauri::command]
#[specta::specta]
pub async fn update_install(app: AppHandle) -> AppResult<()> {
    let cfg = local_config();
    if cfg.simulate.is_some() {
        return simulate_install(&app).await;
    }
    let update = updater_for(&app, &cfg)?
        .check()
        .await
        .map_err(update_err)?
        .ok_or_else(|| update_err("That update is no longer available."))?;

    let mut downloaded: u64 = 0;
    let progress_app = app.clone();
    let finish_app = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                emit_progress(&progress_app, downloaded, total);
            },
            move || {
                log::info!("Update download finished; installing");
                emit_progress(&finish_app, 0, None);
            },
        )
        .await
        .map_err(|e| {
            // On Windows our jobs were stopped just before the installer was
            // to start; it didn't, so the app keeps running and needs them.
            crate::state::resume_work(&app);
            update_err(e)
        })
}

/// Restart into the freshly installed build. A no-op under the simulate
/// override, where nothing was installed. Returns whether a restart happened.
///
/// `request_restart` goes through RunEvent::Exit, where background jobs are
/// shut down; `restart()` on the main thread would skip it.
#[tauri::command]
#[specta::specta]
pub fn update_relaunch(app: AppHandle) -> bool {
    if local_config().simulate.is_some() {
        log::info!("Simulated update: skipping relaunch");
        return false;
    }
    app.request_restart();
    true
}

fn emit_progress(app: &AppHandle, downloaded: u64, total: Option<u64>) {
    let ev = UpdateProgress { downloaded, total };
    if let Err(e) = ev.emit(app) {
        log::warn!("update progress emit failed: {e}");
    }
}

async fn simulate_install(app: &AppHandle) -> AppResult<()> {
    log::info!("Simulated update: faking a {FAKE_BYTES}-byte download");
    for step in 1..=FAKE_STEPS {
        tokio::time::sleep(Duration::from_millis(FAKE_STEP_MS)).await;
        emit_progress(app, FAKE_BYTES * step / FAKE_STEPS, Some(FAKE_BYTES));
    }
    Ok(())
}
