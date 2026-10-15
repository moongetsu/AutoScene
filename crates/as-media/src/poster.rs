//! Exact single-frame stills (posters, thumbnails).

use std::path::{Path, PathBuf};

use as_core::{CancelToken, FrameIdx, FrameRange};

use crate::filters::{display_chain, scale_to_height};
use crate::{MediaError, MediaResult, Source, Tools, run_ffmpeg};

/// Seconds of pre-roll before a cut point. Input seeking lands on the
/// keyframe before `t - PREROLL`; `trim` then selects frames by timestamp.
pub(crate) const PREROLL: f64 = 3.0;

/// How big a still is made. Both keep the frame's shape and never upscale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StillSize {
    /// This many pixels tall.
    Height(u32),
    /// Just big enough to cover a `w`×`h` box, so a 16:9 tile shows a 4:3
    /// or scope frame as sharp as a 16:9 one.
    Cover(u32, u32),
}

impl StillSize {
    fn scale(self) -> String {
        match self {
            Self::Height(h) => scale_to_height(h),
            Self::Cover(w, h) => format!(
                "scale='min({w},iw)':'min({h},ih)':force_original_aspect_ratio=increase:force_divisible_by=2:flags=bicubic"
            ),
        }
    }
}

/// Write frame `f` of `src` (display-corrected, SDR) as one JPEG per entry
/// of `outs`, all from a single decode. The files appear complete or not at
/// all: a cancelled or failed run never leaves a truncated picture behind.
pub fn extract_frame_jpegs(
    tools: &Tools,
    src: Source,
    f: FrameIdx,
    outs: &[(StillSize, &Path)],
    cancel: &CancelToken,
) -> MediaResult<()> {
    let Source { path: src, info, index } = src;
    for (_, out) in outs {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let seek = (index.t(f) - index.t0() - PREROLL).max(0.0);
    let (trim_start, trim_end) = index.trim_us(FrameRange::new(f, f + 1));
    let mut chain = vec![format!("trim=start={trim_start}us:end={trim_end}us"), "setpts=PTS-STARTPTS".into()];
    chain.extend(display_chain(info, None, true));
    chain.push(format!("split={}", outs.len()));
    let mut graph = format!("[0:{}]{}", info.video.index, chain.join(","));
    for i in 0..outs.len() {
        graph.push_str(&format!("[s{i}]"));
    }
    for (i, (size, _)) in outs.iter().enumerate() {
        graph.push_str(&format!(";[s{i}]{}[o{i}]", size.scale()));
    }
    let mut args: Vec<String> = [
        "-v",
        "error",
        "-y",
        "-copyts",
        "-noaccurate_seek",
        "-ss",
        &format!("{seek:.6}"),
        "-i",
        &src.display().to_string(),
        "-filter_complex",
        &graph,
    ]
    .map(String::from)
    .to_vec();
    let parts: Vec<PathBuf> = outs.iter().map(|(_, out)| out.with_extension("part.jpg")).collect();
    let remove_parts = || {
        for p in &parts {
            let _ = std::fs::remove_file(p);
        }
    };
    // A run that was killed may have left some behind.
    remove_parts();
    for (i, part) in parts.iter().enumerate() {
        args.extend(["-map".into(), format!("[o{i}]"), "-frames:v".into(), "1".into(), "-q:v".into(), "3".into()]);
        args.push(part.display().to_string());
    }
    let result = run_ffmpeg(tools, &args, cancel, |_| {}).and_then(|()| {
        // ffmpeg succeeds without writing anything when no frame reaches the trim.
        if let Some(empty) = parts.iter().find(|p| !std::fs::metadata(p).is_ok_and(|m| m.len() > 0)) {
            return Err(MediaError::Ffmpeg(format!("frame {f} wasn't written to {}", empty.display())));
        }
        for (part, (_, out)) in parts.iter().zip(outs) {
            rename_with_retry(part, out)?;
        }
        Ok(())
    });
    if result.is_err() {
        remove_parts();
    }
    result
}

/// Windows can hold a just-written file for a moment (antivirus, the
/// indexer, the webview reading the picture it replaces); retry briefly.
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if attempt < 4 && e.kind() != std::io::ErrorKind::NotFound => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
            }
            result => return result,
        }
    }
}
