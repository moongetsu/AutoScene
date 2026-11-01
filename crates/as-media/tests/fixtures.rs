//! Golden tests against `cargo xtask fixtures` output. Skipped (with a note)
//! when fixtures haven't been generated.

use std::path::Path;
use std::process::Stdio;

use as_core::CancelToken;
use as_media::poster::StillSize;
use as_media::{Source, Tools, build_frame_index, idet, poster, probe, proxy};
use as_testkit::{Truth, barcode, fixtures_dir, load_truth};

fn setup() -> Option<(Tools, Truth)> {
    let truth = load_truth()?;
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
    out.stdout.as_chunks::<{ 640 * 360 }>().0.iter().map(|f| barcode::read_luma(f, 640, 360, 640)).collect()
}

fn proxy_audio(tools: &Tools, file: &Path) -> Vec<i16> {
    let out = tools
        .ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "s16le", "-"])
        .stdout(Stdio::piped())
        .output()
        .expect("decode audio");
    out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

/// Power of frequency `hz` in `x` (Goertzel).
fn goertzel(x: &[i16], hz: f64) -> f64 {
    let k = 2.0 * (2.0 * std::f64::consts::PI * hz / 48_000.0).cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for &v in x {
        let s = v as f64 + k * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    s1 * s1 + s2 * s2 - k * s1 * s2
}

/// Time near `around` (seconds) where tone `from` gives way to tone `to`,
/// scanning 5 ms windows in 1 ms steps.
fn tone_switch(pcm: &[i16], around: f64, from: f64, to: f64) -> f64 {
    let win = 240;
    (-40..=40)
        .map(|ms| around + ms as f64 / 1000.0)
        .find(|&t| {
            let c = (t * 48_000.0) as usize;
            let x = &pcm[c - win / 2..c + win / 2];
            goertzel(x, to) > goertzel(x, from)
        })
        .unwrap_or(f64::NAN)
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
    for file in [
        "cfr23976_h264.mp4",
        "offset725_h264.mp4",
        "transport_h264.ts",
        "vfr_h264.mkv",
        "hevc10_hdr.mkv",
        "interlaced_h264.ts",
        "interlaced_xvid.avi",
    ] {
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
fn interlacing_is_found_with_or_without_a_flag() {
    let Some((tools, truth)) = setup() else { return };
    assert!(truth.variants.iter().filter(|v| v.interlaced).count() >= 2, "stale fixtures: no interlaced variants");
    for v in &truth.variants {
        let path = fixtures_dir().join(&v.file);
        let info = probe(&tools, &path).unwrap();
        // Content analysis alone must get every variant right, flagged or not.
        let found = idet::detect_field_order(&tools, &path, &info).unwrap();
        assert_eq!(found, v.interlaced.then_some("tt"), "{}: idet", v.file);
        // What the container says: the TS is flagged, the AVI says nothing.
        match v.file.as_str() {
            "interlaced_h264.ts" => assert!(info.is_interlaced(), "{}: flagged top field first", v.file),
            "interlaced_xvid.avi" => assert_eq!(info.video.field_order, None, "{}: unflagged", v.file),
            _ => assert!(!info.is_interlaced(), "{}: progressive", v.file),
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
            let small = work.path().join(format!("{file}-{f}-small.jpg"));
            let src = Source { path: &path, info: &info, index: &idx };
            let outs = [(StillSize::Height(720), jpg.as_path()), (StillSize::Cover(480, 270), small.as_path())];
            poster::extract_frame_jpegs(&tools, src, f, &outs, &CancelToken::new()).unwrap();
            for (out, size) in [(&jpg, (640, 360)), (&small, (480, 270))] {
                assert_eq!(dimensions(&tools, out), size, "{file}: never upscaled, made to cover");
                let code = barcodes(&tools, out)[0].expect("barcode in poster");
                assert_eq!(code.frame as u32, f, "{file}: poster of frame {f}");
            }
        }
    }
}

/// While a media is indexed: its frames counted as they are read (about as
/// many as its file states), and a glimpse of where it is — the keyframe at
/// about that place, small, for every kind of file.
#[test]
fn indexing_counts_its_frames_and_can_be_glimpsed() {
    let Some((tools, truth)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    for v in &truth.variants {
        let path = fixtures_dir().join(&v.file);
        let info = probe(&tools, &path).unwrap();
        let mut told = Vec::new();
        build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |p| told.push(p)).unwrap();
        let last = told.last().expect("progress told");
        assert_eq!(last.frames, u64::from(truth.frame_count), "{}: every frame counted", v.file);
        assert!(told.iter().all(|p| p.at_sec <= 24.5 && (0.0..=1.0).contains(&p.fraction)), "{}: {told:?}", v.file);
        if !v.vfr {
            let estimate = info.frames_estimate().expect("the file states its length and rate") as f64;
            let off = (estimate - truth.frame_count as f64).abs() / truth.frame_count as f64;
            assert!(off < 0.05, "{}: about {estimate} frames, not {}", v.file, truth.frame_count);
        }
        for (at, frame) in [(0.0, 0u32), (10.0, 240)] {
            let jpeg = poster::keyframe_jpeg(&tools, &path, &info, at, 288).unwrap().expect("a keyframe");
            let out = work.path().join(format!("{}-{at}.jpg", v.file));
            std::fs::write(&out, jpeg).unwrap();
            assert_eq!(dimensions(&tools, &out), (512, 288), "{}: small, square pixels", v.file);
            // Plain progressive pictures read their barcode (a combed or
            // tone-mapped one needn't): the keyframe before the place, or in
            // a transport stream (no index to seek with) maybe the next one.
            if !v.interlaced && v.video_codec == "h264" && !v.vfr {
                let code = barcodes(&tools, &out)[0].expect("barcode in the glimpse");
                let after = if v.file.ends_with(".ts") { 48 } else { 0 };
                assert!(code.frame as u32 <= frame + after, "{}: {code:?} at {at}", v.file);
                assert!(at > 0.0 || code.frame == 0, "{}: {code:?} at the start", v.file);
            }
        }
    }
}

/// Portraits are a part of the exact frame, at exactly the size asked for.
#[test]
fn portraits_are_cut_from_the_exact_frame() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    for file in ["offset725_h264.mp4", "vfr_h264.mkv", "interlaced_h264.ts"] {
        let path = fixtures_dir().join(file);
        let info = probe(&tools, &path).unwrap();
        let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
        let src = Source { path: &path, info: &info, index: &idx };
        for f in [1u32, 300] {
            // The whole frame as a "region": its barcode says which frame it is.
            let whole = work.path().join(format!("{file}-{f}-whole.jpg"));
            let face = work.path().join(format!("{file}-{f}-face.jpg"));
            let outs = [
                (StillSize::Region([0.0, 0.0, 1.0, 1.0], 640, 360), whole.as_path()),
                (StillSize::Region([0.4, 0.2, 0.55, 0.6], 450, 600), face.as_path()),
            ];
            poster::extract_frame_jpegs(&tools, src, f, &outs, &CancelToken::new()).unwrap();
            assert_eq!(barcodes(&tools, &whole)[0].map(|c| c.frame as u32), Some(f), "{file}: frame {f}");
            assert_eq!(dimensions(&tools, &face), (450, 600), "{file}: a portrait is exactly its size");
        }
    }
}

/// Stills of an interlaced source are deinterlaced like everything else, not
/// two fields woven together: neighbouring rows of a moving picture differ
/// less than rows two apart (the same field) — combing turns that around.
#[test]
fn stills_of_interlaced_sources_are_not_combed() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    let path = fixtures_dir().join("interlaced_h264.ts");
    let info = probe(&tools, &path).unwrap();
    assert!(info.is_interlaced());
    let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    let src = Source { path: &path, info: &info, index: &idx };
    let (w, h) = (info.video.width as usize, info.video.height as usize);
    for f in [50u32, 100, 200, 300] {
        let still = work.path().join(format!("{f}.jpg"));
        let outs = [(StillSize::Region([0.0, 0.0, 1.0, 1.0], w as u32, h as u32), still.as_path())];
        poster::extract_frame_jpegs(&tools, src, f, &outs, &CancelToken::new()).unwrap();
        let out = tools
            .ffmpeg()
            .args(["-v", "error", "-i"])
            .arg(&still)
            .args(["-vf", "format=gray", "-f", "rawvideo", "-"])
            .output()
            .expect("ffmpeg");
        let g = &out.stdout;
        assert_eq!(g.len(), w * h);
        let rows = |gap: usize| -> u64 {
            (0..h - 2)
                .flat_map(|y| (0..w).map(move |x| (y, x)))
                .map(|(y, x)| g[y * w + x].abs_diff(g[(y + gap) * w + x]) as u64)
                .sum()
        };
        let comb = rows(1) as f64 / rows(2).max(1) as f64;
        assert!(comb < 0.85, "frame {f}: combed ({comb:.2})");
    }
}

/// A region is the part it names — of the upright, square-pixel frame —
/// and one reaching past an edge is moved inside.
#[test]
fn portrait_regions_are_where_they_say() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    // Red left half, blue right half; anamorphic too (64:45 makes 720×576 a 1024×576 picture).
    for (size, sar) in [("640x360", "1/1"), ("720x576", "64/45")] {
        let (w, h) = size.split_once('x').unwrap();
        let half = w.parse::<u32>().unwrap() / 2;
        let path = work.path().join(format!("{size}.mkv"));
        let graph = format!(
            "color=c=red:s={half}x{h}:r=25:d=1[a];color=c=blue:s={half}x{h}:r=25:d=1[b];[a][b]hstack,setsar={sar}[v]"
        );
        let status = tools
            .ffmpeg()
            .args(["-v", "error", "-y", "-filter_complex", &graph, "-map", "[v]"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .expect("ffmpeg");
        assert!(status.success(), "{size}: test source");
        let info = probe(&tools, &path).unwrap();
        let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
        let src = Source { path: &path, info: &info, index: &idx };
        let cases = [
            ("left", [0.05, 0.1, 0.4, 0.9], [255, 0, 0]),
            ("right", [0.6, 0.1, 0.95, 0.9], [0, 0, 255]),
            ("past the right edge", [0.8, 0.1, 1.15, 0.9], [0, 0, 255]),
            ("past the left edge", [-0.2, 0.1, 0.15, 0.9], [255, 0, 0]),
        ];
        for (name, region, rgb) in cases {
            let out = work.path().join(format!("{size}-{name}.jpg"));
            let outs = [(StillSize::Region(region, 90, 120), out.as_path())];
            poster::extract_frame_jpegs(&tools, src, 5, &outs, &CancelToken::new()).unwrap();
            assert_eq!(dimensions(&tools, &out), (90, 120), "{size} {name}");
            let mean = mean_rgb(&tools, &out);
            assert!(mean.iter().zip(rgb).all(|(&a, b)| a.abs_diff(b) < 40), "{size} {name}: {mean:?}");
        }
    }
}

/// Face boxes are fractions of the frames the face pass decodes: a region
/// shows what the face pass saw there, whatever the source's pixel shape or
/// rotation.
#[test]
fn portrait_regions_match_what_the_face_pass_sees() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    // Red, green / blue, white quadrants.
    let quads = |size: &str, sar: &str, out: &Path| {
        let (w, h) = size.split_once('x').unwrap();
        let (w, h) = (w.parse::<u32>().unwrap() / 2, h.parse::<u32>().unwrap() / 2);
        let c = |name: &str| format!("color=c={name}:s={w}x{h}:r=25:d=1");
        let graph = format!(
            "{}[a];{}[b];{}[c];{}[d];[a][b]hstack[top];[c][d]hstack[bottom];[top][bottom]vstack,setsar={sar}[v]",
            c("red"),
            c("lime"),
            c("blue"),
            c("white")
        );
        let status = tools
            .ffmpeg()
            .args(["-v", "error", "-y", "-filter_complex", &graph, "-map", "[v]"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"])
            .arg(out)
            .status()
            .expect("ffmpeg");
        assert!(status.success(), "{size}: test source");
    };
    let plain = work.path().join("plain.mkv");
    quads("640x360", "1/1", &plain);
    let anamorphic = work.path().join("anamorphic.mkv");
    quads("720x576", "64/45", &anamorphic);
    // Stored sideways, shown upright (a phone video).
    let stored = work.path().join("stored.mkv");
    quads("640x360", "1/1", &stored);
    let rotated = work.path().join("rotated.mp4");
    let status = tools
        .ffmpeg()
        .args(["-v", "error", "-y", "-display_rotation", "90", "-i"])
        .arg(&stored)
        .args(["-c", "copy"])
        .arg(&rotated)
        .status()
        .expect("ffmpeg");
    assert!(status.success(), "rotated test source");

    for path in [plain, anamorphic, rotated] {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let info = probe(&tools, &path).unwrap();
        assert_eq!(info.video.rotation.rem_euclid(180) == 90, name == "rotated.mp4", "{name}: rotation");
        let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
        let src = Source { path: &path, info: &info, index: &idx };
        // Frame 5 as the face pass decodes it.
        let (w, h) = as_media::frames::sample_size(&info, (160, 160), 2);
        let mut sample = Vec::new();
        as_media::frames::decode_samples(&tools, src, 5, 1, (w, h), &CancelToken::new(), |_, rgb| {
            sample = rgb.to_vec();
            Ok(false)
        })
        .unwrap();
        let (w, h) = (w as usize, h as usize);
        let mean = |x0: usize, y0: usize| -> [u8; 3] {
            let mut sum = [0usize; 3];
            // The middle of the quadrant (away from the seams).
            let (qw, qh) = (w / 2, h / 2);
            let ys = y0 + qh / 4..y0 + 3 * qh / 4;
            let xs = x0 + qw / 4..x0 + 3 * qw / 4;
            let n = ys.len() * xs.len();
            for y in ys {
                for x in xs.clone() {
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += sample[(y * w + x) * 3 + c] as usize;
                    }
                }
            }
            sum.map(|s| (s / n) as u8)
        };
        for (qx, qy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let want = mean(qx * w / 2, qy * h / 2);
            let (fx, fy) = (qx as f32 * 0.5, qy as f32 * 0.5);
            let region = [fx + 0.125, fy + 0.125, fx + 0.375, fy + 0.375];
            let out = work.path().join(format!("{name}-{qx}{qy}.jpg"));
            poster::extract_frame_jpegs(
                &tools,
                src,
                5,
                &[(StillSize::Region(region, 60, 80), out.as_path())],
                &CancelToken::new(),
            )
            .unwrap();
            let got = mean_rgb(&tools, &out);
            assert!(
                got.iter().zip(want).all(|(&a, b)| a.abs_diff(b) < 40),
                "{name} quadrant {qx},{qy}: {got:?} vs {want:?}"
            );
        }
    }
}

/// The mean colour of a picture.
fn mean_rgb(tools: &Tools, file: &Path) -> [u8; 3] {
    let out = tools
        .ffmpeg()
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-vf", "scale=1:1:flags=area", "-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg");
    out.stdout[..3].try_into().expect("one pixel")
}

/// `(width, height)` of a picture or video.
fn dimensions(tools: &Tools, file: &Path) -> (u32, u32) {
    let out = tools
        .ffprobe()
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(file)
        .output()
        .expect("ffprobe");
    let text = String::from_utf8_lossy(&out.stdout);
    let (w, h) = text.trim().split_once(',').expect("width,height");
    (w.parse().unwrap(), h.parse().unwrap())
}

/// Stills keep the picture's shape: square pixels, the height capped, a
/// cover box filled whatever the aspect ratio.
#[test]
fn stills_keep_the_frame_shape() {
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    // (size, sample aspect) → (720p poster, small poster)
    let cases = [
        ("960x720", "1/1", (960, 720), (480, 360)),    // 4:3
        ("1280x536", "1/1", (1280, 536), (644, 270)),  // scope
        ("720x576", "64/45", (1024, 576), (480, 270)), // anamorphic PAL 16:9
        ("2560x1440", "1/1", (1280, 720), (480, 270)), // larger than 720p
    ];
    for (size, sar, big, small) in cases {
        let path = work.path().join(format!("{size}.mkv"));
        let status = tools
            .ffmpeg()
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc2=size={size}:rate=25:duration=1")])
            .args(["-vf", &format!("setsar={sar}"), "-c:v", "libx264", "-preset", "ultrafast"])
            .arg(&path)
            .status()
            .expect("ffmpeg");
        assert!(status.success(), "{size}: test source");
        let info = probe(&tools, &path).unwrap();
        let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
        let (a, b) = (work.path().join(format!("{size}-a.jpg")), work.path().join(format!("{size}-b.jpg")));
        let outs = [(StillSize::Height(720), a.as_path()), (StillSize::Cover(480, 270), b.as_path())];
        let src = Source { path: &path, info: &info, index: &idx };
        poster::extract_frame_jpegs(&tools, src, 10, &outs, &CancelToken::new()).unwrap();
        assert_eq!(dimensions(&tools, &a), big, "{size} poster");
        assert_eq!(dimensions(&tools, &b), small, "{size} small poster");
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
        // A lossy preview may smear the barcode of a frame in heavy noise, but
        // it must never show a different frame.
        let wrong: Vec<_> =
            codes.iter().enumerate().filter(|(i, c)| c.is_some_and(|c| c.frame as usize != *i)).collect();
        let unreadable = codes.iter().filter(|c| c.is_none()).count();
        assert!(wrong.is_empty(), "{file}: proxy shows wrong frames {wrong:?}");
        assert!(unreadable * 100 <= codes.len(), "{file}: {unreadable} unreadable proxy frames");
        assert!(r.t0 >= 0.0 && r.t0 < 0.5, "{file}: proxy t0 {}", r.t0);
        // Player time of frame f is t0 + (t(f) - t(0)), including true VFR timing.
        let probe = tools
            .ffprobe()
            .args(["-select_streams", "v:0", "-show_entries", "frame=pts_time", "-of", "csv=p=0"])
            .arg(&out)
            .output()
            .unwrap();
        let times: Vec<f64> = String::from_utf8_lossy(&probe.stdout)
            .lines()
            .filter_map(|l| l.trim().trim_end_matches(',').parse().ok())
            .collect();
        assert_eq!(times.len() as u32, truth.frame_count);
        for (f, t) in times.iter().enumerate() {
            let want = r.t0 + idx.t(f as u32) - idx.t0();
            assert!((t - want).abs() < 1.5e-3, "{file}: proxy frame {f} at {t:.4}, want {want:.4}");
        }
        // Audio stays in sync: at hard cuts between toned shots, the tone
        // switches when the picture does.
        if audio.is_some() {
            let pcm = proxy_audio(&tools, &out);
            for w in truth.elements.windows(2) {
                let (Some(a), Some(b)) = (w[0].tone_hz, w[1].tone_hz) else { continue };
                if w[0].end - w[0].start < 12 || w[1].end - w[1].start < 12 {
                    continue;
                }
                let cut = r.t0 + idx.t(w[1].start) - idx.t0();
                let switch = tone_switch(&pcm, cut, a as f64, b as f64);
                assert!((switch - cut).abs() < 0.008, "{file}: tone switches at {switch:.4}, picture cuts at {cut:.4}");
            }
        }
    }
    // A plain 24 fps H.264 MP4 plays directly.
    let path = fixtures_dir().join("cfr24_h264.mp4");
    let info = probe(&tools, &path).unwrap();
    let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
    assert!(proxy::playable_direct(&info, &idx));
}

#[test]
fn samples_decoded_on_the_gpu_are_the_frames_the_cpu_decodes() {
    // NVDEC where there is an NVIDIA GPU (and the source suits it), the
    // software fallback elsewhere: either way the same frames, in order.
    use as_media::frames::{Decoder, decode_samples_with, sample_size};
    let Some((tools, _)) = setup() else { return };
    let work = tempfile::tempdir().unwrap();
    for (file, from) in [("cfr24_h264.mp4", 0), ("cfr24_h264.mp4", 7), ("hevc10_hdr.mkv", 0), ("interlaced_h264.ts", 4)]
    {
        let path = fixtures_dir().join(file);
        let info = probe(&tools, &path).unwrap();
        let idx = build_frame_index(&tools, &path, &info, work.path(), &CancelToken::new(), |_| {}).unwrap();
        let src = Source { path: &path, info: &info, index: &idx };
        let (w, h) = sample_size(&info, (640, 640), 4);
        let codes = |decoder| {
            let mut out = Vec::new();
            let n = decode_samples_with(&tools, src, from, 3, (w, h), decoder, &CancelToken::new(), |f, rgb| {
                let luma: Vec<u8> = rgb
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|p| (0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])) as u8)
                    .collect();
                out.push((f, barcode::read_luma(&luma, w, h, w as usize).map(|c| c.frame)));
                Ok(true)
            })
            .unwrap();
            assert_eq!(n as usize, out.len(), "{file}");
            out
        };
        let (cpu, gpu) = (codes(Decoder::Cpu), codes(Decoder::Nvdec));
        assert_eq!(cpu.len() as u32, (idx.len() - from).div_ceil(3), "{file}: every 3rd frame from {from}");
        assert_eq!(gpu, cpu, "{file} from {from}");
        assert!(cpu.iter().all(|&(f, code)| code == Some(f as u16)), "{file}: each sample shows its own frame");
    }
}
