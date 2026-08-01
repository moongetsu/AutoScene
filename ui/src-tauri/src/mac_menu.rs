//! macOS-only native menu bar.
//!
//! Not compiled on Windows/Linux, which keep the custom frameless chrome in
//! `TitleBar.tsx` and no menu at all. Tauri only auto-installs a default menu
//! on macOS, so this replaces that generic menu rather than adding the first one.
//!
//! Items that need app state emit `MENU_EVENT` with their id; the webview
//! listens once and dispatches (see `components/app/MacMenuBridge.tsx`).
//! Predefined items (copy/paste/quit/…) are handled by AppKit selectors and
//! never produce a `MenuEvent`.

use tauri::menu::{
    AboutMetadataBuilder, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder,
    HELP_SUBMENU_ID, WINDOW_SUBMENU_ID,
};
use tauri::{AppHandle, Emitter, Runtime};

/// Single event channel; the payload is the menu item id.
pub const MENU_EVENT: &str = "menu:action";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let pkg = app.package_info();

    let about = AboutMetadataBuilder::new()
        .name(Some("AutoScene"))
        .version(Some(pkg.version.to_string()))
        .build();

    // The first submenu becomes the app menu; macOS titles it from CFBundleName.
    let app_menu = SubmenuBuilder::new(app, "AutoScene")
        .about(Some(about))
        .separator()
        // Directly under About is where macOS apps put this, and where people
        // look for it. Handled in the webview so it shares the launch check's
        // dialog instead of growing a second one.
        .item(&MenuItemBuilder::with_id("app:check-updates", "Check for Updates…").build(app)?)
        .separator()
        .item(
            &MenuItemBuilder::with_id("app:settings", "Settings…")
                .accelerator("CmdOrCtrl+,")
                .build(app)?,
        )
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
        .item(
            &MenuItemBuilder::with_id("file:new-project", "New Project")
                .accelerator("CmdOrCtrl+N")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("file:open-video", "Open Video…")
                .accelerator("CmdOrCtrl+O")
                .build(app)?,
        )
        .separator()
        .item(
            &MenuItemBuilder::with_id("file:export", "Export Merged Video…")
                .accelerator("CmdOrCtrl+E")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("file:cancel-export", "Cancel Export")
                .accelerator("CmdOrCtrl+.")
                .build(app)?,
        )
        .separator()
        .item(
            &MenuItemBuilder::with_id("file:reveal", "Reveal in Finder")
                .accelerator("Shift+CmdOrCtrl+R")
                .build(app)?,
        )
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
        .item(
            &MenuItemBuilder::with_id("edit:copy-timecodes", "Copy Timecodes")
                .accelerator("Shift+CmdOrCtrl+C")
                .build(app)?,
        )
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(
            &MenuItemBuilder::with_id("view:/", "Dashboard")
                .accelerator("CmdOrCtrl+1")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("view:/import", "Import")
                .accelerator("CmdOrCtrl+2")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("view:/gallery", "Gallery")
                .accelerator("CmdOrCtrl+3")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("view:/timeline", "Timeline")
                .accelerator("CmdOrCtrl+4")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("view:/export", "Export")
                .accelerator("CmdOrCtrl+5")
                .build(app)?,
        )
        .separator()
        .item(
            &MenuItemBuilder::with_id("view:toggle-sidebar", "Toggle Sidebar")
                .accelerator("CmdOrCtrl+B")
                .build(app)?,
        )
        .separator()
        .fullscreen()
        .build()?;

    // These two ids make Tauri call set_as_windows_menu_for_nsapp /
    // set_as_help_menu_for_nsapp, which add the window list, "Bring All to
    // Front", and the Help search field.
    let window_menu = SubmenuBuilder::with_id(app, WINDOW_SUBMENU_ID, "Window")
        .minimize()
        .maximize_with_text("Zoom")
        .separator()
        .close_window()
        .build()?;

    let help_menu = SubmenuBuilder::with_id(app, HELP_SUBMENU_ID, "Help")
        .item(&MenuItemBuilder::with_id("help:logs", "Show Logs in Finder").build(app)?)
        .build()?;

    MenuBuilder::new(app)
        .items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ])
        .build()
}

pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id: &str = event.id().as_ref();
    match id {
        // Handled entirely in Rust — no IPC round-trip, no capability needed.
        "help:logs" => {
            let path = crate::api_log_path();
            let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
        }
        _ => {
            if let Err(e) = app.emit(MENU_EVENT, id.to_string()) {
                log::warn!("menu emit failed for {id}: {e}");
            }
        }
    }
}
