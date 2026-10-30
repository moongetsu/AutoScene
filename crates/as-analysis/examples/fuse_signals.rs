//! Dev tool: run shot fusion on saved signals (`shot_signals`), list
//! everything that isn't a plain shot, and optionally compare the hard cuts
//! with a reference list (a JSON file whose first array holds cut frames) or
//! print the raw signals of a frame range (`--tsv FROM TO`), or write the
//! segments as `bb.segments.tsv` (`--tsv <out>`: `start  end  kind` rows, no
//! header, kind `shot`, `dissolve`, `fade_out`, `fade_in`, `black` or `flash`),
//! the shot list `faces_run`, `fast_run` and the anime bench read.
//! `cargo run -p as-analysis --example fuse_signals -- <in.sig> [--anime] [--ref cuts.json] [--tsv FROM TO | --tsv out.tsv]`

use as_analysis::{FusionParams, SegmentKind, TransitionKind, fuse, signals};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name);
    let tsv = tsv(&args);
    let sig = signals::decode(&std::fs::read(&args[1]).unwrap()).expect("a signals file");
    if let Some(Tsv::Raw(from, to)) = tsv {
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
    if let Some(Tsv::Write(out)) = tsv {
        let rows: String = segs.iter().map(|s| format!("{}\t{}\t{}\n", s.start, s.end, kind_name(&s.kind))).collect();
        std::fs::write(out, rows).unwrap();
        println!("wrote {} segments to {out}", segs.len());
    }
}

/// What `--tsv` asks for.
#[derive(Debug, PartialEq)]
enum Tsv<'a> {
    /// Print the raw signals of frames `FROM..TO`.
    Raw(usize, usize),
    /// Write the segments to this file.
    Write(&'a str),
}

/// `--tsv FROM TO` (two numbers) or `--tsv <out>`. A lone number or a flag
/// after `--tsv` is refused, so a typo never creates a file.
fn tsv(args: &[String]) -> Option<Tsv<'_>> {
    let i = args.iter().position(|a| a == "--tsv")?;
    let next = |k: usize| args.get(i + k).map(String::as_str);
    let number = |k: usize| next(k).and_then(|a| a.parse::<usize>().ok());
    if let (Some(from), Some(to)) = (number(1), number(2)) {
        return Some(Tsv::Raw(from, to));
    }
    let out = next(1).expect("--tsv needs FROM TO or an output file");
    assert!(
        out.parse::<f64>().is_err() && !out.starts_with("--"),
        "--tsv {out}: FROM TO takes two numbers, and an output file is neither a number nor a flag"
    );
    Some(Tsv::Write(out))
}

/// A segment's kind as `bb.segments.tsv` names it.
fn kind_name(kind: &SegmentKind) -> &'static str {
    match kind {
        SegmentKind::Shot => "shot",
        SegmentKind::Transition(TransitionKind::Dissolve) => "dissolve",
        SegmentKind::Transition(TransitionKind::FadeOut) => "fade_out",
        SegmentKind::Transition(TransitionKind::FadeIn) => "fade_in",
        SegmentKind::Black => "black",
        SegmentKind::Flash => "flash",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &str) -> Vec<String> {
        a.split(' ').map(String::from).collect()
    }

    #[test]
    fn tsv_prints_a_range_or_writes_a_file() {
        assert_eq!(tsv(&args("x in.sig --anime")), None);
        assert_eq!(tsv(&args("x in.sig --tsv 3 40 --anime")), Some(Tsv::Raw(3, 40)));
        assert_eq!(tsv(&args("x in.sig --anime --tsv bb.segments.tsv")), Some(Tsv::Write("bb.segments.tsv")));
    }

    #[test]
    #[should_panic(expected = "neither a number nor a flag")]
    fn tsv_refuses_a_lone_number() {
        tsv(&args("x in.sig --tsv 3 --anime"));
    }

    #[test]
    #[should_panic(expected = "neither a number nor a flag")]
    fn tsv_refuses_a_flag() {
        tsv(&args("x in.sig --tsv --anime"));
    }

    #[test]
    #[should_panic(expected = "needs FROM TO or an output file")]
    fn tsv_needs_something_after_it() {
        tsv(&args("x in.sig --tsv"));
    }

    #[test]
    fn kinds_named_as_the_segments_file_has_them() {
        let kinds = [
            SegmentKind::Shot,
            SegmentKind::Transition(TransitionKind::Dissolve),
            SegmentKind::Transition(TransitionKind::FadeOut),
            SegmentKind::Transition(TransitionKind::FadeIn),
            SegmentKind::Black,
            SegmentKind::Flash,
        ];
        assert_eq!(kinds.map(|k| kind_name(&k)), ["shot", "dissolve", "fade_out", "fade_in", "black", "flash"]);
    }
}
