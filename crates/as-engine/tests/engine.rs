//! The engine process end to end, including a hard kill in the middle of a
//! job. Needs `cargo xtask fixtures`, `fetch-sidecars` and `fetch-models`;
//! skips with a note when they're missing.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use as_core::CancelToken;
use as_engine::{Config, Engine, EngineError, FacesEvent, Launch, ShotsEvent};
use as_media::{MediaInfo, Tools, build_frame_index, probe};
use as_testkit::{fixtures_dir, load_truth, models_dir, models_required, ort_dir};

struct Media {
    path: PathBuf,
    info: MediaInfo,
    index: Vec<u8>,
    frames: u32,
}

fn setup(idle_exit: Option<Duration>) -> Option<(Engine, Media)> {
    load_truth()?;
    let ort = ort_dir()?;
    let store = as_models::ModelStore::new(models_dir());
    if store.installed(as_models::ids::TRANSNET).unwrap().is_none() {
        assert!(!models_required(), "TransNetV2 missing (run `cargo xtask fetch-models`)");
        eprintln!("TransNetV2 missing; run `cargo xtask fetch-models` (skipping)");
        return None;
    }
    let tools = Tools::locate(None).expect("ffmpeg");
    let path = fixtures_dir().join("cfr24_h264.mp4");
    let info = probe(&tools, &path).unwrap();
    let work = tempfile::tempdir().unwrap();
    let index = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    let cfg = Config { ort_dir: ort, models_dir: models_dir(), ffmpeg_dir: None, gpu: true, idle_exit };
    let engine = Engine::new(Launch {
        program: PathBuf::from(env!("CARGO_BIN_EXE_autoscene-engine")),
        args: as_engine::args(&cfg),
    });
    Some((engine, Media { path, info, frames: index.len(), index: index.to_bytes() }))
}

/// Run a shots job and collect its chunks; `on_chunk` sees each chunk start.
fn collect(
    engine: &Engine,
    m: &Media,
    resume: u32,
    cancel: &CancelToken,
    mut on_chunk: impl FnMut(u32),
) -> (Result<(), EngineError>, BTreeMap<u32, Vec<u8>>) {
    let mut chunks = BTreeMap::new();
    let r = engine.shots(m.path.clone(), m.info.clone(), m.index.clone(), resume, 100, cancel, |e| {
        if let ShotsEvent::Chunk { start, signals } = e {
            assert!(chunks.insert(start, signals).is_none(), "chunk {start} arrived twice");
            on_chunk(start);
        }
        Ok(())
    });
    (r, chunks)
}

#[test]
fn a_killed_engine_resumes_with_identical_signals() {
    let Some((engine, m)) = setup(None) else { return };
    let (r, full) = collect(&engine, &m, 0, &CancelToken::new(), |_| {});
    r.unwrap();
    assert_eq!(full.keys().copied().collect::<Vec<_>>(), (0..m.frames).step_by(100).collect::<Vec<_>>());
    let total: usize = full.values().map(|b| as_analysis::signals::decode(b).unwrap().len()).sum();
    assert_eq!(total as u32, m.frames, "one signal per frame");

    // The same job, but the engine process is killed hard after its first
    // chunk (any later and a fast GPU may already have sent everything).
    let first_pid = engine.pid().expect("engine running");
    let mut killed = false;
    let (r, resumed) = collect(&engine, &m, 0, &CancelToken::new(), |start| {
        if start == 0 && !killed {
            killed = true;
            kill_hard(engine.pid().unwrap());
        }
    });
    r.unwrap();
    assert!(killed);
    assert_ne!(engine.pid(), Some(first_pid), "a fresh engine finished the job");
    assert_eq!(resumed, full, "signals after a hard kill match an uninterrupted run");
}

#[test]
fn cancel_stops_a_job_quickly() {
    let Some((engine, m)) = setup(None) else { return };
    let cancel = CancelToken::new();
    let started = std::time::Instant::now();
    let (r, chunks) = collect(&engine, &m, 0, &cancel, |_| cancel.cancel());
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert!(!chunks.is_empty());
    assert!(started.elapsed() < Duration::from_secs(30));
    // The engine stays usable.
    let (r, _) = collect(&engine, &m, 500, &CancelToken::new(), |_| {});
    r.unwrap();
}

fn kill_hard(pid: u32) {
    #[cfg(windows)]
    let st = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output().unwrap();
    #[cfg(not(windows))]
    let st = std::process::Command::new("kill").args(["-9", &pid.to_string()]).output().unwrap();
    assert!(st.status.success(), "kill {pid}: {}", String::from_utf8_lossy(&st.stderr));
}

#[test]
fn an_idle_engine_exits_and_the_next_job_starts_a_new_one() {
    let Some((engine, m)) = setup(Some(Duration::from_secs(1))) else { return };
    let (r, _) = collect(&engine, &m, 500, &CancelToken::new(), |_| {});
    r.unwrap();
    let first = engine.pid().expect("engine running");
    std::thread::sleep(Duration::from_secs(3));
    let (r, chunks) = collect(&engine, &m, 500, &CancelToken::new(), |_| {});
    r.unwrap();
    assert_eq!(chunks.keys().copied().collect::<Vec<_>>(), [500]);
    assert_ne!(engine.pid(), Some(first), "the idle engine had exited; a new one ran the job");
}

#[test]
fn face_chunks_tile_the_media_even_across_a_kill() {
    let Some((engine, m)) = setup(None) else { return };
    let store = as_models::ModelStore::new(models_dir());
    if as_models::ids::FACES.iter().any(|id| store.installed(id).unwrap().is_none()) {
        assert!(!models_required(), "face models missing (run `cargo xtask fetch-models`)");
        eprintln!("face models missing; run `cargo xtask fetch-models` (skipping)");
        return;
    }
    // Shots of 50 frames; chunks close at the first shot start 100 frames on.
    let shots: Vec<(u32, u32)> = (0..m.frames).step_by(50).map(|s| (s, (s + 50).min(m.frames))).collect();
    let mut chunks: Vec<(u32, u32)> = Vec::new();
    let mut killed = false;
    let r = engine.faces(m.path.clone(), m.info.clone(), m.index.clone(), shots, 0, 100, &CancelToken::new(), |e| {
        if let FacesEvent::Chunk { start, end, tracks } = e {
            // The synthetic fixture shows barcodes, not people.
            assert!(as_analysis::tracks::decode(&tracks).expect("readable tracks").is_empty());
            chunks.push((start, end));
            if chunks.len() == 1 && !killed {
                killed = true;
                kill_hard(engine.pid().unwrap());
            }
        }
        Ok(())
    });
    r.unwrap();
    assert!(killed);
    let mut expect = 0;
    for &(s, e) in &chunks {
        assert_eq!(s, expect, "chunks tile the media: {chunks:?}");
        assert!(e > s);
        expect = e;
    }
    assert_eq!(expect, m.frames);
}
