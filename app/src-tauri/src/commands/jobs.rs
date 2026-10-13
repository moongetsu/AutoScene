//! Background jobs shown in the Activity drawer.

use as_api::JobInfo;
use as_core::JobId;
use tauri::State;

use crate::state::AppState;

/// Newest first. Live updates arrive as `job:changed` / `job:progress`.
#[tauri::command]
#[specta::specta]
pub fn jobs_list(state: State<'_, AppState>) -> Vec<JobInfo> {
    state.jobs.jobs()
}

#[tauri::command]
#[specta::specta]
pub fn job_cancel(state: State<'_, AppState>, job_id: JobId) {
    state.jobs.cancel(job_id);
}

/// Forget finished jobs.
#[tauri::command]
#[specta::specta]
pub fn jobs_clear_finished(state: State<'_, AppState>) {
    state.jobs.clear_finished();
}
