//! AutoScene desktop shell.
//!
//! Owns the window, typed IPC (tauri-specta → `app/src/bindings.ts`), logging
//! and updates. Media analysis runs in the separate `autoscene-engine`
//! process (added in M2) so a GPU/driver crash can never take the UI down.

mod commands;
mod error;
mod events;
#[cfg(target_os = "macos")]
mod mac_menu;
mod updater;

use as_core::AppPaths;
use tauri::Manager;
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
use tauri_specta::{collect_commands, collect_events};

/// Single source of truth for the IPC surface. Also used by the
/// `export-bindings` bin so CI can fail when bindings.ts is stale.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        .dangerously_cast_bigints_to_number()
        .commands(collect_commands![
            commands::system::app_info,
            commands::system::reveal_path,
            commands::system::open_logs_dir,
            updater::update_check,
            updater::update_install,
            updater::update_relaunch,
        ])
        .events(collect_events![events::MenuAction, updater::UpdateProgress])
}

pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");

pub fn export_bindings(builder: &tauri_specta::Builder<tauri::Wry>) {
    builder
        .export(specta_typescript::Typescript::default().header("// @ts-nocheck\n/* eslint-disable */"), BINDINGS_PATH)
        .expect("failed to export TypeScript bindings");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let specta = specta_builder();
    #[cfg(debug_assertions)]
    export_bindings(&specta);

    let paths = AppPaths::resolve();
    let _ = std::fs::create_dir_all(paths.logs());

    let log_plugin = tauri_plugin_log::Builder::default()
        .clear_targets()
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::Folder { path: paths.logs(), file_name: Some("app".into()) }),
        ])
        .rotation_strategy(RotationStrategy::KeepSome(5))
        .max_file_size(8 * 1024 * 1024)
        .level(if cfg!(debug_assertions) { log::LevelFilter::Debug } else { log::LevelFilter::Info })
        // Webview/tao internals are noisy at debug.
        .level_for("tao", log::LevelFilter::Warn)
        .level_for("wry", log::LevelFilter::Warn)
        .build();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        // Must be first: a second launch just focuses the running window.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(log_plugin)
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(specta.invoke_handler());

    #[cfg(target_os = "macos")]
    {
        builder = builder.menu(mac_menu::build).on_menu_event(mac_menu::on_event);
    }

    builder
        .setup(move |app| {
            specta.mount_events(app);
            log::info!("AutoScene {} starting; data dir {}", app.package_info().version, paths.data.display());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AutoScene");
}
