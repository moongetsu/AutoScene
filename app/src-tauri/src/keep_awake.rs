//! Keep the computer from going to sleep while long work (analysis) runs.
//! The display may still turn off.

/// Holds the PC awake until dropped. Create and drop it on the same thread
/// (Windows tracks the request per thread).
pub struct KeepAwake {
    #[cfg(target_os = "macos")]
    child: Option<std::process::Child>,
}

pub fn hold() -> KeepAwake {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
        // SAFETY: plain flag call; undone in Drop on the same thread.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        KeepAwake {}
    }
    #[cfg(target_os = "macos")]
    {
        // `caffeinate -i` prevents idle sleep while it runs; `-w` ends it with us.
        let child = std::process::Command::new("caffeinate")
            .args(["-i", "-w", &std::process::id().to_string()])
            .spawn()
            .map_err(|e| log::debug!("caffeinate: {e}"))
            .ok();
        KeepAwake { child }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    KeepAwake {}
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
            // SAFETY: as above; clears this thread's request.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
        #[cfg(target_os = "macos")]
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
