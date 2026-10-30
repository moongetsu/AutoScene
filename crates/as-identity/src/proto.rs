//! Character prototypes: a few unit vectors that together cover how a
//! character looks across lighting, angles, glasses or make-up. A face
//! resembles a character as much as it resembles the closest prototype.

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter_mut().for_each(|x| *x /= n);
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Weighted mean direction of `members` (embedding, weight).
pub fn centroid(members: &[(&[f32], f32)]) -> Vec<f32> {
    let dim = members.first().map_or(0, |m| m.0.len());
    let mut c = vec![0f32; dim];
    for (e, w) in members {
        for (x, y) in c.iter_mut().zip(e.iter()) {
            *x += w * y;
        }
    }
    normalize(&mut c);
    c
}

/// Up to `k` prototypes by weighted spherical k-means (farthest-point
/// initialisation from the heaviest member, so the result is deterministic).
/// A member at least `dedup` alike to a centre found already is the same
/// look: no centre of its own.
pub fn prototypes(members: &[(&[f32], f32)], k: usize, dedup: f32) -> Vec<Vec<f32>> {
    if members.is_empty() {
        return Vec::new();
    }
    // Small groups: a prototype per 3 members at most.
    let k = k.min(members.len().div_ceil(3)).max(1);
    let first = (0..members.len()).max_by(|&a, &b| members[a].1.total_cmp(&members[b].1)).expect("non-empty");
    let mut centers: Vec<Vec<f32>> = vec![members[first].0.to_vec()];
    let mut closest: Vec<f32> = members.iter().map(|m| dot(m.0, &centers[0])).collect();
    while centers.len() < k {
        // The member least like any centre, weighted by how much it counts.
        let far = (0..members.len())
            .max_by(|&a, &b| ((1.0 - closest[a]) * members[a].1).total_cmp(&((1.0 - closest[b]) * members[b].1)))
            .expect("non-empty");
        if closest[far] > dedup {
            break;
        }
        centers.push(members[far].0.to_vec());
        let c = centers.last().expect("pushed");
        for (i, m) in members.iter().enumerate() {
            closest[i] = closest[i].max(dot(m.0, c));
        }
    }
    for _ in 0..8 {
        let mut sums = vec![vec![0f32; centers[0].len()]; centers.len()];
        for (e, w) in members {
            let j =
                (0..centers.len()).max_by(|&a, &b| dot(e, &centers[a]).total_cmp(&dot(e, &centers[b]))).expect("k>0");
            for (x, y) in sums[j].iter_mut().zip(e.iter()) {
                *x += w * y;
            }
        }
        for (c, s) in centers.iter_mut().zip(sums) {
            if s.iter().any(|&x| x != 0.0) {
                *c = s;
                normalize(c);
            }
        }
    }
    centers
}

/// How much `e` resembles a character with these prototypes.
pub fn similarity(e: &[f32], protos: &[Vec<f32>]) -> f32 {
    protos.iter().map(|p| dot(e, p)).fold(f32::NEG_INFINITY, f32::max)
}
