//! In-app updates for macOS and Windows.
//!
//! Wraps `tauri-plugin-updater` in three commands the webview drives:
//! `update_check`, `update_install`, `update_relaunch`. Keeping the logic in
//! Rust (rather than calling the JS plugin directly) buys two things: the
//! webview needs no updater capability beyond these commands, and the local
//! override file below can fake an update without a release server.
//!
//! ## Release side
//!
//! `bundle.createUpdaterArtifacts` makes `tauri build` emit `<app>.tar.gz` +
//! `.sig` (macOS) and `<app>.nsis.zip` + `.sig` (Windows). Publish those next
//! to a `latest.json` manifest at `plugins.updater.endpoints`. Signing needs
//! `TAURI_SIGNING_PRIVATE_KEY`; the matching public key is `plugins.updater.pubkey`.
//!
//! ## Local override — `~/.autoscene/updates.json`
//!
//! ```json
//! {
//!   "endpoint": "http://127.0.0.1:8000/latest.json",
//!   "simulate": { "version": "0.2.0", "notes": "What's new…", "date": "2026-08-01" }
//! }
//! ```
//!
//! Both keys are optional. `endpoint` repoints the check (handy for staging).
//! `simulate` short-circuits it entirely: `update_check` reports that version
//! as available and `update_install` runs the progress arc without touching
//! the installed app, so the whole prompt can be exercised offline. A file is
//! used rather than an env var because an app launched from Finder or the
//! Start menu inherits no shell environment.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_updater::UpdaterExt;

/// Download progress: `{ downloaded, total }`, total absent when unknown.
pub const PROGRESS_EVENT: &str = "update:progress";

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// `available` · `current` · `unsupported`
    pub state: &'static str,
    /// Version running right now, from `tauri.conf.json`.
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

impl UpdateStatus {
    fn current(version: String, simulated: bool) -> Self {
        Self {
            state: "current",
            current_version: version,
            version: None,
            notes: None,
            date: None,
            simulated,
        }
    }
}

fn config_path() -> std::path::PathBuf {
    crate::autoscene_dir().join("updates.json")
}

/// Missing or malformed file means "no override" — never a failed check.
fn local_config() -> LocalConfig {
    let path = config_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return LocalConfig::default();
    };
    match serde_json::from_str::<LocalConfig>(&raw) {
        Ok(cfg) => cfg,
        Err(e) => {
            log::warn!("Ignoring {}: {e}", path.display());
            LocalConfig::default()
        }
    }
}

fn current_version<R: Runtime>(app: &AppHandle<R>) -> String {
    app.package_info().version.to_string()
}

/// An updater pointed at the configured feed, honouring an `endpoint` override.
fn updater_for<R: Runtime>(
    app: &AppHandle<R>,
    cfg: &LocalConfig,
) -> Result<tauri_plugin_updater::Updater, String> {
    let mut builder = app.updater_builder();
    if let Some(endpoint) = &cfg.endpoint {
        let url = endpoint
            .parse()
            .map_err(|e| format!("Bad endpoint in updates.json: {e}"))?;
        builder = builder
            .endpoints(vec![url])
            .map_err(|e| format!("Bad endpoint in updates.json: {e}"))?;
    }
    builder.build().map_err(|e| e.to_string())
}

/// Present the override as if the feed had returned it.
fn simulated_status(sim: &Simulated, current: String) -> UpdateStatus {
    UpdateStatus {
        state: "available",
        current_version: current,
        version: Some(sim.version.clone().unwrap_or_else(|| "0.2.0".into())),
        notes: Some(sim.notes.clone().unwrap_or_else(|| {
            "Simulated release notes. Set `simulate.notes` in \
             ~/.autoscene/updates.json to change this text."
                .into()
        })),
        date: sim.date.clone(),
        simulated: true,
    }
}

/// Ask the configured feed whether a newer build exists.
///
/// Errors are surfaced to the caller rather than swallowed, so a manual
/// "Check for Updates" can say *why* it failed. The automatic check on launch
/// reports the failure quietly instead.
#[tauri::command]
pub async fn update_check<R: Runtime>(app: AppHandle<R>) -> Result<UpdateStatus, String> {
    let current = current_version(&app);
    let cfg = local_config();

    if let Some(sim) = &cfg.simulate {
        log::info!("Update check answered from ~/.autoscene/updates.json (simulate)");
        return Ok(simulated_status(sim, current));
    }

    let updater = updater_for(&app, &cfg)?;
    match updater.check().await {
        Ok(Some(update)) => Ok(UpdateStatus {
            state: "available",
            current_version: current,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            date: update.date.map(|d| d.date().to_string()),
            simulated: false,
        }),
        Ok(None) => Ok(UpdateStatus::current(current, false)),
        Err(e) => Err(e.to_string()),
    }
}

/// Download and install the pending update. Emits [`PROGRESS_EVENT`] as it goes.
///
/// The feed is re-checked here rather than caching the handle from
/// `update_check`: the extra request is one small JSON, and it keeps the
/// command safe to retry after a failed download.
#[tauri::command]
pub async fn update_install<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let cfg = local_config();

    if cfg.simulate.is_some() {
        return simulate_install(&app).await;
    }

    let updater = updater_for(&app, &cfg)?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "That update is no longer available.".to_string())?;

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
                let _ = finish_app.emit(PROGRESS_EVENT, Progress { downloaded: 0, total: None });
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Restart into the freshly installed build.
///
/// A no-op under the simulate override — nothing was actually installed, so
/// restarting would only re-show the same prompt.
#[tauri::command]
pub fn update_relaunch<R: Runtime>(app: AppHandle<R>) -> Result<bool, String> {
    if local_config().simulate.is_some() {
        log::info!("Simulated update: skipping relaunch");
        return Ok(false);
    }
    app.restart();
}

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

fn emit_progress<R: Runtime>(app: &AppHandle<R>, downloaded: u64, total: Option<u64>) {
    if let Err(e) = app.emit(PROGRESS_EVENT, Progress { downloaded, total }) {
        log::warn!("update progress emit failed: {e}");
    }
}

/// Walk the progress bar to 100% without touching the installed app.
async fn simulate_install<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    log::info!("Simulated update: faking a {FAKE_BYTES}-byte download");
    for step in 1..=FAKE_STEPS {
        tokio::time::sleep(Duration::from_millis(FAKE_STEP_MS)).await;
        emit_progress(app, FAKE_BYTES * step / FAKE_STEPS, Some(FAKE_BYTES));
    }
    Ok(())
}
