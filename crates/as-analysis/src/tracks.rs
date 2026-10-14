//! Face tracks: one face followed through one shot, with an identity
//! embedding. The unit the cast is built from — a character is a set of
//! tracks, every edit (move, "not them", not a face) is an edit of tracks.

use as_core::FrameIdx;
use half::f16;

/// Embedding size (ArcFace).
pub const DIM: usize = as_vision::arcface::DIM;

/// One detection of a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceDet {
    pub frame: FrameIdx,
    /// x1, y1, x2, y2 as fractions of the (upright, square-pixel) frame.
    pub bbox: [f32; 4],
    /// Detector confidence.
    pub score: f32,
    /// Length of the raw embedding: low for profiles, the back of a head,
    /// blur, darkness — the best single quality signal there is.
    pub norm: f32,
    /// 0..=1, how much this detection says about who it is.
    pub quality: f32,
}

impl FaceDet {
    /// Face height as a fraction of the frame height.
    pub fn height(&self) -> f32 {
        self.bbox[3] - self.bbox[1]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FaceTrack {
    /// Frames `start..end` (from the first detection to one stride past the last).
    pub start: FrameIdx,
    pub end: FrameIdx,
    /// Start of the shot the track belongs to.
    pub shot: FrameIdx,
    pub dets: Vec<FaceDet>,
    /// Unit-length identity embedding (quality-weighted mean of the detections).
    pub embedding: Vec<f32>,
    /// Mean quality of the best three detections.
    pub quality: f32,
    /// Square JPEG thumbnail of the best detection.
    pub thumb: Vec<u8>,
}

impl FaceTrack {
    /// Median face height (fraction of the frame height).
    pub fn face_height(&self) -> f32 {
        let mut h: Vec<f32> = self.dets.iter().map(FaceDet::height).collect();
        h.sort_by(f32::total_cmp);
        h.get(h.len() / 2).copied().unwrap_or(0.0)
    }
}

const FORMAT: u8 = 1;

struct Writer(Vec<u8>);

impl Writer {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
}

/// Pack an embedding as little-endian f16 (what the database keeps).
pub fn embedding_to_bytes(e: &[f32]) -> Vec<u8> {
    e.iter().flat_map(|&v| f16::from_f32(v).to_le_bytes()).collect()
}

/// Unpack [`embedding_to_bytes`], renormalised to unit length.
pub fn embedding_from_bytes(b: &[u8]) -> Option<Vec<f32>> {
    if b.len() != DIM * 2 {
        return None;
    }
    let mut v: Vec<f32> = b.as_chunks::<2>().0.iter().map(|c| f16::from_le_bytes(*c).to_f32()).collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter_mut().for_each(|x| *x /= n);
    Some(v)
}

/// Pack detections (what the database keeps): 32 bytes each.
pub fn dets_to_bytes(dets: &[FaceDet]) -> Vec<u8> {
    let mut w = Writer(Vec::with_capacity(dets.len() * 32));
    for d in dets {
        w.u32(d.frame);
        d.bbox.iter().for_each(|&v| w.f32(v));
        w.f32(d.score);
        w.f32(d.norm);
        w.f32(d.quality);
    }
    w.0
}

pub fn dets_from_bytes(b: &[u8]) -> Option<Vec<FaceDet>> {
    if !b.len().is_multiple_of(32) {
        return None;
    }
    let mut r = Reader(b);
    let mut out = Vec::with_capacity(b.len() / 32);
    while !r.0.is_empty() {
        out.push(FaceDet {
            frame: r.u32()?,
            bbox: [r.f32()?, r.f32()?, r.f32()?, r.f32()?],
            score: r.f32()?,
            norm: r.f32()?,
            quality: r.f32()?,
        });
    }
    Some(out)
}

/// Serialise tracks (one checkpoint chunk) for the engine protocol.
pub fn encode(tracks: &[FaceTrack]) -> Vec<u8> {
    let mut w = Writer(vec![FORMAT]);
    w.u32(tracks.len() as u32);
    for t in tracks {
        w.u32(t.start);
        w.u32(t.end);
        w.u32(t.shot);
        w.f32(t.quality);
        w.bytes(&dets_to_bytes(&t.dets));
        w.bytes(&embedding_to_bytes(&t.embedding));
        w.bytes(&t.thumb);
    }
    w.0
}

pub fn decode(blob: &[u8]) -> Option<Vec<FaceTrack>> {
    let (&format, body) = blob.split_first()?;
    if format != FORMAT {
        return None;
    }
    let mut r = Reader(body);
    let n = r.u32()? as usize;
    let mut out = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let (start, end, shot, quality) = (r.u32()?, r.u32()?, r.u32()?, r.f32()?);
        let dets = dets_from_bytes(r.bytes()?)?;
        let embedding = embedding_from_bytes(r.bytes()?)?;
        let thumb = r.bytes()?.to_vec();
        out.push(FaceTrack { start, end, shot, dets, embedding, quality, thumb });
    }
    r.0.is_empty().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_round_trip() {
        let mut e = vec![0f32; DIM];
        e[3] = 0.6;
        e[7] = -0.8;
        let t = FaceTrack {
            start: 30,
            end: 99,
            shot: 12,
            dets: vec![
                FaceDet { frame: 30, bbox: [0.1, 0.2, 0.3, 0.5], score: 0.9, norm: 21.0, quality: 0.8 },
                FaceDet { frame: 96, bbox: [0.12, 0.2, 0.31, 0.52], score: 0.8, norm: 15.5, quality: 0.2 },
            ],
            embedding: e,
            quality: 0.5,
            thumb: vec![0xFF, 0xD8, 1, 2, 3],
        };
        let back = decode(&encode(std::slice::from_ref(&t))).unwrap();
        assert_eq!(back.len(), 1);
        let b = &back[0];
        assert_eq!((b.start, b.end, b.shot, &b.dets, &b.thumb), (30, 99, 12, &t.dets, &t.thumb));
        assert!(b.embedding.iter().zip(&t.embedding).all(|(x, y)| (x - y).abs() < 1e-3));
        assert!((t.face_height() - 0.32).abs() < 1e-6);
        assert!(decode(&encode(&[])).unwrap().is_empty());
        assert!(decode(b"\x01\x05").is_none());
    }
}
