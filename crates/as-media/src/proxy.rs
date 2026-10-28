//! Playback proxies for the in-app player.
//!
//! The webview plays H.264/AAC in MP4 reliably everywhere. Anything else
//! (MKV, HEVC, 10-bit, HDR, interlaced, odd start offsets) gets a 540p proxy
//! with a short GOP and no B-frames so scrubbing and frame stepping are
//! instant. Proxies keep every frame (passthrough timing), so player time maps
//! back to source frames through the frame index plus a measured offset.

use std::ffi::OsString;
use std::path::Path;

use as_core::{CancelToken, FrameIndex};

use crate::filters::display_chain;
use crate::{CodecFamily, MediaError, MediaInfo, MediaResult, Source, Tools, run_ffmpeg};

/// Enough to judge framing and expressions; a 45-minute episode stays well
/// under a gigabyte.
const PROXY_HEIGHT: u32 = 540;

/// aresample options that keep audio on its timestamps: wherever the source
/// has no samples (a late start, a lost codec frame) silence is inserted
/// instead of pulling later sound forward. The 5 ms threshold is below any
/// audio codec frame (AAC 21 ms, AC-3 32 ms, Opus ≥ 10 ms in practice) and
/// above millisecond timestamp rounding (Matroska).
pub const AUDIO_GAP_FILL: &str = "async=1:min_hard_comp=0.005";

/// Can the webview play the source file as-is with frame-accurate mapping?
pub fn playable_direct(info: &MediaInfo, index: &FrameIndex) -> bool {
    let mp4 = info.container.split(',').any(|c| matches!(c, "mp4" | "mov"));
    let v = &info.video;
    let video_ok = v.codec == "h264" && v.bit_depth == 8 && v.pix_fmt == "yuv420p";
    let audio_ok = info.audio.iter().all(|a| matches!(a.codec.as_str(), "aac" | "mp3" | "opus"));
    mp4 && video_ok
        && audio_ok
        && !info.is_interlaced()
        && v.rotation == 0
        && v.height <= 1440
        && index.t0().abs() < 1e-3
}

pub struct ProxyResult {
    /// Player time of frame 0 (`player_time(f) = t0 + t(f) - t(0)`).
    pub t0: f64,
}

pub fn build_proxy(
    tools: &Tools,
    src: Source,
    audio_ordinal: Option<u32>,
    out: &Path,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
) -> MediaResult<ProxyResult> {
    let Source { path: src, info, index } = src;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Unique per build: a replaced build's ffmpeg may still be exiting.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = out.with_extension(format!("{}-{n}.part.mp4", std::process::id()));
    let caps = tools.caps();
    // Absolute timestamps in, then video and audio shifted by the same exact
    // frame-0 time: the proxy starts at 0 with frame f at `t(f) - t(0)` and
    // A/V sync intact. (Letting ffmpeg shift by the container start instead
    // can leave frames on half-frame boundaries that collide when rounded.)
    let mut args: Vec<OsString> = ["-v", "error", "-y", "-copyts"].map(OsString::from).to_vec();
    if let Some(d) = &caps.decode {
        // Decode on the GPU but download: the display filters run on the CPU.
        args.extend(["-hwaccel".into(), d.hwaccel.clone().into()]);
    }
    // Paths as they are: a file name needn't be valid Unicode.
    args.extend(["-i".into(), src.into()]);
    let t0 = index.t0();
    let mut chain = vec!["setpts=PTS-STARTPTS".to_string()];
    chain.extend(display_chain(info, Some(PROXY_HEIGHT), true));
    chain.push("format=yuv420p".into());
    let mut graph = format!("[0:{}]{}[v]", info.video.index, chain.join(","));
    if let Some(a) = audio_ordinal {
        graph +=
            &format!(";[0:a:{a}]atrim=start={t0:.9},asetpts=PTS-{t0:.9}/TB,aresample={AUDIO_GAP_FILL}:first_pts=0[a]");
    }
    args.extend(["-filter_complex".into(), graph.into(), "-map".into(), "[v]".into()]);
    if audio_ordinal.is_some() {
        args.extend(["-map".into(), "[a]".into()]);
    }
    args.extend(["-fps_mode".into(), "passthrough".into()]);
    args.extend(timing_args(index).into_iter().map(OsString::from));

    // Short GOP, no B-frames: every seek decodes at most 11 frames.
    match caps.hw_encoder(CodecFamily::H264).filter(|e| e.name == "h264_nvenc") {
        Some(_) => args.extend(
            ["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", "26", "-b:v", "0", "-g", "12", "-bf", "0"]
                .map(OsString::from),
        ),
        None => args.extend(
            ["-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-g", "12", "-bf", "0", "-tune", "fastdecode"]
                .map(OsString::from),
        ),
    }
    if let Some(a) = audio_ordinal {
        args.extend(["-c:a", "aac", "-b:a", "128k", "-ac", "2"].map(OsString::from));
        let lang = info.audio.iter().find(|s| s.ordinal == a).and_then(|s| s.language.as_deref());
        if let Some(lang) = lang.and_then(crate::lang::canonical) {
            args.extend(["-metadata:s:a:0".into(), format!("language={lang}").into()]);
        }
    }
    // Chapters would become an extra data track; the player needs none of it.
    args.extend(["-map_chapters", "-1", "-map_metadata", "-1"].map(OsString::from));
    args.extend(["-movflags".into(), "+faststart".into(), tmp.clone().into()]);

    let duration = index.duration().max(0.001);
    run_ffmpeg(tools, &args, cancel, |p| progress((p.out_time / duration).clamp(0.0, 1.0))).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;

    let t0 = first_video_pts(tools, &tmp)?;
    let frames = count_frames(tools, &tmp)?;
    if frames != index.len() as u64 {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Verify(format!("proxy has {frames} frames, source has {}", index.len())));
    }
    std::fs::rename(&tmp, out)?;
    Ok(ProxyResult { t0 })
}

/// Encoder time base for frame-preserving encodes. ffmpeg defaults to
/// `1/framerate`, which snaps true variable-rate timestamps onto the nominal
/// grid (and then nudges collisions apart); keep the source's own time base
/// for those. Constant-rate sources keep `1/framerate`, which gives editors
/// the clean, jitter-free timing they expect.
pub fn timing_args(index: &FrameIndex) -> Vec<String> {
    if index.is_vfr() { vec!["-enc_time_base:v".into(), "filter".into()] } else { vec![] }
}

/// Presentation time of the first video frame as players see it (edit lists applied).
pub fn first_video_pts(tools: &Tools, file: &Path) -> MediaResult<f64> {
    let mut cmd = tools.ffprobe();
    cmd.args([
        "-select_streams",
        "v:0",
        "-read_intervals",
        "%+#1",
        "-show_entries",
        "frame=pts_time",
        "-of",
        "csv=p=0",
    ])
    .arg(file);
    let out = crate::proc::output(&mut cmd)?;
    first_csv_field(&String::from_utf8_lossy(&out.stdout))
        .and_then(|f| f.parse().ok())
        .ok_or_else(|| MediaError::Verify(format!("no frames in {}", file.display())))
}

fn count_frames(tools: &Tools, file: &Path) -> MediaResult<u64> {
    let mut cmd = tools.ffprobe();
    cmd.args(["-select_streams", "v:0", "-count_packets", "-show_entries", "stream=nb_read_packets", "-of", "csv=p=0"])
        .arg(file);
    let out = crate::proc::output(&mut cmd)?;
    first_csv_field(&String::from_utf8_lossy(&out.stdout))
        .and_then(|f| f.parse().ok())
        .ok_or_else(|| MediaError::Verify(format!("could not count frames in {}", file.display())))
}

/// First value of ffprobe `-of csv=p=0` output. FFmpeg 9 ends rows with a
/// trailing separator (`586,`), older versions don't.
fn first_csv_field(out: &str) -> Option<&str> {
    out.lines().next()?.split(',').map(str::trim).next().filter(|f| !f.is_empty())
}

#[cfg(test)]
mod tests {
    use super::first_csv_field;

    #[test]
    fn csv_fields_with_and_without_trailing_separator() {
        assert_eq!(
            first_csv_field(
                "586
"
            ),
            Some("586")
        );
        assert_eq!(
            first_csv_field(
                "586,
"
            ),
            Some("586")
        );
        assert_eq!(
            first_csv_field(
                "0.041016,
0.08,
"
            ),
            Some("0.041016")
        );
        assert_eq!(first_csv_field(""), None);
        assert_eq!(
            first_csv_field(
                ",
"
            ),
            None
        );
    }
}
