//! Per-frame signals: everything shot fusion needs, small enough to keep for
//! every frame so fusion can re-run (new parameters, user split/merge edits)
//! without decoding the video again.

use std::collections::VecDeque;

use as_vision::transnet::FRAME_BYTES;

/// How many frames back pixel differences reach (flash detection needs 4).
pub const LAGS: usize = 4;
/// Histogram bins per RGB channel.
pub const BINS: usize = 16;
/// Length of [`FrameSignals::colors`].
pub const COLORS: usize = 3 * BINS;

/// Measurements for one frame (48×27 RGB thumbnail).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSignals {
    /// TransNetV2: probability the frame is a hard cut boundary.
    pub single: f32,
    /// TransNetV2: probability the frame is inside a gradual transition.
    pub many: f32,
    /// Luma mean and standard deviation, 0..255.
    pub mu: f32,
    pub sd: f32,
    /// Mean absolute RGB difference to frame `f - 1 - i`, 0..255
    /// (0 where there is no such frame).
    pub diff: [f32; LAGS],
    /// Chi-squared distance of RGB histograms to the previous frame, 0..1.
    pub hist: f32,
    /// Share of pixels that are black (luma < 12) / near white (> 223). Black
    /// frames with a logo, subtitles or a timecode burn-in are still mostly
    /// black; night scenes, however dark, are not (on real footage they stay
    /// below 0.85 where true black is at 0.9 and up).
    pub dark: f32,
    pub light: f32,
    /// Colour signature: the share of pixels in each of [`BINS`] bins per RGB
    /// channel, in 1/255 (each channel adds up to about 255). Lets fusion
    /// compare any two frames, such as the pictures on both sides of a wipe.
    pub colors: [u8; COLORS],
}

impl Default for FrameSignals {
    fn default() -> Self {
        Self {
            single: 0.0,
            many: 0.0,
            mu: 0.0,
            sd: 0.0,
            diff: [0.0; LAGS],
            hist: 0.0,
            dark: 0.0,
            light: 0.0,
            colors: [0; COLORS],
        }
    }
}

impl FrameSignals {
    /// Pixel difference to `lag` frames back (1 = previous frame).
    pub fn diff_to(&self, lag: usize) -> f32 {
        self.diff[lag - 1]
    }

    /// Chi-squared distance between the colour signatures of two frames, 0..1
    /// (the same measure as [`FrameSignals::hist`], at signature precision).
    pub fn color_distance(&self, other: &Self) -> f32 {
        let chi: f32 = self
            .colors
            .iter()
            .zip(&other.colors)
            .map(|(&a, &b)| {
                let (a, b) = (a as f32, b as f32);
                if a + b > 0.0 { (a - b) * (a - b) / (a + b) } else { 0.0 }
            })
            .sum();
        chi / (2.0 * 3.0 * 255.0)
    }
}

/// Computes the pixel signals of each frame from the last [`LAGS`] frames.
#[derive(Default)]
pub struct PixelMeter {
    recent: VecDeque<(Vec<u8>, [f32; COLORS])>,
}

/// Pixel part of [`FrameSignals`] (TransNet fills in the rest).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelSignals {
    pub mu: f32,
    pub sd: f32,
    pub diff: [f32; LAGS],
    pub hist: f32,
    pub dark: f32,
    pub light: f32,
    pub colors: [u8; COLORS],
}

impl PixelMeter {
    pub fn measure(&mut self, frame: &[u8]) -> PixelSignals {
        debug_assert_eq!(frame.len(), FRAME_BYTES);
        let px = (frame.len() / 3) as f32;
        let mut sum = 0f32;
        let mut sq = 0f32;
        let mut hist = [0f32; COLORS];
        let (mut dark, mut light) = (0u32, 0u32);
        for c in frame.as_chunks::<3>().0 {
            let l = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
            sum += l;
            sq += l * l;
            dark += u32::from(l < 12.0);
            light += u32::from(l > 223.0);
            for (ch, &v) in c.iter().enumerate() {
                hist[ch * BINS + (v as usize >> 4)] += 1.0;
            }
        }
        let mu = sum / px;
        let sd = (sq / px - mu * mu).max(0.0).sqrt();
        let mut diff = [0f32; LAGS];
        for (i, (prev, _)) in self.recent.iter().rev().enumerate() {
            let total: u32 = frame.iter().zip(prev).map(|(a, b)| a.abs_diff(*b) as u32).sum();
            diff[i] = total as f32 / frame.len() as f32;
        }
        let hist_d = match self.recent.back() {
            Some((_, h)) => {
                let chi: f32 =
                    hist.iter().zip(h).map(|(a, b)| if a + b > 0.0 { (a - b) * (a - b) / (a + b) } else { 0.0 }).sum();
                chi / (2.0 * 3.0 * px)
            }
            None => 0.0,
        };
        let colors = hist.map(|c| (c / px * 255.0).round() as u8);
        if self.recent.len() == LAGS {
            self.recent.pop_front();
        }
        self.recent.push_back((frame.to_vec(), hist));
        PixelSignals { mu, sd, diff, hist: hist_d, dark: dark as f32 / px, light: light as f32 / px, colors }
    }
}

// -- storage -----------------------------------------------------------------

const FORMAT: u8 = 3;
const FLOATS: usize = 4 + LAGS + 3;
const RECORD: usize = FLOATS * 4 + COLORS;

/// zstd-compressed records: `FLOATS` little-endian floats, then the colour
/// signature, per frame.
pub fn encode(signals: &[FrameSignals]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(1 + signals.len() * RECORD);
    raw.push(FORMAT);
    for s in signals {
        for v in [s.single, s.many, s.mu, s.sd].into_iter().chain(s.diff).chain([s.hist, s.dark, s.light]) {
            raw.extend_from_slice(&v.to_le_bytes());
        }
        raw.extend_from_slice(&s.colors);
    }
    zstd::encode_all(raw.as_slice(), 9).expect("in-memory zstd")
}

/// The most a stored blob of signals unpacks to: a few dozen bytes a frame
/// (some MB for a film), 512 MB being days of video.
const MAX_UNPACKED: u64 = 512 << 20;

pub fn decode(blob: &[u8]) -> Option<Vec<FrameSignals>> {
    let raw = as_core::time::unpack(blob, MAX_UNPACKED).ok()?;
    let (&format, body) = raw.split_first()?;
    if format != FORMAT || body.len() % RECORD != 0 {
        return None;
    }
    Some(
        body.as_chunks::<RECORD>()
            .0
            .iter()
            .map(|r| {
                let v: Vec<f32> = r[..FLOATS * 4].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
                let mut diff = [0f32; LAGS];
                diff.copy_from_slice(&v[4..4 + LAGS]);
                let mut colors = [0u8; COLORS];
                colors.copy_from_slice(&r[FLOATS * 4..]);
                FrameSignals {
                    single: v[0],
                    many: v[1],
                    mu: v[2],
                    sd: v[3],
                    diff,
                    hist: v[4 + LAGS],
                    dark: v[5 + LAGS],
                    light: v[6 + LAGS],
                    colors,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_and_round_trips() {
        let mut m = PixelMeter::default();
        let black = vec![0u8; FRAME_BYTES];
        let white = vec![255u8; FRAME_BYTES];
        let a = m.measure(&black);
        assert_eq!((a.mu, a.sd, a.diff, a.hist, a.dark, a.light), (0.0, 0.0, [0.0; LAGS], 0.0, 1.0, 0.0));
        assert_eq!((a.colors[0], a.colors[BINS], a.colors[BINS - 1]), (255, 255, 0), "all pixels in the first bin");
        let b = m.measure(&white);
        assert!((b.mu - 255.0).abs() < 0.01);
        assert_eq!(b.diff[0], 255.0);
        assert!(b.hist > 0.99, "opposite histograms: {}", b.hist);
        let c = m.measure(&black);
        assert_eq!((c.diff[0], c.diff[1]), (255.0, 0.0), "back to the frame two before");

        let sig = |p: &PixelSignals| FrameSignals { colors: p.colors, ..Default::default() };
        assert!((sig(&a).color_distance(&sig(&b)) - 1.0).abs() < 1e-6, "opposite signatures");
        assert_eq!(sig(&a).color_distance(&sig(&c)), 0.0);

        let s: Vec<FrameSignals> = (0..100)
            .map(|i| FrameSignals {
                single: i as f32 / 100.0,
                mu: i as f32,
                diff: [i as f32; LAGS],
                colors: [i as u8; COLORS],
                ..Default::default()
            })
            .collect();
        assert_eq!(decode(&encode(&s)).unwrap(), s);
    }
}
