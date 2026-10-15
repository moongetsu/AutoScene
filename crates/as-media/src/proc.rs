//! Child processes that can never outlive the app.
//!
//! Every ffmpeg/ffprobe the app starts goes through [`spawn`] or [`output`].
//! On Windows each child joins a job object created with
//! `KILL_ON_JOB_CLOSE`: the job's only handle belongs to this process, so when
//! it ends — normally, through the updater's `exit(0)`, or in a crash — the
//! system kills every child with it. Nothing is left holding `ffmpeg.exe`
//! open during an update or writing a half-finished file.

use std::io;
use std::process::{Child, Command, Output, Stdio};

/// Start `cmd` and tie the child's lifetime to ours.
pub fn spawn(cmd: &mut Command) -> io::Result<Child> {
    let child = cmd.spawn()?;
    track(&child);
    Ok(child)
}

/// Like [`Command::output`], with the child tied to our lifetime.
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    let child = spawn(cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))?;
    child.wait_with_output()
}

#[cfg(windows)]
mod job {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    pub struct Job(pub HANDLE);
    // SAFETY: a job handle is a process-wide kernel handle; the API is thread-safe.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    /// The process-wide kill-on-close job, created on first use.
    pub fn get() -> Option<&'static Job> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| {
            // SAFETY: plain Win32 calls with valid, initialised arguments. The
            // handle is deliberately never closed: closing it is what kills the
            // children, which must only happen when this process ends.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    tracing::warn!("couldn't create a job object; child processes may outlive the app");
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    tracing::warn!("couldn't configure the job object; child processes may outlive the app");
                    return None;
                }
                Some(Job(h))
            }
        })
        .as_ref()
    }
}

#[cfg(windows)]
fn track(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

    if let Some(job) = job::get() {
        // SAFETY: both handles are valid for the duration of the call.
        let ok = unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) };
        if ok == 0 {
            tracing::debug!(pid = child.id(), "couldn't add a child process to the job object");
        }
    }
}

#[cfg(not(windows))]
fn track(_child: &Child) {}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn children_join_the_kill_on_close_job() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;

        let mut child = spawn(Command::new("cmd").args(["/C", "ping -n 3 127.0.0.1 >NUL"])).unwrap();
        let job = job::get().expect("job object");
        let mut inside = 0;
        // SAFETY: valid process and job handles.
        unsafe { IsProcessInJob(child.as_raw_handle(), job.0, &mut inside) };
        assert_ne!(inside, 0, "child is in our kill-on-close job");
        child.kill().ok();
        child.wait().ok();
    }
}
