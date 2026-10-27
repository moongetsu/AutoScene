//! What real scenepacks weigh next to `estimate_bytes`: each pack of a list
//! is exported as one file, the way the app does, with each H.264/HEVC
//! preset on this machine's encoders. Not run with the other tests:
//!
//! ```text
//! AUTOSCENE_CALIBRATE=packs.json cargo test -p as-export --release --test calibrate -- --ignored --nocapture
//! ```
//!
//! `packs.json`: `[{"name": "…", "source": "episode.mkv", "shots": [[start, end], …]}, …]`,
//! frames on the episode's grid (one episode a pack). `AUTOSCENE_CALIBRATE_PRESETS`
//! picks presets (comma-separated ids), `AUTOSCENE_CALIBRATE_WORK` the folder
//! the parts are made in (an all-intra pack of 15,000 frames takes about
//! 10 GB there; the system's temporary folder otherwise). Each file is
//! deleted once weighed. `video_bpp`: bits a pixel and frame of its picture.

use std::path::{Path, PathBuf};
use std::time::Instant;

use as_core::{CancelToken, FrameRange};
use as_export::preset::VideoCodec;
use as_export::{
    ClipJob, EstimateClip, JoinJob, JoinPart, MergeSource, Part, builtin_presets, estimate_bytes, export_clip,
    join_parts, plan_merge,
};
use as_media::{Source, Tools, build_frame_index, idet, probe};
use serde::Deserialize;

#[derive(Deserialize)]
struct Pack {
    name: String,
    source: PathBuf,
    shots: Vec<(u32, u32)>,
}

/// Bytes of the picture in `file`: its video packets.
fn video_bytes(tools: &Tools, file: &Path) -> f64 {
    let out = tools
        .ffprobe()
        .args(["-select_streams", "v:0", "-show_entries", "packet=size", "-of", "csv=p=0"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse::<f64>().ok()).sum()
}

#[test]
#[ignore = "exports whole scenepacks; run by hand"]
fn real_scenepacks_against_the_estimate() {
    let list = std::env::var("AUTOSCENE_CALIBRATE").expect("AUTOSCENE_CALIBRATE=<packs.json>");
    let packs: Vec<Pack> = serde_json::from_str(&std::fs::read_to_string(list).unwrap()).unwrap();
    let wanted = std::env::var("AUTOSCENE_CALIBRATE_PRESETS").ok();
    let presets: Vec<_> = builtin_presets()
        .into_iter()
        .filter(|p| matches!(p.video, VideoCodec::H264 { .. } | VideoCodec::Hevc { .. }))
        .filter(|p| wanted.as_deref().is_none_or(|w| w.split(',').any(|id| id.trim() == p.id)))
        .collect();
    let tools = Tools::locate(None).expect("ffmpeg on PATH");
    let cancel = CancelToken::new();
    let work = match std::env::var_os("AUTOSCENE_CALIBRATE_WORK") {
        Some(dir) => tempfile::tempdir_in(dir),
        None => tempfile::tempdir(),
    }
    .unwrap();
    println!("pack\tpreset\tencoder\tshots\tframes\tbytes\testimate\tratio\tvideo_bpp\tseconds");
    for pack in &packs {
        // As the app's ingest does: field order from idet where the file has none.
        let mut info = probe(&tools, &pack.source).unwrap();
        if info.video.field_order.is_none()
            && let Some(order) = idet::detect_field_order(&tools, &pack.source, &info).unwrap()
        {
            info.video.field_order = Some(order.into());
        }
        let index = build_frame_index(&tools, &pack.source, &info, work.path(), &cancel, |_| {}).unwrap();
        let fps = index.nominal_fps.to_f64();
        let ranges: Vec<FrameRange> = pack.shots.iter().map(|&(s, e)| FrameRange::new(s, e)).collect();
        let frames: u64 = ranges.iter().map(|r| u64::from(r.len())).sum();
        let sources: Vec<MergeSource> = ranges
            .iter()
            .map(|&range| MergeSource { info: &info, index: &index, range, audio_ordinal: None })
            .collect();
        let clips: Vec<EstimateClip> = ranges
            .iter()
            .map(|r| EstimateClip {
                info: &info,
                audio_ordinal: None,
                frames: u64::from(r.len()),
                seconds: index.t(r.end) - index.t(r.start),
                fps,
            })
            .collect();
        for preset in &presets {
            let started = Instant::now();
            let plan = plan_merge(tools.caps(), preset, &sources).unwrap();
            let dir = work.path().join(format!("{}-{}", pack.name, preset.id));
            std::fs::create_dir_all(&dir).unwrap();
            let mut parts = Vec::new();
            let mut encoder = String::new();
            for (i, (&range, timing)) in ranges.iter().zip(&plan.parts).enumerate() {
                let job = ClipJob {
                    src: Source { path: &pack.source, info: &info, index: &index },
                    range,
                    audio_ordinal: None,
                    preset,
                    out_stem: dir.join(format!("part-{:05}", i + 1)),
                    title: None,
                    part: Some(Part { shape: &plan.shape, samples: timing.samples }),
                };
                let out = export_clip(&tools, &job, &cancel, |_| {}).unwrap();
                assert!(out.verified, "{} {}: part {} failed its check", pack.name, preset.id, i + 1);
                encoder = out.encoder;
                parts.push(JoinPart {
                    path: out.path.into(),
                    frames: out.frames,
                    timing: *timing,
                    title: format!("Shot {}", i + 1),
                });
            }
            let job = JoinJob {
                parts: &parts,
                shape: &plan.shape,
                preset,
                out_stem: work.path().join(format!("{}-{}", pack.name, preset.id)),
                work_dir: &dir,
                title: None,
            };
            let out = join_parts(&tools, &job, &cancel, |_| {}).unwrap();
            assert!(out.verified, "{} {}: the joined file failed its check", pack.name, preset.id);
            let estimate = estimate_bytes(preset, tools.caps(), &clips, true);
            let (w, h) = (f64::from(plan.shape.width), f64::from(plan.shape.height));
            println!(
                "{}\t{}\t{encoder}\t{}\t{frames}\t{:.0}\t{estimate:.0}\t{:.3}\t{:.3}\t{:.0}",
                pack.name,
                preset.id,
                ranges.len(),
                out.bytes,
                estimate / out.bytes,
                video_bytes(&tools, Path::new(&out.path)) * 8.0 / (w * h * frames as f64),
                started.elapsed().as_secs_f64(),
            );
            let _ = std::fs::remove_file(&out.path);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
