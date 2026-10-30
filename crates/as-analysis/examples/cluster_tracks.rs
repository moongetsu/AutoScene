//! Dev tool: cluster saved face tracks (from `faces_run`) into characters and
//! dump each character's thumbnails for a contact sheet.
//! `cargo run --release -p as-analysis --example cluster_tracks -- <file.tracks> <out_dir> [link] [assign]`
//! Writes `<out_dir>/c<rank>_<n>.jpg` (up to 16 tracks per character, spread
//! over the episode) and `<out_dir>/characters.tsv`.

use as_identity::{Assign, Params, TrackIn, identify};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tracks = as_analysis::tracks::decode(&std::fs::read(&a[0]).unwrap()).unwrap();
    let out = std::path::Path::new(&a[1]);
    std::fs::create_dir_all(out).unwrap();
    let mut p = Params::default();
    if let Some(v) = a.get(2) {
        p.link = v.parse().unwrap();
    }
    if let Some(v) = a.get(3) {
        p.assign = v.parse().unwrap();
    }
    let input: Vec<TrackIn> = tracks
        .iter()
        .map(|t| TrackIn {
            media: 0,
            start: t.start,
            end: t.end,
            spans: as_analysis::tracks::presence_spans(&t.dets, t.end, 3, 8),
            seen: t.dets.iter().map(|d| d.frame).collect(),
            embedding: t.embedding.clone(),
            quality: t.quality,
            dets: t.dets.len() as u32,
            weight: 1.0,
            pin: None,
            forbid: vec![],
            previous: None,
        })
        .collect();
    let t0 = std::time::Instant::now();
    let o = identify(&input, &p);
    let secs = t0.elapsed().as_secs_f64();
    let mut members: Vec<Vec<usize>> = vec![vec![]; o.characters.len()];
    let (mut unsure, mut none) = (0, 0);
    for (i, (a, _)) in o.tracks.iter().enumerate() {
        match a {
            Assign::Auto(c) | Assign::Pinned(c) => members[*c].push(i),
            Assign::Unsure(_) => unsure += 1,
            Assign::None => none += 1,
        }
    }
    let screen = |m: &[usize]| m.iter().map(|&i| (tracks[i].end - tracks[i].start) as f64).sum::<f64>() / 23.976;
    let mut order: Vec<usize> = (0..members.len()).collect();
    order.sort_by(|&x, &y| screen(&members[y]).total_cmp(&screen(&members[x])));
    for f in std::fs::read_dir(out).unwrap().flatten() {
        if f.path().extension().is_some_and(|e| e == "jpg") {
            let _ = std::fs::remove_file(f.path());
        }
    }
    let mut tsv = String::from("rank\ttracks\tscreen_s\tfirst\tlast\n");
    for (rank, &c) in order.iter().enumerate() {
        let mut m = members[c].clone();
        m.sort_by_key(|&i| tracks[i].start);
        let pick: Vec<usize> = if m.len() <= 16 { m.clone() } else { (0..16).map(|k| m[k * m.len() / 16]).collect() };
        for (k, &i) in pick.iter().enumerate() {
            std::fs::write(out.join(format!("c{rank:02}_{k:02}.jpg")), &tracks[i].thumb).unwrap();
        }
        // Cohesion: quality-weighted mean pairwise similarity; mean quality and norm.
        let (mut num, mut den) = (0f64, 0f64);
        for (x, &i) in m.iter().enumerate() {
            for &j in &m[x + 1..] {
                let w = (tracks[i].quality * tracks[j].quality) as f64;
                num += w * as_vision::arcface::cosine(&tracks[i].embedding, &tracks[j].embedding) as f64;
                den += w;
            }
        }
        let q = m.iter().map(|&i| tracks[i].quality).sum::<f32>() / m.len() as f32;
        let norm = m
            .iter()
            .map(|&i| tracks[i].dets.iter().map(|d| d.norm).sum::<f32>() / tracks[i].dets.len() as f32)
            .sum::<f32>()
            / m.len() as f32;
        tsv += &format!(
            "{rank}\t{}\t{:.0}\t{}\t{}\t{:.3}\t{q:.2}\t{norm:.1}\n",
            m.len(),
            screen(&m),
            tracks[m[0]].start,
            tracks[*m.last().unwrap()].start,
            num / den.max(1e-9)
        );
    }
    std::fs::write(out.join("characters.tsv"), &tsv).unwrap();
    // Tracks that joined a character by resemblance (not clustered), weakest first.
    let mut joined: Vec<(f32, usize, usize)> = o
        .tracks
        .iter()
        .enumerate()
        .filter_map(|(i, (a, s))| match a {
            Assign::Auto(c) if *s < 1.0 => Some((*s, i, order.iter().position(|x| x == c).unwrap())),
            _ => None,
        })
        .collect();
    joined.sort_by(|a, b| a.0.total_cmp(&b.0));
    let jdir = out.join("joined");
    let _ = std::fs::remove_dir_all(&jdir);
    std::fs::create_dir_all(&jdir).unwrap();
    let mut jt = String::new();
    for (k, &(s, i, rank)) in joined.iter().enumerate() {
        std::fs::write(jdir.join(format!("c{:02}_{:02}.jpg", k / 16, k % 16)), &tracks[i].thumb).unwrap();
        jt += &format!(
            "{k}	{s:.3}	{:.2}	{rank}
",
            tracks[i].quality
        );
    }
    std::fs::write(jdir.join("joined.tsv"), jt).unwrap();
    println!(
        "{} tracks → {} characters in {secs:.2}s; {unsure} unsure, {none} unassigned (link {}, assign {})",
        tracks.len(),
        o.characters.len(),
        p.link,
        p.assign
    );
    print!("{tsv}");
}
