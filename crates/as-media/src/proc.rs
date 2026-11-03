//! Child processes that can never outlive the app.
//!
//! Every ffmpeg/ffprobe the app starts goes through [`spawn`] or [`output`].
//! On Windows each child joins a job object created with
//! `KILL_ON_JOB_CLOSE`: the job's only handle belongs to this process, so when
//! it ends — normally, through the updater's `exit(0)`, or in a crash — the
//! system kills every child with it. Nothing is left holding `ffmpeg.exe`
//! open during an update or writing a half-finished file.
//!
//! Plugins start processes of their own, which ffmpeg never does. A plugin run
//! goes through [`spawn_tree`] instead: its [`Tree`] ends together with
//! everything it started, and can be ended on its own while the app runs on.

use std::io;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::time::Duration;

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

/// A child and every process it starts, ended together.
///
/// On Windows the child runs in a job of its own, nested in the app's job, and
/// it joins that job before it runs its first instruction, so no process it
/// starts itself can slip out. Closing the job's handle kills what is left:
/// dropping the tree does, and so does the app ending in a crash. A process a
/// system service starts for it (through WMI, the Task Scheduler, a COM
/// server) is that service's child, in no job of ours, and outlives the tree.
///
/// On Unix the child leads a process group of its own. A process that leaves
/// the group (`setsid`, `setpgid`) is out of reach, and nothing kills the group
/// when the app crashes. `PR_SET_PDEATHSIG` can't help: it fires when the
/// thread that started the child ends, and the job workers are threads.
pub struct Tree {
    child: Child,
    #[cfg(windows)]
    job: std::os::windows::io::OwnedHandle,
    #[cfg(unix)]
    pgid: libc::pid_t,
    reaped: bool,
}

/// Start `cmd` as the root of a [`Tree`]. The caller sets stdio; the creation
/// flags (Windows) and the process group (Unix) are set here, and stay set on
/// `cmd`: on Windows it keeps only `CREATE_NO_WINDOW`.
pub fn spawn_tree(cmd: &mut Command) -> io::Result<Tree> {
    #[cfg(windows)]
    {
        let tree = spawn_suspended(cmd)?;
        // On an error the tree is dropped, which ends it.
        resume(tree.child.id())?;
        Ok(tree)
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let child = cmd.process_group(0).spawn()?;
        Ok(Tree { pgid: child.id() as libc::pid_t, child, reaped: false })
    }
}

impl Tree {
    // Only the pipes are handed out: on Unix, waiting on the root anywhere but
    // here would reap it while its group may still need a signal.

    /// The root's stdin, if it was piped. Only the first call gets it.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    /// The root's stdout, if it was piped. Only the first call gets it.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    /// The root's stderr, if it was piped. Only the first call gets it.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    /// The root process's id.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Whether the root process ended. On Unix it stays unreaped, which keeps
    /// its process group's id from being given to anyone else.
    pub fn has_exited(&mut self) -> io::Result<bool> {
        if self.reaped {
            return Ok(true);
        }
        #[cfg(windows)]
        {
            Ok(self.child.try_wait()?.is_some())
        }
        #[cfg(unix)]
        {
            group::exited(self.pgid)
        }
    }

    /// Ask the tree to stop, give the root up to `grace` to end, then kill
    /// every process left and reap the root. The root's own children are asked
    /// too, but they get no longer than the root does. Only Unix asks
    /// (SIGTERM): windowless processes on Windows have no polite signal, so
    /// there everything ends at once.
    pub fn kill_tree(&mut self, grace: Duration) -> io::Result<ExitStatus> {
        #[cfg(unix)]
        if !self.reaped {
            group::signal(self.pgid, libc::SIGTERM);
            // A grace too long to add up is no deadline at all.
            let deadline = std::time::Instant::now().checked_add(grace);
            loop {
                match self.has_exited() {
                    Ok(true) => break,
                    Ok(false) => {}
                    // Stop waiting; the kill below still ends everything.
                    Err(e) => {
                        tracing::debug!(error = %e, pid = self.child.id(), "couldn't tell whether a process ended");
                        break;
                    }
                }
                if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                    break;
                }
                std::thread::sleep(group::POLL);
            }
        }
        #[cfg(windows)]
        let _ = grace;
        self.end()
    }

    /// Once the root has ended: kill what it left running, reap it and return
    /// its status.
    pub fn finish(&mut self) -> io::Result<ExitStatus> {
        self.end()
    }

    /// Kill every process left at once, then reap the root.
    fn end(&mut self) -> io::Result<ExitStatus> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;

            // SAFETY: the job handle is ours and open.
            if unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } == 0 {
                let e = io::Error::last_os_error();
                tracing::warn!(error = %e, pid = self.child.id(), "couldn't end a process tree");
                // The root at least must end, or the wait below never returns;
                // closing the job's handle takes the rest.
                self.child.kill().ok();
            }
        }
        // Once the root is reaped, its group's id can belong to strangers.
        #[cfg(unix)]
        if !self.reaped {
            group::signal(self.pgid, libc::SIGKILL);
        }
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        if !self.reaped {
            self.end().ok();
        }
    }
}

/// Start `cmd` suspended, in a kill-on-close job of its own.
#[cfg(windows)]
fn spawn_suspended(cmd: &mut Command) -> io::Result<Tree> {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};

    let job = job::kill_on_close()?;
    // One call: `creation_flags` replaces the flags set before.
    let spawned = cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED).spawn();
    // Anything else `cmd` started would stay suspended for good.
    cmd.creation_flags(CREATE_NO_WINDOW);
    let mut child = spawned?;
    // The app's job first: a process can only join a job that is empty or
    // already around it, so the run's job then nests inside the app's.
    if let Some(app) = job::get() {
        // SAFETY: both handles are valid for the duration of the call.
        if unsafe { AssignProcessToJobObject(app.0, child.as_raw_handle()) } == 0 {
            tracing::debug!(pid = child.id(), "couldn't add a child process to the job object");
        }
    }
    // SAFETY: as above.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0 {
        let e = io::Error::last_os_error();
        // A child outside its job never runs.
        child.kill().ok();
        child.wait().ok();
        return Err(e);
    }
    Ok(Tree { child, job, reaped: false })
}

/// Let a process started suspended run: resume its only thread.
#[cfg(windows)]
fn resume(pid: u32) -> io::Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    // SAFETY: plain Win32 calls with valid, initialised arguments; each handle
    // is wrapped right after it is checked, so it is closed on every path.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snap = OwnedHandle::from_raw_handle(snap);
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut more = Thread32First(snap.as_raw_handle(), &mut entry) != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = OwnedHandle::from_raw_handle(thread);
                if ResumeThread(thread.as_raw_handle()) == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            more = Thread32Next(snap.as_raw_handle(), &mut entry) != 0;
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, format!("process {pid} has no thread to resume")))
}

#[cfg(windows)]
mod job {
    use std::io;
    use std::os::windows::io::{FromRawHandle, IntoRawHandle, OwnedHandle};
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
        JOB.get_or_init(|| match kill_on_close() {
            // The handle is deliberately never closed: closing it is what
            // kills the children, which must only happen when this process ends.
            Ok(job) => Some(Job(job.into_raw_handle())),
            Err(e) => {
                tracing::warn!(error = %e, "couldn't set up a job object; child processes may outlive the app");
                None
            }
        })
        .as_ref()
    }

    /// A new job that kills its processes when its last handle closes.
    pub fn kill_on_close() -> io::Result<OwnedHandle> {
        // SAFETY: plain Win32 calls with valid, initialised arguments; the
        // handle is wrapped right after it is checked.
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = OwnedHandle::from_raw_handle(h);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
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

/// A tree's process group. The root leads it and is reaped only after the
/// last signal: a group whose leader is unreaped keeps its id.
#[cfg(unix)]
mod group {
    use std::io;
    use std::time::Duration;

    /// How often [`super::Tree::kill_tree`] looks whether the root has ended.
    pub const POLL: Duration = Duration::from_millis(25);

    /// Whether the group's leader ended, leaving it unreaped.
    pub fn exited(pid: libc::pid_t) -> io::Result<bool> {
        // SAFETY: an all-zero siginfo_t is valid; waitid only writes into it,
        // and WNOWAIT leaves the child to be reaped later.
        unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
            if libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, flags) != 0 {
                return Err(io::Error::last_os_error());
            }
            // Left at zero while the child still runs. macOS may report a
            // stopped child even without WSTOPPED; that one hasn't ended.
            Ok(info.si_signo == libc::SIGCHLD
                && matches!(info.si_code, libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED))
        }
    }

    /// Send `sig` to every process of the group that is left.
    pub fn signal(pgid: libc::pid_t, sig: libc::c_int) {
        // SAFETY: a plain syscall; the caller hasn't reaped the leader, so the
        // group is still the tree's.
        if unsafe { libc::killpg(pgid, sig) } != 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::ESRCH) {
                tracing::debug!(error = %e, pgid, sig, "couldn't signal a process group");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Whether `done` turns true within `limit`.
    fn eventually(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while !done() {
            if start.elapsed() > limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        true
    }

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

    /// Whether `job` kills what it holds once its last handle closes.
    #[cfg(windows)]
    fn kills_on_close(job: windows_sys::Win32::Foundation::HANDLE) -> bool {
        use windows_sys::Win32::System::JobObjects::{
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, QueryInformationJobObject,
        };

        // SAFETY: a valid job handle and a buffer of the size passed.
        unsafe {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            let ok = QueryInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw mut info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );
            assert_ne!(ok, 0, "{}", io::Error::last_os_error());
            info.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE != 0
        }
    }

    /// Kill-on-close is what ends the trees when the app crashes, and no
    /// other test would notice it missing.
    #[cfg(windows)]
    #[test]
    fn jobs_kill_their_processes_when_closed() {
        use std::os::windows::io::AsRawHandle;

        assert!(kills_on_close(job::get().expect("job object").0), "the app's job");
        let tree = cmd("exit 0");
        assert!(kills_on_close(tree.job.as_raw_handle()), "a tree's job");
    }

    /// Run `script` as a tree under cmd.
    #[cfg(windows)]
    fn cmd(script: &str) -> Tree {
        spawn_tree(Command::new("cmd").args(["/C", script]).stdin(Stdio::null())).unwrap()
    }

    /// A second handle to the tree's job, to count its processes after the
    /// tree is gone.
    #[cfg(windows)]
    fn job_of(tree: &Tree) -> std::os::windows::io::OwnedHandle {
        tree.job.try_clone().unwrap()
    }

    /// The processes still running in `job`.
    #[cfg(windows)]
    fn running(job: &std::os::windows::io::OwnedHandle) -> u32 {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation, QueryInformationJobObject,
        };

        // SAFETY: a valid job handle and a buffer of the size passed.
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            let ok = QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&raw mut info).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );
            assert_ne!(ok, 0, "{}", io::Error::last_os_error());
            info.ActiveProcesses
        }
    }

    /// The pings running in `job`, the grandchildren the tests start. Counting
    /// processes alone would also count cmd's console host.
    #[cfg(windows)]
    fn pings(job: &std::os::windows::io::OwnedHandle) -> usize {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
        };
        use windows_sys::Win32::System::JobObjects::{JobObjectBasicProcessIdList, QueryInformationJobObject};

        /// JOBOBJECT_BASIC_PROCESS_ID_LIST with room for more than one id.
        #[repr(C)]
        struct Ids {
            assigned: u32,
            listed: u32,
            ids: [usize; 32],
        }

        // SAFETY: a valid job handle, buffers of the sizes passed, and a
        // snapshot handle that is checked before it is wrapped.
        unsafe {
            let mut list: Ids = std::mem::zeroed();
            let ok = QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicProcessIdList,
                (&raw mut list).cast(),
                size_of::<Ids>() as u32,
                std::ptr::null_mut(),
            );
            assert_ne!(ok, 0, "{}", io::Error::last_os_error());
            let in_job = &list.ids[..list.listed as usize];

            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            assert_ne!(snap, windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE, "{}", io::Error::last_os_error());
            let snap = OwnedHandle::from_raw_handle(snap);
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut found = 0;
            let mut more = Process32FirstW(snap.as_raw_handle(), &mut entry) != 0;
            while more {
                let name =
                    &entry.szExeFile[..entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len())];
                if in_job.contains(&(entry.th32ProcessID as usize))
                    && String::from_utf16_lossy(name).eq_ignore_ascii_case("ping.exe")
                {
                    found += 1;
                }
                more = Process32NextW(snap.as_raw_handle(), &mut entry) != 0;
            }
            found
        }
    }

    /// Run `script` as a tree; its first line is a grandchild's pid.
    #[cfg(unix)]
    fn shell(script: &str) -> (Tree, libc::pid_t) {
        use std::io::BufRead;

        let mut tree =
            spawn_tree(Command::new("sh").args(["-c", script]).stdin(Stdio::null()).stdout(Stdio::piped())).unwrap();
        let mut line = String::new();
        io::BufReader::new(tree.take_stdout().unwrap()).read_line(&mut line).unwrap();
        (tree, line.trim().parse().unwrap())
    }

    #[cfg(unix)]
    fn gone(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid, 0) == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) }
    }

    #[test]
    fn grandchildren_die_when_the_tree_is_killed() {
        #[cfg(windows)]
        {
            let mut tree = cmd("start /B ping -n 30 127.0.0.1 >NUL & ping -n 30 127.0.0.1 >NUL");
            let job = job_of(&tree);
            assert!(eventually(Duration::from_secs(5), || pings(&job) == 2), "the grandchildren run");
            let status = tree.kill_tree(Duration::from_secs(1)).unwrap();
            assert!(!status.success());
            assert!(eventually(Duration::from_secs(2), || running(&job) == 0), "{} still running", running(&job));
        }
        #[cfg(unix)]
        {
            let (mut tree, grandchild) = shell("sleep 30 & echo $!; wait");
            assert!(!gone(grandchild));
            let status = tree.kill_tree(Duration::from_secs(1)).unwrap();
            assert!(!status.success());
            assert!(eventually(Duration::from_secs(2), || gone(grandchild)), "the grandchild still runs");
        }
    }

    #[test]
    fn leftovers_die_when_the_tree_finishes() {
        #[cfg(windows)]
        {
            let mut tree = cmd("start /B ping -n 30 127.0.0.1 >NUL");
            let job = job_of(&tree);
            assert!(eventually(Duration::from_secs(5), || tree.has_exited().unwrap()));
            assert_eq!(pings(&job), 1, "the grandchild outlives the child");
            assert!(tree.finish().unwrap().success(), "the child's own status");
            assert!(eventually(Duration::from_secs(2), || running(&job) == 0), "{} still running", running(&job));
        }
        #[cfg(unix)]
        {
            let (mut tree, grandchild) = shell("sleep 30 & echo $!");
            assert!(eventually(Duration::from_secs(5), || tree.has_exited().unwrap()));
            assert!(!gone(grandchild), "the grandchild outlives the child");
            assert!(tree.finish().unwrap().success(), "the child's own status");
            assert!(eventually(Duration::from_secs(2), || gone(grandchild)), "the grandchild still runs");
        }
    }

    #[test]
    fn dropping_a_tree_kills_it() {
        #[cfg(windows)]
        {
            let tree = cmd("start /B ping -n 30 127.0.0.1 >NUL & ping -n 30 127.0.0.1 >NUL");
            let job = job_of(&tree);
            assert!(eventually(Duration::from_secs(5), || pings(&job) == 2), "the grandchildren run");
            drop(tree);
            assert!(eventually(Duration::from_secs(2), || running(&job) == 0), "{} still running", running(&job));
        }
        #[cfg(unix)]
        {
            let (tree, grandchild) = shell("sleep 30 & echo $!; wait");
            assert!(!gone(grandchild));
            drop(tree);
            assert!(eventually(Duration::from_secs(2), || gone(grandchild)), "the grandchild still runs");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_tree_is_asked_to_stop_before_it_is_killed() {
        // A root killed first would report SIGKILL, not its own exit.
        let (mut tree, grandchild) = shell("trap 'exit 0' TERM; sleep 30 & echo $!; wait");
        assert!(tree.kill_tree(Duration::from_secs(5)).unwrap().success(), "the root was asked first");
        assert!(eventually(Duration::from_secs(2), || gone(grandchild)), "the grandchild still runs");
    }

    #[cfg(unix)]
    #[test]
    fn a_tree_that_ignores_the_ask_is_killed_after_the_grace() {
        use std::os::unix::process::ExitStatusExt;

        let (mut tree, grandchild) = shell("trap '' TERM; sleep 30 & echo $!; wait");
        let grace = Duration::from_millis(500);
        let start = Instant::now();
        let status = tree.kill_tree(grace).unwrap();
        assert!(start.elapsed() >= grace, "killed after {:?}", start.elapsed());
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        assert!(eventually(Duration::from_secs(2), || gone(grandchild)), "the grandchild still runs");
    }

    #[cfg(windows)]
    #[test]
    fn a_tree_is_in_its_own_job_before_it_runs() {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;

        let dir = tempfile::tempdir().unwrap();
        let ran = dir.path().join("ran");
        let mut command = Command::new("cmd");
        command.raw_arg(format!("/C type nul > \"{}\"", ran.display()));
        let mut tree = spawn_suspended(&mut command).unwrap();
        let in_job = |job| {
            let mut inside = 0;
            // SAFETY: valid process and job handles.
            unsafe { IsProcessInJob(tree.child.as_raw_handle(), job, &mut inside) };
            inside != 0
        };
        assert!(in_job(tree.job.as_raw_handle()), "the child is in its own job");
        if let Some(app) = job::get() {
            assert!(in_job(app.0), "its job is nested in the app's");
        }
        std::thread::sleep(Duration::from_millis(300));
        assert!(!ran.exists(), "the child waits for its job");

        resume(tree.id()).unwrap();
        assert!(eventually(Duration::from_secs(5), || tree.has_exited().unwrap()));
        assert!(tree.finish().unwrap().success());
        assert!(ran.exists(), "the child ran once resumed");

        // The command is left without CREATE_SUSPENDED.
        std::fs::remove_file(&ran).unwrap();
        let mut again = command.spawn().unwrap();
        let ended = eventually(Duration::from_secs(5), || again.try_wait().unwrap().is_some());
        again.kill().ok();
        assert!(ended && ran.exists(), "the command's next child starts suspended");
    }
}
