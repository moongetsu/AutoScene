//! Running ffmpeg with progress, cancellation and useful error messages.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use as_core::CancelToken;

use crate::{MediaError, MediaResult, Tools};

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
    mut on_progress: impl FnMut(&FfProgress),
) -> MediaResult<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = tools.ffmpeg();
    cmd.args(["-progress", "pipe:1", "-nostats"]).args(args);

    tracing::debug!(cmd = ?cmd, "ffmpeg");
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
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
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        let s = String::from_utf8_lossy(&buf).into_owned();
        let tail: Vec<&str> = s.lines().rev().take(12).collect();
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
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
    let err_tail = stderr_thread.join().unwrap_or_default();
    if status.success() {
        if !err_tail.is_empty() {
            tracing::debug!(stderr = %err_tail, "ffmpeg finished with messages");
        }
        Ok(())
    } else {
        Err(MediaError::Ffmpeg(if err_tail.is_empty() { format!("exit status {status}") } else { err_tail }))
    }
}
