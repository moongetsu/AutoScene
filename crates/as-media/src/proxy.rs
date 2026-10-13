//! Playback proxies for the in-app player.
//!
//! The webview plays H.264/AAC in MP4 reliably everywhere. Anything else
//! (MKV, HEVC, 10-bit, HDR, interlaced, odd start offsets) gets a 720p proxy
//! with a short GOP and no B-frames so scrubbing and frame stepping are
//! instant. Proxies keep every frame (passthrough timing), so player time maps
//! back to source frames through the frame index plus a measured offset.

use std::path::Path;

use as_core::{CancelToken, FrameIndex};

use crate::filters::display_chain;
use crate::{CodecFamily, MediaError, MediaInfo, MediaResult, Source, Tools, run_ffmpeg};

const PROXY_HEIGHT: u32 = 720;

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
    let tmp = out.with_extension("part.mp4");
    let caps = tools.caps();
    let mut args: Vec<String> = vec!["-v".into(), "error".into(), "-y".into()];
    if let Some(d) = &caps.decode {
        // Decode on the GPU but download: the display filters run on the CPU.
        args.extend(["-hwaccel".into(), d.hwaccel.clone()]);
    }
    args.extend(["-i".into(), src.display().to_string(), "-map".into(), format!("0:{}", info.video.index)]);
    if let Some(a) = audio_ordinal {
        args.extend(["-map".into(), format!("0:a:{a}")]);
    }
    let mut chain = display_chain(info, Some(PROXY_HEIGHT), true);
    chain.push("format=yuv420p".into());
    args.extend(["-vf".into(), chain.join(","), "-fps_mode".into(), "passthrough".into()]);

    // Short GOP, no B-frames: every seek decodes at most 11 frames.
    match caps.hw_encoder(CodecFamily::H264).filter(|e| e.name == "h264_nvenc") {
        Some(_) => args.extend(
            ["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", "26", "-b:v", "0", "-g", "12", "-bf", "0"]
                .map(String::from),
        ),
        None => args.extend(
            ["-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-g", "12", "-bf", "0", "-tune", "fastdecode"]
                .map(String::from),
        ),
    }
    if audio_ordinal.is_some() {
        args.extend(["-c:a", "aac", "-b:a", "128k", "-ac", "2"].map(String::from));
    }
    args.extend(["-movflags".into(), "+faststart".into(), tmp.display().to_string()]);

    let duration = index.duration().max(0.001);
    run_ffmpeg(tools, &args, cancel, |p| progress((p.out_time / duration).clamp(0.0, 1.0)))?;

    let t0 = first_video_pts(tools, &tmp)?;
    let frames = count_frames(tools, &tmp)?;
    if frames != index.len() as u64 {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Verify(format!("proxy has {frames} frames, source has {}", index.len())));
    }
    std::fs::rename(&tmp, out)?;
    Ok(ProxyResult { t0 })
}

/// Presentation time of the first video frame as players see it (edit lists applied).
pub fn first_video_pts(tools: &Tools, file: &Path) -> MediaResult<f64> {
    let out = tools
        .ffprobe()
        .args([
            "-select_streams",
            "v:0",
            "-read_intervals",
            "%+#1",
            "-show_entries",
            "frame=pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(file)
        .output()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|l| l.trim().trim_end_matches(',').parse().ok())
        .ok_or_else(|| MediaError::Verify(format!("no frames in {}", file.display())))
}

fn count_frames(tools: &Tools, file: &Path) -> MediaResult<u64> {
    let out = tools
        .ffprobe()
        .args(["-select_streams", "v:0", "-count_packets", "-show_entries", "stream=nb_read_packets", "-of", "csv=p=0"])
        .arg(file)
        .output()?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| MediaError::Verify(format!("could not count frames in {}", file.display())))
}
