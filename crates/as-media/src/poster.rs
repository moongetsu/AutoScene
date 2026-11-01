//! Exact single-frame stills (posters, portraits, thumbnails).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use as_core::{CancelToken, FrameIdx, FrameRange};

use crate::filters::{TONEMAP_SDR, deinterlace, picture_chain, scale_to_height};
use crate::{HdrKind, MediaError, MediaInfo, MediaResult, Source, Tools, run_ffmpeg};

/// Seconds of pre-roll before a cut point. Input seeking lands on the
/// keyframe before `t - PREROLL`; `trim` then selects frames by timestamp.
pub(crate) const PREROLL: f64 = 3.0;

/// How big a still is made. Whole frames keep their shape and are never
/// upscaled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StillSize {
    /// This many pixels tall.
    Height(u32),
    /// Just big enough to cover a `w`×`h` box, so a 16:9 tile shows a 4:3
    /// or scope frame as sharp as a 16:9 one.
    Cover(u32, u32),
    /// The part `[x1, y1, x2, y2]` (fractions of the upright, square-pixel
    /// frame) at exactly `w`×`h`: a face portrait. The part should have the
    /// shape of `w`×`h`; one reaching past an edge is moved inside the frame.
    Region([f32; 4], u32, u32),
}

impl StillSize {
    fn scale(self) -> String {
        match self {
            Self::Height(h) => scale_to_height(h),
            Self::Cover(w, h) => format!(
                "scale='min({w},iw)':'min({h},ih)':force_original_aspect_ratio=increase:force_divisible_by=2:flags=bicubic"
            ),
            // Lanczos: a face is often enlarged a little, and stays crisp.
            Self::Region([x1, y1, x2, y2], w, h) => format!(
                "crop=w=iw*{}:h=ih*{}:x=iw*{x1}:y=ih*{y1},scale={w}:{h}:flags=lanczos,setsar=1",
                (x2 - x1).clamp(0.0, 1.0),
                (y2 - y1).clamp(0.0, 1.0),
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
    // Deinterlace before the trim: alone, the frame has no neighbours and
    // its fields would just be woven together (combed).
    let mut chain: Vec<String> = deinterlace(info).into_iter().collect();
    chain.extend([format!("trim=start={trim_start}us:end={trim_end}us"), "setpts=PTS-STARTPTS".into()]);
    chain.extend(picture_chain(info, None, true));
    chain.push(format!("split={}", outs.len()));
    let mut graph = format!("[0:{}]{}", info.video.index, chain.join(","));
    for i in 0..outs.len() {
        graph.push_str(&format!("[s{i}]"));
    }
    for (i, (size, _)) in outs.iter().enumerate() {
        graph.push_str(&format!(";[s{i}]{}[o{i}]", size.scale()));
    }
    let mut args: Vec<OsString> =
        ["-v", "error", "-y", "-copyts", "-noaccurate_seek", "-ss"].map(OsString::from).to_vec();
    // Paths as they are: a file name needn't be valid Unicode.
    args.extend([format!("{seek:.6}").into(), "-i".into(), src.into(), "-filter_complex".into(), graph.into()]);
    let parts: Vec<PathBuf> = outs.iter().map(|(_, out)| out.with_extension("part.jpg")).collect();
    let remove_parts = || {
        for p in &parts {
            let _ = std::fs::remove_file(p);
        }
    };
    // A run that was killed may have left some behind.
    remove_parts();
    for (i, part) in parts.iter().enumerate() {
        args.extend(["-map", &format!("[o{i}]"), "-frames:v", "1", "-q:v", "3"].map(OsString::from));
        args.push(part.into());
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

/// Seconds of a media read for [`keyframe_jpeg`] before it gives up: a
/// stream with no keyframe in them isn't decoded to its end.
const KEYFRAME_WITHIN: f64 = 10.0;

/// A glimpse of `src` at about `at_sec` (from its start) without its frame
/// index: the keyframe at or before it, upright and SDR, `height` pixels
/// tall, as a JPEG. Only that picture is decoded (the frames after a
/// keyframe need the ones before them). A file without an index to seek
/// with (a transport stream) may give the keyframe after it instead. `None`
/// when no keyframe comes within [`KEYFRAME_WITHIN`].
pub fn keyframe_jpeg(
    tools: &Tools,
    src: &Path,
    info: &MediaInfo,
    at_sec: f64,
    height: u32,
) -> MediaResult<Option<Vec<u8>>> {
    // Tone mapped once small: a glimpse needn't be mapped at full size.
    let mut chain = picture_chain(info, Some(height), false);
    if info.video.hdr != HdrKind::None {
        chain.push(TONEMAP_SDR.into());
    }
    let mut cmd = tools.ffmpeg();
    cmd.args(["-v", "error", "-skip_frame", "nokey", "-noaccurate_seek", "-ss", &format!("{:.3}", at_sec.max(0.0))]);
    // Paths as they are: a file name needn't be valid Unicode.
    cmd.args(["-t", &KEYFRAME_WITHIN.to_string(), "-i"]).arg(src);
    cmd.args(["-map", &format!("0:{}", info.video.index), "-frames:v", "1", "-vf", &chain.join(",")]);
    cmd.args(["-c:v", "mjpeg", "-q:v", "4", "-f", "image2pipe", "-"]);
    let out = crate::proc::output(&mut cmd)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(MediaError::Ffmpeg(if err.is_empty() { format!("exit status {}", out.status) } else { err }));
    }
    Ok((!out.stdout.is_empty()).then_some(out.stdout))
}

/// How alike two pictures are: the structural similarity (SSIM) of their
/// greys at a small common size, 0..1 — the same frame scores above 0.9
/// whatever its size or JPEG, another picture far lower.
pub fn picture_similarity(tools: &Tools, a: &Path, b: &Path) -> MediaResult<f64> {
    let out = crate::proc::output(
        tools.ffmpeg().args(["-hide_banner", "-nostats", "-v", "info", "-i"]).arg(a).arg("-i").arg(b).args([
            "-filter_complex",
            "[0:v]scale=320:180,format=gray[a];[1:v]scale=320:180,format=gray[b];[a][b]ssim",
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ]),
    )?;
    let log = String::from_utf8_lossy(&out.stderr);
    log.lines()
        .rev()
        .find_map(|l| l.split("All:").nth(1)?.split_whitespace().next()?.parse::<f64>().ok())
        .ok_or_else(|| MediaError::Ffmpeg(format!("no SSIM in ffmpeg's output: {}", log.trim())))
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
