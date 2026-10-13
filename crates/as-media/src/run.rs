//! Running ffmpeg with progress, cancellation and useful error messages.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use as_core::CancelToken;

use crate::{MediaError, MediaResult, Tools};

/// stderr lines kept for error messages.
const TAIL_LINES: usize = 12;

/// Parsed `-progress` sample.
#[derive(Clone, Debug, Default)]
pub struct FfProgress {
    /// Output position in seconds.
    pub out_time: f64,
    /// Frames written so far.
    pub frame: u64,
    /// Encoding speed relative to real time.
    pub speed: f64,
}

/// Run ffmpeg with `args` to completion. Adds `-progress pipe:1 -nostats`,
/// kills the process when `cancel` fires, and returns the tail of stderr in
/// the error on failure.
pub fn run_ffmpeg<I, S>(
    tools: &Tools,
    args: I,
    cancel: &CancelToken,
    on_progress: impl FnMut(&FfProgress),
) -> MediaResult<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_ffmpeg_counting(tools, args, cancel, on_progress, None).map(|_| ())
}

/// [`run_ffmpeg`], also counting the stderr lines that contain `pattern`
/// anywhere in the run (not just in the tail kept for errors).
pub fn run_ffmpeg_counting<I, S>(
    tools: &Tools,
    args: I,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(&FfProgress),
    pattern: Option<&'static str>,
) -> MediaResult<usize>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = tools.ffmpeg();
    cmd.args(["-progress", "pipe:1", "-nostats"]).args(args);

    tracing::debug!(cmd = ?cmd, "ffmpeg");
    let mut child = crate::proc::spawn(cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()))?;

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = mpsc::channel::<FfProgress>();
    let progress_thread = thread::spawn(move || {
        let mut p = FfProgress::default();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k {
                "out_time_us" => p.out_time = v.trim().parse::<f64>().unwrap_or(0.0) / 1e6,
                "frame" => p.frame = v.trim().parse().unwrap_or(p.frame),
                "speed" => p.speed = v.trim().trim_end_matches('x').parse().unwrap_or(0.0),
                "progress" => {
                    let _ = tx.send(p.clone());
                }
                _ => {}
            }
        }
    });
    // Keep only the tail of stderr; long jobs can log a lot.
    let stderr_thread = thread::spawn(move || {
        let (mut tail, mut matches) = (VecDeque::with_capacity(TAIL_LINES), 0);
        let mut reader = BufReader::new(stderr);
        let mut line = Vec::new();
        while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            let text = String::from_utf8_lossy(&line).trim_end().to_string();
            line.clear();
            if pattern.is_some_and(|p| text.contains(p)) {
                matches += 1;
            }
            if tail.len() == TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(text);
        }
        (Vec::from(tail).join("\n"), matches)
    });

    let status = loop {
        while let Ok(p) = rx.try_recv() {
            on_progress(&p);
        }
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = progress_thread.join();
            let _ = stderr_thread.join();
            return Err(MediaError::Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(40));
    };
    let _ = progress_thread.join();
    while let Ok(p) = rx.try_recv() {
        on_progress(&p);
    }
    let (err_tail, matches) = stderr_thread.join().unwrap_or_default();
    if status.success() {
        if !err_tail.is_empty() {
            tracing::debug!(stderr = %err_tail, "ffmpeg finished with messages");
        }
        Ok(matches)
    } else {
        let detail = if err_tail.is_empty() { format!("exit status {status}") } else { err_tail };
        tracing::warn!(%detail, "ffmpeg failed");
        Err(MediaError::Ffmpeg(detail))
    }
}
