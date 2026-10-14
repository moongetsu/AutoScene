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
        let mut v: Vec<f32> = (0..DIM).map(|_| self.gauss()).collect();
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
    let noise = rng.unit();
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
        embedding,
        quality,
        dets: 10,
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
    let n = truth.len();
    let (mut p, mut r) = (0.0, 0.0);
    for i in 0..n {
        let same_cluster: Vec<usize> =
            (0..n).filter(|&j| assigned[i].is_some() && assigned[j] == assigned[i] || j == i).collect();
        let same_truth = (0..n).filter(|&j| truth[j] == truth[i]).count();
        let correct = same_cluster.iter().filter(|&&j| truth[j] == truth[i]).count();
        p += correct as f32 / same_cluster.len() as f32;
        r += correct as f32 / same_truth as f32;
    }
    let (p, r) = (p / n as f32, r / n as f32);
    2.0 * p * r / (p + r)
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
fn people_on_screen_together_are_never_merged() {
    let mut rng = Rng(11);
    // Two look-alikes (their centres are 0.8 alike), always in the same shots.
    let a = rng.unit();
    let b = around(&mut rng, &a, 0.8);
    let mut tracks = Vec::new();
    let mut truth = Vec::new();
    for i in 0..12u32 {
        tracks.push(track(0, i * 100, around(&mut rng, &a, 0.8), 0.8));
        tracks.push(track(0, i * 100 + 10, around(&mut rng, &b, 0.8), 0.8));
        truth.extend([0, 1]);
    }
    let o = identify(&tracks, &Params::default());
    let got = chars(&o);
    for i in 0..12 {
        assert_ne!(got[2 * i], got[2 * i + 1], "pair {i} merged");
    }
    assert!(bcubed(&got, &truth) > 0.9);
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
    let mut rng = Rng(37);
    let unit = |rng: &mut Rng| {
        let mut v: Vec<f32> = (0..96).map(|_| rng.gauss()).collect();
        proto_normalize(&mut v);
        v
    };
    let person = unit(&mut rng);
    let mut tracks: Vec<TrackIn> = (0..4100u32).map(|i| track(i % 8, (i / 8) * 100, unit(&mut rng), 0.9)).collect();
    let first = tracks.len();
    for m in 0..8u32 {
        let noise = unit(&mut rng);
        let mut e: Vec<f32> = person.iter().zip(&noise).map(|(c, n)| 0.9 * c + 0.436 * n).collect();
        proto_normalize(&mut e);
        tracks.push(track(m, 1_000_000, e, 0.6));
    }
    let o = identify(&tracks, &Params::default());
    let c = o.tracks[first].0.character();
    assert!(c.is_some(), "the person is no character");
    assert!(o.tracks[first..].iter().all(|(a, _)| a.character() == c));
}
