//! Dev tool: measure the shot signals of a video and save them (the format
//! the app stores) for `fuse_signals`.
//! `cargo run -p as-analysis --example shot_signals -- <ort_dir> <transnet.onnx> <video> <out.sig>`

use std::path::{Path, PathBuf};
use std::time::Instant;

use as_analysis::{CHUNK, shot_signals, signals};
use as_core::CancelToken;
use as_media::{Source, Tools, build_frame_index, probe};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 {
        eprintln!("usage: shot_signals <ort_dir> <transnet.onnx> <video> <out.sig>");
        std::process::exit(2);
    }
    let rt = as_infer::Runtime::load(Path::new(&a[1]), true).unwrap();
    let mut net = as_vision::TransNet::load(&rt, Path::new(&a[2])).unwrap();
    let tools = Tools::locate(None).unwrap();
    let path = PathBuf::from(&a[3]);
    let mut info = probe(&tools, &path).unwrap();
    if info.video.field_order.is_none()
        && let Some(order) = as_media::idet::detect_field_order(&tools, &path, &info).unwrap()
    {
        info.video.field_order = Some(order.into());
    }
    let work = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let index = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    eprintln!("index: {} frames in {:.1}s", index.len(), t.elapsed().as_secs_f64());
    let t = Instant::now();
    let mut all = Vec::with_capacity(index.len() as usize);
    shot_signals(
        &tools,
        Source { path: &path, info: &info, index: &index },
        &mut net,
        0,
        CHUNK,
        &CancelToken::new(),
        |_| {},
        |_, s| {
            all.extend_from_slice(s);
            Ok(())
        },
    )
    .unwrap();
    let secs = t.elapsed().as_secs_f64();
    eprintln!("signals ({:?}): {:.1}s, {:.0} fps", net.device(), secs, all.len() as f64 / secs);
    std::fs::write(&a[4], signals::encode(&all)).unwrap();
}
