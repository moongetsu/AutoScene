//! Dev tool: draw the cells `bodies_bench sheets` chose — each a frame with
//! its boxes and small pictures (cut from other frames) of who the
//! pipeline says it is — six to a JPEG sheet, numbered, to look at.
//! `cargo run --release -p as-analysis --example bodies_render -- <ffmpeg_dir> <dir with
//! bb.index/bb.info.json/bb.path> <items.tsv> <out_dir>`

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use as_analysis::{FaceParams, frame_sizes};
use as_core::{CancelToken, FrameIndex};
use as_media::{MediaInfo, Source, Tools};
use as_vision::image;

/// Cell size and inset height.
const CW: usize = 640;
const CH: usize = 360;
const INSET: usize = 110;
/// Cells per sheet: 2 across, 3 down.
const COLS: usize = 2;
const ROWS: usize = 3;

struct Item {
    sheet: usize,
    cell: usize,
    frame: u32,
    boxes: Vec<([f32; 4], [u8; 3], usize)>,
    insets: Vec<(u32, [f32; 4])>,
}

fn quad(s: &str) -> [f32; 4] {
    let v: Vec<f32> = s.split(',').map(|x| x.parse().unwrap()).collect();
    [v[0], v[1], v[2], v[3]]
}

fn colour(hex: &str) -> [u8; 3] {
    let v = u32::from_str_radix(hex, 16).unwrap();
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn read_items(path: &Path) -> Vec<Item> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .skip(1)
        .map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            let boxes = v[4]
                .split(';')
                .filter(|s| !s.is_empty())
                .map(|b| {
                    let p: Vec<&str> = b.split(',').collect();
                    (quad(&p[..4].join(",")), colour(p[4]), p[5].parse().unwrap())
                })
                .collect();
            let insets = v[5]
                .split(';')
                .filter(|s| !s.is_empty())
                .map(|i| {
                    let (f, b) = i.split_once(':').unwrap();
                    (f.parse().unwrap(), quad(b))
                })
                .collect();
            Item {
                sheet: v[0].parse().unwrap(),
                cell: v[1].parse().unwrap(),
                frame: v[3].parse().unwrap(),
                boxes,
                insets,
            }
        })
        .collect()
}

/// A rectangle outline `t` pixels thick (box in pixels, clipped).
fn rect(img: &mut [u8], (w, h): (usize, usize), b: [f32; 4], c: [u8; 3], t: usize) {
    let x0 = (b[0].max(0.0) as usize).min(w - 1);
    let y0 = (b[1].max(0.0) as usize).min(h - 1);
    let x1 = (b[2].max(0.0) as usize).min(w - 1);
    let y1 = (b[3].max(0.0) as usize).min(h - 1);
    let mut put = |x: usize, y: usize| {
        if x < w && y < h {
            img[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&c);
        }
    };
    for k in 0..t {
        for x in x0..=x1 {
            put(x, y0 + k);
            put(x, y1.saturating_sub(k));
        }
        for y in y0..=y1 {
            put(x0 + k, y);
            put(x1.saturating_sub(k), y);
        }
    }
}

fn paste(img: &mut [u8], w: usize, src: &[u8], (sw, sh): (usize, usize), (x, y): (usize, usize)) {
    for r in 0..sh {
        let d = ((y + r) * w + x) * 3;
        img[d..d + sw * 3].copy_from_slice(&src[r * sw * 3..(r + 1) * sw * 3]);
    }
}

/// 3×5 digits.
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

fn number(img: &mut [u8], w: usize, n: usize, (x, y): (usize, usize), k: usize) {
    let s = n.to_string();
    let (bw, bh) = (s.len() * 4 * k + k, 7 * k);
    for yy in y..y + bh {
        for xx in x..x + bw {
            img[(yy * w + xx) * 3..(yy * w + xx) * 3 + 3].copy_from_slice(&[0, 0, 0]);
        }
    }
    for (i, ch) in s.bytes().enumerate() {
        let g = DIGITS[(ch - b'0') as usize];
        for (r, row) in g.iter().enumerate() {
            for c in 0..3 {
                if row >> (2 - c) & 1 == 1 {
                    for dy in 0..k {
                        for dx in 0..k {
                            let (px, py) = (x + k + (i * 4 + c) * k + dx, y + k + r * k + dy);
                            img[(py * w + px) * 3..(py * w + px) * 3 + 3].copy_from_slice(&[255, 255, 255]);
                        }
                    }
                }
            }
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (ffmpeg, dir, items_path, out) = (&a[0], Path::new(&a[1]), Path::new(&a[2]), Path::new(&a[3]));
    let items = read_items(items_path);
    let index = FrameIndex::from_bytes(&std::fs::read(dir.join("bb.index")).unwrap()).unwrap();
    let info: MediaInfo = serde_json::from_str(&std::fs::read_to_string(dir.join("bb.info.json")).unwrap()).unwrap();
    let path = PathBuf::from(std::fs::read_to_string(dir.join("bb.path")).unwrap().trim());
    let tools = Tools::locate(Some(Path::new(ffmpeg))).unwrap();
    let params = FaceParams::default();
    let ((w, h), _) = frame_sizes(&info, &params);

    // What each needed frame gives: a cell picture, and insets.
    let mut cells_at: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut insets_at: HashMap<u32, Vec<(usize, usize)>> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        cells_at.entry(it.frame).or_default().push(i);
        for (k, (f, _)) in it.insets.iter().enumerate() {
            insets_at.entry(*f).or_default().push((i, k));
        }
    }
    let mut pictures: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut small: HashMap<(usize, usize), (Vec<u8>, usize)> = HashMap::new();
    let last =
        items.iter().flat_map(|it| std::iter::once(it.frame).chain(it.insets.iter().map(|x| x.0))).max().unwrap_or(0);
    let cancel = CancelToken::new();
    as_media::frames::decode_samples(
        &tools,
        Source { path: &path, info: &info, index: &index },
        0,
        params.stride,
        (w as u32, h as u32),
        &cancel,
        |f, rgb| {
            if let Some(v) = cells_at.get(&f) {
                let mut p = vec![0u8; CW * CH * 3];
                image::shrink_into(rgb, (w, h), &mut p, CW, 0, (CW, CH));
                for &i in v {
                    pictures.insert(i, p.clone());
                }
            }
            if let Some(v) = insets_at.get(&f) {
                for &(i, k) in v {
                    let b = items[i].insets[k].1;
                    // A little room around the box, at most twice as wide as tall.
                    let (bw, bh) = ((b[2] - b[0]) * w as f32, (b[3] - b[1]) * h as f32);
                    let pad = 0.15 * bw.max(bh);
                    let region =
                        [b[0] * w as f32 - pad, b[1] * h as f32 - pad, b[2] * w as f32 + pad, b[3] * h as f32 + pad];
                    let iw =
                        ((INSET as f32 * (bw + 2.0 * pad) / (bh + 2.0 * pad)).round() as usize).clamp(24, 2 * INSET);
                    small.insert((i, k), (image::resize_region(rgb, (w, h), region, (iw, INSET)), iw));
                }
            }
            if f % 6000 == 0 {
                eprint!("  {:.0}%", 100.0 * f as f64 / last.max(1) as f64);
            }
            Ok(f < last)
        },
    )
    .unwrap();
    eprintln!();
    std::fs::create_dir_all(out).unwrap();
    let sheets = items.iter().map(|it| it.sheet).max().map_or(0, |s| s + 1);
    let (sw, sh) = (CW * COLS, CH * ROWS);
    for s in 0..sheets {
        let mut sheet = vec![32u8; sw * sh * 3];
        for (i, it) in items.iter().enumerate().filter(|(_, it)| it.sheet == s) {
            let Some(mut p) = pictures.remove(&i) else { continue };
            for &(b, c, t) in &it.boxes {
                let px = [b[0] * CW as f32, b[1] * CH as f32, b[2] * CW as f32, b[3] * CH as f32];
                rect(&mut p, (CW, CH), px, c, t.max(1) / 2 + 1);
            }
            let mut x = 4;
            for k in 0..it.insets.len() {
                if let Some((img, iw)) = small.get(&(i, k)) {
                    if x + iw + 4 > CW {
                        break;
                    }
                    rect(
                        &mut p,
                        (CW, CH),
                        [x as f32 - 2.0, 2.0, (x + iw + 1) as f32, (INSET + 5) as f32],
                        [255, 255, 255],
                        2,
                    );
                    paste(&mut p, CW, img, (*iw, INSET), (x, 4));
                    x += iw + 8;
                }
            }
            number(&mut p, CW, it.cell, (0, CH - 7 * 4 - 4), 4);
            let (cx, cy) = (it.cell % COLS * CW, it.cell / COLS * CH);
            for r in 0..CH {
                let d = ((cy + r) * sw + cx) * 3;
                sheet[d..d + CW * 3].copy_from_slice(&p[r * CW * 3..(r + 1) * CW * 3]);
            }
        }
        // Cell borders.
        for c in 1..COLS {
            for y in 0..sh {
                let d = (y * sw + c * CW) * 3;
                sheet[d..d + 6].copy_from_slice(&[255, 255, 255, 255, 255, 255]);
            }
        }
        for r in 1..ROWS {
            let d = r * CH * sw * 3;
            sheet[d..d + sw * 3 * 2].fill(255);
        }
        std::fs::write(out.join(format!("sheet_{s:03}.jpg")), image::jpeg(&sheet, sw, sh, 82)).unwrap();
    }
    println!("{sheets} sheets in {}", out.display());
}
