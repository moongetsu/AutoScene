//! Dev tool: the person detectors and the person ReID on a real episode —
//! speed on the GPU and the CPU, GPU-vs-CPU parity, batch-vs-single parity.
//! `cargo run --release -p as-analysis --example persons_bench -- <ort_dir> <persons_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path> <from,from,...> <samples per start>`
//! `<persons_dir>` holds InsightFace's `pp_det_med.onnx`, `pp_det_small_320.onnx`,
//! `scrfd_person_2.5g.onnx`, `reid_0265.onnx` (batch 1) and `reid_0265_b.onnx`
//! (its copy that batches: `tools/model-prep/inspect_onnx.py free-batch`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use as_analysis::{FaceParams, frame_sizes};
use as_core::{CancelToken, FrameIndex};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::person::{self, Fit, Norm, PersonBox, PpDet};
use as_vision::scrfd::{Heads, iou};
use as_vision::{PersonReid, Scrfd, arcface, image};

/// Frames per start checked on the CPU too (it is slow).
const CPU_FRAMES: usize = 20;
const THRESHOLD: f32 = 0.5;
const SCRFD_BATCH: usize = 4;

/// A detector under test: one configuration on the GPU and on the CPU.
#[allow(clippy::large_enum_variant)]
enum Det {
    Pp(PpDet),
    /// SCRFD's person model on half-size frames.
    Scrfd(Scrfd),
}

impl Det {
    fn detect(&mut self, frames: &[&[u8]], half: &[&[u8]], (w, h): (usize, usize)) -> Vec<Vec<PersonBox>> {
        match self {
            Det::Pp(d) => frames.iter().map(|f| d.detect(f, THRESHOLD).unwrap()).collect(),
            Det::Scrfd(d) => d
                .detect_many(half, THRESHOLD)
                .unwrap()
                .into_iter()
                .map(|found| found.iter().map(|b| person::from_scrfd(&b.scaled(2.0), (w, h))).collect())
                .collect(),
        }
    }
}

#[derive(Default)]
struct Stats {
    frames: usize,
    secs: f64,
    cpu_frames: usize,
    cpu_secs: f64,
    found: usize,
    tall: usize,
    /// GPU boxes with a CPU box at IoU ≥ 0.9 / without; CPU boxes without a GPU box.
    matched: usize,
    gpu_only: usize,
    cpu_only: usize,
    max_dscore: f32,
}

/// Pair the people of two runs on the same frame (IoU ≥ 0.9).
fn compare(a: &[PersonBox], b: &[PersonBox], s: &mut Stats) {
    let mut used = vec![false; b.len()];
    for p in a {
        let best = (0..b.len())
            .filter(|&j| !used[j])
            .max_by(|&i, &j| iou(&p.bbox, &b[i].bbox).total_cmp(&iou(&p.bbox, &b[j].bbox)));
        match best.filter(|&j| iou(&p.bbox, &b[j].bbox) >= 0.9) {
            Some(j) => {
                used[j] = true;
                s.matched += 1;
                s.max_dscore = s.max_dscore.max((p.score - b[j].score).abs());
            }
            None => s.gpu_only += 1,
        }
    }
    s.cpu_only += used.iter().filter(|u| !**u).count();
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (ort, dir_p, ffmpeg, dir) = (Path::new(&a[0]), Path::new(&a[1]), &a[2], Path::new(&a[3]));
    let starts: Vec<u32> = a[4].split(',').map(|s| s.parse().unwrap()).collect();
    let samples: usize = a[5].parse().unwrap();

    let index = FrameIndex::from_bytes(&std::fs::read(dir.join("bb.index")).unwrap()).unwrap();
    let info: MediaInfo = serde_json::from_str(&std::fs::read_to_string(dir.join("bb.info.json")).unwrap()).unwrap();
    let path = PathBuf::from(std::fs::read_to_string(dir.join("bb.path")).unwrap().trim());
    let tools = Tools::locate(Some(Path::new(ffmpeg))).unwrap();
    let gpu = Runtime::load(ort, true).unwrap();
    let cpu = gpu.on_cpu();
    let params = FaceParams::default();
    let ((w, h), half_size) = frame_sizes(&info, &params);
    // Benchmarks the face pass's size for HD sources: SCRFD on half frames,
    // pp_det squeezed into 640×640.
    assert!(half_size != (w, h) && h >= 640, "persons_bench takes sources wider than 960 and at least 640 tall");

    let med = dir_p.join("pp_det_med.onnx");
    let small = dir_p.join("pp_det_small_320.onnx");
    let scrfd = dir_p.join("scrfd_person_2.5g.onnx");
    let pp =
        |rt: &Runtime, m: &Path, input, fit| Det::Pp(PpDet::load(rt, m, (w, h), input, fit, Norm::IMAGENET).unwrap());
    let sc = |rt: &Runtime, batch| {
        Det::Scrfd(Scrfd::load_heads(rt, &scrfd, half_size, batch, Heads::PERSONS).unwrap().with_nms_iou(0.6))
    };
    let t = Instant::now();
    let mut dets: Vec<(&str, Det, Det)> = vec![
        (
            "pp_det_med 640x640 squeeze",
            pp(&gpu, &med, (640, 640), Fit::Squeeze),
            pp(&cpu, &med, (640, 640), Fit::Squeeze),
        ),
        ("pp_det_med 640x384 pad", pp(&gpu, &med, (640, 384), Fit::Pad), pp(&cpu, &med, (640, 384), Fit::Pad)),
        (
            "pp_det_small 320x320 squeeze",
            pp(&gpu, &small, (320, 320), Fit::Squeeze),
            pp(&cpu, &small, (320, 320), Fit::Squeeze),
        ),
        ("scrfd_person 640x384 b1", sc(&gpu, 1), sc(&cpu, 1)),
    ];
    let mut scrfd_batched =
        Scrfd::load_heads(&gpu, &scrfd, half_size, SCRFD_BATCH, Heads::PERSONS).unwrap().with_nms_iou(0.6);
    let mut reid1 = PersonReid::load(&gpu, &dir_p.join("reid_0265.onnx"), 1, Norm::IMAGENET).unwrap();
    let mut reid32 = PersonReid::load(&gpu, &dir_p.join("reid_0265_b.onnx"), 32, Norm::IMAGENET).unwrap();
    let mut reid_cpu = PersonReid::load(&cpu, &dir_p.join("reid_0265.onnx"), 1, Norm::IMAGENET).unwrap();
    println!(
        "sessions {:.1?}; frames {w}x{h}, SCRFD on {half_size:?}; ReID on {:?}/{:?}",
        t.elapsed(),
        reid32.device(),
        reid_cpu.device()
    );

    let mut stats: Vec<Stats> = (0..dets.len()).map(|_| Stats::default()).collect();
    let mut batched = Stats::default();
    let (mut crops, mut t_shrink) = (Vec::new(), 0f64);
    let cancel = CancelToken::new();
    let mut warm = false;
    for &from in &starts {
        let mut frames: Vec<Vec<u8>> = Vec::new();
        as_media::frames::decode_samples(
            &tools,
            Source { path: &path, info: &info, index: &index },
            from - from % params.stride,
            params.stride,
            (w as u32, h as u32),
            &cancel,
            |_, rgb| {
                frames.push(rgb.to_vec());
                Ok(frames.len() < samples)
            },
        )
        .unwrap();
        let halves: Vec<Vec<u8>> = frames.iter().map(|f| image::half(f, w, h)).collect();
        let full: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
        let half: Vec<&[u8]> = halves.iter().map(Vec::as_slice).collect();
        let n = full.len();
        let cn = n.min(CPU_FRAMES);
        if !warm {
            for (_, g, _) in &mut dets {
                g.detect(&full[..1], &half[..1], (w, h));
            }
            scrfd_batched.detect_many(&half[..SCRFD_BATCH.min(n)], THRESHOLD).unwrap();
            warm = true;
        }
        let mut canvas = vec![0u8; 640 * 640 * 3];
        let t = Instant::now();
        for f in &full {
            image::shrink_into(f, (w, h), &mut canvas, 640, 0, (640, 640));
        }
        t_shrink += t.elapsed().as_secs_f64();
        for (k, (_, g, c)) in dets.iter_mut().enumerate() {
            let s = &mut stats[k];
            let t = Instant::now();
            let found = g.detect(&full, &half, (w, h));
            s.secs += t.elapsed().as_secs_f64();
            s.frames += n;
            s.found += found.iter().map(Vec::len).sum::<usize>();
            s.tall += found.iter().flatten().filter(|p| p.height() >= 0.08 * h as f32).count();
            let t = Instant::now();
            let on_cpu = c.detect(&full[..cn], &half[..cn], (w, h));
            s.cpu_secs += t.elapsed().as_secs_f64();
            s.cpu_frames += cn;
            for (a, b) in found.iter().zip(&on_cpu) {
                compare(a, b, s);
            }
            if k == 0 {
                for (f, people) in full.iter().zip(&found) {
                    for p in people.iter().filter(|p| p.height() >= 0.08 * h as f32) {
                        crops.push(PersonReid::crop(f, (w, h), p.visible));
                    }
                }
            }
            if k == 3 {
                let t = Instant::now();
                let many = scrfd_batched.detect_many(&half, THRESHOLD).unwrap();
                batched.secs += t.elapsed().as_secs_f64();
                batched.frames += n;
                for (a, b) in many.iter().zip(&found) {
                    let a: Vec<PersonBox> = a.iter().map(|x| person::from_scrfd(&x.scaled(2.0), (w, h))).collect();
                    compare(&a, b, &mut batched);
                }
            }
        }
        println!("  {n} samples from {from} done");
    }
    println!("shrink 1280x720 -> 640x640 on the CPU: {:.2} ms/frame", 1000.0 * t_shrink / stats[0].frames as f64);
    for ((name, g, _), s) in dets.iter().zip(&stats) {
        let device = match g {
            Det::Pp(d) => d.device(),
            Det::Scrfd(d) => d.device(),
        };
        println!(
            "{name:30} {device:?} {:6.2} ms/frame | CPU {:6.1} ms/frame | {:.2} people/frame ({:.2} >= 8% tall) | GPU vs CPU: {} matched, {} GPU only, {} CPU only, max dscore {:.3}",
            1000.0 * s.secs / s.frames as f64,
            1000.0 * s.cpu_secs / s.cpu_frames.max(1) as f64,
            s.found as f64 / s.frames as f64,
            s.tall as f64 / s.frames as f64,
            s.matched,
            s.gpu_only,
            s.cpu_only,
            s.max_dscore
        );
    }
    println!(
        "scrfd_person batch {SCRFD_BATCH}: {:.2} ms/frame; vs batch 1: {} matched, {} only batched, {} only single, max dscore {:.3}",
        1000.0 * batched.secs / batched.frames as f64,
        batched.matched,
        batched.gpu_only,
        batched.cpu_only,
        batched.max_dscore
    );

    let refs: Vec<&[u8]> = crops.iter().map(Vec::as_slice).collect();
    let _ = reid32.embed(&refs[..refs.len().min(32)]).unwrap();
    let _ = reid1.embed(&refs[..1]).unwrap();
    let t = Instant::now();
    let many = reid32.embed(&refs).unwrap();
    let t32 = t.elapsed().as_secs_f64() / refs.len() as f64;
    let some = &refs[..refs.len().min(256)];
    let t = Instant::now();
    let single = reid1.embed(some).unwrap();
    let t1 = t.elapsed().as_secs_f64() / some.len() as f64;
    let few = &refs[..refs.len().min(64)];
    let t = Instant::now();
    let on_cpu = reid_cpu.embed(few).unwrap();
    let tc = t.elapsed().as_secs_f64() / few.len() as f64;
    let worst = |a: &[as_vision::Embedding], b: &[as_vision::Embedding]| {
        a.iter().zip(b).map(|(x, y)| arcface::cosine(&x.v, &y.v)).fold(1f32, f32::min)
    };
    println!(
        "ReID on {} crops: batch 32 {:.3} ms/crop, batch 1 {:.3} ms/crop, CPU {:.2} ms/crop; min cosine batch-vs-single {:.5}, GPU-vs-CPU {:.5}",
        refs.len(),
        1000.0 * t32,
        1000.0 * t1,
        1000.0 * tc,
        worst(&many, &single),
        worst(&single, &on_cpu)
    );
}
