//! Shot detection against the fixtures' ground truth. Needs `cargo xtask
//! fixtures`, `cargo xtask fetch-sidecars` (ONNX Runtime) and `cargo xtask
//! fetch-models` (TransNetV2); skips with a note when they're missing.

use std::path::PathBuf;
use std::sync::OnceLock;

use as_analysis::{CHUNK, FrameSignals, FusionParams, Segment, SegmentKind, fuse, shot_signals};
use as_core::CancelToken;
use as_infer::Runtime;
use as_media::{Source, Tools, build_frame_index, probe};
use as_testkit::{Truth, fixtures_dir, load_truth, models_dir, models_required, ort_dir};
use as_vision::TransNet;

struct Env {
    tools: Tools,
    truth: Truth,
    rt: Runtime,
    model: PathBuf,
}

/// One runtime per test process (ONNX Runtime binds once).
fn env() -> Option<&'static Env> {
    static ENV: OnceLock<Option<Env>> = OnceLock::new();
    ENV.get_or_init(|| {
        let truth = load_truth()?;
        let ort = ort_dir()?;
        let store = as_models::ModelStore::new(models_dir());
        let model = match store.installed(as_models::ids::TRANSNET).unwrap() {
            Some(p) => p,
            None if models_required() => {
                panic!("TransNetV2 missing in {} (run `cargo xtask fetch-models`)", models_dir().display())
            }
            None => {
                eprintln!("TransNetV2 missing; run `cargo xtask fetch-models` (skipping)");
                return None;
            }
        };
        let rt = Runtime::load(&ort, true).expect("onnxruntime loads");
        Some(Env { tools: Tools::locate(None).expect("ffmpeg"), truth, rt, model })
    })
    .as_ref()
}

fn signals(env: &Env, file: &str, resume: u32, chunk: u32) -> (Vec<FrameSignals>, Vec<u32>) {
    let path = fixtures_dir().join(file);
    let mut info = probe(&env.tools, &path).unwrap();
    if info.video.field_order.is_none()
        && let Some(order) = as_media::idet::detect_field_order(&env.tools, &path, &info).unwrap()
    {
        info.video.field_order = Some(order.into());
    }
    let work = tempfile::tempdir().unwrap();
    let index = build_frame_index(&env.tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    let mut net = TransNet::load(&env.rt, &env.model).unwrap();
    let mut out = Vec::new();
    let mut starts = Vec::new();
    shot_signals(
        &env.tools,
        Source { path: &path, info: &info, index: &index },
        &mut net,
        resume,
        chunk,
        &CancelToken::new(),
        |_| {},
        |start, s| {
            starts.push(start);
            out.extend_from_slice(s);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(out.len() as u32, index.len() - resume, "{file}: one signal per frame");
    (out, starts)
}

/// Starts of segments after a cut (neither side a transition or a flash).
fn hard_cuts(segs: &[Segment]) -> Vec<u32> {
    let cut_side = |k: &SegmentKind| matches!(k, SegmentKind::Shot | SegmentKind::Black);
    segs.windows(2).filter(|w| cut_side(&w[0].kind) && cut_side(&w[1].kind)).map(|w| w[1].start).collect()
}

/// Spans of consecutive transition segments (a fade through black counts as one).
fn transition_spans(segs: &[Segment]) -> Vec<(u32, u32)> {
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        let inside = matches!(s.kind, SegmentKind::Transition(_))
            || (s.kind == SegmentKind::Black
                && i > 0
                && i + 1 < segs.len()
                && matches!(segs[i - 1].kind, SegmentKind::Transition(_))
                && matches!(segs[i + 1].kind, SegmentKind::Transition(_)));
        if !inside {
            continue;
        }
        match spans.last_mut() {
            Some(last) if last.1 == s.start => last.1 = s.end,
            _ => spans.push((s.start, s.end)),
        }
    }
    spans
}

#[test]
fn transnet_gives_probabilities() {
    let Some(env) = env() else { return };
    let (sig, _) = signals(env, "master.mkv", 0, CHUNK);
    for (f, s) in sig.iter().enumerate() {
        assert!((0.0..=1.0).contains(&s.single) && (0.0..=1.0).contains(&s.many), "frame {f}: {s:?}");
    }
}

#[test]
fn shots_match_the_truth_on_every_variant() {
    let Some(env) = env() else { return };
    let truth = &env.truth;
    let want_cuts = truth.hard_cuts();
    let want_spans: Vec<(u32, u32)> = truth.transitions().map(|t| (t.start, t.end)).collect();
    let flashes: Vec<u32> = truth.shots().flat_map(|s| s.flashes.iter().copied()).collect();
    for file in
        ["master.mkv", "cfr24_h264.mp4", "transport_h264.ts", "vfr_h264.mkv", "hevc10_hdr.mkv", "interlaced_h264.ts"]
    {
        let (sig, _) = signals(env, file, 0, CHUNK);
        let segs = fuse(&sig, &FusionParams::live());
        let got_cuts = hard_cuts(&segs);
        assert_eq!(got_cuts, want_cuts, "{file}: hard cuts (segments: {segs:?})");
        for f in &flashes {
            assert!(!got_cuts.contains(f) && !got_cuts.contains(&(f + 1)), "{file}: flash at {f} became a cut");
        }
        let got_spans = transition_spans(&segs);
        assert_eq!(got_spans.len(), want_spans.len(), "{file}: transitions {got_spans:?} vs {want_spans:?}");
        for (g, w) in got_spans.iter().zip(&want_spans) {
            assert!(g.0.abs_diff(w.0) <= 2 && g.1.abs_diff(w.1) <= 2, "{file}: transition {g:?} vs truth {w:?}");
        }
        // Every frame belongs to exactly one segment.
        assert_eq!(segs.first().map(|s| s.start), Some(0));
        assert!(segs.windows(2).all(|w| w[0].end == w[1].start));
        assert_eq!(segs.last().map(|s| s.end), Some(sig.len() as u32));
    }
}

#[test]
fn a_resumed_pass_matches_an_uninterrupted_one() {
    let Some(env) = env() else { return };
    let (full, starts) = signals(env, "cfr24_h264.mp4", 0, 100);
    assert_eq!(starts, (0..full.len() as u32).step_by(100).collect::<Vec<_>>(), "chunks in order");
    // As if the app had stopped after the first three chunks.
    let (resumed, _) = signals(env, "cfr24_h264.mp4", 300, 100);
    assert_eq!(resumed, full[300..], "signals after resuming at 300");
}
