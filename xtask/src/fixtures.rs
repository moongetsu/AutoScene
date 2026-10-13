//! Synthetic test media with ground truth.
//!
//! 1. Each timeline element (shot, transition, black gap) is rendered to an
//!    exact-length lossless FFV1/PCM file stamped with its element id.
//! 2. Elements are concatenated losslessly and every frame is stamped with
//!    its master frame index → `master.mkv` (24 fps, 640×360).
//! 3. Variants exercise real-world timing problems: 23.976, B-pyramid H.264,
//!    a 7.25 s start offset (MP4 edit list), MPEG-TS (non-zero start),
//!    10-bit HEVC with open GOP and HDR10 tags, true VFR, two audio tracks,
//!    and an intra ProRes copy.
//!
//! `truth.json` records the element ranges and per-variant timing so tests
//! can check shot detection, frame indexing and frame-exact export.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use anyhow::{Context, Result, bail};
use as_testkit::barcode::{self, CELLS, FRAME_BITS, STRIP};
use as_testkit::{Element, ElementKind, Truth, Variant};

const FPS: u32 = 24;
const SR: u32 = 48_000;
const W: u32 = barcode::MASTER_W;
const H: u32 = barcode::MASTER_H;

#[derive(Clone)]
enum Spec {
    Shot {
        label: &'static str,
        src: String,
        extra: &'static str,
        frames: u32,
        flashes: &'static [u32],
        held: bool,
        tone: u32,
    },
    Trans {
        kind: &'static str,
        frames: u32,
    },
    Black {
        frames: u32,
    },
}

fn src(kind: &str) -> String {
    format!("{kind}=s={W}x{H}:r={FPS}")
}

fn timeline() -> Vec<Spec> {
    use Spec::*;
    let shot =
        |label, src: String, extra, frames, tone| Shot { label, src, extra, frames, flashes: &[], held: false, tone };
    vec![
        shot("testsrc2", src("testsrc2"), "", 48, 220),
        shot("smpte bars", src("smptehdbars"), "", 36, 330),
        shot("mandelbrot", src("mandelbrot"), "", 60, 440),
        Trans { kind: "dissolve", frames: 12 },
        shot("cellauto", format!("{}:rule=30:random_fill_ratio=0.5", src("cellauto")), "", 48, 550),
        Trans { kind: "fadeblack", frames: 16 },
        shot("life", format!("{}:mold=10:ratio=0.2:life_color=#e8c070:death_color=#203050", src("life")), "", 48, 660),
        shot("red 1f", format!("color=c=#c02020:s={W}x{H}:r={FPS}"), "", 1, 770),
        shot("blue 2f", format!("color=c=#2040c0:s={W}x{H}:r={FPS}"), "", 2, 880),
        shot("gradient 3f", format!("{}:speed=0.02", src("gradients")), "", 3, 990),
        Shot {
            label: "flash",
            src: src("testsrc2"),
            extra: "hue=h=240,",
            frames: 60,
            flashes: &[20, 21],
            held: false,
            tone: 1100,
        },
        Trans { kind: "wipeleft", frames: 12 },
        shot("rgb bars", src("rgbtestsrc"), "", 48, 1210),
        Black { frames: 12 },
        Shot {
            label: "held on 3s",
            src: format!("{}:start_scale=1.5", src("mandelbrot")),
            extra: "fps=8,fps=24,",
            frames: 72,
            flashes: &[],
            held: true,
            tone: 1320,
        },
        Trans { kind: "fadewhite", frames: 12 },
        shot("testsrc2 long", src("testsrc2"), "hue=h=120,", 96, 1430),
    ]
}

fn frames_of(s: &Spec) -> u32 {
    match s {
        Spec::Shot { frames, .. } | Spec::Trans { frames, .. } | Spec::Black { frames } => *frames,
    }
}

/// drawbox chain: black strip + the set id bits (constant per element).
fn id_stamp(id: u8) -> String {
    let (sx, sy, sw, sh) = STRIP;
    let mut f = format!("drawbox=x={sx}:y={sy}:w={sw}:h={sh}:color=black:t=fill");
    for bit in 0..(CELLS - FRAME_BITS) {
        if id & (1 << bit) != 0 {
            let (x, y, w, h) = barcode::cell_rect(FRAME_BITS + bit);
            f += &format!(",drawbox=x={x}:y={y}:w={w}:h={h}:color=white:t=fill");
        }
    }
    f
}

/// drawbox chain for the frame-index bits, driven by the frame number `n`.
fn frame_stamp() -> String {
    (0..FRAME_BITS)
        .map(|bit| {
            let (x, y, w, h) = barcode::cell_rect(bit);
            format!("drawbox=x={x}:y={y}:w={w}:h={h}:color=white:t=fill:enable='eq(mod(floor(n/{}),2),1)'", 1u32 << bit)
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn audio_src(tone: Option<u32>) -> String {
    match tone {
        Some(f) => format!("sine=f={f}:sample_rate={SR}"),
        None => format!("anullsrc=r={SR}:cl=mono"),
    }
}

/// Neighbouring shot sources used to synthesise a transition.
fn shot_src(s: &Spec) -> (String, &'static str) {
    match s {
        Spec::Shot { src, extra, .. } => (src.clone(), extra),
        _ => (format!("color=c=black:s={W}x{H}:r={FPS}"), ""),
    }
}

fn render_element(ff: &str, dir: &Path, idx: usize, tl: &[Spec]) -> Result<PathBuf> {
    let spec = &tl[idx];
    let id = (idx + 1) as u8;
    let n = frames_of(spec);
    let out = dir.join(format!("el_{idx:02}.mkv"));
    let samples = n * SR / FPS;
    let stamp = id_stamp(id);
    let mut cmd = Command::new(ff);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);

    let (video_graph, tone) = match spec {
        Spec::Shot { src, extra, flashes, tone, .. } => {
            cmd.args(["-f", "lavfi", "-i", src]);
            let flash = flashes
                .iter()
                .map(|f| format!("drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='eq(n,{f})',"))
                .collect::<String>();
            (format!("[0:v]{extra}{flash}format=yuv420p,{stamp}[v]"), Some(*tone))
        }
        Spec::Black { .. } => {
            cmd.args(["-f", "lavfi", "-i", &format!("color=c=black:s={W}x{H}:r={FPS}")]);
            (format!("[0:v]format=yuv420p,{stamp}[v]"), None)
        }
        Spec::Trans { kind, frames } => {
            let (a, ea) = shot_src(&tl[idx - 1]);
            let (b, eb) = shot_src(&tl[idx + 1]);
            cmd.args(["-f", "lavfi", "-i", &a, "-f", "lavfi", "-i", &b]);
            let d = *frames as f64 / FPS as f64;
            (
                format!(
                    "[0:v]{ea}trim=end_frame={frames},setpts=PTS-STARTPTS,format=yuv420p[a];\
                     [1:v]{eb}trim=end_frame={frames},setpts=PTS-STARTPTS,format=yuv420p[b];\
                     [a][b]xfade=transition={kind}:duration={d}:offset=0,{stamp}[v]"
                ),
                None,
            )
        }
    };
    let audio_input = if matches!(spec, Spec::Trans { .. }) { 2 } else { 1 };
    cmd.args(["-f", "lavfi", "-i", &audio_src(tone)]);
    cmd.args([
        "-filter_complex",
        &format!("{video_graph};[{audio_input}:a]atrim=end_sample={samples},asetpts=N/SR/TB[a]"),
        "-map",
        "[v]",
        "-map",
        "[a]",
        "-frames:v",
        &n.to_string(),
        "-c:v",
        "ffv1",
        "-c:a",
        "pcm_s16le",
    ]);
    cmd.arg(&out);
    run(&mut cmd).with_context(|| format!("element {idx}"))?;
    Ok(out)
}

fn probe_first_pts(file: &Path) -> Result<f64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-read_intervals",
            "%+#1",
            "-show_entries",
            "frame=pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(file)
        .output()
        .context("running ffprobe")?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines()
        .next()
        .and_then(|l| l.trim().trim_end_matches(',').parse().ok())
        .with_context(|| format!("no first pts for {}", file.display()))
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("spawning {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{:?} failed: {}", cmd.get_args().collect::<Vec<_>>(), String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

pub fn generate(out_dir: &Path, ff: &str) -> Result<()> {
    run(Command::new(ff).arg("-version")).context("ffmpeg not found; pass --ffmpeg PATH")?;
    let work = out_dir.join("work");
    fs::create_dir_all(&work)?;
    let tl = timeline();

    // 1. Elements, in parallel.
    println!("rendering {} timeline elements…", tl.len());
    let parts: Vec<PathBuf> = thread::scope(|s| {
        let handles: Vec<_> = (0..tl.len())
            .map(|i| {
                s.spawn({
                    let tl = &tl;
                    let work = &work;
                    move || render_element(ff, work, i, tl)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("render thread panicked")).collect::<Result<Vec<_>>>()
    })?;

    // 2. Lossless concat + frame-index stamp → master.
    let list = work.join("concat.txt");
    fs::write(
        &list,
        parts.iter().map(|p| format!("file '{}'\n", p.display().to_string().replace('\\', "/"))).collect::<String>(),
    )?;
    let master = out_dir.join("master.mkv");
    println!("stamping master…");
    run(Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i"])
        .arg(&list)
        .args(["-vf", &frame_stamp(), "-c:v", "ffv1", "-c:a", "pcm_s16le"])
        .arg(&master))?;

    // 3. Variants.
    let total: u32 = tl.iter().map(frames_of).sum();
    let mut variants = Vec::new();
    let h264 = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-pix_fmt", "yuv420p", "-g", "48", "-bf", "3"];
    let aac = ["-c:a", "aac", "-b:a", "192k"];
    let base = |v: &mut Vec<Variant>, file: &str, desc: &str, codec: &str, fps: &str, start: f64, tracks: u32| {
        v.push(Variant {
            file: file.into(),
            description: desc.into(),
            video_codec: codec.into(),
            fps: fps.into(),
            vfr: false,
            start_sec: start,
            frame_times_sec: None,
            audio_tracks: tracks,
        })
    };
    let ffv = |extra: &[&str], file: &str| -> Result<()> {
        println!("  {file}");
        let mut c = Command::new(ff);
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&master).args(extra).arg(out_dir.join(file));
        run(&mut c)
    };

    println!("encoding variants…");
    ffv(&[&h264[..], &aac[..], &["-movflags", "+faststart"]].concat(), "cfr24_h264.mp4")?;
    base(&mut variants, "cfr24_h264.mp4", "24 fps H.264 with B-pyramid", "h264", "24/1", 0.0, 1);

    ffv(
        &[
            &[
                "-vf",
                "settb=1/24000,setpts=N*1001",
                "-af",
                "atempo=0.999000999",
                "-r",
                "24000/1001",
                "-video_track_timescale",
                "24000",
            ][..],
            &h264[..],
            &aac[..],
        ]
        .concat(),
        "cfr23976_h264.mp4",
    )?;
    base(&mut variants, "cfr23976_h264.mp4", "23.976 fps H.264", "h264", "24000/1001", 0.0, 1);

    ffv(&[&h264[..], &aac[..], &["-output_ts_offset", "7.25"]].concat(), "offset725_h264.mp4")?;
    base(&mut variants, "offset725_h264.mp4", "H.264 MP4 whose first frame is at 7.25 s", "h264", "24/1", 7.25, 1);

    ffv(&[&h264[..], &aac[..], &["-f", "mpegts"]].concat(), "transport_h264.ts")?;
    // Mux delay + B-frame reorder delay; the exact value is measured below.
    base(&mut variants, "transport_h264.ts", "MPEG-TS (non-zero start time)", "h264", "24/1", 0.0, 1);

    ffv(
        &[
            &[
                "-c:v",
                "libx265",
                "-preset",
                "fast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p10le",
                "-x265-params",
                "log-level=error:open-gop=1:keyint=48:bframes=4:hdr10=1:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc",
                "-color_primaries",
                "bt2020",
                "-color_trc",
                "smpte2084",
                "-colorspace",
                "bt2020nc",
            ][..],
            &aac[..],
        ]
        .concat(),
        "hevc10_hdr.mkv",
    )?;
    base(&mut variants, "hevc10_hdr.mkv", "10-bit HEVC, open GOP, HDR10 tags", "hevc", "24/1", 0.0, 1);

    // True VFR: 24 fps, then 60 fps, then 30 fps (ticks of 1/120 s).
    let (a, b) = (200u32, 400u32);
    let ticks = |n: u32| -> u32 {
        if n < a {
            n * 5
        } else if n < b {
            a * 5 + (n - a) * 2
        } else {
            a * 5 + (b - a) * 2 + (n - b) * 4
        }
    };
    let expr = format!(
        "settb=1/120,setpts=if(lt(N\\,{a})\\,N*5\\,if(lt(N\\,{b})\\,{}+(N-{a})*2\\,{}+(N-{b})*4))",
        a * 5,
        a * 5 + (b - a) * 2
    );
    ffv(
        &[&["-an", "-vf", &expr, "-fps_mode", "passthrough", "-enc_time_base", "1/120"][..], &h264[..]].concat(),
        "vfr_h264.mkv",
    )?;
    variants.push(Variant {
        file: "vfr_h264.mkv".into(),
        description: "Variable frame rate (24 → 60 → 30 fps), video only".into(),
        video_codec: "h264".into(),
        fps: "24/1".into(),
        vfr: true,
        start_sec: 0.0,
        frame_times_sec: Some((0..total).map(|n| ticks(n) as f64 / 120.0).collect()),
        audio_tracks: 0,
    });

    ffv(
        &[
            &[
                "-filter_complex",
                "[0:a]asplit[a0][a1];[a1]volume=0.5[a1v]",
                "-map",
                "0:v",
                "-map",
                "[a0]",
                "-map",
                "[a1v]",
            ][..],
            &h264[..],
            &aac[..],
            &["-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=jpn"][..],
        ]
        .concat(),
        "two_audio_h264.mkv",
    )?;
    base(&mut variants, "two_audio_h264.mkv", "H.264 with two audio tracks (eng, jpn)", "h264", "24/1", 0.0, 2);

    ffv(&["-c:v", "prores_ks", "-profile:v", "3", "-c:a", "pcm_s24le"], "prores.mov")?;
    base(&mut variants, "prores.mov", "ProRes 422 HQ intra", "prores", "24/1", 0.0, 1);

    // Record the first frame's real presentation time (encoder/mux delays are
    // implementation details, so measure rather than assume), and check the
    // ones we set explicitly.
    for v in &mut variants {
        let measured = probe_first_pts(&out_dir.join(&v.file))?;
        if v.start_sec != 0.0 && (measured - v.start_sec).abs() > 1e-3 {
            bail!("{}: expected first frame at {} s, got {measured}", v.file, v.start_sec);
        }
        v.start_sec = measured;
    }

    // 4. Truth.
    let mut start = 0u32;
    let elements = tl
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let n = frames_of(s);
            let e = match s {
                Spec::Shot { label, flashes, held, tone, .. } => Element {
                    kind: ElementKind::Shot,
                    start,
                    end: start + n,
                    id: (i + 1) as u8,
                    label: (*label).into(),
                    transition: None,
                    flashes: flashes.iter().map(|f| start + f).collect(),
                    held: *held,
                    tone_hz: Some(*tone),
                },
                Spec::Trans { kind, .. } => Element {
                    kind: ElementKind::Transition,
                    start,
                    end: start + n,
                    id: (i + 1) as u8,
                    label: (*kind).into(),
                    transition: Some((*kind).into()),
                    flashes: vec![],
                    held: false,
                    tone_hz: None,
                },
                Spec::Black { .. } => Element {
                    kind: ElementKind::Black,
                    start,
                    end: start + n,
                    id: (i + 1) as u8,
                    label: "black".into(),
                    transition: None,
                    flashes: vec![],
                    held: false,
                    tone_hz: None,
                },
            };
            start += n;
            e
        })
        .collect();
    let truth = Truth { fps: FPS, width: W, height: H, frame_count: total, audio_sample_rate: SR, elements, variants };
    fs::write(out_dir.join("truth.json"), serde_json::to_string_pretty(&truth)?)?;
    fs::remove_dir_all(&work).ok();
    println!("wrote {} ({} frames, {} variants)", out_dir.display(), total, truth.variants.len());
    Ok(())
}
