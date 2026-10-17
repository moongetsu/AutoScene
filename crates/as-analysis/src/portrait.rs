//! Character portraits: a track's clearest face framed head and shoulders,
//! cut from the exact frame at the source's own resolution — the picture
//! the cast shows big. (Thumbnails come from the face pass's samples,
//! which are decoded small.)

/// Portrait width as a multiple of the face box's longer side: room for
/// hair, ears and a little of the shoulders.
const CONTEXT: f32 = 2.0;
/// Where the face's centre sits, as a fraction of the portrait's height
/// from the top: eyes in the upper third, shoulders below.
const FACE_Y: f32 = 0.4;

/// The part of a `w`×`h` frame (x1, y1, x2, y2 as fractions of it) that
/// shows face box `bbox` (fractions of the frame) as a portrait of `aspect`
/// (width / height). It stays inside the frame: a face near an edge sits off
/// centre and a face too big for the framing is framed tighter, rather than
/// next to black. `None` for an empty or invalid box.
pub fn portrait_region(bbox: [f32; 4], (w, h): (u32, u32), aspect: f32) -> Option<[f32; 4]> {
    let (w, h) = (w as f32, h as f32);
    let (bw, bh) = ((bbox[2] - bbox[0]) * w, (bbox[3] - bbox[1]) * h);
    let side = bw.max(bh);
    if !(side > 0.0 && side.is_finite() && w > 0.0 && h > 0.0 && aspect > 0.0) || bbox.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let (cx, cy) = ((bbox[0] + bbox[2]) / 2.0 * w, (bbox[1] + bbox[3]) / 2.0 * h);
    let (rw, rh) = (side * CONTEXT, side * CONTEXT / aspect);
    let fit = (w / rw).min(h / rh).min(1.0);
    let (rw, rh) = (rw * fit, rh * fit);
    // `min` then `max` (not `clamp`): rounding may leave a hair less room than the region.
    let x0 = (cx - rw / 2.0).min(w - rw).max(0.0);
    let y0 = (cy - rh * FACE_Y).min(h - rh).max(0.0);
    Some([x0 / w, y0 / h, ((x0 + rw) / w).min(1.0), ((y0 + rh) / h).min(1.0)])
}

/// How much someone else takes of the portrait of face `bbox` (as
/// [`portrait_region`] frames it): the most any of `others` — the other
/// faces of the same frame — covers of the portrait, as a multiple of the
/// face's own size. 0: alone; 1 or more: another face is as big in the
/// picture (a guest behind the person in front: the picture shows the
/// person in front).
pub fn crowding(bbox: [f32; 4], others: impl IntoIterator<Item = [f32; 4]>, frame: (u32, u32), aspect: f32) -> f32 {
    let Some(r) = portrait_region(bbox, frame, aspect) else { return 0.0 };
    let area = |b: [f32; 4]| (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    let own = area(bbox);
    if own <= 0.0 {
        return 0.0;
    }
    others
        .into_iter()
        .map(|o| area([o[0].max(r[0]), o[1].max(r[1]), o[2].min(r[2]), o[3].min(r[3])]) / own)
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_HD: (u32, u32) = (1920, 1080);

    #[test]
    fn a_bigger_face_in_the_picture_crowds_it() {
        // A guest's face at the back, and the host's bigger one in front of
        // it, a little to the left.
        let guest = [0.50, 0.20, 0.55, 0.29];
        let host = [0.44, 0.22, 0.53, 0.42];
        assert!(crowding(guest, [host], FULL_HD, 0.75) > 1.0);
        // The host's portrait shows the guest small: barely crowded.
        assert!(crowding(host, [guest], FULL_HD, 0.75) < 0.4);
        // Someone far away in the frame takes nothing.
        assert_eq!(crowding(guest, [[0.0, 0.2, 0.1, 0.4]], FULL_HD, 0.75), 0.0);
        assert_eq!(crowding(guest, [], FULL_HD, 0.75), 0.0);
    }

    /// The region in frame pixels.
    fn px(r: [f32; 4], (w, h): (u32, u32)) -> [f32; 4] {
        [r[0] * w as f32, r[1] * h as f32, r[2] * w as f32, r[3] * h as f32]
    }

    fn assert_near(a: [f32; 4], b: [f32; 4]) {
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01), "{a:?} vs {b:?}");
    }

    #[test]
    fn a_face_is_framed_head_and_shoulders() {
        // A 200 px face in the middle of a 1080p frame: 400×533, its centre
        // 40 % down and in the middle.
        let bbox = [860.0 / 1920.0, 400.0 / 1080.0, 1060.0 / 1920.0, 600.0 / 1080.0];
        let r = px(portrait_region(bbox, FULL_HD, 0.75).unwrap(), FULL_HD);
        assert_near(r, [760.0, 500.0 - 0.4 * 533.333, 1160.0, 500.0 + 0.6 * 533.333]);
        // A tall box frames by its height.
        let tall = [900.0 / 1920.0, 400.0 / 1080.0, 1000.0 / 1920.0, 600.0 / 1080.0];
        let r = px(portrait_region(tall, FULL_HD, 0.75).unwrap(), FULL_HD);
        assert!(((r[2] - r[0]) - 400.0).abs() < 0.01);
    }

    #[test]
    fn the_region_stays_inside_the_frame() {
        let cases = [
            // Top-left corner: off centre, against the edges.
            ([0.0, 0.0, 0.05, 0.08], [0.0, 0.0]),
            // Bottom-right corner.
            ([0.95, 0.92, 1.0, 1.0], [1.0, 1.0]),
        ];
        for (bbox, corner) in cases {
            let r = portrait_region(bbox, FULL_HD, 0.75).unwrap();
            let p = px(r, FULL_HD);
            assert!(((p[2] - p[0]) / (p[3] - p[1]) - 0.75).abs() < 1e-3, "{bbox:?}: 3:4");
            assert!(r.iter().all(|v| (0.0..=1.0).contains(v)), "{bbox:?}: {r:?}");
            let at = if corner[0] == 0.0 { [r[0], r[1]] } else { [r[2], r[3]] };
            assert_eq!(at, corner, "{bbox:?}");
        }
    }

    #[test]
    fn a_face_too_big_for_the_framing_is_framed_tighter() {
        // A face 80 % of the frame tall: the portrait is as tall as the
        // frame, still 3:4, and centred on the face sideways.
        let bbox = [0.4, 0.1, 0.6, 0.9];
        let r = px(portrait_region(bbox, FULL_HD, 0.75).unwrap(), FULL_HD);
        assert_near(r, [960.0 - 405.0, 0.0, 960.0 + 405.0, 1080.0]);
        // A portrait-shaped (phone) frame: as wide as the frame.
        let phone = (1080, 1920);
        let r = px(portrait_region([0.0, 0.3, 1.0, 0.6], phone, 0.75).unwrap(), phone);
        assert!((r[2] - r[0] - 1080.0).abs() < 0.01 && (r[3] - r[1] - 1440.0).abs() < 0.01, "{r:?}");
    }

    #[test]
    fn an_empty_box_has_no_portrait() {
        assert_eq!(portrait_region([0.5, 0.5, 0.5, 0.5], FULL_HD, 0.75), None);
        assert_eq!(portrait_region([0.6, 0.5, 0.4, 0.4], FULL_HD, 0.75), None);
        assert_eq!(portrait_region([f32::NAN, 0.5, 0.6, 0.6], FULL_HD, 0.75), None);
        assert_eq!(portrait_region([0.4, 0.4, 0.6, 0.6], (0, 0), 0.75), None);
    }
}
