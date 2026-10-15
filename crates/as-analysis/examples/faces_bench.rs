//! Dev tool: face detection + embedding throughput on a real episode, and
//! parity of batched against one-at-a-time inference.
//! `cargo run --release -p as-analysis --example faces_bench -- <ort_dir> <models_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path> <from> <samples> [detect batch]`

use std::path::{Path, PathBuf};
use std::time::Instant;

use as_analysis::{DETECT_BATCH, FaceParams, frame_sizes};
use as_core::{CancelToken, FrameIndex};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::arcface::{self, ArcFace};
use as_vision::{FaceBox, Scrfd, image};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (ort, models, ffmpeg, dir) = (&a[0], Path::new(&a[1]), &a[2], Path::new(&a[3]));
    let (from, samples): (u32, usize) = (a[4].parse().unwrap(), a[5].parse().unwrap());
    let batch: usize = a.get(6).map_or(DETECT_BATCH, |b| b.parse().unwrap());

    let index = FrameIndex::from_bytes(&std::fs::read(dir.join("bb.index")).unwrap()).unwrap();
    let info: MediaInfo = serde_json::from_str(&std::fs::read_to_string(dir.join("bb.info.json")).unwrap()).unwrap();
    let path = PathBuf::from(std::fs::read_to_string(dir.join("bb.path")).unwrap().trim());
    let tools = Tools::locate(Some(Path::new(ffmpeg))).unwrap();
    let rt = Runtime::load(Path::new(ort), true).unwrap();
    let params = FaceParams::default();
    let ((w, h), det_size) = frame_sizes(&info, &params);
    let halve = det_size != (w, h);

    let t = Instant::now();
    let det_model = models.join("det_10g.onnx");
    let mut det = Scrfd::load(&rt, &det_model, det_size, batch).unwrap();
    let mut det1 = Scrfd::load(&rt, &det_model, det_size, 1).unwrap();
    let mut rec = ArcFace::load(&rt, &models.join("arcface_glintr100.onnx"), 32).unwrap();
    let mut rec1 = ArcFace::load(&rt, &models.join("arcface_glintr100.onnx"), 1).unwrap();
    println!(
        "sessions {:.2?} on {:?}/{:?}; decode {w}x{h}, detect {det_size:?} in batches of {batch}",
        t.elapsed(),
        det.device(),
        rec.device()
    );

    // Decode first, so the stages below are timed on their own.
    let cancel = CancelToken::new();
    let t = Instant::now();
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut small: Vec<Vec<u8>> = Vec::new();
    let mut t_half = 0f64;
    as_media::frames::decode_samples(
        &tools,
        Source { path: &path, info: &info, index: &index },
        from,
        params.stride,
        (w as u32, h as u32),
        &cancel,
        |_, rgb| {
            frames.push(rgb.to_vec());
            if halve {
                let t = Instant::now();
                small.push(image::half(rgb, w, h));
                t_half += t.elapsed().as_secs_f64();
            }
            Ok(frames.len() < samples)
        },
    )
    .unwrap();
    let n = frames.len();
    println!("decoded {n} samples in {:.1}s", t.elapsed().as_secs_f64());
    let inputs: Vec<&[u8]> = if halve {
        small.iter().map(|f| f.as_slice()).collect()
    } else {
        frames.iter().map(|f| f.as_slice()).collect()
    };

    let _ = det.detect_many(&inputs[..batch.min(n)], params.det_threshold).unwrap(); // warm-up
    let _ = det1.detect_many(&inputs[..1], params.det_threshold).unwrap();
    let t = Instant::now();
    let found = det.detect_many(&inputs, params.det_threshold).unwrap();
    let t_det = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let single = det1.detect_many(&inputs, params.det_threshold).unwrap();
    let t_det1 = t.elapsed().as_secs_f64();
    let close = |a: &[FaceBox], b: &[FaceBox]| {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.bbox.iter().zip(y.bbox).all(|(p, q)| (p - q).abs() < 0.05))
    };
    let differ = found.iter().zip(&single).filter(|(a, b)| !close(a, b)).count();
    println!(
        "detect {:.2} ms/frame at batch {batch}, {:.2} at batch 1 (+ half {:.2}); frames whose boxes differ: {differ}",
        1000.0 * t_det / n as f64,
        1000.0 * t_det1 / n as f64,
        1000.0 * t_half / n as f64
    );

    let k = if halve { 2.0 } else { 1.0 };
    let (mut crops, mut t_align, mut t_thumb, mut small_faces) = (Vec::new(), 0f64, 0f64, 0usize);
    for (rgb, faces) in frames.iter().zip(&found) {
        for f in faces.iter().map(|f| f.scaled(k)) {
            if f.height() < 48.0 {
                small_faces += 1;
            }
            let t = Instant::now();
            crops.push(arcface::align(rgb, w, h, &f.kps));
            t_align += t.elapsed().as_secs_f64();
            let t = Instant::now();
            let th = image::face_thumb(rgb, w, h, f.bbox, params.thumb_context, params.thumb_size);
            std::hint::black_box(image::jpeg(&th, params.thumb_size, params.thumb_size, 85));
            t_thumb += t.elapsed().as_secs_f64();
        }
    }
    println!(
        "{} faces ({small_faces} under 48 px): align {:.2} ms/sample, thumbs {:.2} ms/sample",
        crops.len(),
        1000.0 * t_align / n as f64,
        1000.0 * t_thumb / n as f64
    );

    let refs: Vec<&[u8]> = crops.iter().map(|c| c.as_slice()).collect();
    let _ = rec.embed(&refs[..refs.len().min(32)]).unwrap(); // warm-up
    let t = Instant::now();
    let embs = rec.embed(&refs).unwrap();
    let t_rec = t.elapsed().as_secs_f64();
    let some = &refs[..refs.len().min(64)];
    let t = Instant::now();
    let embs1 = rec1.embed(some).unwrap();
    let t_rec1 = t.elapsed().as_secs_f64() / some.len().max(1) as f64;
    let worst = embs.iter().zip(&embs1).map(|(a, b)| arcface::cosine(&a.v, &b.v)).fold(1f32, f32::min);
    println!(
        "embed {:.0} faces/s at batch 32 vs {:.0}/s at batch 1 = {:.2} ms/sample; batch-vs-single min cosine {worst:.5}",
        embs.len() as f64 / t_rec,
        1.0 / t_rec1,
        1000.0 * t_rec / n as f64
    );
}
