//! Dev tool: run shot fusion on saved signals (`shot_signals`), list
//! everything that isn't a plain shot, and optionally compare the hard cuts
//! with a reference list (a JSON file whose first array holds cut frames) or
//! print the raw signals of a frame range.
//! `cargo run -p as-analysis --example fuse_signals -- <in.sig> [--anime] [--ref cuts.json] [--tsv FROM TO]`

use as_analysis::{FusionParams, SegmentKind, fuse, signals};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name);
    let sig = signals::decode(&std::fs::read(&args[1]).unwrap()).expect("a signals file");
    if let Some(i) = flag("--tsv") {
        let (from, to): (usize, usize) = (args[i + 1].parse().unwrap(), args[i + 2].parse().unwrap());
        println!("f\tsingle\tmany\tmu\tsd\td1\td2\td3\td4\thist\tdark\tlight");
        for (f, x) in sig.iter().enumerate().take(to.min(sig.len())).skip(from) {
            println!(
                "{f}\t{:.3}\t{:.3}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.3}\t{:.2}\t{:.2}",
                x.single, x.many, x.mu, x.sd, x.diff[0], x.diff[1], x.diff[2], x.diff[3], x.hist, x.dark, x.light
            );
        }
        return;
    }
    let params = if flag("--anime").is_some() { FusionParams::anime() } else { FusionParams::live() };
    let segs = fuse(&sig, &params);
    let count = |k: fn(&SegmentKind) -> bool| segs.iter().filter(|s| k(&s.kind)).count();
    println!(
        "{} frames → {} segments: {} shots ({} tiny), {} transitions, {} black, {} flash",
        sig.len(),
        segs.len(),
        count(|k| *k == SegmentKind::Shot),
        segs.iter().filter(|s| s.tiny).count(),
        count(|k| matches!(k, SegmentKind::Transition(_))),
        count(|k| *k == SegmentKind::Black),
        count(|k| *k == SegmentKind::Flash),
    );
    let boundary = |k: &SegmentKind| !matches!(k, SegmentKind::Transition(_) | SegmentKind::Flash);
    let hard: Vec<u32> =
        segs.windows(2).filter(|w| boundary(&w[0].kind) && boundary(&w[1].kind)).map(|w| w[1].start).collect();
    println!("hard cuts: {}", hard.len());
    for s in segs.iter().filter(|s| !s.is_shot() || s.tiny) {
        println!("  {:>6}..{:<6} {:?}{}", s.start, s.end, s.kind, if s.tiny { " (tiny)" } else { "" });
    }
    if let Some(i) = flag("--ref") {
        let j = std::fs::read_to_string(&args[i + 1]).unwrap();
        let start = j.find('[').unwrap();
        let end = j[start..].find(']').unwrap() + start;
        let reference: Vec<u32> = j[start + 1..end].split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let near = |a: u32, list: &[u32]| list.iter().any(|&b| a.abs_diff(b) <= 3);
        let missing: Vec<u32> = reference.iter().copied().filter(|&c| !near(c, &hard)).collect();
        let extra: Vec<u32> = hard.iter().copied().filter(|&c| !near(c, &reference)).collect();
        println!(
            "reference {} | matched {} | missing {}: {:?}",
            reference.len(),
            reference.len() - missing.len(),
            missing.len(),
            missing
        );
        println!("extra {}: {:?}", extra.len(), extra);
    }
}
