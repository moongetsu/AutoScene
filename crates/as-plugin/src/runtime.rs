//! Running a process plugin: one [`run`] per action, spoken to with the
//! protocol ([`crate::protocol`]).
//!
//! The program starts by its path in the plugin's folder (a uv script's
//! too), in the plugin's data folder, with only the environment variables
//! of [`allowed_env`] and a few the run sets. It starts as the root of an
//! [`as_media::proc::Tree`], so everything it starts ends with it. Its
//! stdin is written and its stdout and stderr are read on threads of their
//! own from the start, so a plugin that floods a pipe or never reads its
//! stdin blocks only itself; lines are read no longer than
//! [`MAX_LINE_BYTES`]. What isn't a message goes to the plugin's log
//! ([`crate::log`]).
//!
//! A cancelled run is asked to stop with a cancel line, and ended with its
//! tree [`CANCEL_GRACE`] later. A plugin that is still running
//! [`EXIT_GRACE`] after its result or error line is ended too, and its line
//! counts. A run is never retried: what a plugin did (a post to a webhook,
//! say) would happen twice.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use as_core::CancelToken;
use as_media::proc;

use crate::error::{PluginError, PluginResult};
use crate::log::PluginLog;
use crate::manifest::{Launch, PROTOCOL, clean, cut};
use crate::protocol::{HostMessage, PluginMessage, RunMessage};

/// How long a plugin may take to stop after the cancel line.
pub const CANCEL_GRACE: Duration = Duration::from_secs(5);
/// How long the tree's processes may take to end once asked by the system
/// (SIGTERM, then SIGKILL; Windows ends them at once).
pub const TERM_GRACE: Duration = Duration::from_secs(2);
/// How long a plugin may run on after its result or error line.
pub const EXIT_GRACE: Duration = Duration::from_secs(10);
/// How often a run looks at its plugin while it hears nothing.
pub const TICK: Duration = Duration::from_millis(100);
pub const MAX_LINE_BYTES: usize = 64 << 10;
pub const MAX_STAGE_CHARS: usize = 80;
pub const MAX_MESSAGE_CHARS: usize = 300;
/// stderr lines kept for the message of a crash.
pub const STDERR_TAIL: usize = 20;

/// The longest error detail a plugin's line hands to the app's log.
const MAX_DETAIL_CHARS: usize = 2000;
/// The longest stderr line kept in a crash's detail.
const TAIL_LINE_CHARS: usize = 500;
/// Lines read but not yet handled. A plugin that writes faster than its
/// log takes them waits on its pipe.
const QUEUE: usize = 256;
/// How long a run waits for the rest of its plugin's output once the tree
/// has ended (a process that left it may hold a pipe open).
const DRAIN: Duration = Duration::from_secs(2);

/// One run of a plugin.
pub struct RunSpec {
    /// The plugin's name, as messages show it.
    pub name: String,
    /// The plugin's folder.
    pub dir: PathBuf,
    pub launch: Launch,
    /// The plugin's own folder for files; the run starts there.
    pub data_dir: PathBuf,
    /// The plugin's log file.
    pub log_path: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub message: RunMessage,
}

/// Runs a plugin's action to its end: the message of its result line, if
/// it gave one. `report` gets its progress: the stage (cleaned, at most
/// [`MAX_STAGE_CHARS`]; the plugin's name if it named none) and how far it
/// is, if known.
///
/// Fails with the plugin's own error line ([`PluginError::Failed`]), with
/// [`PluginError::Crashed`] when it exits with an error and said neither,
/// and with [`PluginError::Cancelled`] when `cancel` fires before it gave
/// its result (a run cancelled before it starts starts nothing). A cancel
/// after that only stops the wait for it to exit.
///
/// The run line's protocol, data folder and FFmpeg paths are the run's own,
/// whatever `message` held, so they match the environment's.
pub fn run(spec: RunSpec, cancel: &CancelToken, report: &dyn Fn(&str, Option<f64>)) -> PluginResult<Option<String>> {
    if cancel.is_cancelled() {
        return Err(PluginError::Cancelled);
    }
    let RunSpec { name, dir, launch, data_dir, log_path, ffmpeg, ffprobe, mut message } = spec;
    let mut cmd = command(&name, &dir, &launch, find_uv)?;
    fs::create_dir_all(&data_dir)?;
    let data_dir = std::path::absolute(data_dir)?;
    message.protocol = PROTOCOL;
    message.plugin.data_dir = data_dir.to_string_lossy().into_owned();
    message.tools.ffmpeg = ffmpeg.to_string_lossy().into_owned();
    message.tools.ffprobe = ffprobe.to_string_lossy().into_owned();
    let mut log = PluginLog::open(&log_path, &message.run_id, &message.action.id)?;
    cmd.current_dir(&data_dir)
        .env_clear()
        .envs(env::vars_os().filter(|(name, _)| allowed_env(name)))
        .env("AUTOSCENE_FFMPEG", &ffmpeg)
        .env("AUTOSCENE_FFPROBE", &ffprobe)
        .env("AUTOSCENE_PLUGIN_DATA", &data_dir)
        .env("AUTOSCENE_PLUGIN_ID", message.plugin.id.as_str())
        .env("AUTOSCENE_PROTOCOL", PROTOCOL.to_string())
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut tree = proc::spawn_tree(&mut cmd).inspect_err(|e| {
        log.note("host", &format!("couldn't start: {e}"));
        tracing::warn!(error = %e, plugin = %message.plugin.id.as_str(), "a plugin couldn't start");
    })?;

    let (tx, rx) = mpsc::sync_channel(QUEUE);
    read_stdout(tree.take_stdout().expect("piped stdout"), tx.clone());
    read_stderr(tree.take_stderr().expect("piped stderr"), tx);
    let mut to_plugin = Some(writer(tree.take_stdin().expect("piped stdin")));
    if let Some(tx) = &to_plugin {
        tx.send(HostMessage::Run(Box::new(message)).line()).ok();
    }

    let shown = sanitize(&name, MAX_STAGE_CHARS);
    let mut heard = Heard { name: &shown, report, log, outcome: None, tail: VecDeque::new(), open: 2 };
    let mut cancel_at: Option<Instant> = None;
    // The tree's status when the run had to end it, else None once it ended.
    let ended = loop {
        if cancel_at.is_none() && cancel.is_cancelled() {
            cancel_at = Some(Instant::now());
            if heard.outcome.is_none()
                && let Some(tx) = &to_plugin
            {
                tx.send(HostMessage::Cancel.line()).ok();
                heard.log.note("host", "asked to cancel");
            }
        }
        let overdue = match (cancel_at, heard.outcome.as_ref().map(|o| o.at)) {
            // Asked to stop once it had said its last word: nothing to wait for.
            (Some(c), Some(o)) if c >= o => Some("cancelled after its result"),
            (Some(c), _) if c.elapsed() >= CANCEL_GRACE => Some("still running after cancel"),
            (None, Some(o)) if o.elapsed() >= EXIT_GRACE => Some("still running after its result"),
            _ => None,
        };
        if let Some(why) = overdue {
            heard.log.note("host", &format!("{why}; ended"));
            break Some(tree.kill_tree(TERM_GRACE)?);
        }
        match rx.recv_timeout(TICK) {
            Ok(m) => heard.take(m),
            Err(RecvTimeoutError::Timeout) => {}
            // Both pipes are closed; the process may still run.
            Err(RecvTimeoutError::Disconnected) => thread::sleep(TICK),
        }
        if heard.outcome.is_some() {
            // Its stdin closes: the run has nothing more to say.
            to_plugin = None;
        }
        if tree.has_exited()? {
            break None;
        }
    };
    let status = match ended {
        Some(status) => status,
        None => tree.finish()?,
    };
    // The reader threads are left to end on their own: the tree's end
    // closed their pipes, unless a process that left the tree holds one.
    // That reader ends when the process closes its pipe or writes to it
    // again. The run waits for none of it past DRAIN, however fast it comes.
    let until = Instant::now() + DRAIN;
    while heard.open > 0 && Instant::now() < until {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(m) => heard.take(m),
            Err(_) => break,
        }
    }
    heard.log.note("host", &status.to_string());
    decide(name, status, cancel_at, heard)
}

/// Who wins once the tree has ended: a cancel before any result or error,
/// then the plugin's line, then its exit status.
fn decide(
    name: String,
    status: ExitStatus,
    cancel_at: Option<Instant>,
    heard: Heard<'_>,
) -> PluginResult<Option<String>> {
    if cancel_at.is_some_and(|c| heard.outcome.as_ref().is_none_or(|o| c < o.at)) {
        return Err(PluginError::Cancelled);
    }
    match heard.outcome.map(|o| o.said) {
        Some(Said::Done(message)) => Ok(message),
        Some(Said::Failed { message, detail }) => Err(PluginError::Failed { message, detail }),
        None if status.success() => Ok(None),
        None => {
            let tail = Vec::from(heard.tail).join("\n");
            let detail = if tail.is_empty() { status.to_string() } else { format!("{status}; {tail}") };
            Err(PluginError::Crashed { name, detail })
        }
    }
}

/// `text` as the app shows it: without control characters or invisible
/// ones, whitespace collapsed, at most `max_chars` (the last an ellipsis).
pub fn sanitize(text: &str, max_chars: usize) -> String {
    cut(&clean(text), max_chars)
}

/// Whether the environment variable `name` reaches plugins. Every other one
/// is left out: the app's own (`AUTOSCENE_DATA_DIR`, `RUST_LOG`) and
/// whatever secrets the person keeps in theirs. What is passed on is what
/// programs need to work at all (Windows can't start a socket without
/// `SystemRoot`; Python and uv read `PROCESSOR_ARCHITECTURE`), find their
/// files and the person's language, reach the network through its proxy
/// and see the GPU. Windows names are compared in any case.
pub fn allowed_env(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else { return false };
    #[cfg(windows)]
    let (is, starts) = (
        |n: &str| name.eq_ignore_ascii_case(n),
        |p: &str| name.get(..p.len()).is_some_and(|head| head.eq_ignore_ascii_case(p)),
    );
    #[cfg(unix)]
    let (is, starts) = (|n: &str| name == n, |p: &str| name.starts_with(p));
    EVERYWHERE.iter().chain(HERE).any(|n| is(n)) || PREFIXES.iter().any(|p| starts(p))
}

const EVERYWHERE: &[&str] = &[
    "PATH",
    "LANG",
    "TZ",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "CUDA_VISIBLE_DEVICES",
];

#[cfg(windows)]
const HERE: &[&str] = &[
    "SystemRoot",
    "windir",
    "SystemDrive",
    "PATHEXT",
    "COMSPEC",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_ARCHITEW6432",
    "OS",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
];

#[cfg(unix)]
const HERE: &[&str] = &["HOME", "USER", "LOGNAME", "TMPDIR", "DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"];

#[cfg(windows)]
const PREFIXES: &[&str] = &["CUDA_PATH"];

#[cfg(unix)]
const PREFIXES: &[&str] = &["CUDA_PATH", "LC_", "XDG_"];

#[cfg(windows)]
const UV: &str = "uv.exe";
#[cfg(unix)]
const UV: &str = "uv";

/// Where uv is: on the PATH (its absolute folders, but the Store's app
/// aliases), else in a folder its installers use. Apps started from the
/// macOS Dock or Finder don't get the shell's PATH, so the folders matter.
pub fn find_uv() -> Option<PathBuf> {
    let home = |var: &str| env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute());
    #[cfg(windows)]
    let folders = {
        let (profile, local) = (home("USERPROFILE"), home("LOCALAPPDATA"));
        [
            profile.as_ref().map(|p| p.join(".local").join("bin")),
            profile.as_ref().map(|p| p.join(".cargo").join("bin")),
            local.map(|p| p.join("Microsoft").join("WinGet").join("Links")),
        ]
    };
    #[cfg(unix)]
    let folders = {
        let home = home("HOME");
        [
            home.as_ref().map(|p| p.join(".local").join("bin")),
            home.as_ref().map(|p| p.join(".cargo").join("bin")),
            Some(PathBuf::from("/opt/homebrew/bin")),
            Some(PathBuf::from("/usr/local/bin")),
        ]
    };
    find_uv_in(env::var_os("PATH").as_deref(), folders.into_iter().flatten())
}

fn find_uv_in(path: Option<&OsStr>, folders: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let on_path = path
        .into_iter()
        .flat_map(env::split_paths)
        .filter(|d| d.is_absolute() && !d.to_string_lossy().to_ascii_lowercase().contains("windowsapps"));
    // One not marked to run is skipped, as a shell skips it.
    on_path.chain(folders).map(|d| d.join(UV)).find(|f| f.is_file() && runnable(f))
}

/// Whether the file may be run: on Linux and macOS, whether it is marked to.
#[cfg(unix)]
fn runnable(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    file.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn runnable(_: &Path) -> bool {
    true
}

/// The command that starts `launch` from the plugin's folder `dir`, by its
/// absolute path: neither the PATH nor the run's folder decides what runs.
/// `uv` finds uv, for a script. Its errors say what is wrong, not what to
/// do: only the host knows whether the plugin can be reinstalled.
fn command(name: &str, dir: &Path, launch: &Launch, uv: impl FnOnce() -> Option<PathBuf>) -> PluginResult<Command> {
    let entry = || -> PluginResult<PathBuf> {
        let entry = std::path::absolute(launch.entry_in(dir))?;
        if !entry.is_file() {
            return Err(PluginError::Invalid(format!("{name} can't start: one of its files is missing.")));
        }
        Ok(entry)
    };
    match launch {
        Launch::Exe { args, .. } => {
            let program = entry()?;
            if !runnable(&program) {
                return Err(PluginError::Invalid(format!("{name} can't start: its program isn't marked as runnable.")));
            }
            let mut cmd = Command::new(program);
            cmd.args(args);
            Ok(cmd)
        }
        Launch::UvScript { .. } => {
            let uv = uv().ok_or(PluginError::UvMissing)?;
            let mut cmd = Command::new(uv);
            cmd.args(["run", "--script", "--locked"]).arg(entry()?);
            Ok(cmd)
        }
    }
}

/// What a reader thread hands the run.
enum Msg {
    Plugin(PluginMessage),
    /// A stdout line that isn't a message.
    Stdout(String),
    Stderr(String),
    /// A pipe reached its end.
    Closed,
}

/// The plugin's last word.
enum Said {
    Done(Option<String>),
    Failed { message: String, detail: Option<String> },
}

struct Outcome {
    said: Said,
    at: Instant,
}

/// What a run has heard from its plugin so far.
struct Heard<'a> {
    /// The plugin's name, as a stage shows it: the stage of a progress line
    /// without one.
    name: &'a str,
    report: &'a dyn Fn(&str, Option<f64>),
    log: PluginLog,
    /// The first result or error line; later ones go to the log only.
    outcome: Option<Outcome>,
    tail: VecDeque<String>,
    /// Pipes not yet at their end.
    open: usize,
}

impl Heard<'_> {
    fn take(&mut self, m: Msg) {
        match m {
            Msg::Plugin(PluginMessage::Progress { fraction, stage }) => {
                let stage = stage.map(|s| sanitize(&s, MAX_STAGE_CHARS)).filter(|s| !s.is_empty());
                let fraction = fraction.filter(|f| f.is_finite()).map(|f| f.clamp(0.0, 1.0));
                (self.report)(stage.as_deref().unwrap_or(self.name), fraction);
            }
            Msg::Plugin(PluginMessage::Log { level, message }) => self.log.line(level.name(), &message),
            Msg::Plugin(PluginMessage::Result { message }) => {
                self.log_said("result", message.as_deref().unwrap_or("(no message)"));
                let message = message.map(|m| sanitize(&m, MAX_MESSAGE_CHARS)).filter(|m| !m.is_empty());
                self.said(Said::Done(message));
            }
            Msg::Plugin(PluginMessage::Error { message, detail }) => {
                let logged = match &detail {
                    Some(detail) => format!("{message} ({detail})"),
                    None => message.clone(),
                };
                self.log_said("failed", &logged);
                let mut message = sanitize(&message, MAX_MESSAGE_CHARS);
                if message.is_empty() {
                    message = format!("{} failed.", self.name);
                }
                let detail = detail.map(|d| sanitize(&d, MAX_DETAIL_CHARS)).filter(|d| !d.is_empty());
                self.said(Said::Failed { message, detail });
            }
            Msg::Stdout(line) => self.log.line("stdout", &line),
            Msg::Stderr(line) => {
                self.log.line("stderr", &line);
                if self.tail.len() == STDERR_TAIL {
                    self.tail.pop_front();
                }
                // Cleaned: the app's log quotes it in a crash's detail.
                self.tail.push_back(sanitize(&line, TAIL_LINE_CHARS));
            }
            Msg::Closed => self.open = self.open.saturating_sub(1),
        }
    }

    /// Logs a result or error line: the one that counts as the host's word,
    /// so it outlasts a flood, and later ones as the plugin's output.
    fn log_said(&mut self, tag: &str, text: &str) {
        if self.outcome.is_none() {
            self.log.note(tag, text);
        } else {
            self.log.line(tag, text);
        }
    }

    fn said(&mut self, said: Said) {
        if self.outcome.is_none() {
            self.outcome = Some(Outcome { said, at: Instant::now() });
        }
    }
}

/// Writes each line it's sent to the plugin's stdin, on a thread of its own:
/// a plugin that doesn't read its stdin blocks that thread, never the run.
/// Dropping the sender closes the pipe; a failed write ends the thread.
fn writer(mut stdin: ChildStdin) -> mpsc::Sender<String> {
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for mut line in rx {
            line.push('\n');
            if stdin.write_all(line.as_bytes()).and_then(|()| stdin.flush()).is_err() {
                return;
            }
        }
    });
    tx
}

fn read_stdout(stdout: impl Read + Send + 'static, tx: SyncSender<Msg>) {
    thread::spawn(move || {
        for (line, was_cut) in capped_lines(stdout, MAX_LINE_BYTES) {
            // A cut line is no message: its end is gone.
            let m = if !was_cut && let Some(m) = PluginMessage::read(&line) {
                Msg::Plugin(m)
            } else {
                Msg::Stdout(marked(line, was_cut))
            };
            if tx.send(m).is_err() {
                return;
            }
        }
        tx.send(Msg::Closed).ok();
    });
}

fn read_stderr(stderr: impl Read + Send + 'static, tx: SyncSender<Msg>) {
    thread::spawn(move || {
        for (line, was_cut) in capped_lines(stderr, MAX_LINE_BYTES) {
            if tx.send(Msg::Stderr(marked(line, was_cut))).is_err() {
                return;
            }
        }
        tx.send(Msg::Closed).ok();
    });
}

fn marked(mut line: String, was_cut: bool) -> String {
    if was_cut {
        line.push_str(" [cut]");
    }
    line
}

/// The lines of a pipe, whatever their encoding (a stray byte must not end
/// the reader: the plugin would block on a full pipe), without their line
/// ends (`\r\n` too), and each with whether it was cut: a line keeps its
/// first `max` bytes, and the rest of it is read and dropped, so a plugin
/// that never ends a line can't fill the memory.
fn capped_lines(r: impl Read, max: usize) -> impl Iterator<Item = (String, bool)> {
    let mut r = BufReader::new(r);
    std::iter::from_fn(move || {
        let (mut line, mut was_cut) = (Vec::new(), false);
        loop {
            let buf = match r.fill_buf() {
                Ok(buf) => buf,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            };
            if buf.is_empty() {
                // The end of the pipe; a last line without its newline counts.
                return (!line.is_empty() || was_cut).then(|| decoded(line, was_cut, max));
            }
            let (len, ends) = match buf.iter().position(|&b| b == b'\n') {
                Some(i) => (i, true),
                None => (buf.len(), false),
            };
            // A byte more, for the \r of a \r\n.
            let room = max + 1 - line.len();
            was_cut |= len > room;
            line.extend_from_slice(&buf[..len.min(room)]);
            r.consume(len + usize::from(ends));
            if ends {
                return Some(decoded(line, was_cut, max));
            }
        }
    })
}

fn decoded(mut line: Vec<u8>, mut was_cut: bool, max: usize) -> (String, bool) {
    while line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.len() > max {
        line.truncate(max);
        was_cut = true;
    }
    (String::from_utf8_lossy(&line).into_owned(), was_cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_uv_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.py"), "print()").unwrap();
        let launch = Launch::UvScript { script: "main.py".into() };
        let e = command("Hello", dir.path(), &launch, || None).unwrap_err();
        assert!(matches!(e, PluginError::UvMissing), "{e:?}");
        assert_eq!(
            e.to_string(),
            "This plugin needs uv, which isn't installed. Install it from docs.astral.sh/uv, then try again."
        );
    }

    #[test]
    fn uv_scripts_start_through_uv_by_their_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src").join("main.py"), "print()").unwrap();
        let uv = dir.path().join("tools").join(UV);
        let launch = Launch::UvScript { script: "src/main.py".into() };
        let cmd = command("Hello", dir.path(), &launch, || Some(uv.clone())).unwrap();
        assert_eq!(cmd.get_program(), uv.as_os_str());
        let script = dir.path().join("src").join("main.py");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, [OsStr::new("run"), OsStr::new("--script"), OsStr::new("--locked"), script.as_os_str()]);
    }

    #[test]
    fn programs_start_by_their_path_in_the_plugin_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("bin")).unwrap();
        let program = dir.path().join("bin").join("tool.exe");
        let launch = Launch::Exe { program: "bin/tool.exe".into(), args: vec!["--quiet".into()] };
        let e = command("Hello", dir.path(), &launch, || None).unwrap_err();
        assert_eq!(e.to_string(), "Hello can't start: one of its files is missing.");
        fs::write(&program, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let e = command("Hello", dir.path(), &launch, || None).unwrap_err();
            assert_eq!(e.to_string(), "Hello can't start: its program isn't marked as runnable.");
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let cmd = command("Hello", dir.path(), &launch, || None).unwrap();
        assert_eq!(cmd.get_program(), program.as_os_str());
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["--quiet"]);
        // A script's folder doesn't count as its file.
        fs::create_dir(dir.path().join("main.py")).unwrap();
        let launch = Launch::UvScript { script: "main.py".into() };
        assert!(matches!(
            command("Hello", dir.path(), &launch, || Some(program.clone())),
            Err(PluginError::Invalid(_))
        ));
    }

    #[test]
    fn uv_is_looked_for_on_the_path_then_in_its_install_folders() {
        let dir = tempfile::tempdir().unwrap();
        let folder = |name: &str, with_uv: bool| {
            let d = dir.path().join(name);
            fs::create_dir_all(&d).unwrap();
            if with_uv {
                fs::write(d.join(UV), "").unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(d.join(UV), fs::Permissions::from_mode(0o755)).unwrap();
                }
            }
            d
        };
        let empty = folder("empty", false);
        let store = folder("Microsoft/WindowsApps", true);
        let on_path = folder("on-path", true);
        let installed = folder("installed", true);
        let path = env::join_paths([&store, Path::new("relative"), &empty, &on_path]).unwrap();
        assert_eq!(find_uv_in(Some(&path), [installed.clone()]), Some(on_path.join(UV)));
        let path = env::join_paths([&store, &empty]).unwrap();
        assert_eq!(find_uv_in(Some(&path), [empty.clone(), installed.clone()]), Some(installed.join(UV)));
        assert_eq!(find_uv_in(None, [empty.clone()]), None);
        #[cfg(unix)]
        {
            // A uv not marked to run is skipped, as a shell skips it.
            use std::os::unix::fs::PermissionsExt;
            let stale = folder("stale", true);
            fs::set_permissions(stale.join(UV), fs::Permissions::from_mode(0o644)).unwrap();
            let path = env::join_paths([&stale, &on_path]).unwrap();
            assert_eq!(find_uv_in(Some(&path), [installed.clone()]), Some(on_path.join(UV)));
            assert_eq!(find_uv_in(None, [stale, installed.clone()]), Some(installed.join(UV)));
        }
        // A uv that is a folder is no uv.
        fs::create_dir(empty.join(UV)).unwrap();
        assert_eq!(find_uv_in(None, [empty]), None);
    }

    #[test]
    fn only_listed_variables_pass() {
        let allowed = |name: &str| allowed_env(OsStr::new(name));
        for name in
            ["PATH", "LANG", "TZ", "https_proxy", "HTTPS_PROXY", "CUDA_VISIBLE_DEVICES", "CUDA_PATH", "CUDA_PATH_V13_0"]
        {
            assert!(allowed(name), "{name}");
        }
        for name in [
            "AUTOSCENE_DATA_DIR",
            "AUTOSCENE_FFMPEG_DIR",
            "AUTOSCENE_TEST_SECRET",
            "RUST_LOG",
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "PATH2",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "PYTHONPATH",
            "",
        ] {
            assert!(!allowed(name), "{name}");
        }
        #[cfg(windows)]
        {
            for name in ["Path", "SYSTEMROOT", "windir", "ProgramFiles(x86)", "PROCESSOR_ARCHITECTURE", "cuda_path_v12"]
            {
                assert!(allowed(name), "{name}");
            }
            for name in ["HOME", "LC_ALL", "XDG_RUNTIME_DIR"] {
                assert!(!allowed(name), "{name}");
            }
        }
        #[cfg(unix)]
        {
            for name in ["HOME", "LC_ALL", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "WAYLAND_DISPLAY"] {
                assert!(allowed(name), "{name}");
            }
            for name in ["path", "SystemRoot", "Http_Proxy", "lc_all"] {
                assert!(!allowed(name), "{name}");
            }
        }
    }

    #[test]
    fn long_lines_are_cut_and_odd_bytes_kept() {
        let input = [
            b"short\r\n".as_slice(),
            &[b'x'; 40],
            b"\n",
            b"caf\xC3\xA9 \xFF\n",
            b"\n",
            // Just the limit, ended as Windows ends lines, and a byte over it.
            &[b'w'; 16],
            b"\r\n",
            &[b'v'; 17],
            b"\n",
            b"last",
        ]
        .concat();
        let lines: Vec<_> = capped_lines(input.as_slice(), 16).collect();
        assert_eq!(
            lines,
            [
                ("short".into(), false),
                ("x".repeat(16), true),
                ("café \u{FFFD}".into(), false),
                (String::new(), false),
                ("w".repeat(16), false),
                ("v".repeat(16), true),
                ("last".into(), false),
            ]
        );
        // A line that never ends is cut too, at the pipe's end.
        let endless = vec![b'y'; 100_000];
        let lines: Vec<_> = capped_lines(endless.as_slice(), 16).collect();
        assert_eq!(lines, [("y".repeat(16), true)]);
    }

    #[test]
    fn shown_text_is_cleaned_and_cut() {
        assert_eq!(sanitize("  Posting\u{7}  clips\n\tto\u{202E} Discord ", 80), "Posting clips to Discord");
        assert_eq!(sanitize("abcdefghij", 5), "abcd…");
        assert_eq!(sanitize("\u{200B}\n", 5), "");
    }
}
