//! Who is who: face tracks → characters.
//!
//! 1. **Core tracks** — clear enough to define a character (quality, length)
//!    — are grouped by constrained average-linkage clustering. Two tracks on
//!    screen at the same time are two people (cannot-link); tracks the user
//!    pinned to one character start out together (must-link), pinned to
//!    different characters never meet.
//! 2. Groups with enough screen time (or a pin) become characters. Each gets
//!    a handful of prototypes covering its looks (glasses, lighting, age).
//! 3. Every other track joins the character it clearly resembles; a close
//!    call between two goes to *unsure* with a suggestion, the rest stays
//!    unassigned (extras, background faces).
//! 4. Characters keep their identity across reclusters: a group inherits the
//!    character most of its tracks belonged to before, so names and edits
//!    stay put when episodes are added.
//!
//! Pure computation; the project layer loads tracks and stores the result.

mod ahc;
mod proto;

use std::collections::{HashMap, HashSet};

pub use ahc::cluster;

/// A character as the caller knows it (index into its own list).
pub type CharKey = u32;

#[derive(Clone, Debug)]
pub struct TrackIn {
    /// Media the track belongs to: tracks of one media on screen together can't be one person.
    pub media: u32,
    pub start: u32,
    pub end: u32,
    /// Frames the face is actually on screen (sorted, within `start..end`);
    /// empty means all of `start..end`. A gap is a face that was gone for a
    /// while (see `as_analysis::tracks::presence_spans`).
    pub spans: Vec<(u32, u32)>,
    /// Unit-length identity embedding.
    pub embedding: Vec<f32>,
    pub quality: f32,
    pub dets: u32,
    /// The user put this track on this character.
    pub pin: Option<CharKey>,
    /// The user said this track is not these characters.
    pub forbid: Vec<CharKey>,
    /// Character before this run (auto or pinned), for stable identities.
    pub previous: Option<CharKey>,
}

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Average similarity at which two groups merge.
    pub link: f32,
    /// Tracks at or above this quality with at least `core_dets` detections define characters.
    pub core_quality: f32,
    pub core_dets: u32,
    /// A group becomes a character with this many tracks and frames on screen.
    pub min_tracks: usize,
    pub min_frames: u32,
    /// ... and when its tracks agree this much on average: groups of blurry,
    /// dark or masked faces that only resemble "a face" fall below.
    pub min_cohesion: f32,
    /// Other tracks join a character at this similarity, if the runner-up is `margin` behind.
    pub assign: f32,
    pub margin: f32,
    /// Below `assign` but at least this: unsure, with a suggestion.
    pub unsure: f32,
    /// Prototypes per character.
    pub prototypes: usize,
    /// A group inherits a previous character when that many of its tracks
    /// (quality-weighted share) were that character.
    pub inherit: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            link: 0.36,
            core_quality: 0.45,
            core_dets: 2,
            min_tracks: 3,
            min_frames: 96,
            min_cohesion: 0.5,
            assign: 0.5,
            margin: 0.08,
            unsure: 0.35,
            prototypes: 12,
            inherit: 0.3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Assign {
    None,
    /// Clustered or clearly resembling the character.
    Auto(usize),
    /// Resembles the character, but not clearly enough.
    Unsure(usize),
    /// The user's decision.
    Pinned(usize),
}

impl Assign {
    pub fn character(self) -> Option<usize> {
        match self {
            Self::Auto(c) | Self::Pinned(c) => Some(c),
            Self::None | Self::Unsure(_) => None,
        }
    }
}

/// A character in the result: an existing one (`key`) or a new one.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterOut {
    pub key: Option<CharKey>,
    /// Quality-weighted mean embedding of its tracks.
    pub centroid: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub characters: Vec<CharacterOut>,
    /// Per input track: assignment (index into `characters`) and the
    /// similarity behind it (1 for pins and clustered tracks).
    pub tracks: Vec<(Assign, f32)>,
}

impl TrackIn {
    /// Frames the face is on screen.
    fn presence(&self) -> Vec<(u32, u32)> {
        if self.spans.is_empty() { vec![(self.start, self.end)] } else { self.spans.clone() }
    }
}

/// Whether two lists of frame ranges share a frame.
fn overlaps(a: &[(u32, u32)], b: &[(u32, u32)]) -> bool {
    a.iter().any(|&(s, e)| b.iter().any(|&(s2, e2)| s < e2 && s2 < e))
}

/// Two tracks of one media whose faces are on screen at the same time: two
/// people.
pub(crate) fn together(a: &TrackIn, b: &TrackIn) -> bool {
    a.media == b.media && a.start < b.end && b.start < a.end && overlaps(&a.presence(), &b.presence())
}

/// Quality-weighted mean similarity between the different tracks of a group:
/// `(|S|² − Σw²) / (W² − Σw²)` with `S = Σ wᵢeᵢ`, `W = Σ wᵢ` (unit `eᵢ`).
fn cohesion(tracks: &[TrackIn], g: &[usize]) -> f32 {
    if g.len() < 2 {
        return 1.0;
    }
    let dim = tracks[g[0]].embedding.len();
    let mut sum = vec![0f32; dim];
    let (mut w, mut w2) = (0f32, 0f32);
    for &i in g {
        let q = tracks[i].quality.max(0.05);
        for (s, x) in sum.iter_mut().zip(&tracks[i].embedding) {
            *s += q * x;
        }
        w += q;
        w2 += q * q;
    }
    let s2: f32 = sum.iter().map(|x| x * x).sum();
    (s2 - w2) / (w * w - w2).max(1e-9)
}

/// Group `tracks` into characters.
pub fn identify(tracks: &[TrackIn], p: &Params) -> Outcome {
    let n = tracks.len();
    // A track the user said "not them" about doesn't shape any group: it
    // would drag the rest of its person away from that character.
    let is_core: Vec<bool> = tracks
        .iter()
        .map(|t| t.pin.is_some() || (t.forbid.is_empty() && t.quality >= p.core_quality && t.dets >= p.core_dets))
        .collect();
    let core: Vec<usize> = (0..n).filter(|&i| is_core[i]).collect();

    // 1. Constrained clustering of the core tracks.
    let groups = cluster(tracks, &core, p.link);

    // 2. Which groups are characters; which existing character each one is.
    let frames = |g: &[usize]| -> u32 { g.iter().map(|&i| tracks[i].end - tracks[i].start).sum() };
    let pinned_key = |g: &[usize]| g.iter().find_map(|&i| tracks[i].pin);
    let mut chars: Vec<(Vec<usize>, Option<CharKey>)> = Vec::new();
    let mut loose: Vec<usize> = (0..n).filter(|&i| !is_core[i]).collect();
    for g in groups {
        let qualifies = g.len() >= p.min_tracks && frames(&g) >= p.min_frames && cohesion(tracks, &g) >= p.min_cohesion;
        if pinned_key(&g).is_some() || qualifies {
            chars.push((g, None));
        } else {
            loose.extend(g);
        }
    }
    // Pins decide first; then the biggest previous overlaps, one to one.
    let mut taken: HashSet<CharKey> = HashSet::new();
    for (g, key) in &mut chars {
        if let Some(k) = pinned_key(g) {
            *key = Some(k);
            taken.insert(k);
        }
    }
    // How much of each earlier character's clear tracks this run has.
    let mut before: HashMap<CharKey, f32> = HashMap::new();
    for &i in &core {
        if let Some(k) = tracks[i].previous {
            *before.entry(k).or_default() += tracks[i].quality;
        }
    }
    let mut inherits: Vec<(f32, usize, CharKey)> = Vec::new();
    for (ci, (g, key)) in chars.iter().enumerate() {
        if key.is_some() {
            continue;
        }
        let total: f32 = g.iter().map(|&i| tracks[i].quality).sum::<f32>().max(1e-6);
        let mut by: HashMap<CharKey, f32> = HashMap::new();
        for &i in g {
            if let Some(k) = tracks[i].previous {
                *by.entry(k).or_default() += tracks[i].quality;
            }
        }
        for (k, w) in by {
            // Enough of the group was that character — or most of that
            // character went into the group (it grew with new episodes).
            if w / total >= p.inherit || w / before[&k].max(1e-6) > 0.5 {
                inherits.push((w, ci, k));
            }
        }
    }
    inherits.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (_, ci, k) in inherits {
        if chars[ci].1.is_none() && !taken.contains(&k) {
            chars[ci].1 = Some(k);
            taken.insert(k);
        }
    }

    // 3. Members, prototypes, and everyone else.
    let mut out = vec![(Assign::None, 0f32); n];
    let mut characters = Vec::with_capacity(chars.len());
    let mut protos: Vec<Vec<Vec<f32>>> = Vec::with_capacity(chars.len());
    // Per character and media, the members' time spans (cannot-link for the rest).
    let mut spans: Vec<HashMap<u32, Vec<(u32, u32)>>> = Vec::with_capacity(chars.len());
    for (ci, (g, key)) in chars.iter().enumerate() {
        let mut sp: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for &i in g {
            out[i] = (if tracks[i].pin.is_some() { Assign::Pinned(ci) } else { Assign::Auto(ci) }, 1.0);
            sp.entry(tracks[i].media).or_default().extend(tracks[i].presence());
        }
        let members: Vec<(&[f32], f32)> =
            g.iter().map(|&i| (tracks[i].embedding.as_slice(), tracks[i].quality.max(0.05))).collect();
        characters.push(CharacterOut { key: *key, centroid: proto::centroid(&members) });
        protos.push(proto::prototypes(&members, p.prototypes));
        spans.push(sp);
    }
    // Every other track, most similar first: a character already on screen
    // in a frame (a member, or a closer look-alike placed before) can't take
    // a second face there. A forbidden character never gets a track.
    let mut loose: Vec<(usize, Vec<(f32, usize)>)> = loose
        .into_iter()
        .map(|i| {
            let t = &tracks[i];
            let mut scores: Vec<(f32, usize)> = (0..characters.len())
                .filter(|&ci| characters[ci].key.is_none_or(|k| !t.forbid.contains(&k)))
                .map(|ci| (proto::similarity(&t.embedding, &protos[ci]), ci))
                .collect();
            scores.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            (i, scores)
        })
        .collect();
    let top = |s: &[(f32, usize)]| s.first().map_or(f32::NEG_INFINITY, |x| x.0);
    loose.sort_by(|a, b| top(&b.1).total_cmp(&top(&a.1)).then(a.0.cmp(&b.0)));
    for (i, scores) in loose {
        let t = &tracks[i];
        let here = t.presence();
        let mut free =
            scores.into_iter().filter(|&(_, ci)| spans[ci].get(&t.media).is_none_or(|v| !overlaps(&here, v)));
        let Some((s1, c1)) = free.next() else { continue };
        let s2 = free.next().map_or(-1.0, |x| x.0);
        out[i] = if s1 >= p.assign && s1 - s2 >= p.margin {
            spans[c1].entry(t.media).or_default().extend(here);
            (Assign::Auto(c1), s1)
        } else if s1 >= p.unsure {
            (Assign::Unsure(c1), s1)
        } else {
            (Assign::None, s1)
        };
    }
    // Forbidden assignments inside clusters (the cluster turned out to be
    // that character) are undone.
    for (i, t) in tracks.iter().enumerate() {
        if let Assign::Auto(ci) | Assign::Unsure(ci) = out[i].0
            && characters[ci].key.is_some_and(|k| t.forbid.contains(&k))
        {
            out[i] = (Assign::None, 0.0);
        }
    }
    Outcome { characters, tracks: out }
}

#[cfg(test)]
mod tests;
