use std::fs::{self, OpenOptions};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use tauri::Manager;

#[cfg(target_os = "macos")]
mod mac_menu;

struct BackendProcess(Mutex<Option<Child>>);

const API_HOST: &str = "127.0.0.1";
const API_PORT: u16 = 8741;

fn api_ready() -> bool {
    TcpStream::connect_timeout(
        &format!("{API_HOST}:{API_PORT}").parse().expect("addr"),
        Duration::from_millis(250),
    )
    .is_ok()
}

/// TCP probe for the UI boot gate — avoids noisy fetch() CONNECTION_REFUSED logs.
#[tauri::command]
fn probe_api_port() -> bool {
    api_ready()
}

pub(crate) fn api_log_path() -> std::path::PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = home.join(".autoscene").join("logs");
    let _ = fs::create_dir_all(&dir);
    dir.join("api.log")
}

fn spawn_stdio() -> (Stdio, Stdio) {
    let path = api_log_path();
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            let err = file
                .try_clone()
                .map(Stdio::from)
                .unwrap_or_else(|_| Stdio::null());
            (Stdio::from(file), err)
        }
        Err(_) => (Stdio::null(), Stdio::null()),
    }
}

fn spawn_backend() -> Option<Child> {
    if api_ready() {
        log::info!("AutoScene API already listening on {API_HOST}:{API_PORT}");
        return None;
    }

    let (out, err) = spawn_stdio();
    let autoscene = Command::new("autoscene")
        .args([
            "serve",
            "--api-only",
            "--host",
            API_HOST,
            "--port",
            &API_PORT.to_string(),
        ])
        .stdout(out)
        .stderr(err)
        .spawn();

    match autoscene {
        Ok(child) => {
            log::info!(
                "Started autoscene serve (pid {}) — logs {}",
                child.id(),
                api_log_path().display()
            );
            Some(child)
        }
        Err(err) => {
            log::warn!("autoscene not on PATH ({err}); trying uvicorn directly");
            // macOS and most Linux distros ship `python3` with no bare `python`
            // shim, so trying only `python` strands the fallback path.
            for exe in ["python3", "python"] {
                let (out, err_io) = spawn_stdio();
                match Command::new(exe)
                    .args([
                        "-m",
                        "uvicorn",
                        "autoscene.api:app",
                        "--host",
                        API_HOST,
                        "--port",
                        &API_PORT.to_string(),
                    ])
                    .stdout(out)
                    .stderr(err_io)
                    .spawn()
                {
                    Ok(child) => {
                        log::info!(
                            "Started {} uvicorn (pid {}) — logs {}",
                            exe,
                            child.id(),
                            api_log_path().display()
                        );
                        return Some(child);
                    }
                    Err(e) => {
                        log::warn!("Could not spawn {exe}: {e}");
                    }
                }
            }
            log::error!("Failed to spawn AutoScene API (autoscene, python3, python all failed)");
            None
        }
    }
}

/// Non-blocking: wait in background so the UI thread stays responsive.
fn wait_for_api_async() {
    thread::spawn(|| {
        for i in 0..450 {
            if api_ready() {
                log::info!("AutoScene API TCP ready after ~{}ms", i * 200);
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
        log::error!(
            "AutoScene API did not become ready on {API_HOST}:{API_PORT}. See {}",
            api_log_path().display()
        );
    });
}

fn stop_backend(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<BackendProcess>() {
        if let Ok(mut guard) = state.0.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![probe_api_port]);

    // macOS gets a full native menu bar. Windows/Linux keep the custom
    // frameless chrome and no menu — Tauri only auto-installs one on macOS.
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .menu(mac_menu::build)
            .on_menu_event(mac_menu::on_event);
    }

    builder
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Spawn API without blocking the UI thread (avoids Windows "Not responding").
            let child = spawn_backend();
            if child.is_some() || !api_ready() {
                wait_for_api_async();
            }
            app.manage(BackendProcess(Mutex::new(child)));

            // Hard lock: fixed 1280x800, never maximize / fullscreen.
            // macOS is fully resizable and takes its bounds from
            // tauri.macos.conf.json, so this lock must not run there.
            #[cfg(not(target_os = "macos"))]
            {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_maximizable(false);
                    let _ = win.set_resizable(false);
                    let _ = win.set_fullscreen(false);
                    if win.is_maximized().unwrap_or(false) {
                        let _ = win.unmaximize();
                    }
                }
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                stop_backend(app_handle);
            }
        });
}
