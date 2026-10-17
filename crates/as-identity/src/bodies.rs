//! Characters passed along bodies. A body track tied to a face track
//! (`as_analysis::link`) is whoever that face is — also while the face is
//! turned away. A face too blurred or turned to say who it is, left
//! unassigned, is whoever the other faces of its body are. And a body that
//! never shows a face may be found again by its clothes among the bodies
//! around it in the same episode (one outfit per scene): clearly more
//! alike to one character than to anyone else there, and never where that
//! character already is.
//!
//! Pure computation after [`crate::identify`] or [`crate::reassign`]. What
//! it derives never shapes a character: no clustering, no prototypes, and a
//! body found by its clothes isn't asked to find anyone else. The user's
//! word on a body (it is X, it isn't X) holds over all of it.

use std::collections::HashMap;

use crate::{Assign, CharKey, Outcome, TrackIn, overlaps};

/// A face track seen on a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceOn {
    /// Index into the tracks given to [`propagate`].
    pub track: usize,
    /// Samples that tied them.
    pub samples: u32,
}

/// A body track as [`propagate`] needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyIn {
    pub media: u32,
    /// Frames per second of its media (how many frames the window is).
    pub fps: f32,
    /// Frames the body is on screen (sorted, not empty).
    pub spans: Vec<(u32, u32)>,
    pub faces: Vec<FaceOn>,
    /// Unit-length appearance of the whole track, and of its sightings
    /// without a face (how the person looks turned away); `None` without
    /// any that say something.
    pub reid: Option<Vec<f32>>,
    pub reid_away: Option<Vec<f32>>,
    /// The surest of its detections: a match by clothes needs a person
    /// the detector is sure of (a dark object can look like someone's coat).
    pub score: f32,
    /// The user put this body on this character ...
    pub pin: Option<CharKey>,
    /// ... or said it is not these characters.
    pub forbid: Vec<CharKey>,
}

impl BodyIn {
    fn first(&self) -> u32 {
        self.spans.first().map_or(0, |s| s.0)
    }

    fn last(&self) -> u32 {
        self.spans.last().map_or(0, |s| s.1)
    }

    fn window(&self, p: &BodyParams) -> u32 {
        (p.window_s * self.fps).round().max(0.0) as u32
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BodyParams {
    /// Faces left unsure, not only unassigned, take their body's character.
    pub rescue_unsure: bool,
    /// ... if they say too little to stand on their own: quality below this
    /// (a clear face that resembles no one is someone else — an extra, a
    /// face in a poster). An unsure face the body agrees with is taken at
    /// any quality.
    pub rescue_below: f32,
    /// A body without a face looks for its person among the bodies with a
    /// face at most this many seconds before or after it ...
    pub window_s: f32,
    /// ... and takes the character that is at least this alike, and this
    /// much more alike than any other character there ...
    pub reid_min: f32,
    pub reid_margin: f32,
    /// ... if the detector was at least this sure of it once.
    pub min_score: f32,
}

impl Default for BodyParams {
    /// Set on bench/bodies (Breaking Bad S01E01): 57 of 58 matches by
    /// clothes right, the wrong one a dark object the detector was unsure of.
    fn default() -> Self {
        Self {
            rescue_unsure: true,
            rescue_below: 0.45,
            window_s: 180.0,
            reid_min: 0.65,
            reid_margin: 0.15,
            min_score: 0.65,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyAssign {
    None,
    /// The character of its faces.
    Face(usize),
    /// Found again by appearance: similarity to the character, and to the
    /// runner-up (−1 with no one else around).
    Reid {
        character: usize,
        score: f32,
        runner_up: f32,
    },
    /// Its faces are different characters.
    Conflict,
    /// The user's decision.
    Pinned(usize),
}

impl BodyAssign {
    pub fn character(self) -> Option<usize> {
        match self {
            Self::Face(c) | Self::Pinned(c) | Self::Reid { character: c, .. } => Some(c),
            Self::None | Self::Conflict => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyOutcome {
    /// Per body (index into the input).
    pub bodies: Vec<BodyAssign>,
    /// Face tracks that took their body's character: (track, character).
    pub promoted: Vec<(usize, usize)>,
}

/// A body's faces vote this share for one character, or it is a conflict.
const MAJORITY: f32 = 0.9;

/// Characters for `bodies`, given what `outcome` made of `tracks`.
pub fn propagate(tracks: &[TrackIn], outcome: &Outcome, bodies: &[BodyIn], p: &BodyParams) -> BodyOutcome {
    // Characters by the key the caller knows them by.
    let by_key: HashMap<CharKey, usize> =
        outcome.characters.iter().enumerate().filter_map(|(i, c)| Some((c.key?, i))).collect();
    let forbidden = |b: &BodyIn, c: usize| outcome.characters[c].key.is_some_and(|k| b.forbid.contains(&k));
    // Where each character is on screen, per media: its faces, then the
    // bodies and faces this adds.
    let mut on: HashMap<(usize, u32), Vec<(u32, u32)>> = HashMap::new();
    for (t, (a, _)) in tracks.iter().zip(&outcome.tracks) {
        if let Some(c) = a.character() {
            on.entry((c, t.media)).or_default().extend(t.presence());
        }
    }

    // (a) Bodies are their faces' character — or the user's.
    let mut assign = vec![BodyAssign::None; bodies.len()];
    let mut weight = vec![0f32; bodies.len()];
    // The user's for a character with no faces placed now: still theirs,
    // so it says nothing about anyone here.
    let mut elsewhere = vec![false; bodies.len()];
    for (bi, b) in bodies.iter().enumerate() {
        if let Some(k) = b.pin {
            match by_key.get(&k) {
                Some(&c) => {
                    assign[bi] = BodyAssign::Pinned(c);
                    weight[bi] = f32::INFINITY;
                    on.entry((c, b.media)).or_default().extend(b.spans.iter().copied());
                }
                None => elsewhere[bi] = true,
            }
            continue;
        }
        let mut votes: Vec<(usize, f32)> = Vec::new();
        for f in &b.faces {
            let (a, _) = outcome.tracks[f.track];
            let (c, pin) = match a {
                Assign::Auto(c) => (c, 1.0),
                Assign::Pinned(c) => (c, 2.0),
                Assign::None | Assign::Unsure(_) => continue,
            };
            if forbidden(b, c) {
                continue;
            }
            let w = f.samples as f32 * tracks[f.track].quality.max(0.05) * pin;
            match votes.iter_mut().find(|v| v.0 == c) {
                Some(v) => v.1 += w,
                None => votes.push((c, w)),
            }
        }
        let total: f32 = votes.iter().map(|v| v.1).sum();
        votes.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        assign[bi] = match votes.first() {
            None => BodyAssign::None,
            Some(&(c, w)) if w >= MAJORITY * total => BodyAssign::Face(c),
            Some(_) => BodyAssign::Conflict,
        };
        weight[bi] = total;
        if let BodyAssign::Face(c) = assign[bi] {
            on.entry((c, b.media)).or_default().extend(b.spans.iter().copied());
        }
    }

    // (b) Faces nobody could name are their body's character, strongest
    // bodies first, where the character's face isn't on screen already (the
    // body's own span is theirs anyway).
    let mut faces_of: HashMap<(usize, u32), Vec<(u32, u32)>> = HashMap::new();
    for (t, (a, _)) in tracks.iter().zip(&outcome.tracks) {
        if let Some(c) = a.character() {
            faces_of.entry((c, t.media)).or_default().extend(t.presence());
        }
    }
    let mut order: Vec<usize> =
        (0..bodies.len()).filter(|&i| matches!(assign[i], BodyAssign::Face(_) | BodyAssign::Pinned(_))).collect();
    order.sort_by(|&a, &b| weight[b].total_cmp(&weight[a]).then(a.cmp(&b)));
    let mut promoted: Vec<(usize, usize)> = Vec::new();
    for bi in order {
        let Some(c) = assign[bi].character() else { unreachable!("bodies with a character") };
        let key = outcome.characters[c].key;
        for f in &bodies[bi].faces {
            let t = &tracks[f.track];
            let unclear = t.quality < p.rescue_below;
            let open = match outcome.tracks[f.track].0 {
                Assign::None => unclear,
                Assign::Unsure(s) => p.rescue_unsure && (s == c || unclear),
                Assign::Auto(_) | Assign::Pinned(_) => false,
            };
            if !open || key.is_some_and(|k| t.forbid.contains(&k)) || promoted.iter().any(|&(j, _)| j == f.track) {
                continue;
            }
            let here = t.presence();
            let theirs = faces_of.entry((c, t.media)).or_default();
            if overlaps(&here, theirs) {
                continue;
            }
            theirs.extend(here.iter().copied());
            promoted.push((f.track, c));
            on.entry((c, t.media)).or_default().extend(here);
        }
    }

    // (c) Bodies without anyone: the character their clothes match among
    // the bodies with a face around them.
    // Per media, by first frame: a query looks only at its window (a
    // series has tens of thousands of bodies).
    let mut anchors: HashMap<u32, Vec<(u32, usize, usize)>> = HashMap::new();
    for (i, b) in bodies.iter().enumerate() {
        if let BodyAssign::Face(c) | BodyAssign::Pinned(c) = assign[i]
            && (b.reid.is_some() || b.reid_away.is_some())
        {
            anchors.entry(b.media).or_default().push((b.first(), i, c));
        }
    }
    let mut longest: HashMap<u32, u32> = HashMap::new();
    for (media, v) in &mut anchors {
        v.sort_unstable();
        longest.insert(*media, v.iter().map(|&(_, i, _)| bodies[i].last() - bodies[i].first()).max().unwrap_or(0));
    }
    // The character a body looks most like, its similarity and the runner-up's
    // — every character around counts, those on screen then too: when the
    // best match is already there, the body is no one (not the runner-up).
    // Characters the user said it isn't don't count.
    let best = |q: &BodyIn, on: &HashMap<(usize, u32), Vec<(u32, u32)>>| -> Option<(usize, f32, f32)> {
        let e = q.reid.as_ref()?;
        let near = anchors.get(&q.media)?;
        let window = q.window(p);
        let from = q.first().saturating_sub(window.saturating_add(longest[&q.media]));
        let until = q.last().saturating_add(window);
        let mut by: Vec<(usize, f32)> = Vec::new();
        for &(_, ai, c) in near[near.partition_point(|a| a.0 < from)..].iter().take_while(|a| a.0 <= until) {
            let a = &bodies[ai];
            let gap = a.first().saturating_sub(q.last()).max(q.first().saturating_sub(a.last()));
            if gap > window || forbidden(q, c) {
                continue;
            }
            let s = [&a.reid, &a.reid_away].into_iter().flatten().map(|x| dot(e, x)).fold(f32::NEG_INFINITY, f32::max);
            match by.iter_mut().find(|v| v.0 == c) {
                Some(v) => v.1 = v.1.max(s),
                None => by.push((c, s)),
            }
        }
        by.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let &(c, s) = by.first()?;
        let there = on.get(&(c, q.media)).is_some_and(|v| overlaps(&q.spans, v));
        (!there && s >= p.reid_min && s - by.get(1).map_or(-1.0, |v| v.1) >= p.reid_margin)
            .then(|| (c, s, by.get(1).map_or(-1.0, |v| v.1)))
    };
    let mut queries: Vec<(f32, usize, usize)> = (0..bodies.len())
        .filter(|&i| assign[i] == BodyAssign::None && !elsewhere[i] && bodies[i].score >= p.min_score)
        .filter_map(|i| best(&bodies[i], &on).map(|(c, s, _)| (s, i, c)))
        .collect();
    queries.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, qi, c) in queries {
        // Again, now that more of the cast has a place: still the same
        // character, and not on screen by now.
        let q = &bodies[qi];
        if let Some((c2, score, runner_up)) = best(q, &on).filter(|&(c2, ..)| c2 == c) {
            assign[qi] = BodyAssign::Reid { character: c2, score, runner_up };
            on.entry((c2, q.media)).or_default().extend(q.spans.iter().copied());
        }
    }
    BodyOutcome { bodies: assign, promoted }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CharacterOut;

    fn face(media: u32, start: u32, end: u32, quality: f32) -> TrackIn {
        TrackIn {
            media,
            start,
            end,
            spans: vec![],
            embedding: vec![1.0],
            quality,
            dets: 3,
            pin: None,
            forbid: vec![],
            previous: None,
        }
    }

    fn outcome(assign: &[Assign], chars: usize) -> Outcome {
        Outcome {
            characters: (0..chars).map(|k| CharacterOut { key: Some(k as u32 + 10), centroid: vec![1.0] }).collect(),
            tracks: assign.iter().map(|&a| (a, 0.5)).collect(),
        }
    }

    fn unit(axis: usize, cos: f32) -> Vec<f32> {
        let mut v = vec![0f32; 4];
        v[axis] = cos;
        v[3] = (1.0 - cos * cos).max(0.0).sqrt();
        v
    }

    fn body(span: (u32, u32), faces: &[(usize, u32)], reid: Option<Vec<f32>>) -> BodyIn {
        BodyIn {
            media: 0,
            fps: 10.0,
            spans: vec![span],
            faces: faces.iter().map(|&(track, samples)| FaceOn { track, samples }).collect(),
            reid_away: None,
            reid,
            score: 0.9,
            pin: None,
            forbid: vec![],
        }
    }

    const P: BodyParams = BodyParams {
        rescue_unsure: false,
        rescue_below: 0.45,
        window_s: 100.0,
        reid_min: 0.8,
        reid_margin: 0.1,
        min_score: 0.65,
    };

    #[test]
    fn bodies_are_their_faces_character_and_rescue_their_unnamed_faces() {
        // Track 0 is character 0; track 1 on the same body nobody could name;
        // track 2 unsure.
        let tracks = vec![face(0, 0, 30, 0.8), face(0, 40, 60, 0.1), face(0, 70, 90, 0.2)];
        let o = outcome(&[Assign::Auto(0), Assign::None, Assign::Unsure(1)], 2);
        let bodies = vec![body((0, 90), &[(0, 10), (1, 6), (2, 6)], None)];
        let r = propagate(&tracks, &o, &bodies, &P);
        assert_eq!(r.bodies, [BodyAssign::Face(0)]);
        assert_eq!(r.promoted, [(1, 0)]);
        let r = propagate(&tracks, &o, &bodies, &BodyParams { rescue_unsure: true, ..P });
        assert_eq!(r.promoted, [(1, 0), (2, 0)]);
    }

    #[test]
    fn a_clear_face_that_resembles_no_one_stays_unnamed() {
        // Track 1 is clear (an extra, a face in a poster) but nobody's; track
        // 2 is clear and unsure about the body's character: the body agrees.
        let tracks = vec![face(0, 0, 30, 0.8), face(0, 40, 60, 0.8), face(0, 70, 90, 0.8)];
        let o = outcome(&[Assign::Auto(0), Assign::None, Assign::Unsure(0)], 2);
        let bodies = vec![body((0, 90), &[(0, 10), (1, 6), (2, 6)], None)];
        assert!(propagate(&tracks, &o, &bodies, &P).promoted.is_empty());
        let r = propagate(&tracks, &o, &bodies, &BodyParams { rescue_unsure: true, ..P });
        assert_eq!(r.promoted, [(2, 0)]);
    }

    #[test]
    fn faces_of_two_characters_make_a_conflict_unless_one_dominates() {
        let tracks = vec![face(0, 0, 30, 0.8), face(0, 40, 60, 0.8)];
        let o = outcome(&[Assign::Auto(0), Assign::Auto(1)], 2);
        let r = propagate(&tracks, &o, &[body((0, 60), &[(0, 10), (1, 9)], None)], &P);
        assert_eq!(r.bodies, [BodyAssign::Conflict]);
        let r = propagate(&tracks, &o, &[body((0, 60), &[(0, 95), (1, 5)], None)], &P);
        assert_eq!(r.bodies, [BodyAssign::Face(0)]);
    }

    #[test]
    fn a_rescue_never_puts_a_character_in_two_places() {
        // Track 1 (unnamed, on the body) is on screen while track 2, also
        // character 0, is elsewhere in the frame; and track 3 said "not 0".
        let mut tracks = vec![face(0, 0, 30, 0.8), face(0, 40, 60, 0.1), face(0, 45, 55, 0.9), face(0, 70, 80, 0.1)];
        tracks[3].forbid = vec![10];
        let o = outcome(&[Assign::Auto(0), Assign::None, Assign::Auto(0), Assign::None], 1);
        let bodies = vec![body((0, 80), &[(0, 10), (1, 6), (3, 3)], None)];
        assert!(propagate(&tracks, &o, &bodies, &P).promoted.is_empty());
    }

    #[test]
    fn a_body_without_a_face_is_found_again_by_its_clothes() {
        // Bodies 0 and 1 have faces (characters 0 and 1); body 2 has none.
        let tracks = vec![face(0, 0, 30, 0.8), face(0, 0, 30, 0.8)];
        let o = outcome(&[Assign::Auto(0), Assign::Auto(1)], 2);
        let mk = |q: Vec<f32>, span| {
            vec![
                body((0, 30), &[(0, 10)], Some(unit(0, 1.0))),
                body((0, 30), &[(1, 10)], Some(unit(1, 1.0))),
                body(span, &[], Some(q)),
            ]
        };
        // Close to character 0, far from 1: found.
        let r = propagate(&tracks, &o, &mk(unit(0, 0.9), (100, 130)), &P);
        assert!(matches!(r.bodies[2], BodyAssign::Reid { character: 0, .. }), "{:?}", r.bodies[2]);
        // Too far away in time (5 s at 10 fps), or not alike enough: nobody.
        let far = BodyParams { window_s: 5.0, ..P };
        assert_eq!(propagate(&tracks, &o, &mk(unit(0, 0.9), (100, 130)), &far).bodies[2], BodyAssign::None);
        assert_eq!(propagate(&tracks, &o, &mk(unit(0, 0.7), (100, 130)), &P).bodies[2], BodyAssign::None);
        // On screen with both: neither can be a third body there.
        assert_eq!(propagate(&tracks, &o, &mk(unit(0, 0.9), (10, 20)), &P).bodies[2], BodyAssign::None);
        // Something the detector was never sure was a person isn't anyone's coat.
        let mut unsure = mk(unit(0, 0.9), (100, 130));
        unsure[2].score = 0.55;
        assert_eq!(propagate(&tracks, &o, &unsure, &P).bodies[2], BodyAssign::None);
    }

    #[test]
    fn a_body_most_like_someone_on_screen_is_no_one() {
        // Character 0 is on screen (frames 0..30) and looks exactly like the
        // body (a second box on the same person); character 1, around later,
        // looks like it too (0.82). Not character 1: nobody.
        let tracks = vec![face(0, 0, 30, 0.8), face(0, 100, 130, 0.8)];
        let o = outcome(&[Assign::Auto(0), Assign::Auto(1)], 2);
        let q = unit(0, 1.0);
        let bodies = vec![
            body((0, 30), &[(0, 10)], Some(unit(0, 1.0))),
            body((100, 130), &[(1, 10)], Some(vec![0.82, 0.0, 0.0, (1.0f32 - 0.82 * 0.82).sqrt()])),
            body((10, 20), &[], Some(q)),
        ];
        let p = BodyParams { reid_margin: 0.0, ..P };
        assert_eq!(propagate(&tracks, &o, &bodies, &p).bodies[2], BodyAssign::None);
    }

    #[test]
    fn a_close_runner_up_leaves_a_body_unnamed() {
        let tracks = vec![face(0, 0, 30, 0.8), face(0, 0, 30, 0.8)];
        let o = outcome(&[Assign::Auto(0), Assign::Auto(1)], 2);
        // About as alike to both characters (0.73 and 0.69).
        let n = (0.85f32 * 0.85 + 0.8 * 0.8).sqrt();
        let q = vec![0.85 / n, 0.8 / n, 0.0, 0.0];
        let bodies = vec![
            body((0, 30), &[(0, 10)], Some(unit(0, 1.0))),
            body((0, 30), &[(1, 10)], Some(unit(1, 1.0))),
            body((100, 130), &[], Some(q)),
        ];
        assert_eq!(propagate(&tracks, &o, &bodies, &BodyParams { reid_min: 0.5, ..P }).bodies[2], BodyAssign::None);
    }

    #[test]
    fn the_users_word_on_a_body_holds() {
        let tracks = vec![face(0, 0, 30, 0.8), face(0, 0, 30, 0.8), face(0, 40, 60, 0.1)];
        let o = outcome(&[Assign::Auto(0), Assign::Auto(1), Assign::None], 2);
        let mk = || {
            vec![
                body((0, 30), &[(0, 10)], Some(unit(0, 1.0))),
                body((0, 30), &[(1, 10)], Some(unit(1, 1.0))),
                body((100, 130), &[], Some(unit(0, 0.9))),
                body((40, 60), &[(2, 4)], None),
            ]
        };
        // "Not character 0" (key 10): its clothes match no one else enough.
        let mut b = mk();
        b[2].forbid = vec![10];
        assert_eq!(propagate(&tracks, &o, &b, &P).bodies[2], BodyAssign::None);
        // A face's body the user says isn't 0 isn't, even with 0's face on it.
        let mut b = mk();
        b[0].forbid = vec![10];
        assert_eq!(propagate(&tracks, &o, &b, &P).bodies[0], BodyAssign::None);
        // Pinned to character 1: it is, rescues its unnamed face and finds others.
        let mut b = mk();
        b[3].pin = Some(11);
        let r = propagate(&tracks, &o, &b, &P);
        assert_eq!(r.bodies[3], BodyAssign::Pinned(1));
        assert_eq!(r.promoted, [(2, 1)]);
        assert_eq!(BodyAssign::Pinned(1).character(), Some(1));
    }

    #[test]
    fn a_body_pinned_to_someone_without_faces_now_says_nothing() {
        // The user said body 0 is a character none of whose faces is placed
        // now (key 99): not the character of the face on it, no name for its
        // unclear face, and no anchor for clothes like its own.
        let tracks = vec![face(0, 0, 30, 0.8), face(0, 40, 60, 0.1)];
        let o = outcome(&[Assign::Auto(0), Assign::None], 1);
        let mut bodies =
            vec![body((0, 60), &[(0, 10), (1, 4)], Some(unit(0, 1.0))), body((100, 130), &[], Some(unit(0, 1.0)))];
        let before = propagate(&tracks, &o, &bodies, &P);
        assert_eq!((before.bodies[0], before.promoted.len()), (BodyAssign::Face(0), 1));
        assert!(matches!(before.bodies[1], BodyAssign::Reid { character: 0, .. }));
        bodies[0].pin = Some(99);
        let r = propagate(&tracks, &o, &bodies, &P);
        assert_eq!(r.bodies, [BodyAssign::None, BodyAssign::None]);
        assert!(r.promoted.is_empty());
    }
}
