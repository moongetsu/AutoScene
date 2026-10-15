//! AutoScene desktop shell.
//!
//! Owns the window, typed IPC (tauri-specta → `app/src/bindings.ts`), the
//! library, background jobs (indexing, previews, analysis, exports), logging
//! and updates. AI analysis runs in a separate engine process (this same
//! executable started with `--engine`) so a GPU or driver crash can never
//! take the UI down.

mod commands;
mod error;
mod events;
mod keep_awake;
#[cfg(target_os = "macos")]
mod mac_menu;
mod state;
mod updater;
mod window;
mod work;

use as_core::AppPaths;
use as_project::Library;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
use tauri_plugin_window_state::StateFlags;
use tauri_specta::{collect_commands, collect_events};

/// Single source of truth for the IPC surface. Also used by the
/// `export-bindings` bin so CI can fail when bindings.ts is stale.
pub fn specta_builder() -> tauri_specta::Builder<tauri::DynRuntime> {
    tauri_specta::Builder::<tauri::DynRuntime>::new()
        .error_handling(tauri_specta::ErrorHandlingMode::Throw)
        .dangerously_cast_bigints_to_number()
        .commands(collect_commands![
            commands::system::app_info,
            commands::system::reveal_path,
            commands::system::clipboard_write,
            commands::system::open_logs_dir,
            commands::system::media_tools,
            commands::library::projects_list,
            commands::library::project_get,
            commands::library::project_create,
            commands::library::project_rename,
            commands::library::project_delete,
            commands::library::import_scan,
            commands::library::settings_get,
            commands::library::settings_update,
            commands::media::media_list,
            commands::media::media_get,
            commands::media::media_add,
            commands::media::media_remove,
            commands::media::media_retry,
            commands::media::media_prepare_preview,
            commands::media::media_set_audio,
            commands::media::media_frame_map,
            commands::media::media_segments,
            commands::media::media_analyze,
            commands::cast::cast_get,
            commands::cast::cast_tracks,
            commands::cast::cast_edit,
            commands::cast::cast_portrait,
            commands::cast::media_presence,
            commands::cast::character_clips,
            commands::cast::character_shots,
            commands::export::export_presets,
            commands::export::export_default_dir,
            commands::export::export_start,
            commands::export::exports_list,
            commands::jobs::jobs_list,
            commands::jobs::job_cancel,
            commands::jobs::jobs_clear_finished,
            updater::update_check,
            updater::update_install,
            updater::update_relaunch,
        ])
        .events(collect_events![
            events::MenuAction,
            events::JobChanged,
            events::JobProgressEvent,
            events::LibraryChanged,
            events::MediaChanged,
            events::ExportChanged,
            events::AnalysisChanged,
            events::CastChanged,
            events::PortraitsChanged,
            updater::UpdateProgress,
        ])
}

pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");

pub fn export_bindings(builder: &tauri_specta::Builder<tauri::DynRuntime>) {
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
        .runtime(tauri_runtime_wry::Wry::default())
        // Must be first: a second launch just focuses the running window.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(log_plugin)
        // Size and place come back, never the frame: the window's chrome is ours
        // (a state saved by an older, framed build would double the title bar).
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() - StateFlags::DECORATIONS)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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
            let library = match Library::open(&paths) {
                Ok(l) => l,
                Err(e) => {
                    log::error!("opening the library failed: {e}");
                    app.dialog()
                        .message(format!(
                            "AutoScene couldn't open its library.

{e}"
                        ))
                        .title("AutoScene")
                        .kind(MessageDialogKind::Error)
                        .blocking_show();
                    std::process::exit(1);
                }
            };
            let handle = app.handle().clone();
            app.manage(state::AppState::new(&handle, library, &paths));
            // After the state: the page calls commands as soon as it loads.
            window::create_main(app)?;
            // Resume interrupted indexing and previews; locate ffmpeg and probe
            // the hardware off the UI thread so the first job starts warm.
            std::thread::spawn(move || {
                // Models a better one replaced (their media are found again below).
                if let Err(e) = handle.state::<state::AppState>().models.prune() {
                    log::warn!("removing unused models: {}", e.detail());
                }
                work::recover_all(&handle);
                if let Ok(tools) = handle.state::<state::AppState>().tools() {
                    let _ = tools.caps();
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building AutoScene")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Never leave ffmpeg processes running after the window closes.
                state::shutdown_work(app);
            }
        });
}
