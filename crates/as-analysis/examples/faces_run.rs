//! Dev tool: run the face pass over a real episode and save its tracks.
//! `cargo run --release -p as-analysis --example faces_run -- <ort_dir> <models_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path/bb.segments.tsv> <out_prefix> [resume]`
//! Writes `<out_prefix>.tracks` (as_analysis::tracks::encode of all tracks),
//! `<out_prefix>.chunks` (one `start end tracks` line per chunk) and the
//! shot sprites as `<out_prefix>.sprites/<shot>.jpg`.
//! `EMBEDDER=<model.onnx>` embeds with another model than
//! `arcface_glintr100.onnx` (in `<models_dir>`, next to `det_10g.onnx`);
//! `NORMS=<floor>,<full>` sets the embedding lengths quality is read from,
//! `MIN_TRACK_Q=<q>` the quality below which tracks are dropped.
//! `PEOPLE=<dir with pp_det_med.onnx and reid_0265.onnx>` looks for people
//! too, as the app does, and writes them to `<out_prefix>.found`
//! (as_analysis::bodies::encode; faces by their place in `.tracks`).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use as_analysis::bodies::FoundBody;
use as_analysis::{BodyModels, Faces, PERSON_INPUT, PeopleParams};
use as_core::{CancelToken, FrameIndex, FrameRange};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::{ArcFace, Fit, Norm, PersonReid, PpDet, Scrfd};

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
    let mut params = as_analysis::FaceParams::default();
    if let Ok(v) = std::env::var("NORMS") {
        let (floor, full) = v.split_once(',').expect("NORMS=<floor>,<full>");
        (params.norm_floor, params.norm_full) = (floor.parse().unwrap(), full.parse().unwrap());
    }
    if let Ok(v) = std::env::var("MIN_TRACK_Q") {
        params.min_track_quality = v.parse().expect("MIN_TRACK_Q=<quality>");
    }
    let embedder = std::env::var("EMBEDDER").map_or_else(|_| models.join("arcface_glintr100.onnx"), PathBuf::from);
    let (size, det_size) = as_analysis::frame_sizes(&info, &params);
    let mut det = Scrfd::load(&rt, &models.join("det_10g.onnx"), det_size, as_analysis::DETECT_BATCH).unwrap();
    let mut rec = ArcFace::load(&rt, &embedder, 32).unwrap();
    let people = std::env::var("PEOPLE").ok().map(PathBuf::from);
    let mut person_models = people.as_ref().map(|dir| {
        let d = PpDet::load(&rt, &dir.join("pp_det_med.onnx"), size, PERSON_INPUT, Fit::Pad, Norm::IMAGENET).unwrap();
        (d, PersonReid::load(&rt, &dir.join("reid_0265.onnx"), 32, Norm::IMAGENET).unwrap())
    });
    let mut found: Vec<FoundBody> = Vec::new();
    let t0 = Instant::now();
    let mut all = Vec::new();
    let mut chunks = std::fs::File::create(format!("{out}.chunks")).unwrap();
    let sprites = PathBuf::from(format!("{out}.sprites"));
    std::fs::create_dir_all(&sprites).unwrap();
    let (mut sprite_count, mut sprite_bytes) = (0usize, 0usize);
    let bodies =
        person_models.as_mut().map(|(detector, reid)| BodyModels { detector, reid, params: PeopleParams::default() });
    as_analysis::face_tracks(
        &tools,
        Source { path: &path, info: &info, index: &index },
        Faces::Find { detector: &mut det, embedder: &mut rec },
        bodies,
        &shots,
        resume,
        2400,
        &params,
        &CancelToken::new(),
        |p| eprint!("\r{:5.1}% {:5.0}s", 100.0 * p, t0.elapsed().as_secs_f64()),
        |c| {
            writeln!(chunks, "{} {} {} {}", c.start, c.end, c.tracks.len(), c.sprites.len()).unwrap();
            // The chunk's faces by their place in the whole list.
            let first = all.len() as u32;
            found.extend(c.bodies.iter().cloned().map(|mut b| {
                b.faces.iter_mut().for_each(|f| f.0 += first);
                b
            }));
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
    if people.is_some() {
        std::fs::write(format!("{out}.found"), as_analysis::bodies::encode(&found)).unwrap();
        let tied = found.iter().filter(|b| !b.faces.is_empty()).count();
        println!("\n{} people found ({tied} tied to a face)", found.len());
    }
    let dets: usize = all.iter().map(|t| t.dets.len()).sum();
    let good = all.iter().filter(|t| t.quality >= 0.5).count();
    println!(
        "\n{} frames from {resume} in {secs:.0}s ({:.0} fps): {} tracks ({good} with quality >= 0.5), {dets} faces",
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
