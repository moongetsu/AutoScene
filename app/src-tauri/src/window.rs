//! The main window. On Windows it is created here rather than from the
//! config (`create: false` in `tauri.windows.conf.json`): Windows 11 gets that
//! config's transparent window with Mica behind the rail; Windows 10 has no
//! Mica, so its window stays solid — a transparent one would show the desktop
//! wherever the page hasn't painted yet (start-up, live resize).

use tauri::utils::config::Color;
use tauri::{App, Manager, WebviewWindowBuilder};

/// Create the main window unless the config already did (macOS, Linux).
pub fn create_main(app: &App) -> tauri::Result<()> {
    if app.get_webview_window("main").is_some() {
        return Ok(());
    }
    let Some(mut config) = app.config().app.windows.iter().find(|w| w.label == "main").cloned() else {
        return Ok(());
    };
    if !has_mica() {
        config.transparent = false;
        config.no_redirection_bitmap = false;
        config.window_effects = None;
        config.background_color = Some(Color(9, 9, 11, 255));
    }
    WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?;
    Ok(())
}

/// Windows 11 (build 22000 and up) draws Mica.
#[cfg(windows)]
fn has_mica() -> bool {
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut v = OSVERSIONINFOW { dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
    // SAFETY: `v` is a valid OSVERSIONINFOW whose size field is set, as the call requires.
    let status = unsafe { RtlGetVersion(&mut v) };
    status == 0 && v.dwBuildNumber >= 22000
}

/// Elsewhere the config decides.
#[cfg(not(windows))]
fn has_mica() -> bool {
    true
}
