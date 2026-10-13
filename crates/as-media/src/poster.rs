//! Exact single-frame stills (posters, thumbnails).

use std::path::Path;

use as_core::{CancelToken, FrameIdx, FrameRange};

use crate::filters::display_chain;
use crate::{MediaResult, Source, Tools, run_ffmpeg};

/// Seconds of pre-roll before a cut point. Input seeking lands on the
/// keyframe before `t - PREROLL`; `trim` then selects frames by timestamp.
pub(crate) const PREROLL: f64 = 3.0;

/// Write frame `f` of `src` as a JPEG `height` px tall (display-corrected, SDR).
pub fn extract_frame_jpeg(tools: &Tools, src: Source, f: FrameIdx, height: u32, out: &Path) -> MediaResult<()> {
    let Source { path: src, info, index } = src;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let seek = (index.t(f) - index.t0() - PREROLL).max(0.0);
    let (trim_start, trim_end) = index.trim_us(FrameRange::new(f, f + 1));
    let mut chain = vec![format!("trim=start={trim_start}us:end={trim_end}us"), "setpts=PTS-STARTPTS".into()];
    chain.extend(display_chain(info, Some(height), true));
    let args: Vec<String> = [
        "-v",
        "error",
        "-y",
        "-copyts",
        "-noaccurate_seek",
        "-ss",
        &format!("{seek:.6}"),
        "-i",
        &src.display().to_string(),
        "-map",
        &format!("0:{}", info.video.index),
        "-vf",
        &chain.join(","),
        "-frames:v",
        "1",
        "-q:v",
        "3",
        &out.display().to_string(),
    ]
    .map(String::from)
    .to_vec();
    run_ffmpeg(tools, &args, &CancelToken::new(), |_| {})
}
