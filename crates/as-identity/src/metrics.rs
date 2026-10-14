//! How well an assignment of tracks to characters matches the truth.

use std::collections::HashMap;

/// BCubed precision (how much of an item's character is its person), recall
/// (how much of its person is in its character) and their F-measure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BCubed {
    pub precision: f32,
    pub recall: f32,
    pub f: f32,
}

/// BCubed of `assigned` (a character per item; `None` is a character of its
/// own) against `truth` (a person per item), each item weighing `weight`
/// (e.g. its screen time). The average runs over the items `counted` picks
/// — the main cast, say — while every item still counts as a neighbour, so
/// extras pulled into a main character cost precision.
pub fn bcubed(assigned: &[Option<usize>], truth: &[usize], weight: &[f32], counted: impl Fn(usize) -> bool) -> BCubed {
    assert!(assigned.len() == truth.len() && truth.len() == weight.len());
    let mut by_char: HashMap<usize, f64> = HashMap::new();
    let mut by_person: HashMap<usize, f64> = HashMap::new();
    let mut both: HashMap<(usize, usize), f64> = HashMap::new();
    for i in 0..truth.len() {
        let w = weight[i] as f64;
        *by_person.entry(truth[i]).or_default() += w;
        if let Some(c) = assigned[i] {
            *by_char.entry(c).or_default() += w;
            *both.entry((c, truth[i])).or_default() += w;
        }
    }
    let (mut p, mut r, mut total) = (0f64, 0f64, 0f64);
    for i in (0..truth.len()).filter(|&i| counted(i)) {
        let w = weight[i] as f64;
        let (shared, size) = match assigned[i] {
            Some(c) => (both[&(c, truth[i])], by_char[&c]),
            None => (w, w),
        };
        p += w * shared / size.max(1e-12);
        r += w * shared / by_person[&truth[i]].max(1e-12);
        total += w;
    }
    if total == 0.0 {
        return BCubed { precision: 1.0, recall: 1.0, f: 1.0 };
    }
    let (p, r) = ((p / total) as f32, (r / total) as f32);
    BCubed { precision: p, recall: r, f: if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_split_and_merged() {
        let truth = [0, 0, 1, 1];
        let w = [1.0; 4];
        let all = |_| true;
        assert_eq!(bcubed(&[Some(0), Some(0), Some(1), Some(1)], &truth, &w, all).f, 1.0);
        // Everyone in one character: precision 1/2, recall 1.
        let one = bcubed(&[Some(0); 4], &truth, &w, all);
        assert_eq!((one.precision, one.recall), (0.5, 1.0));
        // Nobody assigned: precision 1, recall 1/2.
        let none = bcubed(&[None; 4], &truth, &w, all);
        assert_eq!((none.precision, none.recall), (1.0, 0.5));
    }

    #[test]
    fn extras_in_a_main_character_cost_precision() {
        // Items 0..3 are the lead; item 3 is an extra put on the lead.
        let truth = [0, 0, 0, 9];
        let got = [Some(0), Some(0), Some(0), Some(0)];
        let lead = bcubed(&got, &truth, &[1.0; 4], |i| truth[i] == 0);
        assert!((lead.precision - 0.75).abs() < 1e-6 && lead.recall == 1.0);
        // Weighted: the extra is on screen briefly.
        let lead = bcubed(&got, &truth, &[1.0, 1.0, 1.0, 0.1], |i| truth[i] == 0);
        assert!((lead.precision - 3.0 / 3.1).abs() < 1e-6);
    }
}
