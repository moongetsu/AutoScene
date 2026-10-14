use as_analysis::signals::{self, COLORS};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let sig = signals::decode(&std::fs::read(&a[1]).unwrap()).unwrap();
    let (ia, ib): (usize, usize) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let mean = |r: std::ops::RangeInclusive<usize>| {
        let k = (r.end() - r.start() + 1) as f32;
        let mut m = [0f32; COLORS];
        for x in r {
            m.iter_mut().zip(&sig[x].colors).for_each(|(m, &c)| *m += c as f32 / k);
        }
        m
    };
    let ap = mean(ia..=ia + 2);
    let bp = mean(ib - 2..=ib);
    let dir: Vec<f32> = bp.iter().zip(&ap).map(|(b, a)| b - a).collect();
    let norm: f32 = dir.iter().map(|d| d * d).sum();
    println!("total cd {:.3}", sig[ia].color_distance(&sig[ib]));
    for x in ia..=ib {
        let p = sig[x].colors.iter().zip(&ap).zip(&dir).map(|((&c, a), d)| (c as f32 - a) * d).sum::<f32>() / norm;
        println!(
            "{x}\t{p:.3}\tcdA {:.3}\tcdB {:.3}\td1 {:.1}",
            sig[x].color_distance(&sig[ia]),
            sig[x].color_distance(&sig[ib]),
            sig[x].diff[0]
        );
    }
}
