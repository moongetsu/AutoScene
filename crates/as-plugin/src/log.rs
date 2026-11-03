//! A plugin's log file: what its runs wrote that wasn't a message for the
//! app (stdout lines outside the protocol, all of stderr, its log lines),
//! and how each run ended, for whoever looks into why a plugin misbehaved.
//!
//! Each run appends under a header line. A run writes at most
//! [`MAX_LOG_BYTES`]: the plugin's output until [`RESERVE`] is left, then
//! one line saying the rest was dropped (its pipes are still read to the
//! end), and in what is left the host's own lines on how the run went. A
//! file larger than that when a run opens it is moved to `<file>.1` first,
//! so a plugin's logs stay at a few MB.
//!
//! Only the host starts a line with `---` or `[`: a line break in what a
//! plugin wrote starts an indented line. One plugin's runs never overlap
//! (the host starts one only while no other run of its plugin runs), so a
//! run's lines follow its header.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::manifest::cut;

/// The most a run writes to its log, and the size past which a log is
/// moved aside when the next run opens it.
pub const MAX_LOG_BYTES: u64 = 1 << 20;
/// The end of a run's share that the plugin's output leaves to the host's
/// lines, so a run that floods its log still says how it ended.
pub const RESERVE: u64 = 8 << 10;
/// The longest host line written into that reserve.
const MAX_RESERVED_CHARS: usize = 500;

/// One run's part of its plugin's log.
pub struct PluginLog {
    /// None after a write failed.
    file: Option<File>,
    written: u64,
    /// Whether the plugin's output used up its part of the share.
    dropped: bool,
}

impl PluginLog {
    /// Opens the log at `path` (making its folder) for the run `run_id` of
    /// `action`, and writes the run's header.
    pub fn open(path: &Path, run_id: &str, action: &str) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        if fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
            let mut old = path.as_os_str().to_owned();
            old.push(".1");
            // Appending to it is no worse than losing it.
            if let Err(e) = fs::rename(path, &old) {
                tracing::debug!(error = %e, "couldn't move a full plugin log aside");
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(format!("--- {} run {run_id} {action}\n", utc(SystemTime::now())).as_bytes())?;
        Ok(Self { file: Some(file), written: 0, dropped: false })
    }

    /// Writes `[tag] text`, for what the plugin wrote or said. Once only
    /// [`RESERVE`] is left it writes that more output was dropped, once,
    /// and none of the plugin's lines after. A failed write ends the log
    /// quietly: it never fails the run.
    pub fn line(&mut self, tag: &str, text: &str) {
        if self.dropped {
            return;
        }
        let line = format!("[{tag}] {}\n", indented(text));
        if self.written + line.len() as u64 > MAX_LOG_BYTES - RESERVE {
            self.dropped = true;
            self.write("[host] more output dropped\n");
        } else {
            self.write(&line);
        }
    }

    /// Writes `[tag] text`, for the host's own word on the run: why it
    /// ended, its exit status, the line that counted. In the reserve it is
    /// cut to `MAX_RESERVED_CHARS`; once the share is used up it is left
    /// out.
    pub fn note(&mut self, tag: &str, text: &str) {
        let text = indented(text);
        let mut line = format!("[{tag}] {text}\n");
        if self.written + line.len() as u64 > MAX_LOG_BYTES - RESERVE {
            line = format!("[{tag}] {}\n", cut(&text, MAX_RESERVED_CHARS));
        }
        if self.written + line.len() as u64 <= MAX_LOG_BYTES {
            self.write(&line);
        }
    }

    fn write(&mut self, line: &str) {
        let Some(file) = &mut self.file else { return };
        if let Err(e) = file.write_all(line.as_bytes()) {
            tracing::debug!(error = %e, "couldn't write a plugin log");
            self.file = None;
            return;
        }
        self.written += line.len() as u64;
    }
}

/// `text` as lines of the log: each line break in it (`\n`, `\r\n`, a lone
/// `\r`, and the rarer ones some editors honour) starts a line indented by
/// two spaces, so a plugin can't write a line that reads as the host's.
fn indented(text: &str) -> String {
    let breaks = |c: char| matches!(c, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}');
    let text = text.trim_end_matches(breaks).replace("\r\n", "\n");
    text.split(breaks).collect::<Vec<_>>().join("\n  ")
}

/// `t` in UTC, to the second, as ISO 8601: `2026-11-02T09:30:00Z`.
fn utc(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, day_secs) = ((secs / 86_400) as i64, secs % 86_400);
    // Days to a civil date (Howard Hinnant's algorithm), in eras of 400
    // years that start on 1 March, so the leap day ends a year.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let of_era = z.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * of_year + 2) / 153;
    let day = of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 { month_from_march + 3 } else { month_from_march - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let (h, m, s) = (day_secs / 3600, day_secs / 60 % 60, day_secs % 60);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn log_times_are_utc_dates() {
        let at = |secs: u64| utc(UNIX_EPOCH + Duration::from_secs(secs));
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(at(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(at(4_107_542_399), "2100-02-28T23:59:59Z");
        assert_eq!(at(4_107_542_400), "2100-03-01T00:00:00Z");
    }

    #[test]
    fn each_run_starts_with_its_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("dev.autoscene.hello.log");
        PluginLog::open(&path, "run-1", "post").unwrap().line("stdout", "hello");
        PluginLog::open(&path, "run-2", "post").unwrap().line("stderr", "again");
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[0].starts_with("--- ") && lines[0].ends_with("Z run run-1 post"), "{text}");
        assert_eq!(lines[1], "[stdout] hello");
        assert!(lines[2].ends_with("Z run run-2 post"), "{text}");
        assert_eq!(lines[3], "[stderr] again");
    }

    #[test]
    fn a_run_writes_its_share_then_says_the_rest_was_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.log");
        let mut log = PluginLog::open(&path, "run-1", "post").unwrap();
        let text = "x".repeat(1000);
        for _ in 0..2000 {
            log.line("stdout", &text);
        }
        // The host's word still goes in, cut short in the reserve.
        log.note("result", &"r".repeat(5000));
        log.note("host", "exit code: 0");
        log.line("stdout", "late");
        let plugin_part = fs::read_to_string(&path).unwrap();
        assert!(plugin_part.len() as u64 <= MAX_LOG_BYTES, "{}", plugin_part.len());
        assert!(plugin_part.len() as u64 > MAX_LOG_BYTES - RESERVE - 2000, "{}", plugin_part.len());
        assert_eq!(plugin_part.matches("[host] more output dropped\n").count(), 1);
        let end = format!("[host] more output dropped\n[result] {}…\n[host] exit code: 0\n", "r".repeat(499));
        assert!(plugin_part.ends_with(&end), "{}", &plugin_part[plugin_part.len() - 200..]);
        // No more than the share, however much the host has to say.
        for _ in 0..100 {
            log.note("host", &"h".repeat(5000));
        }
        drop(log);
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.len() as u64 <= MAX_LOG_BYTES + 100, "{}", written.len());
        assert!(written.len() > plugin_part.len());
    }

    #[test]
    fn a_line_break_in_what_a_plugin_wrote_starts_an_indented_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.log");
        let mut log = PluginLog::open(&path, "run-1", "post").unwrap();
        log.line("stdout", "one\r\n[host] exit code: 0\r--- 2026-01-01T00:00:00Z run forged post");
        log.line("stderr", "rare\u{2028}breaks\u{85}too\u{0C}\n\n");
        log.note("failed", "Broke.\nTraceback:\n  File \"main.py\"");
        drop(log);
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().skip(1).collect();
        assert_eq!(
            lines,
            [
                "[stdout] one",
                "  [host] exit code: 0",
                "  --- 2026-01-01T00:00:00Z run forged post",
                "[stderr] rare",
                "  breaks",
                "  too",
                "[failed] Broke.",
                "  Traceback:",
                "    File \"main.py\"",
            ]
        );
    }

    #[test]
    fn a_full_log_is_moved_aside_when_a_run_opens_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.log");
        let old = dir.path().join("p.log.1");
        fs::write(&old, "older still").unwrap();
        fs::write(&path, vec![b'o'; MAX_LOG_BYTES as usize]).unwrap();
        // Not over the limit yet: the next run appends.
        PluginLog::open(&path, "run-1", "post").unwrap();
        assert_eq!(fs::read_to_string(&old).unwrap(), "older still");
        // Over it: moved aside, in place of the one before.
        PluginLog::open(&path, "run-2", "post").unwrap().line("stdout", "fresh");
        assert!(fs::metadata(&old).unwrap().len() > MAX_LOG_BYTES);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("--- ") && text.ends_with("run run-2 post\n[stdout] fresh\n"), "{text}");
    }
}
