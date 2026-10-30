//! Synthetic identities: random directions in 512-d with noisy tracks
//! around them, the way face embeddings scatter around a person.

use super::*;

const DIM: usize = 512;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }
    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.next(), self.next());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
    fn unit(&mut self) -> Vec<f32> {
        self.unit_d(DIM)
    }
    fn unit_d(&mut self, dim: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..dim).map(|_| self.gauss()).collect();
        proto_normalize(&mut v);
        v
    }
}

fn proto_normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
}

/// A track of identity `center` with cosine ≈ `fidelity` to it.
fn around(rng: &mut Rng, center: &[f32], fidelity: f32) -> Vec<f32> {
    let noise = rng.unit_d(center.len());
    let k = (1.0 - fidelity * fidelity).sqrt();
    let mut v: Vec<f32> = center.iter().zip(&noise).map(|(c, n)| fidelity * c + k * n).collect();
    proto_normalize(&mut v);
    v
}

fn track(media: u32, start: u32, embedding: Vec<f32>, quality: f32) -> TrackIn {
    TrackIn {
        media,
        start,
        end: start + 48,
        spans: vec![],
        // The face pass looks at every third frame of the media.
        seen: (start..start + 48).filter(|f| f % 3 == 0).collect(),
        embedding,
        quality,
        dets: 10,
        weight: 1.0,
        pin: None,
        forbid: vec![],
        previous: None,
    }
}

/// `people` identities, `per` core tracks each, on screen one after another.
fn cast(rng: &mut Rng, people: usize, per: usize) -> (Vec<Vec<f32>>, Vec<TrackIn>, Vec<usize>) {
    let centers: Vec<Vec<f32>> = (0..people).map(|_| rng.unit()).collect();
    let mut tracks = Vec::new();
    let mut truth = Vec::new();
    let mut t = 0;
    for _ in 0..per {
        for (p, c) in centers.iter().enumerate() {
            tracks.push(track(0, t, around(rng, c, 0.75), 0.8));
            truth.push(p);
            t += 100;
        }
    }
    (centers, tracks, truth)
}

/// BCubed F-measure of an assignment against the truth (unassigned = own cluster).
fn bcubed(assigned: &[Option<usize>], truth: &[usize]) -> f32 {
    crate::metrics::bcubed(assigned, truth, &vec![1.0; truth.len()], |_| true).f
}

fn chars(o: &Outcome) -> Vec<Option<usize>> {
    o.tracks.iter().map(|(a, _)| a.character()).collect()
}

#[test]
fn separate_people_become_separate_characters() {
    let mut rng = Rng(7);
    let (_, tracks, truth) = cast(&mut rng, 6, 15);
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 6);
    let f = bcubed(&chars(&o), &truth);
    assert!(f > 0.99, "BCubed F {f}");
}

#[test]
fn anime_params_group_wide_embeddings() {
    // CCIP's 768-d vectors under [`Params::anime`]: two well-separated
    // characters, tracks of each one after another, come out as two.
    let mut rng = Rng(23);
    let a = rng.unit_d(768);
    let b = rng.unit_d(768);
    let mut tracks = Vec::new();
    let mut truth = Vec::new();
    for i in 0..15u32 {
        tracks.push(track(0, i * 100, around(&mut rng, &a, 0.85), 0.8));
        tracks.push(track(0, i * 100 + 50, around(&mut rng, &b, 0.85), 0.8));
        truth.extend([0, 1]);
    }
    let o = identify(&tracks, &Params::anime());
    assert_eq!(o.characters.len(), 2);
    let f = bcubed(&chars(&o), &truth);
    assert!(f > 0.99, "BCubed F {f}");
}

#[test]
fn people_on_screen_together_are_never_merged() {
    let mut rng = Rng(11);
    // Two look-alikes (their centres are 0.8 alike — as alike as twins),
    // in the same shots, and now and then one without the other.
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.8);
    let mut tracks = Vec::new();
    let mut truth = Vec::new();
    for i in 0..12u32 {
        tracks.push(track(0, i * 100, around(&mut rng, &a, 0.8), 0.8));
        tracks.push(track(0, i * 100 + 10, around(&mut rng, &b, 0.8), 0.8));
        truth.extend([0, 1]);
    }
    for i in 0..4u32 {
        tracks.push(track(1, i * 100, around(&mut rng, &a, 0.8), 0.8));
        tracks.push(track(2, i * 100, around(&mut rng, &b, 0.8), 0.8));
        truth.extend([0, 1]);
    }
    let o = identify(&tracks, &Params::default());
    let got = chars(&o);
    for i in 0..12 {
        assert_ne!(got[2 * i], got[2 * i + 1], "pair {i} merged");
    }
    assert_eq!(o.characters.len(), 2);
    assert!(bcubed(&got, &truth) > 0.9);
}

/// A person in two looks (lighting, angle), each a group of its own at
/// first: `n` tracks per look, one after another in media `media`. Returns
/// the tracks' indices per look.
fn two_looks(rng: &mut Rng, tracks: &mut Vec<TrackIn>, person: &[f32], media: u32, n: u32) -> [Vec<usize>; 2] {
    let looks = [around(rng, person, 0.9), around(rng, person, 0.9)];
    let mut out = [Vec::new(), Vec::new()];
    for i in 0..n {
        for (l, look) in looks.iter().enumerate() {
            out[l].push(tracks.len());
            tracks.push(track(media, (2 * i + l as u32) * 100, around(rng, look, 0.8), 0.8));
        }
    }
    out
}

/// The character each of `ids` is in (all the same), or a panic.
fn one_character(o: &Outcome, ids: &[usize]) -> usize {
    let c = o.tracks[ids[0]].0.character().expect("a character");
    for &i in ids {
        assert_eq!(o.tracks[i].0.character(), Some(c), "track {i} is elsewhere");
    }
    c
}

#[test]
fn a_mirror_does_not_split_a_person() {
    let mut rng = Rng(41);
    let person = rng.unit();
    let other = rng.unit();
    let mut tracks = Vec::new();
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 0, 10);
    for i in 0..10u32 {
        tracks.push(track(0, 5000 + i * 100, around(&mut rng, &other, 0.8), 0.8));
    }
    // In front of a mirror: one face of each look in the same frames, at
    // two places (the person and their reflection) — and someone else
    // beside them.
    let at = tracks[l1[3]].start;
    let others: Vec<usize> = (20..30).collect();
    for i in [l2[3], others[0]] {
        tracks[i].start = at;
        tracks[i].end = at + 48;
        tracks[i].seen = tracks[l1[3]].seen.clone();
    }
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 2, "one character per person");
    let rest: Vec<usize> = l1.iter().chain(&l2).copied().filter(|&i| i != l1[3] && i != l2[3]).collect();
    let c = one_character(&o, &rest);
    assert_ne!(one_character(&o, &others), c);
    // One of the two faces in the mirror shot is theirs; the other can't be
    // (a character is on screen once per frame).
    let mirror = [o.tracks[l1[3]].0.character(), o.tracks[l2[3]].0.character()];
    assert_eq!(mirror.iter().filter(|&&m| m == Some(c)).count(), 1, "{mirror:?}");
    assert!(mirror.iter().all(|m| m.is_none() || *m == Some(c)));
}

#[test]
fn a_face_the_tracker_found_twice_is_one_person() {
    let mut rng = Rng(43);
    let person = rng.unit();
    let mut tracks = Vec::new();
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 0, 10);
    // The tracker lost a face of look 1 for a moment and found it again as
    // another track (of look 2) inside the gap: never two faces at once.
    let (s, t) = (l1[5], l2[5]);
    let at = tracks[s].start;
    tracks[s].end = at + 150;
    tracks[s].spans = vec![(at, at + 150)];
    tracks[s].seen = (at..at + 150).filter(|f| f % 3 == 0 && !(at + 60..at + 90).contains(f)).collect();
    tracks[t].start = at + 60;
    tracks[t].end = at + 90;
    tracks[t].seen = (at + 60..at + 90).filter(|f| f % 3 == 0).collect();
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 1);
    one_character(&o, &[l1.clone(), l2.clone()].concat());
}

#[test]
fn a_strangers_face_in_a_group_does_not_split_a_person() {
    let mut rng = Rng(47);
    let person = rng.unit();
    let stranger = rng.unit();
    let mut tracks = Vec::new();
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 0, 12);
    let strangers: Vec<usize> = (0..10u32)
        .map(|i| {
            tracks.push(track(1, i * 100, around(&mut rng, &stranger, 0.8), 0.8));
            tracks.len() - 1
        })
        .collect();
    // A blurry face of the stranger that resembles look 1 enough to join
    // it, on screen with a face of look 2.
    let mut mixed: Vec<f32> = stranger.iter().zip(&tracks[l1[0]].embedding).map(|(s, p)| 0.55 * s + 0.85 * p).collect();
    proto_normalize(&mut mixed);
    let stray_at = tracks.len();
    tracks.push(track(0, tracks[l2[7]].start, mixed, 0.5));
    let o = identify(&tracks, &Params::default());
    let c = one_character(&o, &[l1.clone(), l2.clone()].concat());
    assert_ne!(o.tracks[stray_at].0.character(), Some(c), "a face seen with them is not them");
    one_character(&o, &strangers);
    assert_eq!(o.characters.len(), 2);
}

#[test]
fn a_reflection_seen_only_with_its_person_is_no_character() {
    let mut rng = Rng(53);
    let person = rng.unit();
    let other = rng.unit();
    let mut tracks: Vec<TrackIn> = (0..12u32).map(|i| track(0, i * 100, around(&mut rng, &person, 0.8), 0.8)).collect();
    for i in 0..8u32 {
        tracks.push(track(0, 5000 + i * 100, around(&mut rng, &other, 0.8), 0.8));
    }
    // A long scene in front of a mirror: shot after shot their reflection,
    // alike to each other (the same mirror, the same light), with them.
    let mirror = around(&mut rng, &person, 0.85);
    let reflections: Vec<usize> = (0..5)
        .map(|i| {
            let mut t = track(0, tracks[i].start, around(&mut rng, &mirror, 0.9), 0.8);
            t.seen = tracks[i].seen.clone();
            tracks.push(t);
            tracks.len() - 1
        })
        .collect();
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 2, "no character for the reflection");
    one_character(&o, &(0..12).collect::<Vec<_>>());
    // Nobody's, not even a guess: it is most like someone on screen with it.
    assert!(reflections.iter().all(|&i| o.tracks[i].0 == Assign::None), "{:?}", o.tracks);
}

/// `n` tracks of `person`, each seen in the same frames as a track of
/// `beside` (another person) when `with(i)`, else on its own.
fn seen_with(
    rng: &mut Rng,
    tracks: &mut Vec<TrackIn>,
    person: &[f32],
    beside: &[usize],
    with: impl Fn(usize) -> bool,
) -> Vec<usize> {
    (0..beside.len())
        .map(|i| {
            let at = tracks[beside[i]].start;
            let mut t =
                track(tracks[beside[i]].media, if with(i) { at } else { at + 200_000 }, around(rng, person, 0.8), 0.8);
            if with(i) {
                t.seen = tracks[beside[i]].seen.clone();
            }
            tracks.push(t);
            tracks.len() - 1
        })
        .collect()
}

#[test]
fn look_alikes_always_on_screen_together_stay_two() {
    let mut rng = Rng(59);
    // Siblings who look alike (mean faces 0.7 alike), the younger one only
    // ever seen with the older: two faces at every moment, never one face
    // seen twice.
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.7);
    let mut tracks: Vec<TrackIn> = (0..14u32).map(|i| track(0, i * 100, around(&mut rng, &a, 0.8), 0.8)).collect();
    let older: Vec<usize> = (0..14).collect();
    let younger = seen_with(&mut rng, &mut tracks, &b, &older[..8], |_| true);
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 2);
    let (ca, cb) = (one_character(&o, &older), one_character(&o, &younger));
    assert_ne!(ca, cb);
}

#[test]
fn a_merge_despite_faces_seen_together_needs_clearly_one_person() {
    let mut rng = Rng(61);
    // Two people half alike (mean faces 0.5) — their faces alike enough
    // to be grouped (average 0.36) — seen together once.
    let p = rng.unit();
    let q = around(&mut rng, &p, 0.5);
    let mut tracks: Vec<TrackIn> = (0..10u32).map(|i| track(0, i * 100, around(&mut rng, &p, 0.85), 0.8)).collect();
    let first: Vec<usize> = (0..10).collect();
    let second = seen_with(&mut rng, &mut tracks, &q, &first, |i| i == 4);
    for &i in &second {
        tracks[i].embedding = around(&mut rng, &q, 0.85);
    }
    let o = identify(&tracks, &Params::default());
    assert_ne!(one_character(&o, &first[..4]), one_character(&o, &second[..4]), "two people");
}

#[test]
fn a_pinned_face_never_steps_aside() {
    let mut rng = Rng(67);
    let person = rng.unit();
    let mut tracks = Vec::new();
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 0, 10);
    // The user named look 1. In front of a mirror, a face of look 1 that is
    // the least typical of all, and one of look 2, at the same moment.
    for &i in &l1 {
        tracks[i].pin = Some(7);
    }
    tracks[l1[3]].embedding = around(&mut rng, &person, 0.6);
    let at = tracks[l1[3]].start;
    tracks[l2[3]].start = at;
    tracks[l2[3]].end = at + 48;
    tracks[l2[3]].seen = tracks[l1[3]].seen.clone();
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 1);
    assert_eq!(o.characters[0].key, Some(7));
    assert!(l1.iter().all(|&i| o.tracks[i].0 == Assign::Pinned(0)));
    let rest: Vec<usize> = l2.iter().copied().filter(|&i| i != l2[3]).collect();
    one_character(&o, &rest);
    assert_ne!(o.tracks[l2[3]].0.character(), Some(0));
}

#[test]
fn a_pinned_copy_stays_a_character() {
    let mut rng = Rng(71);
    let person = rng.unit();
    let mut tracks: Vec<TrackIn> = (0..12u32).map(|i| track(0, i * 100, around(&mut rng, &person, 0.8), 0.8)).collect();
    // Their face on a screen, shot after shot with them — and the user made
    // it a character of its own.
    let screen = around(&mut rng, &person, 0.75);
    let copies: Vec<usize> = (0..5)
        .map(|i| {
            let mut t = track(0, tracks[i].start, around(&mut rng, &screen, 0.9), 0.8);
            t.seen = tracks[i].seen.clone();
            t.pin = Some(3);
            tracks.push(t);
            tracks.len() - 1
        })
        .collect();
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 2);
    let c = one_character(&o, &copies);
    assert_eq!(o.characters[c].key, Some(3));
}

#[test]
fn a_face_most_like_someone_beside_it_is_no_guess_for_anyone_else() {
    let mut rng = Rng(73);
    let (_, mut tracks, truth) = cast(&mut rng, 2, 10);
    let p0 = (0..tracks.len()).find(|&i| truth[i] == 0).unwrap();
    // A weak face, most like person 0 but on screen with them, and somewhat
    // like person 1 — a guess for person 1 if it were alone.
    let (c0, c1) =
        (tracks[p0].embedding.clone(), tracks[(0..tracks.len()).find(|&i| truth[i] == 1).unwrap()].embedding.clone());
    let mut e: Vec<f32> = c0.iter().zip(&c1).map(|(x, y)| 0.75 * x + 0.45 * y).collect();
    proto_normalize(&mut e);
    let mut weak = track(0, tracks[p0].start, e.clone(), 0.2);
    weak.seen = tracks[p0].seen.clone();
    let alone = track(5, 0, e, 0.2);
    tracks.extend([weak, alone]);
    let o = identify(&tracks, &Params::default());
    let n = tracks.len();
    assert_eq!(o.tracks[n - 2].0, Assign::None, "beside person 0");
    assert_eq!(o.tracks[n - 1].0.character(), o.tracks[p0].0.character(), "alone, it is person 0");
}

#[test]
fn twins_stay_two_and_a_mirror_splits_no_one_in_a_big_series() {
    // More clear tracks than one clustering level takes, so each episode is
    // grouped on its own first: twins (mean faces 0.75 alike), six tracks of
    // one and three of the other per episode, one of those three on screen
    // with the other twin — and a person with a mirror in one episode.
    let mut rng = Rng(79);
    let mut tracks: Vec<TrackIn> = (0..4100u32).map(|i| track(100 + i % 8, (i / 8) * 100, rng.unit(), 0.9)).collect();
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.75);
    let (mut twin_a, mut twin_b) = (Vec::new(), Vec::new());
    for m in 0..20u32 {
        let first = tracks.len();
        for k in 0..6u32 {
            tracks.push(track(m, 1_000_000 + k * 100, around(&mut rng, &a, 0.8), 0.8));
        }
        twin_a.extend(first..tracks.len());
        let theirs: Vec<usize> = (first..first + 3).collect();
        twin_b.extend(seen_with(&mut rng, &mut tracks, &b, &theirs, |i| i == 0));
    }
    let person = rng.unit();
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 50, 10);
    let at = tracks[l1[3]].start;
    tracks[l2[3]].start = at;
    tracks[l2[3]].end = at + 48;
    tracks[l2[3]].seen = tracks[l1[3]].seen.clone();
    let o = identify(&tracks, &Params::default());
    assert_ne!(one_character(&o, &twin_a), one_character(&o, &twin_b), "the twins are two");
    let rest: Vec<usize> = l1.iter().chain(&l2).copied().filter(|&i| i != l1[3] && i != l2[3]).collect();
    one_character(&o, &rest);
}

/// Twins A and B are two characters: B's faces stay off A's (but a stray
/// blurry one that looks more like A than like B), and most of them are
/// one character of their own.
fn twins_are_two(o: &Outcome, a: &[usize], b: &[usize], what: &str) {
    let ca = one_character(o, a);
    let on_a = b.iter().filter(|&&i| o.tracks[i].0.character() == Some(ca)).count();
    assert!(10 * on_a <= b.len(), "{what}: {on_a} of B's {} faces on A", b.len());
    let mut theirs: HashMap<usize, usize> = HashMap::new();
    for &i in b {
        if let Some(c) = o.tracks[i].0.character().filter(|&c| c != ca) {
            *theirs.entry(c).or_default() += 1;
        }
    }
    let most = theirs.values().max().copied().unwrap_or(0);
    assert!(2 * most >= b.len(), "{what}: B's own character holds {most} of {}", b.len());
}

#[test]
fn twins_stay_two_when_they_are_seen_more_clearly_together() {
    // Two-shots in good light, the rest of their shots in any light: twins
    // whose faces side by side are clearer than their faces elsewhere.
    for (seed, (centres, alone, together)) in [(0.75f32, 0.6f32, 8usize), (0.8, 0.65, 6)].into_iter().enumerate() {
        let mut rng = Rng(109 + seed as u64);
        let a = rng.unit();
        let b = around(&mut rng, &a, centres);
        let mut tracks: Vec<TrackIn> = (0..20u32).map(|i| track(0, i * 100, around(&mut rng, &a, 0.9), 0.8)).collect();
        let first: Vec<usize> = (0..20).collect();
        let second = seen_with(&mut rng, &mut tracks, &b, &first, |i| i < together);
        for (k, &i) in second.iter().enumerate() {
            tracks[i].embedding = around(&mut rng, &b, if k < together { 0.9 } else { alone });
        }
        let o = identify(&tracks, &Params::default());
        twins_are_two(&o, &first, &second, &format!("twins {seed}"));
    }
}

#[test]
fn twins_on_screen_together_now_and_then_stay_two() {
    // In 4 of the 20 shots of the second twin, the first is there too: too
    // often for one person seen twice.
    let mut rng = Rng(113);
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.75);
    let mut tracks: Vec<TrackIn> = (0..20u32).map(|i| track(0, i * 100, around(&mut rng, &a, 0.8), 0.8)).collect();
    let first: Vec<usize> = (0..20).collect();
    let second = seen_with(&mut rng, &mut tracks, &b, &first, |i| i < 4);
    let o = identify(&tracks, &Params::default());
    twins_are_two(&o, &first, &second, "twins");
}

#[test]
fn a_person_seen_twice_twice_is_still_one() {
    // Two moments in front of a mirror, between the same two looks.
    let mut rng = Rng(127);
    let person = rng.unit();
    let mut tracks = Vec::new();
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 0, 20);
    for k in [3, 11] {
        let at = tracks[l1[k]].start;
        tracks[l2[k]].start = at;
        tracks[l2[k]].end = at + 48;
        tracks[l2[k]].seen = tracks[l1[k]].seen.clone();
    }
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 1);
    let rest: Vec<usize> =
        l1.iter().chain(&l2).copied().filter(|&i| ![l1[3], l1[11], l2[3], l2[11]].contains(&i)).collect();
    one_character(&o, &rest);
}

#[test]
fn twins_seen_more_clearly_together_stay_two_in_a_big_series() {
    // Each episode on its own first: two clear shots of the second twin
    // beside the first, four blurrier ones alone.
    let mut rng = Rng(131);
    let mut tracks: Vec<TrackIn> = (0..4100u32).map(|i| track(100 + i % 8, (i / 8) * 100, rng.unit(), 0.9)).collect();
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.75);
    let (mut twin_a, mut twin_b) = (Vec::new(), Vec::new());
    for m in 0..20u32 {
        let first = tracks.len();
        for k in 0..6u32 {
            tracks.push(track(m, 1_000_000 + k * 100, around(&mut rng, &a, 0.8), 0.8));
        }
        twin_a.extend(first..tracks.len());
        let theirs: Vec<usize> = (first..first + 6).collect();
        let second = seen_with(&mut rng, &mut tracks, &b, &theirs, |i| i < 2);
        for (k, &i) in second.iter().enumerate() {
            tracks[i].embedding = around(&mut rng, &b, if k < 2 { 0.9 } else { 0.65 });
        }
        twin_b.extend(second);
    }
    let o = identify(&tracks, &Params::default());
    twins_are_two(&o, &twin_a, &twin_b, "twins in a series");
}

#[test]
fn pins_and_forbids_win() {
    let mut rng = Rng(3);
    let (_, mut tracks, truth) = cast(&mut rng, 3, 10);
    // The user named person 0 (key 40) by pinning one of its tracks, and
    // said one of person 0's tracks is not them.
    let p0: Vec<usize> = (0..tracks.len()).filter(|&i| truth[i] == 0).collect();
    tracks[p0[0]].pin = Some(40);
    tracks[p0[1]].forbid = vec![40];
    // And moved a track of person 1 to person 2's character (key 41).
    let p1 = (0..tracks.len()).find(|&i| truth[i] == 1).unwrap();
    let p2 = (0..tracks.len()).find(|&i| truth[i] == 2).unwrap();
    tracks[p1].pin = Some(41);
    tracks[p2].pin = Some(41);
    let o = identify(&tracks, &Params::default());
    let c40 = o.characters.iter().position(|c| c.key == Some(40)).unwrap();
    let c41 = o.characters.iter().position(|c| c.key == Some(41)).unwrap();
    assert_eq!(o.tracks[p0[0]].0, Assign::Pinned(c40));
    assert_ne!(o.tracks[p0[1]].0.character(), Some(c40));
    assert_eq!(o.tracks[p0[2]].0, Assign::Auto(c40));
    assert_eq!(o.tracks[p1].0, Assign::Pinned(c41));
    assert_eq!(o.tracks[p2].0, Assign::Pinned(c41));
}

#[test]
fn characters_keep_their_identity_when_episodes_are_added() {
    let mut rng = Rng(5);
    let (centers, mut tracks, _) = cast(&mut rng, 4, 10);
    let first = identify(&tracks, &Params::default());
    // Remember: keys 100.. in the order of the first result.
    for (t, (a, _)) in tracks.iter_mut().zip(&first.tracks) {
        t.previous = a.character().map(|c| 100 + c as u32);
    }
    // A second episode, people in another order.
    for i in 0..10u32 {
        for (p, c) in centers.iter().enumerate().rev() {
            tracks.push(track(1, i * 100 + p as u32, around(&mut rng, c, 0.75), 0.8));
        }
    }
    let second = identify(&tracks, &Params::default());
    for (i, (a, _)) in first.tracks.iter().enumerate() {
        let before = 100 + a.character().unwrap() as u32;
        let after = second.characters[second.tracks[i].0.character().unwrap()].key;
        assert_eq!(after, Some(before));
    }
    // The new episode's tracks joined them.
    assert!(second.tracks[40..].iter().all(|(a, _)| matches!(a, Assign::Auto(_))));
}

#[test]
fn weak_tracks_join_clearly_or_wait() {
    let mut rng = Rng(9);
    let (centers, mut tracks, _) = cast(&mut rng, 3, 10);
    let core = tracks.len();
    // Low quality (not core): close to person 0; between persons 1 and 2; nobody.
    tracks.push(track(0, 90_000, around(&mut rng, &centers[0], 0.6), 0.2));
    let mid: Vec<f32> = centers[1].iter().zip(&centers[2]).map(|(a, b)| a + b).collect();
    let mut mid = around(&mut rng, &mid, 0.9);
    proto_normalize(&mut mid);
    tracks.push(track(0, 91_000, mid, 0.2));
    tracks.push(track(0, 92_000, rng.unit(), 0.2));
    let o = identify(&tracks, &Params::default());
    let person_char = |p: usize| o.tracks[p].0.character().unwrap();
    assert_eq!(o.tracks[core].0, Assign::Auto(person_char(0)));
    assert!(matches!(o.tracks[core + 1].0, Assign::Unsure(c) if c == person_char(1) || c == person_char(2)));
    assert_eq!(o.tracks[core + 2].0, Assign::None);
}

#[test]
fn groups_of_vague_faces_are_not_characters() {
    let mut rng = Rng(17);
    let (_, mut tracks, _) = cast(&mut rng, 2, 10);
    // Masked or blurry faces: alike only through what they lack (≈ 0.4 to a common direction).
    let blur = rng.unit();
    for i in 0..10u32 {
        tracks.push(track(1, i * 100, around(&mut rng, &blur, 0.62), 0.6));
    }
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 2);
    assert!(o.tracks[20..].iter().all(|(a, _)| a.character().is_none()));
}

#[test]
fn a_face_seen_with_a_character_is_not_them() {
    let mut rng = Rng(13);
    let (centers, mut tracks, _) = cast(&mut rng, 2, 10);
    // A weak track of person 0, on screen at the same time as a person-0 core track.
    let at = tracks[0].start + 5;
    tracks.push(track(0, at, around(&mut rng, &centers[0], 0.7), 0.2));
    let o = identify(&tracks, &Params::default());
    let c0 = o.tracks[0].0.character().unwrap();
    assert_ne!(o.tracks.last().unwrap().0.character(), Some(c0));
}

#[test]
fn a_face_back_after_a_gap_leaves_the_gap_free() {
    let mut rng = Rng(19);
    let (centers, mut tracks, _) = cast(&mut rng, 2, 10);
    // Person 0 left and came back: one track, on screen 0..100 and 201..330.
    let mut long = track(1, 0, around(&mut rng, &centers[0], 0.75), 0.8);
    (long.end, long.spans) = (330, vec![(0, 100), (201, 330)]);
    tracks.push(long);
    // A weak view of them in between.
    tracks.push(track(1, 110, around(&mut rng, &centers[0], 0.7), 0.2));
    let o = identify(&tracks, &Params::default());
    let c0 = o.tracks[0].0.character().unwrap();
    assert_eq!(o.tracks.last().unwrap().0, Assign::Auto(c0));
}

#[test]
fn a_character_is_on_screen_once_per_frame() {
    let mut rng = Rng(29);
    let (centers, mut tracks, _) = cast(&mut rng, 2, 10);
    // Two weak faces like person 0 at the same time: only the closer one is them.
    let n = tracks.len();
    tracks.push(track(1, 0, around(&mut rng, &centers[0], 0.7), 0.2));
    tracks.push(track(1, 10, around(&mut rng, &centers[0], 0.85), 0.2));
    let o = identify(&tracks, &Params::default());
    let c0 = o.tracks[0].0.character().unwrap();
    assert_eq!(o.tracks[n + 1].0, Assign::Auto(c0));
    assert_ne!(o.tracks[n].0.character(), Some(c0));
}

#[test]
fn characters_found_to_be_one_say_so() {
    let mut rng = Rng(83);
    let person = rng.unit();
    let other = rng.unit();
    let mut tracks = Vec::new();
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 0, 10);
    for k in 0..6u32 {
        tracks.push(track(1, k * 100, around(&mut rng, &other, 0.8), 0.8));
    }
    // An older grouping made the two looks two characters (keys 100 and
    // 101, the second bigger now), and the other person 102.
    for &i in &l1 {
        tracks[i].previous = Some(100);
    }
    for &i in &l2 {
        tracks[i].previous = Some(101);
        tracks[i].quality = 0.9;
    }
    for t in &mut tracks[20..] {
        t.previous = Some(102);
    }
    let o = identify(&tracks, &Params::default());
    let c = one_character(&o, &[l1.clone(), l2.clone()].concat());
    assert_eq!(o.characters[c].key, Some(101));
    assert_eq!(o.absorbed, [(100, c)], "100 is one with 101 now");
    // Nothing joined: nothing to say.
    let again = identify(&tracks[20..], &Params::default());
    assert!(again.absorbed.is_empty());
    // Placing faces after a decision groups nothing either, whatever
    // characters lost their faces.
    let members = vec![(101, [l1.clone(), l2.clone()].concat()), (102, (20..26).collect())];
    assert!(reassign(&tracks, &members, &Params::default(), None).absorbed.is_empty());
}

#[test]
fn a_reflection_that_was_a_character_is_one_with_its_person() {
    let mut rng = Rng(89);
    let person = rng.unit();
    let mut tracks: Vec<TrackIn> = (0..12u32).map(|i| track(0, i * 100, around(&mut rng, &person, 0.8), 0.8)).collect();
    let mirror = around(&mut rng, &person, 0.85);
    for i in 0..5 {
        let mut t = track(0, tracks[i].start, around(&mut rng, &mirror, 0.9), 0.8);
        t.seen = tracks[i].seen.clone();
        t.previous = Some(9);
        tracks.push(t);
    }
    for t in &mut tracks[..12] {
        t.previous = Some(8);
    }
    let o = identify(&tracks, &Params::default());
    let c = one_character(&o, &(0..12).collect::<Vec<_>>());
    assert_eq!(o.characters[c].key, Some(8));
    assert_eq!(o.absorbed, [(9, c)], "its scenepack goes to them");
}

#[test]
fn small_groups_of_one_person_merge_despite_one_mirror() {
    let mut rng = Rng(97);
    let person = rng.unit();
    let mut tracks = Vec::new();
    // Six faces per look: a tenth of the smaller group is less than one face,
    // yet one face may always step aside.
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 0, 6);
    let at = tracks[l1[2]].start;
    tracks[l2[2]].start = at;
    tracks[l2[2]].end = at + 48;
    tracks[l2[2]].seen = tracks[l1[2]].seen.clone();
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.characters.len(), 1);
    let rest: Vec<usize> = l1.iter().chain(&l2).copied().filter(|&i| i != l1[2] && i != l2[2]).collect();
    one_character(&o, &rest);
}

#[test]
fn look_alikes_stay_two_when_one_of_them_is_seen_less_clearly() {
    let mut rng = Rng(103);
    // One actor in two roles (mean faces 0.85 alike): one role always clear
    // (its faces 0.81 alike), the other often blurry (0.51); the blurry one
    // on screen with the clear one in 12 of its 20 shots.
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.85);
    let mut tracks: Vec<TrackIn> = (0..20u32).map(|i| track(0, i * 100, around(&mut rng, &a, 0.9), 0.8)).collect();
    let clear: Vec<usize> = (0..20).collect();
    let blurry = seen_with(&mut rng, &mut tracks, &b, &clear, |i| i < 12);
    for &i in &blurry {
        tracks[i].embedding = around(&mut rng, &b, 0.72);
    }
    let o = identify(&tracks, &Params::default());
    let ca = one_character(&o, &clear);
    let theirs = blurry.iter().filter(|&&i| o.tracks[i].0.character().is_some_and(|c| c != ca)).count();
    assert!(theirs >= 15, "the second role keeps its faces: {theirs} of 20");
}

#[test]
fn a_noisy_group_does_not_make_look_alikes_copies() {
    let mut rng = Rng(107);
    // Look-alikes (mean faces 0.85): one seen clearly (faces 0.81 alike),
    // the other blurry (0.51), on screen with the first in 12 of its 13
    // shots. Their faces side by side are as alike as the blurry one's
    // faces are to each other — but far less than the clear one's: two
    // people, whose groups must not merge.
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.85);
    let mut tracks: Vec<TrackIn> = (0..20u32).map(|i| track(0, i * 100, around(&mut rng, &a, 0.9), 0.8)).collect();
    let clear: Vec<usize> = (0..20).collect();
    let blurry = seen_with(&mut rng, &mut tracks, &b, &clear[..13], |i| i < 12);
    for &i in &blurry {
        tracks[i].embedding = around(&mut rng, &b, 0.714);
    }
    let core: Vec<usize> = (0..tracks.len()).collect();
    let groups = cluster(&tracks, &core, &Params::default());
    let group_of = |t: usize| groups.iter().position(|g| g.contains(&t)).unwrap();
    let theirs = group_of(blurry[0]);
    assert!(blurry[..12].iter().all(|&t| group_of(t) == theirs), "their faces seen with the other stay together");
    assert_ne!(theirs, group_of(clear[0]));
    assert!(groups[theirs].len() >= 12);
}

#[test]
fn characters_keep_their_identity_when_they_grow() {
    let mut rng = Rng(31);
    let (centers, mut tracks, _) = cast(&mut rng, 2, 5);
    let first = identify(&tracks, &Params::default());
    for (t, (a, _)) in tracks.iter_mut().zip(&first.tracks) {
        t.previous = a.character().map(|c| 100 + c as u32);
    }
    let key0 = 100 + first.tracks[0].0.character().unwrap() as u32;
    // A small part in episode 1, a big one in episode 2.
    for i in 0..30u32 {
        tracks.push(track(1, i * 100, around(&mut rng, &centers[0], 0.75), 0.8));
    }
    let second = identify(&tracks, &Params::default());
    let c = second.tracks[0].0.character().unwrap();
    assert_eq!(second.characters[c].key, Some(key0));
    assert!(second.tracks[10..].iter().all(|(a, _)| a.character() == Some(c)));
}

#[test]
fn a_person_seen_once_per_episode_is_found_in_a_big_series() {
    // More clear tracks than one clustering level takes: a person with one
    // track per episode is a single track in each episode's first level.
    // Strangers are random directions in the embedding's 512 dimensions
    // (in fewer, chance alone makes them look alike).
    let mut rng = Rng(37);
    let person = rng.unit();
    let mut tracks: Vec<TrackIn> = (0..4100u32).map(|i| track(i % 8, (i / 8) * 100, rng.unit(), 0.9)).collect();
    let first = tracks.len();
    for m in 0..8u32 {
        let noise = rng.unit();
        let mut e: Vec<f32> = person.iter().zip(&noise).map(|(c, n)| 0.9 * c + 0.436 * n).collect();
        proto_normalize(&mut e);
        tracks.push(track(m, 1_000_000, e, 0.6));
    }
    let o = identify(&tracks, &Params::default());
    let c = o.tracks[first].0.character();
    assert!(c.is_some(), "the person is no character");
    assert!(o.tracks[first..].iter().all(|(a, _)| a.character() == c));
}

/// The groups of `tracks` (all of them clear), clustered per media from
/// `single` groups on and in runs of at most `second` groups after that,
/// and the most groups a run took.
fn cluster_capped(tracks: &[TrackIn], single: usize, second: usize) -> (Vec<Vec<usize>>, usize) {
    let core: Vec<usize> = (0..tracks.len()).collect();
    crate::ahc::BIGGEST_RUN.with(|b| b.set(0));
    let groups = crate::ahc::cluster_in(tracks, &core, &Params::default(), crate::ahc::Levels { single, second });
    (groups, crate::ahc::BIGGEST_RUN.with(|b| b.get()))
}

#[test]
fn a_media_longer_than_one_run_takes_is_grouped_in_stretches_of_time() {
    // Six people one after another in one media, 30 tracks each, at most 60
    // groups a run: three stretches of 60 tracks, each with 10 of everyone's,
    // whose groups then meet as episodes do.
    let mut rng = Rng(139);
    let (_, tracks, truth) = cast(&mut rng, 6, 30);
    let (groups, biggest) = cluster_capped(&tracks, 40, 60);
    assert!(biggest <= 60, "a run of {biggest} groups");
    let people: Vec<Vec<usize>> = (0..6).map(|p| (0..tracks.len()).filter(|&i| truth[i] == p).collect()).collect();
    let mut found = groups.clone();
    found.sort();
    assert_eq!(found, people, "everyone is one group");
    // Just as one run would group them.
    let (whole, biggest) = cluster_capped(&tracks, 40, usize::MAX);
    assert_eq!(biggest, tracks.len());
    assert_eq!(whole, groups);
}

#[test]
fn faces_seen_together_on_either_side_of_a_cut_stay_two() {
    // Twins (mean faces 0.8 alike) on screen together in 90 shots of one
    // media, after somebody else's face: at most 60 groups a run makes
    // stretches of 46 tracks, and the first cut falls between the twins of
    // shot 22.
    let mut rng = Rng(149);
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.8);
    let mut tracks = vec![track(0, 0, rng.unit(), 0.8)];
    let (mut twin_a, mut twin_b) = (Vec::new(), Vec::new());
    for i in 0..90u32 {
        twin_a.push(tracks.len());
        tracks.push(track(0, i * 100 + 50, around(&mut rng, &a, 0.8), 0.8));
        twin_b.push(tracks.len());
        tracks.push(track(0, i * 100 + 60, around(&mut rng, &b, 0.8), 0.8));
    }
    let (groups, biggest) = cluster_capped(&tracks, 40, 60);
    assert!(biggest <= 60, "a run of {biggest} groups");
    let group_of = |t: usize| groups.iter().position(|g| g.contains(&t)).unwrap();
    assert_ne!(group_of(twin_a[22]), group_of(twin_b[22]), "the twins of the shot that was cut");
    for g in &groups {
        for (x, &i) in g.iter().enumerate() {
            for &j in &g[x + 1..] {
                assert!(!together(&tracks[i], &tracks[j]), "{i} and {j} are on screen at once");
            }
        }
    }
    assert!(twin_a.iter().all(|&t| group_of(t) == group_of(twin_a[0])), "the first twin is one group");
    assert!(twin_b.iter().all(|&t| group_of(t) == group_of(twin_b[0])), "the second twin is one group");
}

#[test]
fn a_media_too_long_for_one_run_is_grouped_within_the_bound() {
    // More clear tracks in one media than a run takes (a season in one
    // file, a long recording): strangers one after another, and now and
    // then a person, in two stretches of time.
    let mut rng = Rng(151);
    let person = rng.unit();
    let mut tracks = Vec::new();
    let mut theirs = Vec::new();
    for i in 0..6100u32 {
        if i % 200 == 0 {
            theirs.push(tracks.len());
            tracks.push(track(0, i * 100, around(&mut rng, &person, 0.75), 0.8));
        } else {
            tracks.push(track(0, i * 100, rng.unit(), 0.9));
        }
    }
    let core: Vec<usize> = (0..tracks.len()).collect();
    crate::ahc::BIGGEST_RUN.with(|b| b.set(0));
    let groups = cluster(&tracks, &core, &Params::default());
    let biggest = crate::ahc::BIGGEST_RUN.with(|b| b.get());
    assert!(biggest <= 6000, "a run of {biggest} groups");
    let mut theirs_found = groups[0].clone();
    theirs_found.sort_unstable();
    assert_eq!(theirs_found, theirs, "the person is one group");
}

/// What identify made, as `reassign` takes it: each character's members.
fn members_of(o: &Outcome) -> Vec<(CharKey, Vec<usize>)> {
    (0..o.characters.len())
        .map(|ci| {
            let g = (0..o.tracks.len())
                .filter(|&i| matches!(o.tracks[i], (Assign::Pinned(c), _) | (Assign::Auto(c), 1.0) if c == ci))
                .collect();
            (o.characters[ci].key.unwrap_or(ci as CharKey), g)
        })
        .collect()
}

#[test]
fn reassigning_keeps_the_cast_and_places_everyone_else() {
    let mut rng = Rng(21);
    let (centers, mut tracks, truth) = cast(&mut rng, 4, 10);
    // Weak faces of each person, and a stranger seen often (a group of its
    // own, which only clustering may turn into a character).
    let mut weak = Vec::new();
    for (p, c) in centers.iter().enumerate() {
        for k in 0..3u32 {
            weak.push((tracks.len(), p));
            tracks.push(track(1, 10_000 + (p as u32 * 3 + k) * 100, around(&mut rng, c, 0.75), 0.2));
        }
    }
    let stranger = rng.unit();
    let strangers: Vec<usize> = (0..6u32)
        .map(|k| {
            tracks.push(track(2, k * 100, around(&mut rng, &stranger, 0.8), 0.2));
            tracks.len() - 1
        })
        .collect();
    let o = identify(&tracks[..truth.len()], &Params::default());
    let members = members_of(&o);
    let r = reassign(&tracks, &members, &Params::default(), None);

    assert_eq!(r.characters.len(), 4, "no character made");
    for (ci, (key, g)) in members.iter().enumerate() {
        assert_eq!(r.characters[ci].key, Some(*key));
        assert!(g.iter().all(|&i| r.tracks[i] == (Assign::Auto(ci), 1.0)), "members stay");
    }
    // Each weak face joins its person (characters keep their order).
    let person = |p: usize| o.tracks[truth.iter().position(|&t| t == p).unwrap()].0.character().unwrap();
    for &(i, p) in &weak {
        assert_eq!(r.tracks[i].0, Assign::Auto(person(p)), "weak face {i}");
    }
    assert!(strangers.iter().all(|&i| r.tracks[i].0 == Assign::None));
}

#[test]
fn not_them_on_a_member_takes_its_look_alikes_along() {
    let mut rng = Rng(23);
    let a = rng.unit();
    let b = rng.unit();
    // Character 7 is person a — and, by mistake, one face of person b.
    let mut tracks: Vec<TrackIn> = (0..6u32).map(|k| track(0, k * 100, around(&mut rng, &a, 0.8), 0.9)).collect();
    tracks.push(track(0, 600, around(&mut rng, &b, 0.9), 0.9));
    let wrong = tracks.len() - 1;
    // More faces of person b, nobody's yet.
    let alikes: Vec<usize> = (0..4u32)
        .map(|k| {
            tracks.push(track(1, k * 100, around(&mut rng, &b, 0.9), 0.3));
            tracks.len() - 1
        })
        .collect();
    let members = vec![(7, (0..=wrong).collect::<Vec<_>>())];
    let p = Params::default();
    let before = reassign(&tracks, &members, &p, None);
    assert!(alikes.iter().all(|&i| before.tracks[i].0 == Assign::Auto(0)), "they follow the wrong face");

    tracks[wrong].forbid = vec![7];
    let after = reassign(&tracks, &members, &p, None);
    assert_eq!(after.tracks[wrong].0, Assign::None);
    assert!(alikes.iter().all(|&i| after.tracks[i].0.character().is_none()), "look-alikes leave with it");
    assert!((0..wrong).all(|i| after.tracks[i] == (Assign::Auto(0), 1.0)));
}

#[test]
fn reassigning_never_moves_a_pin_or_breaks_a_forbid() {
    let mut rng = Rng(29);
    let (_, mut tracks, truth) = cast(&mut rng, 3, 8);
    let o = identify(&tracks, &Params::default());
    let members = members_of(&o);
    // A face of person 0 the user put on person 1's character, and one they
    // said isn't person 0's character.
    let p0: Vec<usize> = (0..tracks.len()).filter(|&i| truth[i] == 0).collect();
    let c0 = o.tracks[p0[2]].0.character().unwrap();
    let c1 = o.tracks[(0..tracks.len()).find(|&i| truth[i] == 1).unwrap()].0.character().unwrap();
    let (k0, k1) = (members[c0].0, members[c1].0);
    tracks[p0[0]].pin = Some(k1);
    tracks[p0[1]].forbid = vec![k0];
    let mut members = members.clone();
    for (key, g) in &mut members {
        g.retain(|&i| i != p0[0] && i != p0[1]);
        if *key == k1 {
            g.push(p0[0]);
        }
    }
    let r = reassign(&tracks, &members, &Params::default(), None);
    let at = |k: CharKey| r.characters.iter().position(|c| c.key == Some(k)).unwrap();
    assert_eq!(r.tracks[p0[0]].0, Assign::Pinned(at(k1)));
    assert_ne!(r.tracks[p0[1]].0.character(), Some(at(k0)));
    assert!(!matches!(r.tracks[p0[1]].0, Assign::Unsure(c) if c == at(k0)));
}

/// `people` characters drawn in one style: every face is `style` alike to
/// a look they all share, two of them on screen at once in every scene.
fn drawn(rng: &mut Rng, people: usize, scenes: u32, style: f32) -> (Vec<TrackIn>, Vec<usize>) {
    let shared = rng.unit();
    let centers: Vec<Vec<f32>> = (0..people).map(|_| around(rng, &shared, style)).collect();
    let mut tracks = Vec::new();
    let mut truth = Vec::new();
    for k in 0..scenes {
        for p in [k as usize % people, (k as usize + 1) % people] {
            tracks.push(track(0, k * 100, around(rng, &centers[p], 0.85), 0.8));
            truth.push(p);
        }
    }
    (tracks, truth)
}

#[test]
fn characters_drawn_in_one_style_stay_apart() {
    let mut rng = Rng(23);
    // Two characters' faces 0.35 alike on average: actors' bars would
    // merge them; the faces seen side by side tell how alike strangers look.
    let (tracks, truth) = drawn(&mut rng, 6, 90, 0.7);
    let alike = strangers(&tracks, &Params::default()).unwrap();
    assert!((0.25..0.45).contains(&alike), "strangers {alike}");
    let fixed = Params { calibrate_from: usize::MAX, ..Params::default() };
    let before = bcubed(&chars(&identify(&tracks, &fixed)), &truth);
    let o = identify(&tracks, &Params::default());
    let f = bcubed(&chars(&o), &truth);
    assert_eq!(o.characters.len(), 6);
    assert!(f > 0.97 && before < 0.8, "fitted F {f}, actors' bars F {before}");
    // A decision keeps to the same bars.
    let members: Vec<(CharKey, Vec<usize>)> = (0..6)
        .map(|c| (c as CharKey, (0..tracks.len()).filter(|&i| o.tracks[i].0 == Assign::Auto(c)).collect()))
        .collect();
    assert!(bcubed(&chars(&reassign(&tracks, &members, &Params::default(), None)), &truth) > 0.97);
}

#[test]
fn actors_keep_the_bars_they_were_set_for() {
    let mut rng = Rng(29);
    // Random people: strangers about 0 alike.
    let (tracks, _) = drawn(&mut rng, 6, 90, 0.0);
    let p = Params::default();
    assert!(strangers(&tracks, &p).unwrap().abs() < 0.05);
    let f = p.fitted(&tracks);
    assert_eq!(
        (f.link, f.assign, f.unsure, f.margin, f.copy_link),
        (p.link, p.assign, p.unsure, p.margin, p.copy_link)
    );
    // Too few faces seen at once to tell: as set.
    let (few, _) = drawn(&mut rng, 6, 20, 0.7);
    assert!(strangers(&few, &p).is_none());
    assert_eq!(p.fitted(&few).link, p.link);
}

#[test]
fn footage_said_to_be_animation_is_fitted_however_little_strangers_look_alike() {
    let mut rng = Rng(31);
    // A style whose strangers are ~0.1 alike: in the ramp, half moved.
    let (tracks, _) = drawn(&mut rng, 6, 90, 0.45);
    let p = Params::default();
    let alike = strangers(&tracks, &p).unwrap();
    assert!((0.06..0.14).contains(&alike), "strangers {alike}");
    let auto = p.fitted(&tracks);
    let said = Params { animated: true, ..p }.fitted(&tracks);
    assert!(p.link < auto.link && auto.link < said.link, "{} {} {}", p.link, auto.link, said.link);
    assert_eq!(said.link, p.for_strangers(alike).link);
    // Too few faces seen at once to tell: as set, animation or not.
    let (few, _) = drawn(&mut rng, 6, 20, 0.45);
    assert_eq!(Params { animated: true, ..p }.fitted(&few).link, p.link);
}

// -- the face space fitted to the footage (adapt) ----------------------------

/// A cartoon: `scenes` scenes of two of `people` characters drawn in one
/// style (strangers about `style` alike), every face clear.
fn cartoon(seed: u64, people: usize, scenes: u32, style: f32) -> (Vec<TrackIn>, Vec<usize>) {
    drawn(&mut Rng(seed), people, scenes, style)
}

#[test]
fn scatters_count_every_pair_of_faces() {
    use faer::Mat;
    let mut rng = Rng(41);
    let (n, dim, groups) = (40, 6, 7);
    let x = Mat::<f64>::from_fn(n, dim, |_, _| rng.gauss() as f64);
    let group: Vec<usize> = (0..n).map(|i| (i * 5 + i / 3) % groups).collect();
    // Seen together: each pair of groups once, both ways.
    let mut adjacent = vec![Vec::new(); groups];
    for (a, b) in [(0, 1), (0, 3), (2, 5), (4, 6), (1, 6)] {
        adjacent[a].push(b);
        adjacent[b].push(a);
    }
    let (cs, cd, same, other) = adapt::scatters(&x, &group, &adjacent);
    let (mut bs, mut bd) = (vec![0f64; dim * dim], vec![0f64; dim * dim]);
    let (mut ns, mut nd) = (0.0, 0.0);
    for i in 0..n {
        for j in i + 1..n {
            let (into, count) = if group[i] == group[j] {
                (&mut bs, &mut ns)
            } else if adjacent[group[i]].contains(&group[j]) {
                (&mut bd, &mut nd)
            } else {
                continue;
            };
            *count += 1.0;
            for r in 0..dim {
                for c in 0..dim {
                    into[r * dim + c] += (x[(i, r)] - x[(j, r)]) * (x[(i, c)] - x[(j, c)]);
                }
            }
        }
    }
    assert_eq!((same, other), (ns, nd));
    for r in 0..dim {
        for c in 0..dim {
            assert!((cs[(r, c)] - bs[r * dim + c] / ns).abs() < 1e-10, "C_S {r} {c}");
            assert!((cd[(r, c)] - bd[r * dim + c] / nd).abs() < 1e-10, "C_D {r} {c}");
        }
    }
}

#[test]
fn quantiles_interpolate_like_numpy() {
    let s = [0.1, 0.2, 0.4, 0.8];
    assert!((adapt::quantile(&s, 0.95) - 0.74).abs() < 1e-12);
    assert!((adapt::quantile(&s, 0.5) - 0.3).abs() < 1e-12);
    assert_eq!(adapt::quantile(&[0.7], 0.95), 0.7);
    assert!((adapt::quantile(&[0.2, 0.6], 0.25) - 0.3).abs() < 1e-12);
}

#[test]
fn whitening_keeps_only_the_directions_the_faces_show() {
    use faer::Mat;
    let mut rng = Rng(43);
    let dim = 10;
    // A full-rank spread of one person; three directions in which people differ.
    let a = Mat::<f64>::from_fn(dim, dim, |_, _| rng.gauss() as f64);
    let b = Mat::<f64>::from_fn(dim, 3, |_, _| rng.gauss() as f64);
    let cs = &a * a.transpose();
    let cd = &b * b.transpose();
    let p = adapt::whitening(cs.clone(), &cd).unwrap();
    assert_eq!(p.ncols(), 3);
    // Pᵀ (C_S + ridge) P = I, largest spread of different people first.
    let trace: f64 = (0..dim).map(|i| cs[(i, i)]).sum();
    let ridged =
        Mat::<f64>::from_fn(dim, dim, |r, c| cs[(r, c)] + if r == c { 0.05 * trace / dim as f64 } else { 0.0 });
    let id = p.transpose() * &ridged * &p;
    let spread = p.transpose() * &cd * &p;
    for r in 0..3 {
        for c in 0..3 {
            let want = if r == c { 1.0 } else { 0.0 };
            assert!((id[(r, c)] - want).abs() < 1e-9, "{r} {c}: {}", id[(r, c)]);
        }
    }
    assert!(spread[(0, 0)] >= spread[(1, 1)] && spread[(1, 1)] >= spread[(2, 2)]);
}

#[test]
fn live_action_and_too_few_faces_seen_together_keep_arcfaces_space() {
    let p = Params::default();
    // Actors: strangers about 0 alike.
    let (actors, _) = cartoon(47, 6, 250, 0.0);
    assert!(fit(&actors, &p).is_none());
    let o = identify(&actors, &p);
    assert_eq!(o.space, Space::Faces);
    let today = identify(&actors, &Params { adapt: false, ..p });
    assert_eq!(o.tracks, today.tracks);
    // A cartoon with fewer than 200 pairs of faces seen at once.
    let (few, _) = cartoon(53, 6, MIN_STRANGERS as u32 - 1, 0.7);
    assert!(fit(&few, &p).is_none());
    assert_eq!(identify(&few, &p).space, Space::Faces);
    // Enough of them.
    let (film, truth) = cartoon(53, 6, 250, 0.7);
    let (f, st) = fit(&film, &p).expect("a cartoon gets its own space");
    assert_eq!((st.strangers, st.share), (250, 1.0));
    assert!(f.k >= 1 && f.k <= 64 && (f.weight - 0.35).abs() < 1e-6);
    let o = identify(&film, &p);
    assert_eq!(o.space, Space::Fitted(f));
    assert_eq!(o.characters.len(), 6);
    assert!(bcubed(&chars(&o), &truth) > 0.97);
}

#[test]
fn footage_said_to_be_animation_gets_its_space_however_little_alike() {
    let p = Params::default();
    // Strangers about 0.09 alike: in the ramp.
    let (tracks, _) = cartoon(59, 6, 250, 0.35);
    let (_, st) = fit(&tracks, &p).unwrap();
    assert!(st.share > 0.0 && st.share < 1.0, "share {}", st.share);
    let (f, st) = fit(&tracks, &Params { animated: true, ..p }).unwrap();
    assert_eq!(st.share, 1.0);
    assert!((f.weight - 0.35).abs() < 1e-6);
}

#[test]
fn the_bars_follow_how_alike_strangers_still_look() {
    let p = Params::default();
    let (tracks, _) = cartoon(61, 6, 250, 0.7);
    let (f, st) = fit(&tracks, &p).unwrap();
    let o = (st.share * ((st.tail - p.link) / (1.0 - p.link)).max(0.0)).clamp(0.0, 0.5);
    let oc = (2.5 * o).min(0.5);
    assert!((f.link - (p.link + o * (1.0 - p.link))).abs() < 1e-6);
    assert!((f.copy_link - (p.copy_link + oc * (1.0 - p.copy_link))).abs() < 1e-6);
    assert!((f.copy_alike - (p.copy_alike + oc * (1.0 - p.copy_alike))).abs() < 1e-6);
    // assign, margin and min_cohesion stay; unsure goes with link.
    let q = f.params(&p);
    assert_eq!((q.assign, q.margin, q.min_cohesion, q.unsure), (p.assign, p.margin, p.min_cohesion, f.link));
}

#[test]
fn the_fit_is_the_same_in_any_order() {
    let p = Params::default();
    let (tracks, _) = cartoon(67, 6, 250, 0.7);
    let (f, _) = fit(&tracks, &p).unwrap();
    let mut order: Vec<usize> = (0..tracks.len()).collect();
    let mut rng = Rng(71);
    for i in (1..order.len()).rev() {
        order.swap(i, (rng.next() * (i + 1) as f32) as usize % (i + 1));
    }
    let shuffled: Vec<TrackIn> = order.iter().map(|&i| tracks[i].clone()).collect();
    let (g, _) = fit(&shuffled, &p).unwrap();
    assert_eq!(f.k, g.k);
    assert!((f.link - g.link).abs() < 1e-5 && (f.copy_link - g.copy_link).abs() < 1e-5);
    let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| p * q).sum::<f32>();
    for (a, b) in [(0, 1), (2, 40), (7, 300), (100, 499)] {
        let (za, zb) = (f.embed(&tracks[a].embedding), f.embed(&tracks[b].embedding));
        let (ya, yb) = (g.embed(&tracks[a].embedding), g.embed(&tracks[b].embedding));
        assert!((dot(&za, &zb) - dot(&ya, &yb)).abs() < 1e-5, "{a} {b}");
    }
}

#[test]
fn an_unreadable_face_spoils_nothing() {
    let p = Params::default();
    let (mut tracks, _) = cartoon(73, 6, 250, 0.7);
    let (f, _) = fit(&tracks, &p).unwrap();
    let mut broken = tracks[3].clone();
    broken.embedding[17] = f32::NAN;
    tracks.push(broken);
    let (g, _) = fit(&tracks, &p).unwrap();
    assert_eq!(f, g);
    let o = identify(&tracks, &p);
    assert_eq!(o.tracks.last().unwrap().0, Assign::None);
    assert!(o.characters.iter().all(|c| c.centroid.iter().all(|x| x.is_finite())));
}

#[test]
fn the_users_decisions_dont_shape_the_space() {
    let p = Params::default();
    let (tracks, _) = cartoon(79, 6, 250, 0.7);
    let (f, _) = fit(&tracks, &p).unwrap();
    let mut decided = tracks.clone();
    for (i, t) in decided.iter_mut().enumerate().take(60) {
        match i % 3 {
            0 => t.pin = Some(i as CharKey % 4),
            1 => t.forbid = vec![2],
            _ => t.quality = 0.5,
        }
        t.previous = Some(1);
    }
    let (g, _) = fit(&decided, &p).unwrap();
    assert_eq!(f, g);
}

#[test]
fn a_stored_space_comes_back_as_it_was() {
    let (tracks, _) = cartoon(83, 6, 250, 0.7);
    let (f, _) = fit(&tracks, &Params::default()).unwrap();
    let s = Space::Fitted(f);
    assert_eq!(Space::from_bytes(&s.to_bytes()), Some(s.clone()));
    assert_eq!(Space::from_bytes(&Space::Faces.to_bytes()), Some(Space::Faces));
    let b = s.to_bytes();
    assert_eq!(Space::from_bytes(&b[..b.len() - 1]), None);
    assert_eq!(Space::from_bytes(&[b.as_slice(), &[0]].concat()), None, "a byte too many");
    assert_eq!(Space::from_bytes(b"something else"), None);
    // A weight no fit makes (it would turn every face into NaN).
    let mut w = b.clone();
    w[16..20].copy_from_slice(&1.25f32.to_le_bytes());
    assert_eq!(Space::from_bytes(&w), None);
    // A size that doesn't fit in memory.
    let mut huge = b[..16].to_vec();
    huge[8..16].copy_from_slice(&[0xff; 8]);
    assert_eq!(Space::from_bytes(&huge), None);
}

#[test]
fn decisions_place_faces_in_the_space_the_cast_was_grouped_in() {
    let p = Params::default();
    let (mut tracks, truth) = cartoon(89, 6, 250, 0.7);
    let o = identify(&tracks, &p);
    let Space::Fitted(f) = o.space.clone() else { panic!("a cartoon gets its own space") };
    // A face the user says is no face leaves; the others stay where they were.
    let gone = (0..tracks.len()).find(|&i| matches!(o.tracks[i].0, Assign::Auto(_))).unwrap();
    let mut members: Vec<(CharKey, Vec<usize>)> = (0..o.characters.len())
        .map(|c| (c as CharKey, (0..tracks.len()).filter(|&i| i != gone && o.tracks[i].0 == Assign::Auto(c)).collect()))
        .collect();
    let pinned = members[0].1[0];
    tracks[pinned].pin = Some(0);
    tracks[gone].embedding = vec![f32::NAN; tracks[gone].embedding.len()];
    let r = reassign(&tracks, &members, &p, Some(&o.space));
    assert_eq!(r.space, Space::Fitted(f));
    assert_eq!(r.tracks[pinned].0, Assign::Pinned(0));
    let (before, after) = (chars(&o), chars(&r));
    let moved = (0..tracks.len()).filter(|&i| i != gone && before[i] != after[i]).count();
    assert!(moved <= 2, "{moved} faces moved");
    assert!(bcubed(&after, &truth) > 0.97);
    // A face the user said isn't character 1 never goes there, in this space too.
    let target = members[1].1.remove(0);
    tracks[target].forbid = vec![1];
    let r = reassign(&tracks, &members, &p, Some(&o.space));
    assert_ne!(r.tracks[target].0.character().and_then(|c| r.characters[c].key), Some(1));
}

// -- the anime preset ---------------------------------------------------------

#[test]
fn the_anime_preset_carries_ccips_bars() {
    let p = Params::anime();
    assert_eq!((p.link, p.copy_link), (0.55, 0.70));
    assert_eq!((p.core_quality, p.strangers_alone, p.strangers_style), (0.50, 0.02, 0.25));
    assert_eq!((p.table_dedup_cos, p.merge_top, p.cl_cap), (0.88, 4, 0.25));
    assert!(!p.animated && p.adapt);
    // The rest stays the live-action pass's.
    let d = Params::default();
    assert_eq!(
        (p.assign, p.margin, p.unsure, p.prototypes, p.min_cohesion, p.copy_share),
        (d.assign, d.margin, d.unsure, d.prototypes, d.min_cohesion, d.copy_share)
    );
}

#[test]
fn the_fitted_spaces_share_in_the_mix_is_capped() {
    let p = Params::default();
    let (tracks, _) = cartoon(97, 6, 250, 0.7);
    // Uncapped (1.0), as ever: the fitted space's share is the mix's.
    let (full, _) = fit(&tracks, &Params { animated: true, ..p }).unwrap();
    assert!((full.weight - 0.35).abs() < 1e-6);
    let (capped, _) = fit(&tracks, &Params { animated: true, cl_cap: 0.25, ..p }).unwrap();
    assert!((capped.weight - 0.25).abs() < 1e-6);
    // Same axes, only their share.
    assert_eq!(capped.projection, full.projection);
}

#[test]
fn faces_of_another_embedders_dimension_compare_to_no_one() {
    let mut rng = Rng(101);
    let (centers, mut tracks, _) = cast(&mut rng, 3, 10);
    // A track of another embedder's length among the pass's: it can't
    // compare to anyone and shapes nothing.
    let mut foreign = track(0, 9000, around(&mut rng, &centers[0], 0.8), 0.9);
    foreign.embedding = vec![0.5; 4];
    tracks.push(foreign);
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.tracks.last().unwrap().0, Assign::None);
    assert!(o.characters.iter().all(|c| c.centroid.len() == DIM));
    // Most tracks of another length: the session is theirs — the odd one out.
    for t in &mut tracks {
        t.embedding = vec![8f32.sqrt().recip(); 8];
    }
    tracks[0].embedding = vec![0.01; DIM];
    let o = identify(&tracks, &Params::default());
    assert_eq!(o.tracks[0].0, Assign::None);
    assert!(o.tracks[1..].iter().all(|(a, _)| a.character().is_some()));
    // Placing faces after a decision leaves it out too.
    let members = vec![(5, (1..tracks.len()).collect())];
    assert_eq!(reassign(&tracks, &members, &Params::default(), None).tracks[0].0, Assign::None);
}

#[test]
fn stranger_pairs_weigh_by_the_less_seen_face() {
    let mut rng = Rng(103);
    let p = Params::default();
    // One pair of clear tracks on screen at a time, never two pairs at
    // once: 45 of strangers (~0 alike), 55 of look-alikes (~0.9) nobody
    // sees much of.
    let mut tracks = Vec::new();
    for i in 0..100u32 {
        let a = rng.unit();
        let (cos, w) = if i < 45 { (0.02, 1.0) } else { (0.9, 0.01) };
        let mut ta = track(0, i * 100, a.clone(), 0.8);
        let mut tb = track(0, i * 100, around(&mut rng, &a, cos), 0.8);
        ta.weight = w;
        tb.weight = w;
        tracks.extend([ta, tb]);
    }
    // The little-seen pairs barely count: the median pair is a stranger's.
    assert!(strangers(&tracks, &p).unwrap() < 0.4, "weighed");
    // Every pair the same: the median pair is a look-alike's.
    for t in &mut tracks {
        t.weight = 1.0;
    }
    assert!(strangers(&tracks, &p).unwrap() > 0.6, "unweighed");
}

#[test]
fn a_merge_never_sends_more_faces_aside_than_merge_top() {
    let mut rng = Rng(107);
    let person = rng.unit();
    let mut tracks = Vec::new();
    let [l1, l2] = two_looks(&mut rng, &mut tracks, &person, 0, 20);
    // The same person seen twice at three moments: the merge asks three
    // faces to step aside — copy_share of twenty allows it, a top of two
    // doesn't.
    for k in [3, 5, 7] {
        let at = tracks[l1[k]].start;
        tracks[l2[k]].start = at;
        tracks[l2[k]].end = at + 48;
        tracks[l2[k]].seen = tracks[l1[k]].seen.clone();
    }
    assert_eq!(identify(&tracks, &Params::default()).characters.len(), 1);
    let capped = Params { merge_top: 2, ..Params::default() };
    assert_eq!(identify(&tracks, &capped).characters.len(), 2);
}

#[test]
fn a_look_too_alike_gets_no_prototype_of_its_own() {
    let mut rng = Rng(109);
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.93);
    // Members ~0.93 alike: a second centre at the live bar (0.98), not at
    // the anime one (0.88) — the same look there.
    let members: Vec<(&[f32], f32)> = [&a, &b, &a, &b].iter().map(|e| (e.as_slice(), 1.0)).collect();
    assert_eq!(proto::prototypes(&members, 12, 0.98).len(), 2);
    assert_eq!(proto::prototypes(&members, 12, 0.88).len(), 1);
}
