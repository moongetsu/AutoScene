//! Dev tool: what the face detector finds in chosen frames at each size it
//! can look at, and what each size costs.
//! `cargo run --release -p as-analysis --example detect_bench -- <ort_dir> <models_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path> <frames.txt> <out_dir>`
//! `frames.txt` holds one frame number per line. Each frame is decoded as
//! the face pass decodes its samples and saved as `<out_dir>/frames/<frame>.jpg`;
//! `<out_dir>/dets.json` lists, per frame, every face found down to score
//! `LOW` (default 0.2) at the decoded size (`"1"`), half of it (`"2"`) and a
//! quarter (`"4"`), as `[x1, y1, x2, y2, score]` in decoded pixels.
//! Then each size is timed over all frames (`REPEAT` rounds, default 3), and
//! the face pass's own search ([`as_analysis::find_faces`]) for live action
//! and for 3D animation (`FaceParams::animated`), whose faces go to
//! `<out_dir>/search_<Once|Scales>.json`. The decoded frames are kept in
//! `<out_dir>/frames.rgb` and read from there by the next run.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use as_analysis::{DETECT_BATCH, FaceParams, detect_looks, frame_sizes};
use as_core::{CancelToken, FrameIndex};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::{Scrfd, image};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (ort, models, ffmpeg, dir) = (&a[0], Path::new(&a[1]), &a[2], Path::new(&a[3]));
    let (list, out) = (&a[4], Path::new(&a[5]));
    let low: f32 = std::env::var("LOW").map_or(0.2, |v| v.parse().unwrap());
    let repeat: usize = std::env::var("REPEAT").map_or(3, |v| v.parse().unwrap());
    let wanted: Vec<u32> = std::fs::read_to_string(list)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().unwrap())
        .collect();

    let index = FrameIndex::from_bytes(&std::fs::read(dir.join("bb.index")).unwrap()).unwrap();
    let info: MediaInfo = serde_json::from_str(&std::fs::read_to_string(dir.join("bb.info.json")).unwrap()).unwrap();
    let path = PathBuf::from(std::fs::read_to_string(dir.join("bb.path")).unwrap().trim());
    let tools = Tools::locate(Some(Path::new(ffmpeg))).unwrap();
    let rt = Runtime::load(Path::new(ort), true).unwrap();
    let params = FaceParams::default();
    let ((w, h), _) = frame_sizes(&info, &params);
    std::fs::create_dir_all(out.join("frames")).unwrap();

    // Each frame on its own, exactly as the pass decodes a sample there
    // (kept raw in `<out_dir>/frames.rgb` for the next run).
    let cancel = CancelToken::new();
    let t = Instant::now();
    let (raw, raw_list) = (out.join("frames.rgb"), out.join("frames.rgb.list"));
    let list_text: String = wanted.iter().map(|f| format!("{f}\n")).collect();
    let cached = std::fs::read_to_string(&raw_list).is_ok_and(|l| l == list_text);
    let mut frames: Vec<Vec<u8>> = std::fs::read(&raw)
        .ok()
        .filter(|b| cached && b.len() == wanted.len() * w * h * 3)
        .map(|b| b.chunks_exact(w * h * 3).map(<[u8]>::to_vec).collect())
        .unwrap_or_default();
    let fresh = frames.is_empty();
    for &f in wanted.iter().skip(frames.len()) {
        let mut got = None;
        as_media::frames::decode_samples(
            &tools,
            Source { path: &path, info: &info, index: &index },
            f,
            params.stride,
            (w as u32, h as u32),
            &cancel,
            |_, rgb| {
                got = Some(rgb.to_vec());
                Ok(false)
            },
        )
        .unwrap();
        let rgb = got.expect("a frame");
        std::fs::write(out.join("frames").join(format!("{f}.jpg")), image::jpeg(&rgb, w, h, 92)).unwrap();
        frames.push(rgb);
    }
    if fresh {
        std::fs::write(&raw, frames.concat()).unwrap();
        std::fs::write(&raw_list, list_text).unwrap();
    }
    println!("decoded {} frames of {w}x{h} in {:.1}s", frames.len(), t.elapsed().as_secs_f64());

    // The frame at each size: shrunk 1, 2 and 4 times.
    let shrinks = [1usize, 2, 4];
    let sized: Vec<Vec<Vec<u8>>> = shrinks
        .iter()
        .map(|&k| {
            frames
                .iter()
                .map(|rgb| {
                    let mut x = rgb.clone();
                    let (mut sw, mut sh) = (w, h);
                    while sw > w / k {
                        x = image::half(&x, sw, sh);
                        (sw, sh) = (sw / 2, sh / 2);
                    }
                    x
                })
                .collect()
        })
        .collect();
    let model = models.join("det_10g.onnx");
    let mut detectors: Vec<Scrfd> =
        shrinks.iter().map(|&k| Scrfd::load(&rt, &model, (w / k, h / k), DETECT_BATCH).unwrap()).collect();

    let mut json = String::from("[");
    let found: Vec<Vec<Vec<as_vision::FaceBox>>> = detectors
        .iter_mut()
        .zip(&sized)
        .map(|(d, imgs)| {
            let refs: Vec<&[u8]> = imgs.iter().map(Vec::as_slice).collect();
            d.detect_many(&refs, low).unwrap()
        })
        .collect();
    for (i, &f) in wanted.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        write!(json, "{{\"frame\":{f},\"w\":{w},\"h\":{h}").unwrap();
        for (s, &k) in shrinks.iter().enumerate() {
            write!(json, ",\"{k}\":[").unwrap();
            for (j, b) in found[s][i].iter().map(|b| b.scaled(k as f32)).enumerate() {
                if j > 0 {
                    json.push(',');
                }
                let [x1, y1, x2, y2] = b.bbox;
                write!(json, "[{x1:.1},{y1:.1},{x2:.1},{y2:.1},{:.4}]", b.score).unwrap();
            }
            json.push(']');
        }
        json.push('}');
    }
    json.push(']');
    std::fs::write(out.join("dets.json"), json).unwrap();

    // Cost of each size alone (detector only), at the live-action threshold.
    let n = frames.len();
    for ((d, imgs), k) in detectors.iter_mut().zip(&sized).zip(shrinks) {
        let refs: Vec<&[u8]> = imgs.iter().map(Vec::as_slice).collect();
        let _ = d.detect_many(&refs[..DETECT_BATCH.min(n)], 0.5).unwrap(); // warm-up
        let mut best = f64::MAX;
        for _ in 0..repeat {
            let t = Instant::now();
            std::hint::black_box(d.detect_many(&refs, 0.5).unwrap());
            best = best.min(t.elapsed().as_secs_f64());
        }
        println!("1/{k} ({}x{}): {:.2} ms/frame on {:?}", w / k, h / k, 1000.0 * best / n as f64, d.device());
    }

    // The pass's own search, shrinking included, per footage.
    let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    drop(detectors);
    for p in [FaceParams::default(), FaceParams::animated()] {
        let looks = detect_looks(&info, &p);
        let mut ds: Vec<Scrfd> =
            looks.iter().map(|l| Scrfd::load(&rt, &model, l.size, DETECT_BATCH).unwrap()).collect();
        let _ = as_analysis::find_faces(&mut ds, &looks, (w, h), &p, &refs[..DETECT_BATCH.min(n)]).unwrap();
        let mut best = f64::MAX;
        let mut got = Vec::new();
        for _ in 0..repeat {
            let t = Instant::now();
            got = as_analysis::find_faces(&mut ds, &looks, (w, h), &p, &refs).unwrap();
            best = best.min(t.elapsed().as_secs_f64());
        }
        let faces: usize = got.iter().map(Vec::len).sum();
        // What it kept, to check against the per-size boxes above.
        let mut kept = String::from("[");
        for (i, (f, found)) in wanted.iter().zip(&got).enumerate() {
            let boxes: Vec<String> = found
                .iter()
                .map(|b| {
                    format!("[{:.1},{:.1},{:.1},{:.1},{:.4}]", b.bbox[0], b.bbox[1], b.bbox[2], b.bbox[3], b.score)
                })
                .collect();
            write!(kept, "{}{{\"frame\":{f},\"faces\":[{}]}}", if i > 0 { "," } else { "" }, boxes.join(",")).unwrap();
        }
        kept.push(']');
        std::fs::write(out.join(format!("search_{:?}.json", p.search)), kept).unwrap();
        println!(
            "{:?} at {} {looks:?}: {:.2} ms/frame, {faces} faces",
            p.search,
            p.det_threshold,
            1000.0 * best / n as f64
        );
    }
}
