//! Dev tool: run the anime character pass (`as_analysis::anime`) over a
//! real episode and save its tracks, like `faces_run` does for the face
//! pass.
//! `cargo run --release -p as-analysis --example anime_run -- <ort_dir> <models_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path/bb.segments.tsv> <out_prefix> [resume]`
//! `<models_dir>` is the bench layout `tools/anime-bench/models.py` fetches:
//! `anime_head_detection/head_detect_v2.0_s/model.onnx`,
//! `ccip_onnx/ccip-caformer-24-randaug-pruned/model_feat.onnx` and
//! `anime_person_detection/person_detect_v1.3_s/model.onnx`.
//! Writes `<out_prefix>.tracks` (as_analysis::tracks::encode of all tracks),
//! `<out_prefix>.chunks` (one `start end tracks sprites` line per chunk),
//! `<out_prefix>.dets.tsv` (one `frame score height norm quality` line per
//! head, to calibrate the quality) and the shot sprites as
//! `<out_prefix>.sprites/<shot>.jpg`.
//! `HEAD=<onnx>` and `CCIP=<onnx>` pick other models; `PERSON=<onnx>` turns
//! the second source on (`PERSON=1`: the default person model).
//! `HEAD_INPUT`/`PERSON_INPUT=<w>x<h>` set the detectors' letterboxed
//! inputs (defaults 640x384 and 1280x736), `CCIP_BATCH=<n>` the embedding
//! batch (32).
//! `STRIDE=<n>`, `DET_THRESHOLD=<score>`, `PERSON_THRESHOLD=<score>`,
//! `NORMS=<floor>,<full>`, `MIN_TRACK_Q=<q>`, `CROP_SCALE=<k>`,
//! `PERSON_HEAD=<share>` and `UNION_IOU=<iou>` override `Params::anime()`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use as_analysis::anime::{self, Anime, Params};
use as_core::{CancelToken, FrameIndex, FrameRange};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::ccip::Ccip;
use as_vision::yolo::Yolo;

/// `<w>x<h>` of an env var, `fallback` when unset.
fn input(var: &str, fallback: (usize, usize)) -> (usize, usize) {
    std::env::var(var).ok().map_or(fallback, |v| {
        let (w, h) = v.split_once('x').unwrap_or_else(|| panic!("{var}=<w>x<h>"));
        (w.parse().expect(var), h.parse().expect(var))
    })
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (ort, models, ffmpeg, dir, out) = (&a[0], Path::new(&a[1]), &a[2], Path::new(&a[3]), &a[4]);
    let resume: u32 = a.get(5).map_or(0, |s| s.parse().unwrap());
    let index = FrameIndex::from_bytes(&std::fs::read(dir.join("bb.index")).unwrap()).unwrap();
    let info: MediaInfo = serde_json::from_str(&std::fs::read_to_string(dir.join("bb.info.json")).unwrap()).unwrap();
    let path = PathBuf::from(std::fs::read_to_string(dir.join("bb.path")).unwrap().trim());
    let shots: Vec<FrameRange> = std::fs::read_to_string(dir.join("bb.segments.tsv"))
        .unwrap()
        .lines()
        .filter_map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            (v[2] == "shot").then(|| FrameRange::new(v[0].parse().unwrap(), v[1].parse().unwrap()))
        })
        .collect();
    let tools = Tools::locate(Some(Path::new(ffmpeg))).unwrap();
    let rt = Runtime::load(Path::new(ort), true).unwrap();
    let mut params = Params::anime();
    if let Ok(v) = std::env::var("STRIDE") {
        params.stride = v.parse().expect("STRIDE=<frames>");
    }
    if let Ok(v) = std::env::var("DET_THRESHOLD") {
        params.det_threshold = v.parse().expect("DET_THRESHOLD=<score>");
    }
    if let Ok(v) = std::env::var("PERSON_THRESHOLD") {
        params.person_threshold = v.parse().expect("PERSON_THRESHOLD=<score>");
    }
    if let Ok(v) = std::env::var("NORMS") {
        let (floor, full) = v.split_once(',').expect("NORMS=<floor>,<full>");
        (params.norm_floor, params.norm_full) = (floor.parse().unwrap(), full.parse().unwrap());
    }
    if let Ok(v) = std::env::var("MIN_TRACK_Q") {
        params.min_track_quality = v.parse().expect("MIN_TRACK_Q=<quality>");
    }
    if let Ok(v) = std::env::var("CROP_SCALE") {
        params.crop_scale = v.parse().expect("CROP_SCALE=<k>");
    }
    if let Ok(v) = std::env::var("PERSON_HEAD") {
        params.person_head = v.parse().expect("PERSON_HEAD=<share>");
    }
    if let Ok(v) = std::env::var("UNION_IOU") {
        params.union_iou = v.parse().expect("UNION_IOU=<iou>");
    }
    let model = |var: &str, default: &str| std::env::var(var).map_or_else(|_| models.join(default), PathBuf::from);
    let head_model = model("HEAD", "anime_head_detection/head_detect_v2.0_s/model.onnx");
    let ccip_model = model("CCIP", "ccip_onnx/ccip-caformer-24-randaug-pruned/model_feat.onnx");
    let person_model = std::env::var("PERSON").ok().map(|v| {
        if v == "1" {
            models.join("anime_person_detection/person_detect_v1.3_s/model.onnx")
        } else {
            PathBuf::from(v)
        }
    });
    let size = anime::frame_size(&info, &params);
    let mut heads =
        Yolo::load(&rt, &head_model, size, input("HEAD_INPUT", (640, 384)), as_analysis::DETECT_BATCH).unwrap();
    let ccip_batch: usize = std::env::var("CCIP_BATCH").map_or(32, |v| v.parse().unwrap());
    let mut ccip = Ccip::load(&rt, &ccip_model, ccip_batch).unwrap();
    let mut persons = person_model.map(|m| {
        Yolo::load(&rt, &m, size, input("PERSON_INPUT", (1280, 736)), as_analysis::DETECT_BATCH).unwrap()
    });
    println!("anime pass at {size:?}, stride {}, person model {}", params.stride, persons.is_some());
    let t0 = Instant::now();
    let mut all = Vec::new();
    let mut chunks = std::fs::File::create(format!("{out}.chunks")).unwrap();
    let sprites = PathBuf::from(format!("{out}.sprites"));
    std::fs::create_dir_all(&sprites).unwrap();
    let (mut sprite_count, mut sprite_bytes) = (0usize, 0usize);
    anime::face_tracks(
        &tools,
        Source { path: &path, info: &info, index: &index },
        Anime { heads: &mut heads, persons: persons.as_mut(), ccip: &mut ccip },
        &shots,
        resume,
        2400,
        &params,
        &CancelToken::new(),
        |p| eprint!("\r{:5.1}% {:5.0}s", 100.0 * p, t0.elapsed().as_secs_f64()),
        |c| {
            writeln!(chunks, "{} {} {} {}", c.start, c.end, c.tracks.len(), c.sprites.len()).unwrap();
            all.extend_from_slice(c.tracks);
            for s in c.sprites {
                std::fs::write(sprites.join(format!("{}.jpg", s.shot)), &s.jpeg).unwrap();
                sprite_count += 1;
                sprite_bytes += s.jpeg.len();
            }
            Ok(())
        },
    )
    .unwrap();
    let secs = t0.elapsed().as_secs_f64();
    std::fs::write(format!("{out}.tracks"), as_analysis::tracks::encode(&all)).unwrap();
    let mut dets = std::fs::File::create(format!("{out}.dets.tsv")).unwrap();
    for d in all.iter().flat_map(|t| &t.dets) {
        writeln!(dets, "{}\t{:.4}\t{:.4}\t{:.3}\t{:.4}", d.frame, d.score, d.height(), d.norm, d.quality).unwrap();
    }
    let heads: usize = all.iter().map(|t| t.dets.len()).sum();
    let good = all.iter().filter(|t| t.quality >= 0.5).count();
    println!(
        "\n{} frames from {resume} in {secs:.0}s ({:.0} fps): {} tracks ({good} with quality >= 0.5), {heads} heads",
        index.len() - resume,
        (index.len() - resume) as f64 / secs,
        all.len()
    );
    println!(
        "{sprite_count} sprites for {} shots, {:.1} MB ({:.0} KB each)",
        shots.len(),
        sprite_bytes as f64 / 1e6,
        sprite_bytes as f64 / 1e3 / sprite_count.max(1) as f64
    );
}
