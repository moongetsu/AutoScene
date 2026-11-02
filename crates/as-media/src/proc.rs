//! Child processes that can never outlive the app.
//!
//! Every ffmpeg/ffprobe the app starts goes through [`spawn`] or [`output`].
//! On Windows each child joins a job object created with
//! `KILL_ON_JOB_CLOSE`: the job's only handle belongs to this process, so when
//! it ends — normally, through the updater's `exit(0)`, or in a crash — the
//! system kills every child with it. Nothing is left holding `ffmpeg.exe`
//! open during an update or writing a half-finished file.

use std::io;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use as_core::CancelToken;

/// Start `cmd` and tie the child's lifetime to ours.
///
/// On Unix, child processes are placed in their own process group (`process_group(0)`)
/// so that signal propagation (e.g. SIGTERM/SIGKILL) cleanly terminates any child
/// trees without leaving orphaned processes.
pub fn spawn(cmd: &mut Command) -> io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn()?;
    track(&child);
    Ok(child)
}

/// Terminate a child process and its entire process tree if running.
///
/// On Unix, sends SIGTERM then SIGKILL to `-pgid` to clean up sub-workers.
/// On Windows, the process is already bound to the kill-on-close job object.
pub fn kill_tree(child: &mut Child) -> io::Result<()> {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // Kill the whole process group (-pid)
        unsafe {
            libc_kill_pgid(pid);
        }
    }
    child.kill()
}

#[cfg(unix)]
unsafe fn libc_kill_pgid(pid: i32) {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGTERM: i32 = 15;
    const SIGKILL: i32 = 9;
    // Attempt graceful SIGTERM first, followed by SIGKILL
    unsafe {
        kill(-pid, SIGTERM);
        kill(-pid, SIGKILL);
    }
}

/// Run a command to completion while respecting cancellation and timeouts.
///
/// If `cancel` fires or `timeout` expires, terminates the process tree immediately.
pub fn run_supervised(
    cmd: &mut Command,
    cancel: &CancelToken,
    timeout: Option<Duration>,
) -> io::Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = spawn(cmd)?;
    let started = Instant::now();

    loop {
        if cancel.is_cancelled() {
            let _ = kill_tree(&mut child);
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::Interrupted, "operation cancelled"));
        }

        if let Some(limit) = timeout
            && started.elapsed() > limit
        {
            let _ = kill_tree(&mut child);
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "process execution timed out"));
        }

        match child.try_wait()? {
            Some(status) => {
                let mut stdout_buf = Vec::new();
                let mut stderr_buf = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = std::io::copy(&mut out, &mut stdout_buf);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = std::io::copy(&mut err, &mut stderr_buf);
                }
                return Ok((status, stdout_buf, stderr_buf));
            }
            None => {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
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

    #[cfg(unix)]
    #[test]
    fn supervised_process_terminates_on_cancel() {
        let cancel = CancelToken::new();
        let cancel_clone = cancel.clone();

        // Spawn a thread that cancels shortly after launch
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel_clone.cancel();
        });

        let mut cmd = Command::new("sleep");
        cmd.arg("10");

        let res = run_supervised(&mut cmd, &cancel, Some(Duration::from_secs(5)));
        assert!(res.is_err(), "expected cancelled process to return error");
        let err = res.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    }

    #[cfg(unix)]
    #[test]
    fn supervised_process_times_out() {
        let cancel = CancelToken::new();
        let mut cmd = Command::new("sleep");
        cmd.arg("10");

        let res = run_supervised(&mut cmd, &cancel, Some(Duration::from_millis(150)));
        assert!(res.is_err(), "expected timed out process to return error");
        let err = res.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }
}
