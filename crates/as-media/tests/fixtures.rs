//! Golden tests against `cargo xtask fixtures` output. Skipped (with a note)
//! when fixtures haven't been generated.

use std::path::Path;
use std::process::Stdio;

use as_core::CancelToken;
use as_media::{Source, Tools, build_frame_index, poster, probe, proxy};
use as_testkit::{Truth, barcode, fixtures_dir, load_truth};

fn setup() -> Option<(Tools, Truth)> {
    let Some(truth) = load_truth() else {
        eprintln!("fixtures missing — run `cargo xtask fixtures`; skipping");
        return None;
    };
    Some((Tools::locate(None).expect("ffmpeg on PATH"), truth))
}

/// Decode every frame of `file` to 640×360 grey and read its barcode.
fn barcodes(tools: &Tools, file: &Path) -> Vec<Option<barcode::Code>> {
    let out = tools
        .ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-vf", "scale=640:360,format=gray", "-f", "rawvideo", "-"])
        .stdout(Stdio::piped())
        .output()
        .expect("decode");
    out.stdout.chunks_exact(640 * 360).map(|f| barcode::read_luma(f, 640, 360, 640)).collect()
}

#[test]
fn probe_reads_every_variant() {
    let Some((tools, truth)) = setup() else { return };
    for v in &truth.variants {
        let info = probe(&tools, &fixtures_dir().join(&v.file)).unwrap();
        assert_eq!(info.video.codec, v.video_codec, "{}", v.file);
        assert_eq!(info.audio.len() as u32, v.audio_tracks, "{}", v.file);
        assert_eq!((info.video.width, info.video.height), (truth.width, truth.height));
    }
    let hdr = probe(&tools, &fixtures_dir().join("hevc10_hdr.mkv")).unwrap();
    assert_eq!(hdr.video.hdr, as_media::HdrKind::Pq);
    assert_eq!(hdr.video.bit_depth, 10);
    let two = probe(&tools, &fixtures_dir().join("two_audio_h264.mkv")).unwrap();
    assert_eq!(two.audio[1].language.as_deref(), Some("jpn"));
}

#[test]
fn frame_index_matches_truth_for_every_variant() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    for v in &truth.variants {
        let path = fixtures_dir().join(&v.file);
        let info = probe(&tools, &path).unwrap();
        let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
        assert_eq!(idx.len(), truth.frame_count, "{}: frame count", v.file);
        assert!((idx.t0() - v.start_sec).abs() < 1e-3, "{}: t0 {} vs {}", v.file, idx.t0(), v.start_sec);
        assert_eq!(idx.is_vfr(), v.vfr, "{}: vfr", v.file);
        if let Some(times) = &v.frame_times_sec {
            for (f, &t) in times.iter().enumerate() {
                let got = idx.t(f as u32) - idx.t0();
                assert!((got - t).abs() < 1.5e-3, "{}: frame {f} at {got}, want {t}", v.file);
            }
        }
    }
}

#[test]
fn decoded_frames_carry_their_own_index() {
    // Decoder order == index order == barcode order, for awkward variants.
    let Some((tools, truth)) = setup() else { return };
    for file in ["cfr23976_h264.mp4", "offset725_h264.mp4", "transport_h264.ts", "vfr_h264.mkv", "hevc10_hdr.mkv"] {
        let codes = barcodes(&tools, &fixtures_dir().join(file));
        assert_eq!(codes.len() as u32, truth.frame_count, "{file}");
        for (i, c) in codes.iter().enumerate() {
            let c = c.unwrap_or_else(|| panic!("{file}: no barcode on frame {i}"));
            assert_eq!(c.frame as usize, i, "{file}: frame {i}");
            assert_eq!(c.id, truth.element_at(i as u32).unwrap().id, "{file}: element id at {i}");
        }
    }
}

#[test]
fn poster_extracts_the_exact_frame() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    for file in ["offset725_h264.mp4", "transport_h264.ts", "vfr_h264.mkv"] {
        let path = fixtures_dir().join(file);
        let info = probe(&tools, &path).unwrap();
        let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
        for f in [0u32, 1, 199, 200, 300, 585] {
            let jpg = work.path().join(format!("{file}-{f}.jpg"));
            poster::extract_frame_jpeg(&tools, Source { path: &path, info: &info, index: &idx }, f, 360, &jpg).unwrap();
            let code = barcodes(&tools, &jpg)[0].expect("barcode in poster");
            assert_eq!(code.frame as u32, f, "{file}: poster of frame {f}");
        }
    }
}

#[test]
fn proxy_keeps_every_frame_and_reports_offset() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    for file in ["transport_h264.ts", "hevc10_hdr.mkv", "vfr_h264.mkv"] {
        let path = fixtures_dir().join(file);
        let info = probe(&tools, &path).unwrap();
        let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
        assert!(!proxy::playable_direct(&info, &idx), "{file} should need a proxy");
        let out = work.path().join(format!("{file}.proxy.mp4"));
        let audio = info.default_audio().map(|a| a.ordinal);
        let src = Source { path: &path, info: &info, index: &idx };
        let r = proxy::build_proxy(&tools, src, audio, &out, &CancelToken::new(), |_| {}).unwrap();
        let codes = barcodes(&tools, &out);
        assert_eq!(codes.len() as u32, truth.frame_count);
        assert!(codes.iter().enumerate().all(|(i, c)| c.map(|c| c.frame as usize) == Some(i)), "{file}: proxy frames");
        assert!(r.t0 >= 0.0 && r.t0 < 0.5, "{file}: proxy t0 {}", r.t0);
    }
    // A plain 24 fps H.264 MP4 plays directly.
    let path = fixtures_dir().join("cfr24_h264.mp4");
    let info = probe(&tools, &path).unwrap();
    let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    assert!(proxy::playable_direct(&info, &idx));
}
