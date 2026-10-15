//! macOS-only native menu bar.
//!
//! Windows/Linux keep the custom frameless chrome (`TitleBar.tsx`) and no
//! menu. Items that need app state emit [`MenuAction`] with their id; the
//! webview dispatches it (see `useGlobalBindings` in `app/src/lib/actions.ts`,
//! which has a handler for every id below). Predefined items
//! (copy/paste/quit/…) are handled by AppKit and never produce an event.

use tauri::menu::{
    AboutMetadataBuilder, HELP_SUBMENU_ID, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder,
    WINDOW_SUBMENU_ID,
};
use tauri::{AppHandle, Runtime};
use tauri_specta::Event;

use crate::events::MenuAction;

fn item<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    label: &str,
    accel: Option<&str>,
) -> tauri::Result<tauri::menu::MenuItem<R>> {
    let mut b = MenuItemBuilder::with_id(id, label);
    if let Some(a) = accel {
        b = b.accelerator(a);
    }
    b.build(app)
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let about = AboutMetadataBuilder::new()
        .name(Some("AutoScene"))
        .version(Some(app.package_info().version.to_string()))
        .build();

    let app_menu = SubmenuBuilder::new(app, "AutoScene")
        .about(Some(about))
        .separator()
        .item(&item(app, "app:check-updates", "Check for Updates…", None)?)
        .separator()
        .item(&item(app, "app:settings", "Settings…", Some("CmdOrCtrl+,"))?)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&item(app, "file:new-project", "New Project…", Some("CmdOrCtrl+N"))?)
        .item(&item(app, "file:add-media", "Add Episodes…", Some("CmdOrCtrl+O"))?)
        .separator()
        .item(&item(app, "file:export", "Export Clip…", Some("CmdOrCtrl+E"))?)
        .separator()
        .item(&item(app, "file:reveal", "Reveal in Finder", Some("Shift+CmdOrCtrl+R"))?)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .separator()
        .item(&item(app, "edit:copy-timecodes", "Copy Timecodes", Some("Shift+CmdOrCtrl+C"))?)
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&item(app, "view:command-palette", "Command Palette…", Some("CmdOrCtrl+K"))?)
        .separator()
        .item(&item(app, "view:library", "Library", Some("CmdOrCtrl+1"))?)
        .item(&item(app, "view:episodes", "Episodes", Some("CmdOrCtrl+2"))?)
        .item(&item(app, "view:exports", "Exports", Some("CmdOrCtrl+3"))?)
        .separator()
        .item(&item(app, "view:activity", "Activity", Some("CmdOrCtrl+J"))?)
        .item(&item(app, "view:toggle-sidebar", "Toggle Sidebar", Some("CmdOrCtrl+B"))?)
        .separator()
        .fullscreen()
        .build()?;

    // These ids make Tauri register the window list and the Help search field.
    let window_menu = SubmenuBuilder::with_id(app, WINDOW_SUBMENU_ID, "Window")
        .minimize()
        .maximize_with_text("Zoom")
        .separator()
        .close_window()
        .build()?;

    let help_menu = SubmenuBuilder::with_id(app, HELP_SUBMENU_ID, "Help")
        .item(&item(app, "help:logs", "Show Logs in Finder", None)?)
        .build()?;

    MenuBuilder::new(app).items(&[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu, &help_menu]).build()
}

pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id: &str = event.id().as_ref();
    if id == "help:logs" {
        // Handled in Rust — no IPC round-trip needed.
        let _ = std::process::Command::new("open").arg(as_core::AppPaths::resolve().logs()).spawn();
        return;
    }
    if let Err(e) = MenuAction(id.to_string()).emit(app) {
        log::warn!("menu emit failed for {id}: {e}");
    }
}
