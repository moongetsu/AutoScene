//! The face space fitted to the footage (animation).
//!
//! ArcFace learned real faces. To it the characters of one animated film are
//! first of all faces in one style: two of them look as alike as one actor
//! in two scenes (see [`strangers`](crate::strangers)), and no similarity
//! bar tells them apart. Yet the faces still differ in some directions —
//! hair, eyes, glasses — just not in the ones ArcFace weighs most. The
//! footage shows which: faces grouped at a strict bar are one person, and
//! groups with faces seen in the same frame are two. From those, a linear
//! map (learned whitening) is fitted that shrinks how faces of one person
//! vary and spreads how different people differ, and the faces are compared
//! in a mix of that space and ArcFace's own. The bars come from how alike
//! strangers still look in it (a high quantile: the mixed space's tail of
//! strangers is what merges people).
//!
//! It applies as far as the footage looks like animation: not at all where
//! strangers look as different as actors do (the space is ArcFace's, the
//! bars are today's), fully from a median of 0.15 on, or whenever the user
//! said the footage is animation. A grouping keeps its fit ([`Space`]): the
//! decisions after it place faces in the same space, at once, and a face
//! removed from the cast doesn't move the others.

use std::collections::{HashMap, HashSet};

use faer::{Mat, Side};

use crate::{Params, TrackIn, cluster, together};

/// The fit needs at least this many pairs of clear faces seen at once: a
/// tail quantile of fewer is too noisy (it set bars that merged people, or
/// ones no face reached).
pub const MIN_STRANGERS: usize = 200;
/// Seed groups: faces at least as alike as all but 1 % of strangers.
const SEED_QUANTILE: f64 = 0.99;
/// Ridge added to how faces of one person vary, as a share of its mean
/// eigenvalue.
const RIDGE: f64 = 0.05;
/// Dimensions of the fitted space (fewer when the footage shows fewer
/// independent differences).
const DIMS: usize = 64;
/// Share of the fitted space in the mixed similarity, fully animated.
const MIX: f32 = 0.35;
/// The link bar is this quantile of strangers in the mixed space.
const TAIL: f64 = 0.95;
/// The copy bars move this much further than the link bar: in this film the
/// mean faces of two different people's groups reach 0.78.
const COPY_REACH: f32 = 2.5;
/// How far a bar moves at most (as in [`Params::for_strangers`]).
const SHIFT_CAP: f32 = 0.5;

/// How a grouping compared faces; kept with the cast so the decisions after
/// it compare them the same way.
#[derive(Clone, Debug, PartialEq)]
pub enum Space {
    /// ArcFace's own space, bars fitted by [`Params::fitted`].
    Faces,
    /// The space fitted to the footage.
    Fitted(Fit),
}

/// A face space fitted to the footage (see the module).
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    /// Mean face of the footage.
    pub mean: Vec<f32>,
    /// `mean.len() × k` projection into the fitted space, row-major.
    pub projection: Vec<f32>,
    pub k: usize,
    /// Share of the fitted space in the mixed similarity.
    pub weight: f32,
    /// The bars in the mixed space.
    pub link: f32,
    pub copy_link: f32,
    pub copy_alike: f32,
}

/// What a fit was made from (for logs and the bench).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FitStats {
    pub strangers: usize,
    /// Median similarity of strangers (as [`strangers`](crate::strangers)).
    pub alike: f32,
    /// How far the footage looks like animation, 0 – 1.
    pub share: f32,
    pub seed_bar: f32,
    pub seed_tracks: usize,
    pub seed_groups: usize,
    pub same_pairs: f64,
    pub other_pairs: f64,
    pub tail: f32,
}

impl Fit {
    /// A face in the mixed space: ArcFace's and the fitted one side by side
    /// (`√(1−w)·e ; √w·a`), unit length. Unreadable faces become the zero
    /// vector: like no one.
    pub fn embed(&self, e: &[f32]) -> Vec<f32> {
        let dim = self.mean.len();
        let mut z = vec![0f32; dim + self.k];
        if e.len() != dim || !e.iter().all(|x| x.is_finite()) {
            return z;
        }
        let mut a = vec![0f64; self.k];
        for (j, (&x, &m)) in e.iter().zip(&self.mean).enumerate() {
            let x = (x - m) as f64;
            for (c, v) in a.iter_mut().enumerate() {
                *v += x * self.projection[j * self.k + c] as f64;
            }
        }
        let na = a.iter().map(|v| v * v).sum::<f64>().sqrt();
        let (sr, sa) = ((1.0 - self.weight).sqrt(), self.weight.sqrt());
        for (zj, &x) in z.iter_mut().zip(e) {
            *zj = sr * x;
        }
        if na > 0.0 && na.is_finite() {
            for (zj, v) in z[dim..].iter_mut().zip(&a) {
                *zj = sa * (v / na) as f32;
            }
        }
        let n = z.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n > 0.0 {
            z.iter_mut().for_each(|x| *x /= n);
        }
        z
    }

    /// `tracks` with their faces in the mixed space.
    pub fn tracks(&self, tracks: &[TrackIn]) -> Vec<TrackIn> {
        tracks.iter().map(|t| TrackIn { embedding: self.embed(&t.embedding), ..t.clone() }).collect()
    }

    /// `p` with the bars of the mixed space (the others as they are). The
    /// unsure bar is the link bar, above `assign`: a close call under it is
    /// as likely a stranger, and suggesting it was wrong more often than
    /// right (measured on the Miraculous film).
    pub fn params(&self, p: &Params) -> Params {
        Params { link: self.link, unsure: self.link, copy_link: self.copy_link, copy_alike: self.copy_alike, ..*p }
    }
}

const MAGIC: &[u8; 8] = b"ASFIT\x00\x00\x01";
const FACES: &[u8; 8] = b"ASFACES1";

impl Space {
    pub fn to_bytes(&self) -> Vec<u8> {
        let Space::Fitted(f) = self else { return FACES.to_vec() };
        let mut b = Vec::with_capacity(32 + 4 * (f.mean.len() + f.projection.len()));
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&(f.mean.len() as u32).to_le_bytes());
        b.extend_from_slice(&(f.k as u32).to_le_bytes());
        for x in [f.weight, f.link, f.copy_link, f.copy_alike].iter().chain(&f.mean).chain(&f.projection) {
            b.extend_from_slice(&x.to_le_bytes());
        }
        b
    }

    /// `None`: not a stored space (another version, damaged).
    pub fn from_bytes(b: &[u8]) -> Option<Space> {
        if b == FACES {
            return Some(Space::Faces);
        }
        let rest = b.strip_prefix(MAGIC.as_slice())?;
        let word = |i: usize| rest.get(4 * i..4 * i + 4).map(|w| u32::from_le_bytes(w.try_into().unwrap()));
        let (dim, k) = (word(0)? as usize, word(1)? as usize);
        let bytes = dim.checked_mul(k)?.checked_add(4 + dim)?.checked_mul(4)?.checked_add(8)?;
        if dim == 0 || k == 0 || k > dim || rest.len() != bytes {
            return None;
        }
        let floats: Vec<f32> = rest[8..].as_chunks::<4>().0.iter().map(|w| f32::from_le_bytes(*w)).collect();
        if !floats.iter().all(|x| x.is_finite()) || !(floats[0] > 0.0 && floats[0] <= 1.0) {
            return None;
        }
        Some(Space::Fitted(Fit {
            weight: floats[0],
            link: floats[1],
            copy_link: floats[2],
            copy_alike: floats[3],
            mean: floats[4..4 + dim].to_vec(),
            projection: floats[4 + dim..].to_vec(),
            k,
        }))
    }
}

fn finite(e: &[f32], dim: usize) -> bool {
    e.len() == dim && e.iter().all(|x| x.is_finite())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// How faces of one person vary (`C_S`) and how different people differ
/// (`C_D`): the mean of `(xᵢ − xⱼ)(xᵢ − xⱼ)ᵀ` over the pairs of faces in one
/// group, and over the pairs of faces of two groups seen together — in
/// closed form, every pair counted. `x`: centred faces (rows); `group`: each
/// row's group; `adjacent`: per group, the groups seen with it (each once).
/// Also returns the number of pairs of each kind.
pub(crate) fn scatters(x: &Mat<f64>, group: &[usize], adjacent: &[Vec<usize>]) -> (Mat<f64>, Mat<f64>, f64, f64) {
    let (n, dim, ng) = (x.nrows(), x.ncols(), adjacent.len());
    let mut size = vec![0f64; ng];
    group.iter().for_each(|&g| size[g] += 1.0);
    let mut sums = Mat::<f64>::zeros(ng, dim);
    for r in 0..n {
        for c in 0..dim {
            sums[(group[r], c)] += x[(r, c)];
        }
    }
    // Σ over pairs in G of (xᵢ−xⱼ)(xᵢ−xⱼ)ᵀ = n_G Σ xᵢxᵢᵀ − s_G s_Gᵀ; across
    // groups A, B seen together, n_B Σ_A xᵢxᵢᵀ + n_A Σ_B xⱼxⱼᵀ − s_A s_Bᵀ − s_B s_Aᵀ.
    let same: f64 = size.iter().map(|s| s * (s - 1.0) / 2.0).sum();
    let reach: Vec<f64> = (0..ng).map(|g| adjacent[g].iter().map(|&h| size[h]).sum()).collect();
    let other: f64 = (0..ng).map(|g| size[g] * reach[g]).sum::<f64>() / 2.0;
    let weighted = |w: &[f64]| Mat::<f64>::from_fn(n, dim, |r, c| w[group[r]] * x[(r, c)]);
    let near = Mat::<f64>::from_fn(ng, dim, |g, c| adjacent[g].iter().map(|&h| sums[(h, c)]).sum());
    let mut cs = x.transpose() * weighted(&size) - sums.transpose() * &sums;
    let mut cd = x.transpose() * weighted(&reach) - sums.transpose() * &near;
    for r in 0..dim {
        for c in 0..dim {
            cs[(r, c)] /= same.max(1.0);
            cd[(r, c)] /= other.max(1.0);
        }
    }
    (cs, cd, same, other)
}

/// Learned whitening: `W = C_S^-½` (with a ridge), then the directions in
/// which different people differ most relative to how one person varies —
/// the top eigenvectors of `W C_D W`, largest first, at most [`DIMS`] and
/// only those the data show (a small crowd scene has fewer; the rest would
/// be an arbitrary basis of nothing). Returns the `dim × k` projection.
pub(crate) fn whitening(mut cs: Mat<f64>, cd: &Mat<f64>) -> Option<Mat<f64>> {
    let dim = cs.nrows();
    let trace: f64 = (0..dim).map(|i| cs[(i, i)]).sum();
    // (No number, or no spread: nothing to fit.)
    if trace.is_nan() || trace <= 0.0 {
        return None;
    }
    // Its share of the mean spread of the dimensions faces vary in (an
    // embedder smaller than the tracks' format leaves the rest at zero).
    let live = (0..dim).filter(|&i| cs[(i, i)] > 0.0).count().max(1);
    for i in 0..dim {
        cs[(i, i)] += RIDGE * trace / live as f64;
    }
    let es = cs.self_adjoint_eigen(Side::Lower).ok()?;
    let (u, l) = (es.U(), es.S().column_vector());
    if (0..dim).any(|i| l[i].is_nan() || l[i] <= 0.0) {
        return None;
    }
    let scaled = Mat::<f64>::from_fn(dim, dim, |r, c| u[(r, c)] / l[c].sqrt());
    let whiten = &scaled * u.transpose();
    let m = &whiten * cd * &whiten;
    let m = Mat::<f64>::from_fn(dim, dim, |r, c| (m[(r, c)] + m[(c, r)]) / 2.0);
    let ed = m.self_adjoint_eigen(Side::Lower).ok()?;
    let (v, lam) = (ed.U(), ed.S().column_vector());
    let top = lam[dim - 1];
    if top.is_nan() || top <= 0.0 {
        return None;
    }
    let k = (0..dim).filter(|&i| lam[i] > 1e-9 * top).count().min(DIMS);
    let mut proj = &whiten * Mat::<f64>::from_fn(dim, k, |r, c| v[(r, dim - 1 - c)]);
    for c in 0..k {
        // One sign per direction (it doesn't change any similarity).
        let big = (0..dim).max_by(|&a, &b| proj[(a, c)].abs().total_cmp(&proj[(b, c)].abs())).unwrap_or(0);
        if proj[(big, c)] < 0.0 {
            (0..dim).for_each(|r| proj[(r, c)] = -proj[(r, c)]);
        }
    }
    Some(proj)
}

/// Pairs of readable clear tracks whose faces were found in the same frame
/// (as [`strangers`](crate::strangers) counts them), in a fixed order.
fn stranger_pairs(tracks: &[TrackIn], ok: &[bool], p: &Params) -> Vec<(usize, usize)> {
    let clear = |i: usize| ok[i] && tracks[i].quality >= p.core_quality && tracks[i].dets >= p.core_dets;
    let mut at: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for i in (0..tracks.len()).filter(|&i| clear(i)) {
        for &f in &tracks[i].seen {
            at.entry((tracks[i].media, f)).or_default().push(i);
        }
    }
    let mut pairs: HashSet<(usize, usize)> = HashSet::new();
    for v in at.values() {
        for (k, &a) in v.iter().enumerate() {
            pairs.extend(v[k + 1..].iter().map(|&b| (a.min(b), a.max(b))));
        }
    }
    let mut pairs: Vec<(usize, usize)> = pairs.into_iter().collect();
    pairs.sort_unstable();
    pairs
}

/// Linear-interpolated quantile of ascending `s`.
#[cfg(test)]
pub(crate) fn quantile(s: &[f64], q: f64) -> f64 {
    let h = q * (s.len() - 1) as f64;
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(s.len() - 1);
    s[lo] + (h - lo as f64) * (s[hi] - s[lo])
}

/// `quantile` of `(value, weight)` pairs, ascending by value: each
/// value stands at the middle of its share of the total weight (equal
/// weights: at `(i + 0.5)/n`), linearly interpolated between them.
pub(crate) fn weighted_quantile(s: &[(f64, f64)], q: f64) -> f64 {
    debug_assert!(!s.is_empty());
    let total: f64 = s.iter().map(|&(_, w)| w).sum();
    if total <= 0.0 {
        // Nothing counts: the plain median's place.
        return s[s.len() / 2].0;
    }
    let mut at = 0.0;
    let mut mids: Vec<f64> = Vec::with_capacity(s.len());
    for &(_, w) in s {
        mids.push((at + w / 2.0) / total);
        at += w;
    }
    if q <= mids[0] {
        return s[0].0;
    }
    for (i, &m) in mids.iter().enumerate().skip(1) {
        if q <= m {
            let span = m - mids[i - 1];
            // Equal middles (a weight of nothing between them): the later's.
            if span <= 0.0 {
                return s[i].0;
            }
            return s[i - 1].0 + (q - mids[i - 1]) / span * (s[i].0 - s[i - 1].0);
        }
    }
    s[s.len() - 1].0
}

/// Pairs of `members` (indices into `tracks`) whose faces were seen at once
/// ([`together`]), as positions in `members`; one sweep per media.
fn seen_together(tracks: &[TrackIn], members: &[usize]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..members.len()).collect();
    order.sort_by_key(|&k| (tracks[members[k]].media, tracks[members[k]].start, k));
    let mut out = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    for &k in &order {
        let t = &tracks[members[k]];
        open.retain(|&o| tracks[members[o]].media == t.media && tracks[members[o]].end > t.start);
        for &o in &open {
            if together(&tracks[members[o]], t) {
                out.push((o.min(k), o.max(k)));
            }
        }
        open.push(k);
    }
    out.sort_unstable();
    out
}

/// Fit the face space to `tracks` (see the module). `None`: ArcFace's own
/// space suits them — they don't look like animation, or too few faces
/// were seen at once to tell how.
pub fn fit(tracks: &[TrackIn], p: &Params) -> Option<(Fit, FitStats)> {
    let dim = crate::session_dim(tracks)?;
    let ok: Vec<bool> = tracks.iter().map(|t| finite(&t.embedding, dim)).collect();
    let mut stats = FitStats::default();

    // How alike strangers look; whether and how far to fit.
    let pairs = stranger_pairs(tracks, &ok, p);
    if pairs.len() < MIN_STRANGERS.max(p.calibrate_from) {
        return None;
    }
    // A pair counts for its less-seen face ([`TrackIn::weight`]).
    let mut sorted: Vec<(f64, f64)> = pairs
        .iter()
        .map(|&(a, b)| (dot(&tracks[a].embedding, &tracks[b].embedding) as f64, crate::pair_weight(tracks, a, b)))
        .filter(|&(_, w)| w > 0.0)
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let alike = weighted_quantile(&sorted, 0.5) as f32;
    let share = p.style_share(alike);
    stats.strangers = pairs.len();
    stats.alike = alike;
    stats.share = share;
    if share <= 0.0 {
        return None;
    }

    // Seeds: groups of faces clearly one person, at a strict bar, every
    // face alike (the user's decisions don't shape what faces look like).
    let seed_bar = weighted_quantile(&sorted, SEED_QUANTILE) as f32;
    let seeds_of: Vec<usize> = (0..tracks.len()).filter(|&i| ok[i] && tracks[i].quality >= p.core_quality).collect();
    let plain: Vec<TrackIn> = tracks
        .iter()
        .map(|t| TrackIn { quality: 1.0, pin: None, forbid: Vec::new(), previous: None, ..t.clone() })
        .collect();
    // A copy bar above any similarity: no copies phase.
    let groups = cluster(&plain, &seeds_of, &Params { link: seed_bar, copy_link: 2.0, ..*p });
    let pos: HashMap<usize, usize> = seeds_of.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let mut group = vec![usize::MAX; seeds_of.len()];
    for (gi, g) in groups.iter().enumerate() {
        g.iter().for_each(|i| group[pos[i]] = gi);
    }
    // (Every face is in a group; one that weren't is a group of its own.)
    let mut ng = groups.len();
    for g in group.iter_mut().filter(|g| **g == usize::MAX) {
        *g = ng;
        ng += 1;
    }
    stats.seed_bar = seed_bar;
    stats.seed_tracks = seeds_of.len();
    stats.seed_groups = ng;

    // Groups seen together are two people.
    let mut adjacent: Vec<Vec<usize>> = vec![Vec::new(); ng];
    let mut seen_pair: HashSet<(usize, usize)> = HashSet::new();
    for (a, b) in seen_together(tracks, &seeds_of) {
        let (ga, gb) = (group[a], group[b]);
        if ga != gb && seen_pair.insert((ga.min(gb), ga.max(gb))) {
            adjacent[ga].push(gb);
            adjacent[gb].push(ga);
        }
    }

    let n = seeds_of.len();
    let mut mean = vec![0f64; dim];
    for &i in &seeds_of {
        for (m, &x) in mean.iter_mut().zip(&tracks[i].embedding) {
            *m += x as f64;
        }
    }
    mean.iter_mut().for_each(|m| *m /= n as f64);
    let x = Mat::<f64>::from_fn(n, dim, |r, c| tracks[seeds_of[r]].embedding[c] as f64 - mean[c]);
    let (cs, cd, same_pairs, other_pairs) = scatters(&x, &group, &adjacent);
    stats.same_pairs = same_pairs;
    stats.other_pairs = other_pairs;
    if same_pairs <= 0.0 || other_pairs <= 0.0 {
        return None;
    }
    let proj = whitening(cs, &cd)?;
    let k = proj.ncols();
    let mut fit = Fit {
        mean: mean.iter().map(|&m| m as f32).collect(),
        projection: (0..dim * k).map(|j| proj[(j / k, j % k)] as f32).collect(),
        k,
        weight: (MIX * share).min(p.cl_cap),
        link: p.link,
        copy_link: p.copy_link,
        copy_alike: p.copy_alike,
    };

    // The bars: as far as strangers still look alike in the mixed space.
    let z: HashMap<usize, Vec<f32>> = pairs
        .iter()
        .flat_map(|&(a, b)| [a, b])
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|i| (i, fit.embed(&tracks[i].embedding)))
        .collect();
    let mut mixed: Vec<(f64, f64)> = pairs
        .iter()
        .map(|&(a, b)| (dot(&z[&a], &z[&b]) as f64, crate::pair_weight(tracks, a, b)))
        .filter(|&(_, w)| w > 0.0)
        .collect();
    mixed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let tail = weighted_quantile(&mixed, TAIL) as f32;
    let o = (share * ((tail - p.link) / (1.0 - p.link)).max(0.0)).clamp(0.0, SHIFT_CAP);
    let oc = (COPY_REACH * o).min(SHIFT_CAP);
    let up = |t: f32, o: f32| t + o * (1.0 - t);
    fit.link = up(p.link, o);
    fit.copy_link = up(p.copy_link, oc);
    fit.copy_alike = up(p.copy_alike, oc);
    stats.tail = tail;
    Some((fit, stats))
}
