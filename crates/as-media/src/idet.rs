//! Interlace detection for files whose container doesn't say.
//!
//! Many broadcast captures and DVD rips carry no field-order flag. We run
//! ffmpeg's `idet` on a short window from the middle of the file (openings
//! are often progressive studio logos) and trust its multi-frame verdict.

use std::path::Path;

use crate::{MediaInfo, MediaResult, Tools};

const SAMPLE_FRAMES: u32 = 600;

/// Field order (`tt` / `bb`) if the content is interlaced, `None` if it is progressive.
pub fn detect_field_order(tools: &Tools, path: &Path, info: &MediaInfo) -> MediaResult<Option<&'static str>> {
    let mid = info.duration_sec.unwrap_or(0.0) * 0.5;
    let out = tools
        .ffmpeg()
        .args(["-v", "info", "-nostats", "-ss", &format!("{mid:.3}"), "-i"])
        .arg(path)
        .args(["-map", &format!("0:{}", info.video.index), "-vf", "idet", "-frames:v", &SAMPLE_FRAMES.to_string()])
        .args(["-an", "-f", "null", "-"])
        .output()?;
    Ok(verdict(&String::from_utf8_lossy(&out.stderr)))
}

/// Parse `Multi frame detection: TFF: 12 BFF: 0 Progressive: 571 Undetermined: 17`.
fn verdict(log: &str) -> Option<&'static str> {
    let line = log.lines().rev().find(|l| l.contains("Multi frame detection"))?;
    let count = |key: &str| -> u64 {
        line.split(key).nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0)
    };
    let (tff, bff, prog) = (count("TFF:"), count("BFF:"), count("Progressive:"));
    let decided = tff + bff + prog;
    // Need a real sample and a clear majority; mixed content stays progressive.
    if decided < 100 || (tff + bff) * 10 < decided * 6 {
        return None;
    }
    Some(if tff >= bff { "tt" } else { "bb" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts() {
        let log = |t, b, p| {
            format!(
                "[Parsed_idet_0 @ 0x1] Repeated Fields: Neither: 600\n\
                 [Parsed_idet_0 @ 0x1] Single frame detection: TFF: 1 BFF: 0 Progressive: 2 Undetermined: 597\n\
                 [Parsed_idet_0 @ 0x1] Multi frame detection: TFF: {t} BFF: {b} Progressive: {p} Undetermined: 3\n"
            )
        };
        assert_eq!(verdict(&log(580, 0, 17)), Some("tt"));
        assert_eq!(verdict(&log(2, 560, 30)), Some("bb"));
        assert_eq!(verdict(&log(0, 0, 597)), None);
        assert_eq!(verdict(&log(250, 0, 347)), None, "mixed content is not deinterlaced");
        assert_eq!(verdict(&log(20, 0, 3)), None, "too few frames decided");
        assert_eq!(verdict("no idet output"), None);
    }
}
