//! Dev tool: run the Fast Scan face pass (`as_analysis::fast`) over a real
//! episode and save its tracks, like `faces_run` does for the full pass.
//! `cargo run --release -p as-analysis --example fast_run -- <ort_dir> <models_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path/bb.segments.tsv> <out_prefix> [resume]`
//! `<models_dir>` holds `face_detection_yunet_2023mar.onnx` and
//! `face_recognition_sface_2021dec.onnx`. Writes `<out_prefix>.tracks`
//! (as_analysis::tracks::encode of all tracks), `<out_prefix>.chunks` (one
//! `start end tracks sprites` line per chunk) and `<out_prefix>.dets.tsv`
//! (one `frame score height norm quality` line per face, to calibrate the
//! quality). `NORMS=<floor>,<full>` sets the SFace norms quality is read
//! from, `MIN_TRACK_Q=<q>` the quality below which tracks are dropped,
//! `POOL=<secs>` and `GAP=<secs>` how far apart candidates and looks lie;
//! `DET_THRESHOLD=<score>` sets YuNet's threshold, `SCORES=<floor>,<full>`
//! the scores quality is read from (equal: off), `DECODER=cpu`
//! decodes in software.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use as_analysis::{FastModels, FastParams, fast};
use as_core::{CancelToken, FrameIndex, FrameRange};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::{SFace, YuNet};

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
    let mut params = FastParams::default();
    if let Ok(v) = std::env::var("SCORES") {
        let (floor, full) = v.split_once(',').expect("SCORES=<floor>,<full>");
        (params.score_floor, params.score_full) = (floor.parse().unwrap(), full.parse().unwrap());
    }
    if std::env::var("DECODER").is_ok_and(|v| v == "cpu") {
        params.decoder = as_media::frames::Decoder::Cpu;
    }
    if let Ok(v) = std::env::var("DET_THRESHOLD") {
        params.det_threshold = v.parse().expect("DET_THRESHOLD=<score>");
    }
    if let Ok(v) = std::env::var("NORMS") {
        let (floor, full) = v.split_once(',').expect("NORMS=<floor>,<full>");
        (params.norm_floor, params.norm_full) = (floor.parse().unwrap(), full.parse().unwrap());
    }
    if let Ok(v) = std::env::var("MIN_TRACK_Q") {
        params.min_track_quality = v.parse().expect("MIN_TRACK_Q=<quality>");
    }
    if let Ok(v) = std::env::var("POOL") {
        params.pool_secs = v.parse().expect("POOL=<seconds>");
    }
    if let Ok(v) = std::env::var("GAP") {
        params.max_gap_secs = v.parse().expect("GAP=<seconds>");
    }
    let size = fast::frame_size(&info, &params);
    let mut detector = YuNet::load(&rt, &models.join("face_detection_yunet_2023mar.onnx"), size, 1).unwrap();
    let mut embedder = SFace::load(&rt, &models.join("face_recognition_sface_2021dec.onnx"), 16).unwrap();
    println!("Fast Scan at {size:?}, grid {} frames", params.grid(index.nominal_fps.to_f64()));
    let t0 = Instant::now();
    let mut all = Vec::new();
    let mut chunks = std::fs::File::create(format!("{out}.chunks")).unwrap();
    as_analysis::fast_face_tracks(
        &tools,
        Source { path: &path, info: &info, index: &index },
        FastModels { detector: &mut detector, embedder: &mut embedder },
        &shots,
        resume,
        2400,
        &params,
        &CancelToken::new(),
        |p| eprint!("\r{:5.1}% {:5.0}s", 100.0 * p, t0.elapsed().as_secs_f64()),
        |c| {
            writeln!(chunks, "{} {} {} {}", c.start, c.end, c.tracks.len(), c.sprites.len()).unwrap();
            all.extend_from_slice(c.tracks);
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
    let faces: usize = all.iter().map(|t| t.dets.len()).sum();
    let good = all.iter().filter(|t| t.quality >= 0.5).count();
    println!(
        "\n{} frames from {resume} in {secs:.0}s ({:.0} fps): {} tracks ({good} with quality >= 0.5), {faces} faces",
        index.len() - resume,
        (index.len() - resume) as f64 / secs,
        all.len()
    );
}
