//! Dev tool: run a person detector and the person ReID over the samples of a
//! real episode's face pass (every `stride`-th frame, in shots) and save
//! every person found (`common/bodyfile.rs`), so the body analysis
//! (`bodies_bench`) can be iterated without the GPU.
//! `cargo run --release -p as-analysis --example bodies_run -- <ort_dir> <persons_dir> <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path/bb.segments.tsv> <out.bodies>`
//! `DETECTOR=pp_det_med:640x640:squeeze` (default), `pp_det_med:640x384:pad` or
//! `scrfd_person` (on half-size frames, batch 4); `REID=<file in persons_dir>`
//! (default `reid_0265_b.onnx`, batch 32); `MIN_SCORE` (0.3) keeps weaker
//! people for the tracker; `REID_MIN_H` (0.08: share of the frame height)
//! is the smallest person embedded; `NOOP=1` only decodes; `CPU=1`.

#[path = "common/bodyfile.rs"]
mod bodyfile;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use as_analysis::{FaceParams, frame_sizes};
use as_core::{CancelToken, FrameIndex, FrameRange};
use as_infer::Runtime;
use as_media::{MediaInfo, Source, Tools};
use as_vision::person::{self, Fit, Norm, PersonBox, PpDet};
use as_vision::scrfd::Heads;
use as_vision::{PersonReid, Scrfd, image};
use bodyfile::{Person, Sample};

#[allow(clippy::large_enum_variant)]
enum Det {
    Pp(PpDet),
    Scrfd(Scrfd),
}

/// A sample waiting for the ReID: its people, their crops, and which of
/// them each crop is.
type Pending = (Sample, Vec<Vec<u8>>, Vec<usize>);

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().map_or(default, |v| v.parse().ok().expect(name))
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (ort, dir_p, ffmpeg, dir, out) =
        (Path::new(&a[0]), Path::new(&a[1]), &a[2], Path::new(&a[3]), Path::new(&a[4]));
    let detector = std::env::var("DETECTOR").unwrap_or_else(|_| "pp_det_med:640x640:squeeze".into());
    let reid_file = std::env::var("REID").unwrap_or_else(|_| "reid_0265_b.onnx".into());
    let min_score: f32 = env("MIN_SCORE", 0.3);
    let reid_min_h: f32 = env("REID_MIN_H", 0.08);
    let noop = std::env::var_os("NOOP").is_some();
    let gpu = std::env::var_os("CPU").is_none();

    let index = FrameIndex::from_bytes(&std::fs::read(dir.join("bb.index")).unwrap()).unwrap();
    let info: MediaInfo = serde_json::from_str(&std::fs::read_to_string(dir.join("bb.info.json")).unwrap()).unwrap();
    let path = PathBuf::from(std::fs::read_to_string(dir.join("bb.path")).unwrap().trim());
    let shots: Vec<FrameRange> = std::fs::read_to_string(dir.join("bb.segments.tsv"))
        .unwrap()
        .lines()
        .filter_map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            (v[2].trim() == "shot").then(|| FrameRange::new(v[0].parse().unwrap(), v[1].parse().unwrap()))
        })
        .collect();
    let tools = Tools::locate(Some(Path::new(ffmpeg))).unwrap();
    let rt = Runtime::load(ort, gpu).unwrap();
    let params = FaceParams::default();
    let stride = params.stride;
    let ((w, h), half) = frame_sizes(&info, &params);
    // SCRFD runs where the face pass's detector does: at half size for frames wider than 960.
    let (halve, k) = if half != (w, h) { (true, 2.0) } else { (false, 1.0) };

    let (batch, mut det) = match detector.split(':').collect::<Vec<_>>()[..] {
        ["scrfd_person"] => {
            let d = Scrfd::load_heads(&rt, &dir_p.join("scrfd_person_2.5g.onnx"), half, 4, Heads::PERSONS).unwrap();
            (4, Det::Scrfd(d.with_nms_iou(0.6)))
        }
        [model, size, fit] => {
            let (iw, ih) = size.split_once('x').unwrap();
            let fit = if fit == "pad" { Fit::Pad } else { Fit::Squeeze };
            let input = (iw.parse().unwrap(), ih.parse().unwrap());
            let d = PpDet::load(&rt, &dir_p.join(format!("{model}.onnx")), (w, h), input, fit, Norm::IMAGENET).unwrap();
            (1, Det::Pp(d))
        }
        _ => panic!("DETECTOR={detector}"),
    };
    let mut reid = PersonReid::load(&rt, &dir_p.join(&reid_file), 32, Norm::IMAGENET).unwrap();
    let in_shot = |f: u32| {
        let i = shots.partition_point(|s| s.end <= f);
        shots.get(i).is_some_and(|s| s.contains(f))
    };

    // The detector runs on a thread of its own, a batch of samples at a
    // time (as in the face pass); this one crops, embeds and writes.
    let wall = Instant::now();
    let cancel = CancelToken::new();
    let (mut t_detect, mut t_crop, mut t_reid, mut t_measure) = (0f64, 0f64, 0f64, 0f64);
    let (mut body, mut count, mut people) = (Vec::new(), 0u32, 0usize);
    let mut pending: Vec<Pending> = Vec::new();
    let mut embed = |pending: &mut Vec<Pending>, body: &mut Vec<u8>, t_reid: &mut f64| {
        let crops: Vec<&[u8]> = pending.iter().flat_map(|(_, c, _)| c.iter().map(Vec::as_slice)).collect();
        let t = Instant::now();
        let embs = if crops.is_empty() { Vec::new() } else { reid.embed(&crops).unwrap() };
        *t_reid += t.elapsed().as_secs_f64();
        let mut e = embs.into_iter();
        for (mut s, _, which) in pending.drain(..) {
            for i in which {
                s.people[i].reid = Some(e.next().expect("one embedding per crop").v);
            }
            bodyfile::encode_sample(body, &s);
        }
    };
    std::thread::scope(|scope| {
        let (to_det, frames) = mpsc::sync_channel::<Vec<(u32, Vec<u8>)>>(1);
        let (found_tx, found) = mpsc::channel::<(Vec<(u32, Vec<u8>)>, Vec<Vec<PersonBox>>, f64)>();
        let det = &mut det;
        scope.spawn(move || {
            for group in frames {
                let t = Instant::now();
                let people: Vec<Vec<PersonBox>> = if noop {
                    vec![Vec::new(); group.len()]
                } else {
                    match det {
                        Det::Pp(d) => group.iter().map(|(_, rgb)| d.detect(rgb, min_score).unwrap()).collect(),
                        Det::Scrfd(d) => {
                            let small: Vec<Vec<u8>> = if halve {
                                group.iter().map(|(_, rgb)| image::half(rgb, w, h)).collect()
                            } else {
                                group.iter().map(|(_, rgb)| rgb.clone()).collect()
                            };
                            let refs: Vec<&[u8]> = small.iter().map(Vec::as_slice).collect();
                            d.detect_many(&refs, min_score)
                                .unwrap()
                                .into_iter()
                                .map(|f| f.iter().map(|b| person::from_scrfd(&b.scaled(k), (w, h))).collect())
                                .collect()
                        }
                    }
                };
                if found_tx.send((group, people, t.elapsed().as_secs_f64())).is_err() {
                    return;
                }
            }
        });
        let mut take = |group: Vec<(u32, Vec<u8>)>, found: Vec<Vec<PersonBox>>, secs: f64| {
            t_detect += secs;
            for ((frame, rgb), ps) in group.into_iter().zip(found) {
                let t = Instant::now();
                let mut small = vec![0u8; 320 * 180 * 3];
                image::shrink_into(&rgb, (w, h), &mut small, 320, 0, (320, 180));
                let mut s = Sample {
                    frame,
                    sharpness: image::sharpness(&small, 320, 180),
                    luma: image::mean_luma(&small),
                    people: Vec::new(),
                };
                t_measure += t.elapsed().as_secs_f64();
                let (mut crops, mut which) = (Vec::new(), Vec::new());
                for p in &ps {
                    let t = Instant::now();
                    let crop = PersonReid::crop(&rgb, (w, h), p.visible);
                    t_crop += t.elapsed().as_secs_f64();
                    let t = Instant::now();
                    let (sharpness, luma) = (
                        image::sharpness(&crop, as_vision::reid::WIDTH, as_vision::reid::HEIGHT),
                        image::mean_luma(&crop),
                    );
                    t_measure += t.elapsed().as_secs_f64();
                    let frac = |b: [f32; 4]| [b[0] / w as f32, b[1] / h as f32, b[2] / w as f32, b[3] / h as f32];
                    if p.height() >= reid_min_h * h as f32 {
                        which.push(s.people.len());
                        crops.push(crop);
                    }
                    s.people.push(Person {
                        bbox: frac(p.bbox),
                        visible: frac(p.visible),
                        score: p.score,
                        sharpness,
                        luma,
                        reid: None,
                    });
                }
                people += s.people.len();
                count += 1;
                pending.push((s, crops, which));
            }
            if pending.iter().map(|p| p.1.len()).sum::<usize>() >= 128 {
                embed(&mut pending, &mut body, &mut t_reid);
            }
        };
        let mut queue: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut in_flight = 0usize;
        let send = |queue: &mut Vec<(u32, Vec<u8>)>, in_flight: &mut usize| {
            if !queue.is_empty() {
                to_det.send(std::mem::take(queue)).unwrap();
                *in_flight += 1;
            }
        };
        let n = index.len();
        let delivered = as_media::frames::decode_samples(
            &tools,
            Source { path: &path, info: &info, index: &index },
            0,
            stride,
            (w as u32, h as u32),
            &cancel,
            |f, rgb| {
                if f % (stride * 2000) == 0 {
                    eprint!("  {:.0}%", 100.0 * f as f64 / n as f64);
                }
                if in_shot(f) {
                    queue.push((f, rgb.to_vec()));
                    if queue.len() >= batch {
                        send(&mut queue, &mut in_flight);
                    }
                }
                while let Ok((g, p, s)) = found.try_recv() {
                    in_flight -= 1;
                    take(g, p, s);
                }
                Ok(true)
            },
        )
        .unwrap();
        send(&mut queue, &mut in_flight);
        drop(to_det);
        while in_flight > 0 {
            let (g, p, s) = found.recv().unwrap();
            in_flight -= 1;
            take(g, p, s);
        }
        eprintln!();
        assert_eq!(delivered, n.div_ceil(stride), "every sample decoded");
    });
    embed(&mut pending, &mut body, &mut t_reid);
    let secs = wall.elapsed().as_secs_f64();
    let header = serde_json::json!({
        "detector": detector, "reid": reid_file, "stride": stride, "min_score": min_score,
        "reid_min_h": reid_min_h, "frame": [w, h], "device": format!("{:?}", reid.device()),
        "busy_s": { "detect": t_detect, "crop": t_crop, "measure": t_measure, "reid": t_reid }, "wall_s": secs,
    });
    bodyfile::write(out, &header, count, &body).unwrap();
    println!(
        "{count} samples, {people} people in {secs:.0}s; busy: detect {t_detect:.1}s ({:.2} ms/sample), crop {t_crop:.1}s, measure {t_measure:.1}s, ReID {t_reid:.1}s",
        1000.0 * t_detect / count.max(1) as f64
    );
}
