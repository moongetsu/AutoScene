//! Constrained average-linkage agglomerative clustering.
//!
//! With unit-length embeddings the average pairwise cosine between two groups
//! is `S_a · S_b / (W_a W_b)` (S: weighted sum of embeddings, W: total
//! weight), so merged groups update by the Lance–Williams rule without
//! revisiting their members. Cannot-links are kept per group and inherited
//! on merge; the best allowed neighbour of every group is cached, so a merge
//! costs one row update instead of a full search.

use std::collections::HashMap;

use crate::{CharKey, TrackIn};

/// Above this many groups the clustering runs per media first.
const SINGLE_LEVEL: usize = 4000;
/// At most this many multi-track groups go into the cross-media level.
const SECOND_LEVEL: usize = 6000;

struct Group {
    members: Vec<usize>,
    sum: Vec<f32>,
    weight: f32,
    pin: Option<CharKey>,
}

fn weight(t: &TrackIn) -> f32 {
    t.quality.max(0.05)
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Group the tracks `core` (indices into `tracks`) by identity. Returns the
/// groups (track indices, ascending), largest first.
pub fn cluster(tracks: &[TrackIn], core: &[usize], link: f32) -> Vec<Vec<usize>> {
    let mut groups = initial_groups(tracks, core);
    if groups.len() > SINGLE_LEVEL {
        // Per media, then across media with the groups that formed.
        let mut by_media: HashMap<u32, Vec<Group>> = HashMap::new();
        let mut pinned = Vec::new();
        for g in groups {
            if g.pin.is_some() {
                pinned.push(g);
            } else {
                by_media.entry(tracks[g.members[0]].media).or_default().push(g);
            }
        }
        let mut media: Vec<u32> = by_media.keys().copied().collect();
        media.sort_unstable();
        let mut level1 = pinned;
        for m in media {
            level1.extend(merge(tracks, by_media.remove(&m).expect("listed"), link));
        }
        level1.sort_by(|a, b| b.weight.total_cmp(&a.weight));
        let split = level1.iter().take(SECOND_LEVEL).take_while(|g| g.members.len() > 1 || g.pin.is_some()).count();
        let rest = level1.split_off(split);
        groups = merge(tracks, level1, link);
        groups.extend(rest);
    } else {
        groups = merge(tracks, groups, link);
    }
    let mut out: Vec<Vec<usize>> = groups
        .into_iter()
        .map(|mut g| {
            g.members.sort_unstable();
            g.members
        })
        .collect();
    out.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    out
}

/// Singletons, except that tracks pinned to the same character start together.
fn initial_groups(tracks: &[TrackIn], core: &[usize]) -> Vec<Group> {
    let dim = tracks.first().map_or(0, |t| t.embedding.len());
    let mut by_pin: HashMap<CharKey, usize> = HashMap::new();
    let mut groups: Vec<Group> = Vec::with_capacity(core.len());
    for &i in core {
        let t = &tracks[i];
        let slot = match t.pin {
            Some(k) => *by_pin.entry(k).or_insert_with(|| {
                groups.push(Group { members: vec![], sum: vec![0.0; dim], weight: 0.0, pin: Some(k) });
                groups.len() - 1
            }),
            None => {
                groups.push(Group { members: vec![], sum: vec![0.0; dim], weight: 0.0, pin: None });
                groups.len() - 1
            }
        };
        let g = &mut groups[slot];
        let w = weight(t);
        for (s, x) in g.sum.iter_mut().zip(&t.embedding) {
            *s += w * x;
        }
        g.weight += w;
        g.members.push(i);
    }
    groups
}

/// Square bit matrix.
struct Bits {
    n: usize,
    words: Vec<u64>,
}

impl Bits {
    fn new(n: usize) -> Self {
        Self { n, words: vec![0; (n * n).div_ceil(64)] }
    }
    fn get(&self, a: usize, b: usize) -> bool {
        let i = a * self.n + b;
        self.words[i / 64] >> (i % 64) & 1 == 1
    }
    fn set(&mut self, a: usize, b: usize) {
        for i in [a * self.n + b, b * self.n + a] {
            self.words[i / 64] |= 1 << (i % 64);
        }
    }
}

fn cannot_links(tracks: &[TrackIn], groups: &[Group]) -> Bits {
    let k = groups.len();
    let mut cl = Bits::new(k);
    // Tracks on screen at the same time: sweep each media by start.
    let mut owner: Vec<(usize, usize)> = Vec::new(); // (track, group)
    for (gi, g) in groups.iter().enumerate() {
        owner.extend(g.members.iter().map(|&t| (t, gi)));
    }
    owner.sort_by_key(|&(t, _)| (tracks[t].media, tracks[t].start));
    for a in 0..owner.len() {
        let (ta, ga) = owner[a];
        for &(tb, gb) in &owner[a + 1..] {
            if tracks[tb].media != tracks[ta].media || tracks[tb].start >= tracks[ta].end {
                break;
            }
            if ga != gb {
                cl.set(ga, gb);
            }
        }
    }
    // Different pins; forbidden characters.
    let pins: Vec<(usize, CharKey)> = groups.iter().enumerate().filter_map(|(i, g)| Some((i, g.pin?))).collect();
    for (x, &(ga, ka)) in pins.iter().enumerate() {
        for &(gb, kb) in &pins[x + 1..] {
            if ka != kb {
                cl.set(ga, gb);
            }
        }
    }
    for (gi, g) in groups.iter().enumerate() {
        for &t in &g.members {
            for &(gp, kp) in &pins {
                if gp != gi && tracks[t].forbid.contains(&kp) {
                    cl.set(gi, gp);
                }
            }
        }
    }
    cl
}

fn merge(tracks: &[TrackIn], mut groups: Vec<Group>, link: f32) -> Vec<Group> {
    let k = groups.len();
    if k < 2 {
        return groups;
    }
    let mut cl = cannot_links(tracks, &groups);
    let mut sim = vec![0f32; k * k];
    for a in 0..k {
        for b in a + 1..k {
            let s = dot(&groups[a].sum, &groups[b].sum) / (groups[a].weight * groups[b].weight);
            sim[a * k + b] = s;
            sim[b * k + a] = s;
        }
    }
    let mut alive = vec![true; k];
    let best_of = |a: usize, sim: &[f32], alive: &[bool], cl: &Bits| -> (f32, usize) {
        let mut best = (f32::NEG_INFINITY, usize::MAX);
        for b in 0..k {
            if b != a && alive[b] && !cl.get(a, b) && sim[a * k + b] > best.0 {
                best = (sim[a * k + b], b);
            }
        }
        best
    };
    let mut best: Vec<(f32, usize)> = (0..k).map(|a| best_of(a, &sim, &alive, &cl)).collect();
    // The globally closest allowed pair, while it is close enough.
    while let Some((a, &(s, b))) =
        best.iter().enumerate().filter(|&(i, _)| alive[i]).max_by(|x, y| x.1.0.total_cmp(&y.1.0).then(y.0.cmp(&x.0)))
    {
        if s < link || b == usize::MAX {
            break;
        }
        // Merge b into a.
        let (wa, wb) = (groups[a].weight, groups[b].weight);
        for x in 0..k {
            if x != a && x != b && alive[x] {
                let v = (wa * sim[a * k + x] + wb * sim[b * k + x]) / (wa + wb);
                sim[a * k + x] = v;
                sim[x * k + a] = v;
                if cl.get(b, x) {
                    cl.set(a, x);
                }
            }
        }
        alive[b] = false;
        let gb = std::mem::replace(&mut groups[b], Group { members: vec![], sum: vec![], weight: 0.0, pin: None });
        let ga = &mut groups[a];
        ga.members.extend(gb.members);
        for (x, y) in ga.sum.iter_mut().zip(&gb.sum) {
            *x += y;
        }
        ga.weight += gb.weight;
        ga.pin = ga.pin.or(gb.pin);
        best[a] = best_of(a, &sim, &alive, &cl);
        for x in 0..k {
            if !alive[x] || x == a {
                continue;
            }
            if best[x].1 == a || best[x].1 == b {
                best[x] = best_of(x, &sim, &alive, &cl);
            } else if !cl.get(x, a) && sim[x * k + a] > best[x].0 {
                best[x] = (sim[x * k + a], a);
            }
        }
    }
    groups.into_iter().zip(alive).filter(|(_, a)| *a).map(|(g, _)| g).collect()
}
