//! Typed events pushed to the UI. Names are stable strings so the frontend's
//! event → query-invalidation map can be grepped.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

/// A native menu item was chosen (macOS). Payload is the item id, e.g. `view:cast`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "menu:action")]
pub struct MenuAction(pub String);
