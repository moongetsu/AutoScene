//! Dev tool: run the face pass over a real episode and save its tracks.
//! `cargo run --release -p as-analysis --example faces_run -- <ort_dir> <models_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path/bb.segments.tsv> <out_prefix> [resume]`
//! Writes `<out_prefix>.tracks` (as_analysis::tracks::encode of all tracks)
//! and `<out_prefix>.chunks` (one `start end tracks` line per chunk).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use as_core::{CancelToken, FrameIndex, FrameRange};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::{ArcFace, Scrfd};

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
    let params = as_analysis::FaceParams::default();
    let (_, det_size) = as_analysis::frame_sizes(&info, &params);
    let mut det = Scrfd::load(&rt, &models.join("det_10g.onnx"), det_size, as_analysis::DETECT_BATCH).unwrap();
    let mut rec = ArcFace::load(&rt, &models.join("w600k_r50.onnx"), 32).unwrap();
    let t0 = Instant::now();
    let mut all = Vec::new();
    let mut chunks = std::fs::File::create(format!("{out}.chunks")).unwrap();
    as_analysis::face_tracks(
        &tools,
        Source { path: &path, info: &info, index: &index },
        &mut det,
        &mut rec,
        &shots,
        resume,
        2400,
        &params,
        &CancelToken::new(),
        |p| eprint!("\r{:5.1}% {:5.0}s", 100.0 * p, t0.elapsed().as_secs_f64()),
        |s, e, tracks| {
            writeln!(chunks, "{s} {e} {}", tracks.len()).unwrap();
            all.extend_from_slice(tracks);
            Ok(())
        },
    )
    .unwrap();
    let secs = t0.elapsed().as_secs_f64();
    std::fs::write(format!("{out}.tracks"), as_analysis::tracks::encode(&all)).unwrap();
    let dets: usize = all.iter().map(|t| t.dets.len()).sum();
    let good = all.iter().filter(|t| t.quality >= 0.5).count();
    println!(
        "\n{} frames from {resume} in {secs:.0}s ({:.0} fps): {} tracks ({good} with quality >= 0.5), {dets} faces",
        index.len() - resume,
        (index.len() - resume) as f64 / secs,
        all.len()
    );
}
