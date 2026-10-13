//! Typed events pushed to the UI. Names are stable strings so the frontend's
//! event → query-invalidation map (`app/src/lib/events.ts`) can be grepped.

use as_api::{JobInfo, JobProgress};
use as_core::{ExportId, MediaId, ProjectId};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

/// A native menu item was chosen (macOS). Payload is the item id, e.g. `view:cast`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "menu:action")]
pub struct MenuAction(pub String);

/// A job was queued, started or finished.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "job:changed")]
pub struct JobChanged(pub JobInfo);

/// Throttled progress of a running job.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "job:progress")]
pub struct JobProgressEvent(pub JobProgress);

/// The project list or a project's summary changed.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "library:changed")]
#[serde(rename_all = "camelCase")]
pub struct LibraryChanged {
    pub project_id: Option<ProjectId>,
}

/// Media of a project changed (added, indexed, preview ready, removed).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "media:changed")]
#[serde(rename_all = "camelCase")]
pub struct MediaChanged {
    pub project_id: ProjectId,
    /// `None` when several media changed at once.
    pub media_id: Option<MediaId>,
}

/// An export was created, progressed to another clip, or finished.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "export:changed")]
#[serde(rename_all = "camelCase")]
pub struct ExportChanged {
    pub project_id: ProjectId,
    pub export_id: ExportId,
}

pub fn emit_media_changed(app: &tauri::AppHandle, project_id: ProjectId, media_id: Option<MediaId>) {
    let _ = MediaChanged { project_id, media_id }.emit(app);
}
