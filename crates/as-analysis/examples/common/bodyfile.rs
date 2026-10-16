//! Every person a detector found in an episode's samples, saved once by
//! `bodies_run` so `bodies_bench` can try trackers and thresholds without
//! the GPU (dev artifact, not an app format).
//!
//! `ASBODY`, version byte, a JSON header (length-prefixed) saying how the
//! run was made and how long its stages took, then per sample: frame,
//! sharpness and brightness of the frame, and its people — whole and
//! visible box (fractions of the frame), score, sharpness and brightness of
//! the person's crop, and the ReID embedding (256 × f16) if they were big
//! enough for one.

#![allow(dead_code)]

use half::f16;

const MAGIC: &[u8; 6] = b"ASBODY";
const VERSION: u8 = 1;
pub const REID_DIM: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub bbox: [f32; 4],
    pub visible: [f32; 4],
    pub score: f32,
    pub sharpness: f32,
    pub luma: f32,
    pub reid: Option<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub frame: u32,
    pub sharpness: f32,
    pub luma: f32,
    pub people: Vec<Person>,
}

pub struct BodyFile {
    pub header: serde_json::Value,
    pub samples: Vec<Sample>,
}

fn f32s(out: &mut Vec<u8>, v: &[f32]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

/// Encode samples; the header comes first in the file but is known last.
pub fn encode_sample(out: &mut Vec<u8>, s: &Sample) {
    out.extend_from_slice(&s.frame.to_le_bytes());
    f32s(out, &[s.sharpness, s.luma]);
    out.extend_from_slice(&(s.people.len() as u16).to_le_bytes());
    for p in &s.people {
        f32s(out, &p.bbox);
        f32s(out, &p.visible);
        f32s(out, &[p.score, p.sharpness, p.luma]);
        match &p.reid {
            Some(v) => {
                assert_eq!(v.len(), REID_DIM);
                out.push(1);
                for x in v {
                    out.extend_from_slice(&f16::from_f32(*x).to_le_bytes());
                }
            }
            None => out.push(0),
        }
    }
}

pub fn write(path: &std::path::Path, header: &serde_json::Value, count: u32, body: &[u8]) -> std::io::Result<()> {
    let h = serde_json::to_vec(header).expect("JSON");
    let mut out = Vec::with_capacity(body.len() + h.len() + 16);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(h.len() as u32).to_le_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(body);
    std::fs::write(path, out)
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        assert!(self.0.len() >= n, "body file cut short");
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        a
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }
    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    fn quad(&mut self) -> [f32; 4] {
        [self.f32(), self.f32(), self.f32(), self.f32()]
    }
}

pub fn read(path: &std::path::Path) -> BodyFile {
    let bytes = std::fs::read(path).expect("body file");
    let mut r = Reader(&bytes);
    assert_eq!(r.take(6), MAGIC, "not a body file");
    assert_eq!(r.take(1)[0], VERSION, "body file version");
    let n = r.u32() as usize;
    let header = serde_json::from_slice(r.take(n)).expect("body file header");
    let count = r.u32() as usize;
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        let (frame, sharpness, luma) = (r.u32(), r.f32(), r.f32());
        let people = (0..r.u16())
            .map(|_| {
                let (bbox, visible) = (r.quad(), r.quad());
                let (score, sharpness, luma) = (r.f32(), r.f32(), r.f32());
                let reid = (r.take(1)[0] == 1).then(|| {
                    let raw = r.take(REID_DIM * 2);
                    let mut v: Vec<f32> =
                        raw.as_chunks::<2>().0.iter().map(|c| f16::from_le_bytes(*c).to_f32()).collect();
                    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
                    v.iter_mut().for_each(|x| *x /= n);
                    v
                });
                Person { bbox, visible, score, sharpness, luma, reid }
            })
            .collect();
        samples.push(Sample { frame, sharpness, luma, people });
    }
    assert!(r.0.is_empty(), "trailing bytes in body file");
    BodyFile { header, samples }
}
