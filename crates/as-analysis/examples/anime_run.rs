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
//! `PERSON_HEAD=<share>`, `UNION_IOU=<iou>` and `UNION_COVER=<share>` override
//! `Params::anime()`.
//! `FAST=1` runs it with Fast Scan (`as_analysis::fast`, its frames
//! decoded on the GPU where it can, `DECODER=cpu` in software; detectors one
//! frame at a time); `LOW_CONFIDENCE=<score>` sets the head score below
//! which a look's neighbours are looked at too.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use as_analysis::anime::{self, Anime, FastAnime, Params};
use as_analysis::{FastParams, fast};
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
    if let Ok(v) = std::env::var("UNION_COVER") {
        params.union_cover = v.parse().expect("UNION_COVER=<share>");
    }
    let model = |var: &str, default: &str| std::env::var(var).map_or_else(|_| models.join(default), PathBuf::from);
    let head_model = model("HEAD", "anime_head_detection/head_detect_v2.0_s/model.onnx");
    let ccip_model = model("CCIP", "ccip_onnx/ccip-caformer-24-randaug-pruned/model_feat.onnx");
    let person_model = std::env::var("PERSON").ok().map(|v| {
        if v == "1" { models.join("anime_person_detection/person_detect_v1.3_s/model.onnx") } else { PathBuf::from(v) }
    });
    let fast_params = std::env::var("FAST").is_ok_and(|v| v == "1").then(|| {
        let mut p = FastParams::anime();
        if std::env::var("DECODER").is_ok_and(|v| v == "cpu") {
            p.decoder = as_media::frames::Decoder::Cpu;
        }
        if let Ok(v) = std::env::var("LOW_CONFIDENCE") {
            p.low_confidence = v.parse().expect("LOW_CONFIDENCE=<score>");
        }
        p
    });
    let (size, batch) = match &fast_params {
        Some(p) => (fast::frame_size(&info, p), 1),
        None => (anime::frame_size(&info, &params), as_analysis::DETECT_BATCH),
    };
    let mut heads = Yolo::load(&rt, &head_model, size, input("HEAD_INPUT", (640, 384)), batch).unwrap();
    let ccip_batch: usize = std::env::var("CCIP_BATCH").map_or(32, |v| v.parse().unwrap());
    let mut ccip = Ccip::load(&rt, &ccip_model, ccip_batch).unwrap();
    let mut persons =
        person_model.map(|m| Yolo::load(&rt, &m, size, input("PERSON_INPUT", (1280, 736)), batch).unwrap());
    println!(
        "anime pass at {size:?}, stride {}, person model {}, Fast Scan {}",
        params.stride,
        persons.is_some(),
        fast_params.is_some()
    );
    let t0 = Instant::now();
    let mut all = Vec::new();
    let mut chunks = std::fs::File::create(format!("{out}.chunks")).unwrap();
    let sprites = PathBuf::from(format!("{out}.sprites"));
    std::fs::create_dir_all(&sprites).unwrap();
    let (mut sprite_count, mut sprite_bytes) = (0usize, 0usize);
    let src = Source { path: &path, info: &info, index: &index };
    let models = Anime { heads: &mut heads, persons: persons.as_mut(), ccip: &mut ccip };
    let cancel = CancelToken::new();
    let progress = |p| eprint!("\r{:5.1}% {:5.0}s", 100.0 * p, t0.elapsed().as_secs_f64());
    let on_chunk = |c: as_analysis::FaceChunk| {
        writeln!(chunks, "{} {} {} {}", c.start, c.end, c.tracks.len(), c.sprites.len()).unwrap();
        all.extend_from_slice(c.tracks);
        for s in c.sprites {
            std::fs::write(sprites.join(format!("{}.jpg", s.shot)), &s.jpeg).unwrap();
            sprite_count += 1;
            sprite_bytes += s.jpeg.len();
        }
        Ok(())
    };
    match &fast_params {
        Some(p) => as_analysis::fast_face_tracks(
            &tools,
            src,
            FastAnime { anime: models, params: &params },
            &shots,
            resume,
            2400,
            p,
            &cancel,
            progress,
            on_chunk,
        ),
        None => anime::face_tracks(&tools, src, models, &shots, resume, 2400, &params, &cancel, progress, on_chunk),
    }
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
