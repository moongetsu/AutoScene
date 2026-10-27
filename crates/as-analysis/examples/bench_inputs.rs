//! Dev tool: probe a video and write the inputs `faces_run` and `fast_run`
//! read for it — `bb.path`, `bb.info.json` and `bb.index` (its frame index)
//! — into `<out_dir>`; the shots (`bb.segments.tsv`) come from elsewhere.
//! `cargo run --release -p as-analysis --example bench_inputs -- <ffmpeg_dir> <video> <out_dir>`

use std::path::Path;

use as_core::CancelToken;
use as_media::{Tools, build_frame_index, probe};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (ffmpeg, video, out) = (Path::new(&a[0]), Path::new(&a[1]), Path::new(&a[2]));
    let tools = Tools::locate(Some(ffmpeg)).unwrap();
    std::fs::create_dir_all(out).unwrap();
    let info = probe(&tools, video).unwrap();
    let work = tempfile::tempdir_in(out).unwrap();
    let index = build_frame_index(&tools, video, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    std::fs::write(out.join("bb.path"), video.display().to_string()).unwrap();
    std::fs::write(out.join("bb.info.json"), serde_json::to_string(&info).unwrap()).unwrap();
    std::fs::write(out.join("bb.index"), index.to_bytes()).unwrap();
    println!("{} frames at {} fps", index.len(), index.nominal_fps.to_f64());
}
