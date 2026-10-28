//! Constrained average-linkage agglomerative clustering.
//!
//! With unit-length embeddings the average pairwise cosine between two groups
//! is `S_a · S_b / (W_a W_b)` (S: weighted sum of embeddings, W: total
//! weight), so merged groups update by the Lance–Williams rule without
//! revisiting their members. The best allowed neighbour of every group is
//! cached, so a merge costs one row update instead of a full search.
//!
//! Two faces found in the same frame are two people — nearly always. A
//! mirror, a photo or a screen shows one person twice, and now and then a
//! stranger's face slips into a group: one such pair must not keep two
//! groups of one person apart for good. So the clustering runs twice:
//!
//! 1. Faces seen together are two people (cannot-link, inherited by the
//!    groups they are in) — per media first when there are many groups,
//!    and a long media in stretches of time.
//! 2. Then the groups of two or more faces, each whole: groups with faces
//!    seen together still merge when their mean faces are clearly one
//!    person's (`Params::copy_link`) and few of their faces were seen
//!    together (`Params::copy_share` of the smaller group, at least one).
//!    One face of every such pair steps aside, so no two faces of a group
//!    are on screen at once; it leaves the clustering as a group of its
//!    own, and the caller places it like any other loose track. Whole
//!    groups only: two people who are often on screen together (twins)
//!    can't be joined piece by piece, one episode's fragment at a time.
//!
//! Pins are absolute: groups pinned to different characters never meet,
//! and a pinned track never steps aside.

use std::collections::HashMap;

use crate::{CharKey, Params, TrackIn, together};

/// Pairs of tracks whose faces were seen together.
type Pairs = Vec<(usize, usize)>;

/// How many groups one run of the clustering takes. A run holds the
/// similarity of every two of its groups (`k × k`, 4 bytes each: 6000 groups
/// are 144 MB), so a big project goes in levels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Levels {
    /// Above this many groups the clustering runs per media first.
    pub single: usize,
    /// At most this many groups go into any run after that: the heaviest
    /// meet across media. A media with more clear tracks than that of its
    /// own (a whole season in one file, a long recording) runs in
    /// consecutive stretches of time of at most this many, which then meet
    /// as episodes do. Up to that, a media runs whole.
    pub second: usize,
}

const LEVELS: Levels = Levels { single: 4000, second: 6000 };

#[cfg(test)]
thread_local! {
    /// The most groups one run of the clustering took on this thread.
    pub(crate) static BIGGEST_RUN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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

fn norm(v: &[f32]) -> f32 {
    dot(v, v).sqrt()
}

/// How a run of the clustering takes faces seen together.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Together {
    /// Two people: their groups never meet.
    Apart,
    /// ... unless the groups are clearly one person seen twice.
    Copies,
}

/// Pinned groups first, then the heaviest; at most `cap` of them (all the
/// pinned ones). Returns the rest.
fn heaviest(groups: &mut Vec<Group>, cap: usize) -> Vec<Group> {
    groups.sort_by(|a, b| b.pin.is_some().cmp(&a.pin.is_some()).then(b.weight.total_cmp(&a.weight)));
    let pinned = groups.iter().filter(|g| g.pin.is_some()).count();
    groups.split_off(groups.len().min(cap.max(pinned)))
}

/// Group the tracks `core` (indices into `tracks`) by identity. Returns the
/// groups (track indices, ascending), largest first; tracks that stepped
/// aside for a merge come last, one per group.
pub fn cluster(tracks: &[TrackIn], core: &[usize], p: &Params) -> Vec<Vec<usize>> {
    cluster_in(tracks, core, p, LEVELS)
}

/// [`cluster`] in runs of at most `levels` groups (but all the pinned ones).
pub(crate) fn cluster_in(tracks: &[TrackIn], core: &[usize], p: &Params, levels: Levels) -> Vec<Vec<usize>> {
    let mut groups = initial_groups(tracks, core);
    let mut aside = Vec::new();
    // 1. Faces seen together are two people.
    if groups.len() > levels.single {
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
            for part in stretches(tracks, by_media.remove(&m).expect("listed"), levels.second) {
                level1.extend(merge(tracks, part, p, Together::Apart, &mut aside));
            }
        }
        // Pinned groups and then the heaviest meet across media (and a long
        // media's stretches). A person seen once per episode is a single
        // track in each: those join too, as far as the cap allows. Faces
        // seen together on either side of a cut are still two people: the
        // run finds them from the groups' tracks.
        let rest = heaviest(&mut level1, levels.second);
        groups = merge(tracks, level1, p, Together::Apart, &mut aside);
        groups.extend(rest);
    } else {
        groups = merge(tracks, groups, p, Together::Apart, &mut aside);
    }
    // 2. ... unless their groups, each whole, are clearly one person. A
    // single face held apart is less like the group than the face it was
    // seen with (or it would have joined first): it would only step aside
    // again, so it is left out, and the run stays small.
    let (mut whole, mut rest): (Vec<Group>, Vec<Group>) =
        groups.into_iter().partition(|g| g.pin.is_some() || g.members.len() >= 2);
    rest.extend(heaviest(&mut whole, levels.second));
    groups = merge(tracks, whole, p, Together::Copies, &mut aside);
    groups.extend(rest);
    let mut out: Vec<Vec<usize>> = groups
        .into_iter()
        .map(|mut g| {
            g.members.sort_unstable();
            g.members
        })
        .collect();
    out.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    aside.sort_unstable();
    out.extend(aside.into_iter().map(|t| vec![t]));
    out
}

/// One media's `groups` (single tracks: pinned ones go elsewhere) as they
/// are when at most `cap`; more, in consecutive stretches of time of at most
/// `cap` groups each, as even as can be.
fn stretches(tracks: &[TrackIn], mut groups: Vec<Group>, cap: usize) -> Vec<Vec<Group>> {
    if groups.len() <= cap {
        return vec![groups];
    }
    groups.sort_by_key(|g| (tracks[g.members[0]].start, g.members[0]));
    let size = groups.len().div_ceil(groups.len().div_ceil(cap.max(1)));
    let mut out = Vec::new();
    while groups.len() > size {
        let later = groups.split_off(size);
        out.push(std::mem::replace(&mut groups, later));
    }
    out.push(groups);
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

/// Groups that never meet: pinned to different characters, or holding a
/// track the user said isn't the other's character.
fn pinned_apart(tracks: &[TrackIn], groups: &[Group]) -> Bits {
    let mut apart = Bits::new(groups.len());
    let pins: Vec<(usize, CharKey)> = groups.iter().enumerate().filter_map(|(i, g)| Some((i, g.pin?))).collect();
    for (x, &(ga, ka)) in pins.iter().enumerate() {
        for &(gb, kb) in &pins[x + 1..] {
            if ka != kb {
                apart.set(ga, gb);
            }
        }
    }
    for (gi, g) in groups.iter().enumerate() {
        for &t in &g.members {
            for &(gp, kp) in &pins {
                if gp != gi && tracks[t].forbid.contains(&kp) {
                    apart.set(gi, gp);
                }
            }
        }
    }
    apart
}

/// Per track of `groups`, the other tracks of theirs seen in the same frames.
fn seen_together(tracks: &[TrackIn], groups: &[Group]) -> HashMap<usize, Vec<usize>> {
    let mut all: Vec<usize> = groups.iter().flat_map(|g| g.members.iter().copied()).collect();
    all.sort_by_key(|&t| (tracks[t].media, tracks[t].start));
    let mut with: HashMap<usize, Vec<usize>> = HashMap::new();
    for a in 0..all.len() {
        let ta = all[a];
        for &tb in &all[a + 1..] {
            if tracks[tb].media != tracks[ta].media || tracks[tb].start >= tracks[ta].end {
                break;
            }
            if together(&tracks[ta], &tracks[tb]) {
                with.entry(ta).or_default().push(tb);
                with.entry(tb).or_default().push(ta);
            }
        }
    }
    with
}

/// One run of the clustering over `groups`.
struct Merging<'a> {
    tracks: &'a [TrackIn],
    p: &'a Params,
    together: Together,
    k: usize,
    groups: Vec<Group>,
    alive: Vec<bool>,
    /// Lengths of the groups' sums (for their mean faces).
    norms: Vec<f32>,
    /// Average linkage, `k × k`.
    sim: Vec<f32>,
    apart: Bits,
    /// Per group, how many pairs of its faces and another group's were seen
    /// together.
    clash: Vec<HashMap<usize, u32>>,
    with: HashMap<usize, Vec<usize>>,
    /// The group each track is in (tracks that stepped aside are in none).
    owner: HashMap<usize, usize>,
}

impl Merging<'_> {
    /// Similarity of the two groups' mean faces.
    fn mean_cos(&self, a: usize, b: usize) -> f32 {
        let k = self.k;
        self.sim[a * k + b] * self.groups[a].weight * self.groups[b].weight / (self.norms[a] * self.norms[b]).max(1e-9)
    }

    /// Whether `a` and `b` may become one group, and which tracks step aside
    /// for it: one of every pair of their faces seen together.
    fn allowed(&self, a: usize, b: usize) -> Option<Vec<usize>> {
        if self.apart.get(a, b) {
            return None;
        }
        let Some(&clashes) = self.clash[a].get(&b).filter(|&&n| n > 0) else { return Some(Vec::new()) };
        if self.together == Together::Apart || self.mean_cos(a, b) < self.p.copy_link {
            return None;
        }
        let (small, big) = if self.groups[a].members.len() <= self.groups[b].members.len() { (a, b) } else { (b, a) };
        let mut pairs: Pairs = Vec::with_capacity(clashes as usize);
        for &i in &self.groups[small].members {
            for &j in self.with.get(&i).into_iter().flatten() {
                if self.owner.get(&j) == Some(&big) {
                    pairs.push((i, j));
                }
            }
        }
        // Every pair seen together could be two people — how alike its two
        // faces are can't tell a mirror image from a twin beside them in
        // the same light — so the faces that have to step aside may only be
        // a few: people often on screen together (twins) stay two. Who the
        // merged group is decides which ones: the face least like it first
        // (a stranger's, or the less clear copy), then the lighter one.
        let merged: Vec<f32> = self.groups[a].sum.iter().zip(&self.groups[b].sum).map(|(x, y)| x + y).collect();
        let limit = ((self.p.copy_share * self.groups[small].members.len() as f32) as usize).max(1);
        self.cover(&pairs, &merged, limit)
    }

    /// Faces that step aside so that no pair of `pairs` is left: the face in
    /// most pairs first, then the one least like `merged`, then the lighter.
    /// `None` when that takes more than `limit`, or a pair could only lose a
    /// pinned face.
    fn cover(&self, pairs: &[(usize, usize)], merged: &[f32], limit: usize) -> Option<Vec<usize>> {
        let mut pairs = pairs.to_vec();
        let mut count: HashMap<usize, (u32, f32, f32)> = HashMap::new();
        for &(i, j) in &pairs {
            for t in [i, j] {
                let tr = &self.tracks[t];
                count.entry(t).or_insert_with(|| (0, -dot(&tr.embedding, merged), -weight(tr))).0 += 1;
            }
        }
        let mut aside = Vec::new();
        while !pairs.is_empty() {
            let pick = count
                .iter()
                .filter(|&(&t, c)| c.0 > 0 && self.tracks[t].pin.is_none())
                .max_by(|(x, kx), (y, ky)| {
                    kx.0.cmp(&ky.0).then(kx.1.total_cmp(&ky.1)).then(kx.2.total_cmp(&ky.2)).then(x.cmp(y))
                })
                .map(|(&t, _)| t)?;
            aside.push(pick);
            if aside.len() > limit {
                return None;
            }
            pairs.retain(|&(i, j)| {
                if i != pick && j != pick {
                    return true;
                }
                let other = if i == pick { j } else { i };
                if let Some(c) = count.get_mut(&other) {
                    c.0 -= 1;
                }
                false
            });
            count.remove(&pick);
        }
        Some(aside)
    }

    /// The closest group `a` may merge with.
    fn best_of(&self, a: usize) -> (f32, usize) {
        let k = self.k;
        let mut best = (f32::NEG_INFINITY, usize::MAX);
        for b in 0..k {
            if b != a && self.alive[b] && self.sim[a * k + b] > best.0 && self.allowed(a, b).is_some() {
                best = (self.sim[a * k + b], b);
            }
        }
        best
    }

    /// Merge `b` into `a`; the tracks `aside` leave.
    fn join(&mut self, a: usize, b: usize, aside: &[usize]) {
        let k = self.k;
        let (wa, wb) = (self.groups[a].weight, self.groups[b].weight);
        for x in 0..k {
            if x != a && x != b && self.alive[x] {
                let v = (wa * self.sim[a * k + x] + wb * self.sim[b * k + x]) / (wa + wb);
                self.sim[a * k + x] = v;
                self.sim[x * k + a] = v;
                if self.apart.get(b, x) {
                    self.apart.set(a, x);
                }
            }
        }
        self.alive[b] = false;
        let gb = std::mem::replace(&mut self.groups[b], Group { members: vec![], sum: vec![], weight: 0.0, pin: None });
        for &t in &gb.members {
            self.owner.insert(t, a);
        }
        let ga = &mut self.groups[a];
        ga.members.extend(gb.members);
        for (x, y) in ga.sum.iter_mut().zip(&gb.sum) {
            *x += y;
        }
        ga.weight += gb.weight;
        ga.pin = ga.pin.or(gb.pin);
        // Pairs with b's faces are now a's.
        for (g, n) in std::mem::take(&mut self.clash[b]) {
            if g == a {
                continue;
            }
            *self.clash[a].entry(g).or_default() += n;
            let back = self.clash[g].remove(&b).unwrap_or(0);
            *self.clash[g].entry(a).or_default() += back;
        }
        self.clash[a].remove(&b);
        if !aside.is_empty() {
            for &t in aside {
                self.owner.remove(&t);
                let w = weight(&self.tracks[t]);
                let ga = &mut self.groups[a];
                ga.members.retain(|&m| m != t);
                for (s, x) in ga.sum.iter_mut().zip(&self.tracks[t].embedding) {
                    *s -= w * x;
                }
                ga.weight -= w;
                for &j in self.with.get(&t).into_iter().flatten() {
                    let Some(&g) = self.owner.get(&j) else { continue };
                    if g == a {
                        continue;
                    }
                    for (from, to) in [(a, g), (g, a)] {
                        if let Some(n) = self.clash[from].get_mut(&to) {
                            *n -= 1;
                            if *n == 0 {
                                self.clash[from].remove(&to);
                            }
                        }
                    }
                }
            }
            // The group lost members: its row from the sums.
            for x in 0..k {
                if x != a && self.alive[x] {
                    let v =
                        dot(&self.groups[a].sum, &self.groups[x].sum) / (self.groups[a].weight * self.groups[x].weight);
                    self.sim[a * k + x] = v;
                    self.sim[x * k + a] = v;
                }
            }
        }
        self.norms[a] = norm(&self.groups[a].sum);
        #[cfg(test)]
        self.check(a);
    }

    /// The bookkeeping after `a` took another group, recounted from scratch:
    /// pairs seen together between groups, `a`'s members, sums and row of
    /// similarities — and no two faces of one group on screen at once (but
    /// pinned ones: the user's word).
    #[cfg(test)]
    fn check(&self, a: usize) {
        let mut clash: Vec<HashMap<usize, u32>> = vec![HashMap::new(); self.k];
        for (t, others) in &self.with {
            let Some(&gt) = self.owner.get(t) else { continue };
            for o in others {
                if let Some(&go) = self.owner.get(o)
                    && go != gt
                {
                    *clash[gt].entry(go).or_default() += 1;
                }
            }
        }
        for g in (0..self.k).filter(|&g| self.alive[g]) {
            assert_eq!(self.clash[g], clash[g], "pairs seen together of group {g}");
        }
        let ga = &self.groups[a];
        let (mut w, mut sum) = (0f32, vec![0f32; ga.sum.len()]);
        for &t in &ga.members {
            assert_eq!(self.owner.get(&t), Some(&a), "owner of track {t}");
            let wt = weight(&self.tracks[t]);
            w += wt;
            sum.iter_mut().zip(&self.tracks[t].embedding).for_each(|(s, x)| *s += wt * x);
            for o in self.with.get(&t).into_iter().flatten() {
                if self.owner.get(o) == Some(&a) {
                    assert!(self.tracks[t].pin.is_some() && self.tracks[*o].pin.is_some(), "{t} and {o} at once");
                }
            }
        }
        let close = |x: f32, y: f32| (x - y).abs() <= 1e-3 * (1.0 + w);
        assert!(close(w, ga.weight), "weight of group {a}");
        assert!(sum.iter().zip(&ga.sum).all(|(&x, &y)| close(x, y)), "sum of group {a}");
        for x in (0..self.k).filter(|&x| x != a && self.alive[x]) {
            let v = dot(&ga.sum, &self.groups[x].sum) / (ga.weight * self.groups[x].weight);
            assert!((self.sim[a * self.k + x] - v).abs() < 1e-3, "similarity of {a} and {x}");
        }
    }
}

fn merge(tracks: &[TrackIn], groups: Vec<Group>, p: &Params, together: Together, aside: &mut Vec<usize>) -> Vec<Group> {
    let k = groups.len();
    #[cfg(test)]
    BIGGEST_RUN.with(|b| b.set(b.get().max(k)));
    if k < 2 {
        return groups;
    }
    let mut sim = vec![0f32; k * k];
    for a in 0..k {
        for b in a + 1..k {
            let s = dot(&groups[a].sum, &groups[b].sum) / (groups[a].weight * groups[b].weight);
            sim[a * k + b] = s;
            sim[b * k + a] = s;
        }
    }
    let owner: HashMap<usize, usize> =
        groups.iter().enumerate().flat_map(|(g, gr)| gr.members.iter().map(move |&t| (t, g))).collect();
    let with = seen_together(tracks, &groups);
    let mut clash: Vec<HashMap<usize, u32>> = vec![HashMap::new(); k];
    for (t, others) in &with {
        for o in others {
            let (ga, gb) = (owner[t], owner[o]);
            if ga != gb {
                *clash[ga].entry(gb).or_default() += 1;
            }
        }
    }
    let mut m = Merging {
        tracks,
        p,
        together,
        k,
        norms: groups.iter().map(|g| norm(&g.sum)).collect(),
        apart: pinned_apart(tracks, &groups),
        groups,
        alive: vec![true; k],
        sim,
        clash,
        with,
        owner,
    };
    let mut best: Vec<(f32, usize)> = (0..k).map(|a| m.best_of(a)).collect();
    // The globally closest allowed pair, while it is close enough.
    while let Some((a, &(s, b))) =
        best.iter().enumerate().filter(|&(i, _)| m.alive[i]).max_by(|x, y| x.1.0.total_cmp(&y.1.0).then(y.0.cmp(&x.0)))
    {
        if s < p.link || b == usize::MAX {
            break;
        }
        let Some(out) = m.allowed(a, b) else {
            best[a] = m.best_of(a);
            continue;
        };
        m.join(a, b, &out);
        aside.extend(&out);
        best[a] = m.best_of(a);
        for x in 0..k {
            if !m.alive[x] || x == a {
                continue;
            }
            if best[x].1 == a || best[x].1 == b {
                best[x] = m.best_of(x);
            } else if m.sim[x * k + a] > best[x].0 && m.allowed(x, a).is_some() {
                best[x] = (m.sim[x * k + a], a);
            }
        }
    }
    m.groups.into_iter().zip(m.alive).filter(|(_, a)| *a).map(|(g, _)| g).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: u32) -> TrackIn {
        TrackIn {
            media: 0,
            start,
            end: start + 10,
            spans: vec![],
            seen: vec![],
            embedding: vec![1.0],
            quality: 1.0,
            dets: 2,
            pin: None,
            forbid: vec![],
            previous: None,
        }
    }

    /// The first track of each group, per stretch.
    fn firsts(parts: &[Vec<Group>]) -> Vec<Vec<usize>> {
        parts.iter().map(|p| p.iter().map(|g| g.members[0]).collect()).collect()
    }

    #[test]
    fn a_media_that_fits_in_one_run_goes_in_as_it_is() {
        // Not in time order: the run takes them in the order they came.
        let tracks: Vec<TrackIn> = [50, 10, 30, 20].map(at).into();
        for cap in [4, 5, usize::MAX] {
            assert_eq!(firsts(&stretches(&tracks, initial_groups(&tracks, &[0, 1, 2, 3]), cap)), [[0, 1, 2, 3]]);
        }
    }

    #[test]
    fn a_longer_media_runs_in_even_stretches_of_time() {
        // 11 tracks, at most 4 a run: stretches of 4, 4 and 3, in time order.
        let tracks: Vec<TrackIn> = (0..11u32).map(|i| at(i * 7 % 11 * 100)).collect();
        let core: Vec<usize> = (0..11).collect();
        let starts: Vec<Vec<u32>> = stretches(&tracks, initial_groups(&tracks, &core), 4)
            .iter()
            .map(|p| p.iter().map(|g| tracks[g.members[0]].start).collect())
            .collect();
        assert_eq!(starts, [vec![0, 100, 200, 300], vec![400, 500, 600, 700], vec![800, 900, 1000]]);
    }
}
