//! One file made of many clips, with a chapter for each.
//!
//! A scenepack's clips come from episodes that can differ in picture size,
//! pixel shape, frame rate, colour and sound. Each clip is cut exactly like a
//! clip of its own ([`crate::export_clip`], frame-exact and verified), but as
//! a *part* in one shape every part shares ([`MergeShape`]):
//! - one picture size: the size most frames have; other parts are scaled to
//!   fit and letterboxed, with square pixels;
//! - one colour: HDR stays HDR only when every part is HDR the preset can
//!   carry, otherwise HDR parts are tone mapped; mixed tags become BT.709;
//! - one sound: PCM at 48 kHz in one channel layout, silence for episodes
//!   without sound, cut to whole samples so sound and picture never drift;
//! - one encoder and one time base, so the parts' streams fit together.
//!
//! The parts are then joined by the concat demuxer without encoding the
//! picture again (sound is encoded once, for AAC presets), and the file is
//! checked: every frame, every chapter, all the sound.
//!
//! Frames keep their own timing (no frame rate conversion), like single
//! clips do: on the episode's frame grid, or on its own time base for
//! variable-rate episodes. The file's time base is a whole multiple of every
//! part's, so every frame lands on its exact tick.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use as_core::{CancelToken, FrameIndex, FrameRange, Rational};
use as_media::filters::TONEMAP_SDR;
use as_media::frames::MAX_SIDE;
use as_media::{AudioStream, Caps, ExactFormat, HdrKind, MediaError, MediaInfo, Tools, run_ffmpeg};
use serde_json::Value;

use crate::clip::{AUDIO_RATE, ClipOutcome, Encoding, encoder_fps, resolve_encoding, with_suffix};
use crate::lossless::{self, LosslessCodec};
use crate::preset::{AudioCodec, Container, ExportPreset, HdrMode, VideoCodec};
use crate::{ExportError, ExportResult};

/// The most ticks a second the file is timed in. The concat demuxer places
/// each part in whole microseconds, which round back onto the same tick only
/// below a million ticks a second.
const MAX_TIMESCALE: u64 = 1_000_000;
/// The fewest (the mov muxer's own minimum: coarser ones confuse players).
const MIN_TIMESCALE: u64 = 10_000;
/// The join's work files, next to the parts.
const LIST: &str = "parts.ffconcat";
const CHAPTERS: &str = "chapters.ffmeta";

/// A clip of a merged export, as the plan sees it.
#[derive(Clone, Copy, Debug)]
pub struct MergeSource<'a> {
    pub info: &'a MediaInfo,
    pub index: &'a FrameIndex,
    pub range: FrameRange,
    /// The clip's audio track (`None`: the file's default).
    pub audio_ordinal: Option<u32>,
}

/// Colour tags of a picture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorTags {
    pub primaries: Option<String>,
    pub transfer: Option<String>,
    pub space: Option<String>,
    pub range: Option<String>,
}

impl ColorTags {
    /// The picture's tags; one that isn't a plain name is left out, as if
    /// the picture had none (see [`graph_name`]).
    fn of(info: &MediaInfo) -> Self {
        let v = &info.video;
        let tag = |t: &Option<String>| t.clone().filter(|t| graph_name(t, false));
        Self {
            primaries: tag(&v.color_primaries),
            transfer: tag(&v.color_transfer),
            space: tag(&v.color_space),
            range: tag(&v.color_range),
        }
    }

    fn bt709() -> Self {
        let s = |v: &str| Some(v.to_string());
        Self { primaries: s("bt709"), transfer: s("bt709"), space: s("bt709"), range: s("tv") }
    }
}

/// How every part of a merged export is made, so the parts join without
/// being encoded again.
#[derive(Clone, Debug)]
pub struct MergeShape {
    pub width: u32,
    pub height: u32,
    /// The preset's height cap, which each part's own size is worked out under.
    max_height: Option<u32>,
    /// Ticks per second of the file's video: every part's frames are on whole ticks.
    pub timescale: u32,
    /// HDR parts are tone mapped to SDR.
    pub tonemap: bool,
    pub color: ColorTags,
    /// The tags are BT.709 for parts that differ: SDR parts with another
    /// colour matrix or full range are converted.
    to_bt709: bool,
    /// Channel layout of the sound (`None`: no sound in the file).
    pub audio: Option<String>,
    /// The sound's language, when every part with sound agrees.
    pub language: Option<String>,
    pub(crate) encoding: Encoding,
    /// Bit-Perfect: how the parts keep their sources exactly (`encoding` is
    /// then the lossless one, the picture each part's own).
    pub(crate) lossless: Option<LosslessShape>,
}

/// How every part of a Bit-Perfect file keeps its source exactly.
#[derive(Clone, Debug)]
pub struct LosslessShape {
    pub codec: LosslessCodec,
    /// The sound's PCM codec and sample rate (`None`: no sound in the file).
    pub audio: Option<(&'static str, u32)>,
    /// The frames' colour tags and the encoder's layout (`lossless::Keeper`).
    pub(crate) tag: String,
    pub(crate) repack: Option<String>,
    /// The HDR metadata every part (and the file) must carry.
    pub(crate) hdr: Vec<String>,
}

/// Where a part is in the file, in whole ticks and samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartTiming {
    /// How long it lasts, in [`MergeShape::timescale`] ticks.
    pub ticks: u64,
    /// Its sound in samples (48 kHz; Bit-Perfect: the sources' rate): the
    /// parts' samples add up to the file's length rounded once, so sound
    /// never drifts from the picture.
    pub samples: u64,
}

#[derive(Clone, Debug)]
pub struct MergePlan {
    pub shape: MergeShape,
    /// One per source, in order.
    pub parts: Vec<PartTiming>,
}

impl MergeShape {
    /// Filters after the trim (and `setpts`) that turn a part's frames into
    /// the shape: its frame grid, colour and size, then the pixel format.
    pub(crate) fn picture(&self, info: &MediaInfo, index: &FrameIndex) -> Vec<String> {
        let mut f = Vec::new();
        if !index.is_vfr() {
            // Onto the clean frame grid: in coarse time bases (MKV's
            // milliseconds) frames sit a tick off it.
            let g = grid(index);
            f.push(format!("settb={}/{}", g.num, g.den));
        }
        let tonemapped = self.tonemap && info.video.hdr != HdrKind::None;
        if tonemapped {
            f.push(TONEMAP_SDR.to_string());
        }
        let (w, h) = fit(natural_size(info, self.max_height), (self.width, self.height));
        let mut scale = format!("scale={w}:{h}:flags=bicubic");
        if self.to_bt709 && !tonemapped {
            if matrix_is_bt601(info) {
                scale += ":in_color_matrix=bt601:out_color_matrix=bt709";
            }
            if info.video.color_range.as_deref() == Some("pc") {
                scale += ":in_range=pc:out_range=tv";
            }
        }
        f.extend([scale, "setsar=1".to_string()]);
        if (w, h) != (self.width, self.height) {
            let (x, y) = (((self.width - w) / 2) & !1, ((self.height - h) / 2) & !1);
            f.push(format!("pad={}:{}:{x}:{y}:color=black", self.width, self.height));
        }
        f.push(format!("format={}", self.encoding.pix_fmt));
        // On the frames themselves: encoders take colour tags from the
        // frames, and untagged frames would ignore `-color_primaries`.
        let c = &self.color;
        let tags: Vec<String> = [
            ("color_primaries", &c.primaries),
            ("color_trc", &c.transfer),
            ("colorspace", &c.space),
            ("range", &c.range),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={v}")))
        .collect();
        if !tags.is_empty() {
            f.push(format!("setparams={}", tags.join(":")));
        }
        f
    }

    pub(crate) fn color_args(&self) -> Vec<String> {
        let c = &self.color;
        [
            ("-color_primaries", &c.primaries),
            ("-color_trc", &c.transfer),
            ("-colorspace", &c.space),
            ("-color_range", &c.range),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| [k.to_string(), v.clone()]))
        .flatten()
        .collect()
    }
}

/// Can a probed name (a colour tag, a channel layout) go into a filter
/// graph as it is? ffprobe prints names of letters, digits and `_ . ( ) + -`
/// (layouts with spaces too: "3 channels (FL+FR+TFL)"), but a stored media
/// record can hold anything, and a `,` `;` `:` `=` `[` or quote in it would
/// reshape the graph (another filter could read a file). One that isn't is
/// treated as unknown.
pub(crate) fn graph_name(name: &str, spaces: bool) -> bool {
    !name.is_empty()
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '(' | ')' | '+' | '-') || (spaces && c == ' ')
        })
}

/// Is every clip within its episode's frames? One cut from an episode
/// indexed again since (relinked to a shorter file) may not be: looking it
/// up would go past the index's end.
fn check_ranges(sources: &[MergeSource]) -> ExportResult<()> {
    match sources.iter().find(|s| s.range.is_empty() || s.range.end > s.index.len()) {
        Some(s) => Err(ExportError::BadRange(format!("{:?} outside 0..{}", s.range, s.index.len()))),
        None => Ok(()),
    }
}

/// The audio track a clip takes: the chosen one, else the file's default.
pub(crate) fn audio_stream(info: &MediaInfo, ordinal: Option<u32>) -> Option<&AudioStream> {
    let ord = ordinal.or_else(|| info.default_audio().map(|a| a.ordinal))?;
    info.audio.iter().find(|a| a.ordinal == ord)
}

/// Work out the shape and timing of a merged export of `sources`.
/// Bit-Perfect files are planned from what the sources decode to, with
/// [`plan_lossless_merge`].
pub fn plan_merge(caps: &Caps, preset: &ExportPreset, sources: &[MergeSource]) -> ExportResult<MergePlan> {
    if sources.is_empty() {
        return Err(ExportError::Join("nothing to join".into()));
    }
    check_ranges(sources)?;
    if preset.video == VideoCodec::Lossless {
        return Err(ExportError::BadPreset("a Bit-Perfect file is planned from its sources' exact formats".into()));
    }
    if preset.container == Container::Mp4 && preset.audio == AudioCodec::Pcm24 {
        return Err(ExportError::BadPreset("uncompressed audio needs a MOV file".into()));
    }
    let frames = |s: &MergeSource| u64::from(s.range.len());

    let mut rates: HashMap<Rational, u64> = HashMap::new();
    for s in sources {
        *rates.entry(s.index.nominal_fps).or_default() += frames(s);
    }
    let (width, height) =
        file_size(sources.iter().map(|s| (natural_size(s.info, preset.max_height), frames(s)))).expect("non-empty");
    let fps = rates.into_iter().max_by_key(|&(r, n)| (n, r.num * 1000 / r.den.max(1))).map(|(r, _)| r).unwrap();

    // Every part's encoder is told its own episode's rate: the cap leaves
    // the fastest one's frames their share.
    let fastest = sources.iter().map(|s| encoder_fps(s.info, s.index.nominal_fps.to_f64())).fold(0.0, f64::max);
    let mut encoding = resolve_encoding(&preset.video, caps, fps.to_f64(), (width, height), fastest)?;
    // No B-frames: every part starts on a clean, already ordered frame.
    if matches!(preset.video, VideoCodec::H264 { .. } | VideoCodec::Hevc { .. })
        && !encoding.args.iter().any(|a| a == "-bf")
    {
        encoding.args.extend(["-bf".into(), "0".into()]);
    }

    let (tonemap, color, to_bt709) = colour_plan(preset, sources);
    let (audio, language) = sound_plan(preset, sources);

    let grids: Vec<Rational> = sources.iter().map(|s| grid(s.index)).collect();
    let t = timescale(&grids);
    let ticks: Vec<u64> = sources.iter().map(|s| part_ticks(s.index, s.range, t)).collect();
    let parts = timings(&ticks, t, AUDIO_RATE);
    let shape = MergeShape {
        width,
        height,
        max_height: preset.max_height,
        timescale: t as u32,
        tonemap,
        color,
        to_bt709,
        audio,
        language,
        encoding,
        lossless: None,
    };
    Ok(MergePlan { shape, parts })
}

/// Why clips like `sources` can't share one Bit-Perfect file, as far as
/// their probes tell (`None`: they can, unless what they decode to differs;
/// see [`plan_lossless_merge`]).
pub fn lossless_merge_blocker(sources: &[MergeSource]) -> Option<String> {
    let first = sources.first()?;
    let picture = |s: &MergeSource| {
        let v = &s.info.video;
        (v.width, v.height, v.sar, v.pix_fmt.clone(), v.rotation, v.field_order.clone())
    };
    if sources.iter().any(|s| picture(s) != picture(first)) {
        return Some("their episodes' pictures differ in size or format".into());
    }
    // The colour each copy would be tagged with (untagged sources get what
    // editors assume for their codec).
    let colour = |s: &MergeSource| lossless::assumed_colour(s.info);
    if sources.iter().any(|s| colour(s) != colour(first)) {
        return Some("their episodes' colours are tagged differently".into());
    }
    let sounds: Vec<&AudioStream> = sources.iter().filter_map(|s| audio_stream(s.info, s.audio_ordinal)).collect();
    if sounds.iter().any(|a| (a.sample_rate, a.channels) != (sounds[0].sample_rate, sounds[0].channels)) {
        return Some("their episodes' sound differs in its rate or channels".into());
    }
    None
}

/// Work out the shape and timing of a Bit-Perfect file of `sources`, which
/// decode to `exact` (one each): only clips whose frames and sound are
/// alike can share one, as they are.
pub fn plan_lossless_merge(caps: &Caps, sources: &[MergeSource], exact: &[ExactFormat]) -> ExportResult<MergePlan> {
    if sources.is_empty() || sources.len() != exact.len() {
        return Err(ExportError::Join("nothing to join".into()));
    }
    check_ranges(sources)?;
    let unlike = |why: &str| Err(ExportError::Inexact(format!("its clips can't share one file: {why}")));
    if let Some(why) = lossless_merge_blocker(sources) {
        return unlike(&why);
    }
    let keepers = sources
        .iter()
        .zip(exact)
        .map(|(s, e)| lossless::keeper(caps, e, &s.info.video.codec, (s.info.video.width, s.info.video.height)))
        .collect::<ExportResult<Vec<_>>>()?;
    let keeper = keepers[0].clone();
    let alike = |k: &lossless::Keeper| {
        (k.codec, &k.encoding.args, &k.tag, &k.repack)
            == (keeper.codec, &keeper.encoding.args, &keeper.tag, &keeper.repack)
    };
    if !keepers.iter().all(alike) {
        return unlike("their episodes decode to different picture formats or colours");
    }
    if exact.iter().any(|e| e.interlaced != exact[0].interlaced) {
        return unlike("their episodes' fields come in a different order");
    }
    if exact.iter().any(|e| e.hdr_metadata != exact[0].hdr_metadata) {
        return unlike("their episodes' HDR metadata differs");
    }
    // The sound of every clip that has one, as stored: alike, or all held
    // exactly by 32-bit float (16- and 24-bit samples are).
    let sounds: Vec<(&'static str, u32, u32)> = sources
        .iter()
        .zip(exact)
        .filter_map(|(s, e)| {
            let ord = s.audio_ordinal.or_else(|| s.info.default_audio().map(|a| a.ordinal))?;
            e.audio.iter().find(|a| a.ordinal == ord)
        })
        .map(|a| lossless::pcm_codec(a).map(|c| (c, a.sample_rate.max(1), a.channels)))
        .collect::<ExportResult<_>>()?;
    let audio = match sounds.first() {
        None => None,
        // Layouts of as many channels ("5.1", "5.1(side)") hold the same
        // samples: the file is tagged with the first one's.
        Some((codec, rate, channels)) => {
            if sounds.iter().any(|(_, r, c)| (r, c) != (rate, channels)) {
                return unlike("their episodes' sound differs in its rate or channels");
            }
            let codec = if sounds.iter().all(|(c, ..)| c == codec) {
                *codec
            } else if sounds.iter().all(|(c, ..)| matches!(*c, "pcm_f32le" | "pcm_s16le" | "pcm_s24le")) {
                "pcm_f32le"
            } else {
                return unlike("their episodes' sound is stored in different sample formats");
            };
            Some((codec, *rate))
        }
    };
    let layout = audio
        .map(|_| sources.iter().find_map(|s| audio_stream(s.info, s.audio_ordinal)).map(layout_of).unwrap_or_default());
    let langs: Vec<Option<String>> = sources
        .iter()
        .filter_map(|s| audio_stream(s.info, s.audio_ordinal))
        .map(|a| a.language.as_deref().and_then(as_media::lang::canonical))
        .collect();
    let language = langs.first().cloned().flatten().filter(|l| langs.iter().all(|x| x.as_ref() == Some(l)));

    let grids: Vec<Rational> = sources.iter().map(|s| grid(s.index)).collect();
    let t = timescale(&grids);
    let ticks: Vec<u64> = sources.iter().map(|s| part_ticks(s.index, s.range, t)).collect();
    let parts = timings(&ticks, t, audio.map_or(AUDIO_RATE, |(_, rate)| rate));
    let v = &sources[0].info.video;
    let shape = MergeShape {
        width: v.width,
        height: v.height,
        max_height: None,
        timescale: t as u32,
        tonemap: false,
        color: ColorTags::of(sources[0].info),
        to_bt709: false,
        audio: layout,
        language,
        encoding: keeper.encoding,
        lossless: Some(LosslessShape {
            codec: keeper.codec,
            audio,
            tag: keeper.tag,
            repack: keeper.repack,
            hdr: exact[0].hdr_metadata.clone(),
        }),
    };
    Ok(MergePlan { shape, parts })
}

/// The time base a part's frames are on: its frame grid, or its own time
/// base when it has a variable frame rate.
pub(crate) fn grid(index: &FrameIndex) -> Rational {
    if index.is_vfr() { index.tb } else { index.nominal_fps.recip() }
}

/// Ticks per second that time every grid exactly (the smallest multiple of
/// them all, at least [`MIN_TIMESCALE`]); [`MAX_TIMESCALE`] when that would
/// be finer, which times frames to the microsecond.
fn timescale(grids: &[Rational]) -> u64 {
    let mut t: u64 = 1;
    for g in grids {
        t = lcm(t, g.den.unsigned_abs().max(1));
        if t > MAX_TIMESCALE {
            return MAX_TIMESCALE;
        }
    }
    while t < MIN_TIMESCALE {
        t *= 2;
    }
    t
}

fn lcm(a: u64, b: u64) -> u64 {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x * b
}

/// How long the clip `range` lasts in ticks of `t` a second: whole frames
/// of its grid, or its frames' own times when the rate varies.
fn part_ticks(index: &FrameIndex, range: FrameRange, t: u64) -> u64 {
    let (num, den) = if index.is_vfr() {
        let d = i128::from(index.pts(range.end) - index.pts(range.start));
        (d * i128::from(index.tb.num), i128::from(index.tb.den))
    } else {
        let g = grid(index);
        (i128::from(range.len()) * i128::from(g.num), i128::from(g.den))
    };
    ((num * i128::from(t) * 2 + den) / (2 * den)).max(1) as u64
}

/// The parts' timing: each one's samples (at `rate`) are the file's sound up
/// to its end, rounded, minus what came before (so the rounding never adds
/// up).
fn timings(ticks: &[u64], t: u64, rate: u32) -> Vec<PartTiming> {
    let samples_at = |c: u128| ((c * u128::from(rate) * 2 + u128::from(t)) / (2 * u128::from(t))) as u64;
    let mut at = 0u128;
    ticks
        .iter()
        .map(|&ticks| {
            let before = samples_at(at);
            at += u128::from(ticks);
            PartTiming { ticks, samples: samples_at(at) - before }
        })
        .collect()
}

/// The picture of a file joined from clips of these sizes, each with so
/// many frames: the size most frames have; of two, the bigger.
pub(crate) fn file_size(clips: impl IntoIterator<Item = ((u32, u32), u64)>) -> Option<(u32, u32)> {
    let mut sizes: HashMap<(u32, u32), u64> = HashMap::new();
    for (size, frames) in clips {
        *sizes.entry(size).or_default() += frames;
    }
    sizes.into_iter().max_by_key(|&((w, h), n)| (n, u64::from(w) * u64::from(h), w)).map(|(s, _)| s)
}

/// The picture a clip of `info` shows: upright, square pixels, at most
/// `max_height` tall and [`MAX_SIDE`] a side, in even numbers.
pub(crate) fn natural_size(info: &MediaInfo, max_height: Option<u32>) -> (u32, u32) {
    let v = &info.video;
    // ffmpeg turns rotated pictures upright before the filters see them.
    let quarter = matches!(v.rotation, 90 | 270);
    let (w, h, aspect) =
        if quarter { (v.height, v.width, v.pixel_aspect().recip()) } else { (v.width, v.height, v.pixel_aspect()) };
    let (mut w, mut h) = (f64::from(w) * aspect, f64::from(h));
    if let Some(m) = max_height.map(f64::from)
        && h > m
    {
        w *= m / h;
        h = m;
    }
    // Only a crafted pixel shape (65535:1) makes a picture wider than that.
    let side = f64::from(MAX_SIDE);
    if w.max(h) > side {
        let k = side / w.max(h);
        (w, h) = (w * k, h * k);
    }
    (even(w), even(h))
}

/// `size` scaled to fit inside `frame`, keeping its shape.
fn fit(size: (u32, u32), frame: (u32, u32)) -> (u32, u32) {
    if size == frame {
        return size;
    }
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    let s = (f64::from(frame.0) / w).min(f64::from(frame.1) / h);
    (even(w * s).min(frame.0), even(h * s).min(frame.1))
}

/// The even number of pixels nearest `x`, from 2 to [`MAX_SIDE`] (not a
/// number: 2).
fn even(x: f64) -> u32 {
    (((x / 2.0).round().clamp(1.0, f64::from(MAX_SIDE / 2)) as u32) * 2).max(2)
}

/// Does the picture use the SD colour matrix (tagged, or untagged and SD)?
fn matrix_is_bt601(info: &MediaInfo) -> bool {
    match info.video.color_space.as_deref() {
        Some(m) => matches!(m, "smpte170m" | "bt470bg"),
        None => info.video.height < 720,
    }
}

/// `(tone map HDR parts, the file's tags, convert others to BT.709)`.
fn colour_plan(preset: &ExportPreset, sources: &[MergeSource]) -> (bool, ColorTags, bool) {
    let hdr = |s: &MergeSource| s.info.video.hdr != HdrKind::None;
    let first = &sources[0].info.video;
    let keep_hdr = preset.hdr == HdrMode::Auto
        && preset.video.supports_hdr()
        && sources.iter().all(|s| hdr(s) && s.info.video.color_transfer == first.color_transfer);
    if keep_hdr {
        let main = sources.iter().max_by_key(|s| s.range.len()).expect("not empty");
        return (false, ColorTags::of(main.info), false);
    }
    let tonemap = sources.iter().any(hdr);
    let tags = ColorTags::of(sources[0].info);
    if !tonemap && sources.iter().all(|s| ColorTags::of(s.info) == tags) {
        return (false, tags, false);
    }
    (tonemap, ColorTags::bt709(), true)
}

/// `(channel layout, language)` of the file's sound.
fn sound_plan(preset: &ExportPreset, sources: &[MergeSource]) -> (Option<String>, Option<String>) {
    if preset.audio == AudioCodec::None {
        return (None, None);
    }
    let tracks: Vec<(&AudioStream, u64)> = sources
        .iter()
        .filter_map(|s| audio_stream(s.info, s.audio_ordinal).map(|a| (a, u64::from(s.range.len()))))
        .collect();
    if tracks.is_empty() {
        return (None, None);
    }
    let layouts: Vec<(String, u32, u64)> = tracks.iter().map(|(a, n)| (layout_of(a), a.channels, *n)).collect();
    let layout = if preset.stereo {
        "stereo".to_string()
    } else if layouts.iter().all(|l| l.0 == layouts[0].0) {
        layouts[0].0.clone()
    } else if layouts.iter().all(|l| l.1 == layouts[0].1) {
        // The same channels named differently ("5.1" and "5.1(side)"): the commonest.
        let mut by: HashMap<&str, u64> = HashMap::new();
        for (l, _, n) in &layouts {
            *by.entry(l).or_default() += n;
        }
        by.into_iter().max_by_key(|&(l, n)| (n, l)).map(|(l, _)| l.to_string()).unwrap()
    } else {
        "stereo".to_string()
    };
    let langs: Vec<Option<String>> =
        tracks.iter().map(|(a, _)| a.language.as_deref().and_then(as_media::lang::canonical)).collect();
    let language = langs[0].clone().filter(|_| langs.iter().all(|l| *l == langs[0]));
    (Some(layout), language)
}

/// The track's channel layout (when it is a plain name, see
/// [`graph_name`]), or the usual one for its channel count.
fn layout_of(a: &AudioStream) -> String {
    if let Some(l) = a.channel_layout.as_deref().filter(|l| graph_name(l, true)) {
        return l.to_string();
    }
    match a.channels {
        1 => "mono".into(),
        2 => "stereo".into(),
        3 => "2.1".into(),
        4 => "quad".into(),
        5 => "5.0".into(),
        6 => "5.1".into(),
        7 => "6.1".into(),
        8 => "7.1".into(),
        n => format!("{n}c"),
    }
}

// -- joining ------------------------------------------------------------------

/// A part rendered with [`crate::export_clip`] into the join's work folder.
#[derive(Clone, Debug)]
pub struct JoinPart {
    pub path: PathBuf,
    pub frames: u32,
    pub timing: PartTiming,
    /// Its chapter's title.
    pub title: String,
}

pub struct JoinJob<'a> {
    pub parts: &'a [JoinPart],
    pub shape: &'a MergeShape,
    pub preset: &'a ExportPreset,
    /// Output path without extension (a free name is taken if it's in use).
    pub out_stem: PathBuf,
    /// Where the join's own work files go (the parts' folder).
    pub work_dir: &'a Path,
    pub title: Option<String>,
}

/// Join the parts into one file with a chapter each, and check it.
pub fn join_parts(
    tools: &Tools,
    job: &JoinJob,
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
) -> ExportResult<ClipOutcome> {
    let JoinJob { parts, shape, preset, work_dir, .. } = *job;
    if parts.is_empty() {
        return Err(ExportError::Join("nothing to join".into()));
    }
    let keep = shape.lossless.as_ref();
    // Checking the parts takes a moment for each: a fifth of the way. A
    // Bit-Perfect file is decoded and checked after it is written: half.
    let (probing, copied) = if keep.is_some() { (0.1, 0.5) } else { (0.2, 1.0) };
    let same_headers = probe_parts(tools, parts, cancel, |p| progress(p * probing))?;
    let t = shape.timescale;
    let list = work_dir.join(LIST);
    let chapters = work_dir.join(CHAPTERS);
    std::fs::write(&list, concat_script(parts, t)?)?;
    std::fs::write(&chapters, chapters_meta(parts, t, job.title.as_deref()))?;

    let ext = keep.map_or(preset.container.ext(), |k| k.codec.ext());
    let mov = keep.map_or(preset.container == Container::Mov, |k| k.codec == LosslessCodec::Hevc);
    if keep.is_some_and(|k| k.codec == LosslessCodec::Ffv1) && !same_headers {
        // FFV1 can't carry another part's stream header along.
        return Err(ExportError::Join("the parts' FFV1 stream headers differ".into()));
    }
    let joined = work_dir.join(format!("joined.{ext}"));
    // Full paths in the list (see `concat_script`).
    let mut a: Vec<String> = ["-v", "error", "-y", "-f", "concat", "-safe", "0"].map(String::from).to_vec();
    // With the same stream headers the parts join as they are; parts whose
    // headers differ (hardware encoders write the frame rate into them)
    // carry their own in the stream.
    if same_headers {
        a.extend(["-auto_convert".into(), "0".into()]);
    } else {
        tracing::info!(encoder = %shape.encoding.encoder, "the parts' stream headers differ; joining with each part's own");
    }
    a.extend(["-i".into(), list.display().to_string(), "-f".into(), "ffmetadata".into()]);
    a.extend(["-i".into(), chapters.display().to_string(), "-map".into(), "0:v:0".into()]);
    let audio = shape.audio.is_some() && preset.audio != AudioCodec::None;
    if audio {
        a.extend(["-map".into(), "0:a:0".into()]);
    }
    a.extend(["-map_metadata", "1", "-map_chapters", "1", "-c:v", "copy"].map(String::from));
    if matches!(preset.video, VideoCodec::Hevc { .. }) || keep.is_some_and(|k| k.codec == LosslessCodec::Hevc) {
        a.extend(["-tag:v".into(), "hvc1".into()]);
    }
    match preset.audio {
        AudioCodec::Pcm24 | AudioCodec::Lossless if audio => a.extend(["-c:a", "copy"].map(String::from)),
        // Encoded once over the whole file: no encoder delay between parts.
        AudioCodec::Aac { kbps } if audio => a.extend([
            "-filter:a".into(),
            "asetpts=N/SR/TB".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            format!("{kbps}k"),
        ]),
        _ => {}
    }
    if let Some(lang) = shape.language.as_ref().filter(|_| audio) {
        a.extend(["-metadata:s:a:0".into(), format!("language={lang}")]);
    }
    match keep {
        // An MKV times frames in milliseconds of its own.
        Some(k) if k.codec == LosslessCodec::Ffv1 => {}
        _ => {
            a.extend(["-video_track_timescale".into(), t.to_string(), "-movflags".into()]);
            a.push(if mov { "+write_colr" } else { "+faststart" }.into());
        }
    }
    let format = match keep {
        Some(k) => k.codec.format(),
        None if mov => "mov",
        None => "mp4",
    };
    a.extend(["-f".into(), format.into()]);
    a.push(joined.display().to_string());

    let total: u64 = parts.iter().map(|p| p.timing.ticks).sum();
    let seconds = (total as f64 / f64::from(t)).max(1e-3);
    let tidy = || {
        let _ = std::fs::remove_file(&list);
        let _ = std::fs::remove_file(&chapters);
    };
    let mut copying = |p: f64| progress(probing + (copied - probing) * p);
    run_ffmpeg(tools, &a, cancel, |p| copying((p.out_time / seconds).clamp(0.0, 1.0))).inspect_err(|_| {
        let _ = std::fs::remove_file(&joined);
        tidy();
    })?;
    tidy();
    let mut checked = check_joined(tools, &joined, parts, preset, audio, keep.is_some());
    if let Some(k) = keep.filter(|_| checked.is_ok()) {
        let checking = |p: f64| progress(copied + (1.0 - copied) * p);
        checked = check_joined_frames(tools, &joined, parts, cancel, checking)
            .and_then(|()| lossless::keeps_hdr(tools, &joined, &k.hdr))
            .map_err(|e| match e {
                Ok(why) => why,
                Err(e) => format!("{e:?}"),
            });
    }
    let verified = match checked {
        Ok(()) => true,
        // Bit-Perfect promises the sources: a file that isn't them isn't kept.
        Err(why) if keep.is_some() => {
            let _ = std::fs::remove_file(&joined);
            if cancel.is_cancelled() {
                return Err(MediaError::Cancelled.into());
            }
            return Err(ExportError::Inexact(format!("the joined file isn't its parts exactly ({why})")));
        }
        Err(why) => {
            tracing::error!(file = %joined.display(), reason = %why, "the joined file could not be verified");
            false
        }
    };
    let out = free_path(&job.out_stem, ext);
    std::fs::rename(&joined, &out)?;
    let bytes = std::fs::metadata(&out).map(|m| m.len() as f64).unwrap_or(0.0);
    Ok(ClipOutcome {
        path: out.display().to_string(),
        frames: parts.iter().map(|p| p.frames).sum(),
        verified,
        encoder: shape.encoding.encoder.clone(),
        bytes,
    })
}

/// `stem.ext`, or `stem_2.ext`… when that is taken (a rename would replace it).
pub(crate) fn free_path(stem: &Path, ext: &str) -> PathBuf {
    let first = with_suffix(stem, &format!(".{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|i| with_suffix(stem, &format!("_{i}.{ext}"))).find(|p| !p.exists()).expect("unbounded")
}

fn ffprobe_json(tools: &Tools, file: &Path, args: &[&str]) -> ExportResult<Value> {
    let out = as_media::proc::output(tools.ffprobe().args(args).args(["-of", "json"]).arg(file))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ExportError::Join(format!("{}: {msg}", file.display())));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| ExportError::Join(format!("{}: {e}", file.display())))
}

/// Are the parts' streams alike enough to join, and are their stream
/// headers the same byte for byte (`Ok(true)`)?
fn probe_parts(
    tools: &Tools,
    parts: &[JoinPart],
    cancel: &CancelToken,
    mut progress: impl FnMut(f64),
) -> ExportResult<bool> {
    const STREAMS: &str = "stream=codec_type,codec_name,profile,width,height,pix_fmt,sample_aspect_ratio,\
        field_order,color_range,color_space,color_transfer,color_primaries,time_base,sample_fmt,sample_rate,\
        channels,channel_layout,extradata_hash";
    let mut first: Option<(Vec<String>, Option<Value>)> = None;
    let mut same = true;
    for (i, p) in parts.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(MediaError::Cancelled.into());
        }
        progress(i as f64 / parts.len() as f64);
        let v = ffprobe_json(tools, &p.path, &["-show_data_hash", "CRC32", "-show_entries", STREAMS])?;
        let mut shape = Vec::new();
        let mut header = None;
        for st in v.get("streams").and_then(Value::as_array).into_iter().flatten() {
            let mut st = st.clone();
            let hash = st.as_object_mut().and_then(|o| {
                // Side data (light levels…) only describes the stream; it can't break a join.
                o.remove("side_data_list");
                o.remove("extradata_hash")
            });
            if st.get("codec_type").and_then(Value::as_str) == Some("video") {
                header = hash;
            }
            shape.push(st.to_string());
        }
        match &first {
            None => first = Some((shape, header)),
            Some((want, want_header)) => {
                if *want != shape {
                    return Err(ExportError::Join(format!(
                        "{} isn't like the first part: {} vs {}",
                        p.path.display(),
                        shape.join(" "),
                        want.join(" ")
                    )));
                }
                same &= *want_header == header;
            }
        }
    }
    Ok(same)
}

/// Whole microseconds closest to `ticks` of `t` a second.
fn micros(ticks: u64, t: u32) -> u64 {
    ((u128::from(ticks) * 2_000_000 + u128::from(t)) / (2 * u128::from(t))) as u64
}

/// The concat list: every part by its full path, each lasting exactly up to
/// where the next one starts (whatever its last frame says). Not by a name
/// relative to the list: ffmpeg resolves those like URLs, and a `#` in a
/// folder's name ("#Alive") would cut the path off there.
fn concat_script(parts: &[JoinPart], t: u32) -> ExportResult<String> {
    let mut s = String::from("ffconcat version 1.0\n");
    let mut at = 0u64;
    for p in parts {
        let path = std::path::absolute(&p.path)?;
        let path = path.to_str().ok_or_else(|| ExportError::Join(format!("{} can't be named", p.path.display())))?;
        let next = at + p.timing.ticks;
        // Quoted: only a quote itself needs escaping (`'\''`).
        s += &format!("file '{}'\nduration {}us\n", path.replace('\'', "'\\''"), micros(next, t) - micros(at, t));
        at = next;
    }
    Ok(s)
}

/// FFMETADATA with the file's title and a chapter per part.
fn chapters_meta(parts: &[JoinPart], t: u32, title: Option<&str>) -> String {
    let mut s = String::from(";FFMETADATA1\n");
    if let Some(title) = title {
        s += &format!("title={}\n", escape_meta(title));
    }
    let mut at = 0u64;
    for p in parts {
        let next = at + p.timing.ticks;
        s += &format!("[CHAPTER]\nTIMEBASE=1/{t}\nSTART={at}\nEND={next}\ntitle={}\n", escape_meta(&p.title));
        at = next;
    }
    s
}

/// FFMETADATA values escape `=`, `;`, `#`, `\` and line breaks.
fn escape_meta(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().filter(|&c| c != '\r') {
        if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Does a Bit-Perfect file decode to its parts' frames, as each was checked
/// against its source (`lossless::checked_frames`)?
fn check_joined_frames(
    tools: &Tools,
    file: &Path,
    parts: &[JoinPart],
    cancel: &CancelToken,
    progress: impl FnMut(f64),
) -> Result<(), Result<String, ExportError>> {
    let mut want = lossless::FrameHashes::default();
    for p in parts {
        let text = std::fs::read_to_string(lossless::checked_frames(&p.path)).map_err(|e| Err(e.into()))?;
        let part = lossless::read_framemd5(&text);
        if want.hashes.is_empty() {
            (want.dimensions, want.sar) = (part.dimensions, part.sar);
        }
        want.hashes.extend(part.hashes);
    }
    let frames = with_suffix(file, ".frames.txt");
    let n = u32::try_from(want.hashes.len()).unwrap_or(u32::MAX);
    lossless::decode_hashes(tools, file, &frames, None, n, cancel, progress).map_err(Err)?;
    let got = std::fs::read_to_string(&frames).map(|t| lossless::read_framemd5(&t));
    let _ = std::fs::remove_file(&frames);
    lossless::compare(&want, &got.map_err(|e| Err(e.into()))?, None).map_err(Ok)
}

/// Does the joined file have every part's frames, a chapter each and all
/// their sound? A Bit-Perfect file's frames are checked one by one
/// ([`check_joined_frames`]); its sound is counted by its frames' samples
/// (an MKV keeps no count of its own).
fn check_joined(
    tools: &Tools,
    file: &Path,
    parts: &[JoinPart],
    preset: &ExportPreset,
    audio: bool,
    lossless: bool,
) -> Result<(), String> {
    let v = ffprobe_json(tools, file, &["-show_entries", "stream=codec_type,nb_frames,duration_ts:chapter=id"])
        .map_err(|e| format!("{e:?}"))?;
    let streams = v.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    let stream = |ty: &str| streams.iter().find(|s| s.get("codec_type").and_then(Value::as_str) == Some(ty));
    let number = |s: &Value, k: &str| -> Option<u64> {
        match s.get(k)? {
            Value::String(x) => x.parse().ok(),
            Value::Number(n) => n.as_u64(),
            _ => None,
        }
    };
    let frames: u64 = parts.iter().map(|p| u64::from(p.frames)).sum();
    let got = stream("video").and_then(|s| number(s, "nb_frames"));
    if !lossless && got != Some(frames) {
        return Err(format!("{got:?} frames, expected {frames}"));
    }
    let chapters = v.get("chapters").and_then(Value::as_array).map_or(0, Vec::len);
    if chapters != parts.len() {
        return Err(format!("{chapters} chapters, expected {}", parts.len()));
    }
    if audio {
        let samples: u64 = parts.iter().map(|p| p.timing.samples).sum();
        let got = if lossless {
            sound_samples(tools, file).ok_or("no sound")?
        } else {
            stream("audio").and_then(|s| number(s, "duration_ts")).ok_or("no sound")?
        };
        // AAC adds its encoder delay at the start and pads its last frame.
        let slack = if matches!(preset.audio, AudioCodec::Pcm24 | AudioCodec::Lossless) { 0 } else { 2048 };
        if got.abs_diff(samples) > slack {
            return Err(format!("{got} audio samples, expected {samples}"));
        }
    }
    Ok(())
}

/// How many samples `file`'s first sound track holds, frame by frame.
fn sound_samples(tools: &Tools, file: &Path) -> Option<u64> {
    let out = as_media::proc::output(
        tools
            .ffprobe()
            .args(["-select_streams", "a:0", "-show_entries", "frame=nb_samples", "-of", "csv=p=0"])
            .arg(file),
    )
    .ok()
    .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let samples = text.lines().filter_map(|l| l.trim().trim_end_matches(',').parse::<u64>().ok());
    Some(samples.sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64, d: i64) -> Rational {
        Rational::new(n, d)
    }

    #[test]
    fn the_timescale_times_every_grid_exactly() {
        // 24, 23.976 and a millisecond (MKV) time base.
        assert_eq!(timescale(&[r(1, 24), r(1001, 24000), r(1, 1000)]), 24_000);
        // 29.97 joins in.
        assert_eq!(timescale(&[r(1001, 24000), r(1001, 30000)]), 120_000);
        // Coarse grids are doubled up to the mov muxer's minimum.
        assert_eq!(timescale(&[r(1, 24)]), 12_288);
        assert_eq!(timescale(&[r(1, 25), r(1, 24)]), 19_200);
        // Grids with no small common multiple: microseconds.
        assert_eq!(timescale(&[r(1, 12_288), r(1001, 24000), r(1, 90_000)]), MAX_TIMESCALE);
    }

    #[test]
    fn parts_add_up_to_the_sample() {
        let t = 24_000;
        // 23.976 fps: 1001 ticks a frame; 1, 7 and 1000 frames.
        let ticks = [1001, 7 * 1001, 1000 * 1001];
        let parts = timings(&ticks, t, AUDIO_RATE);
        let all: u64 = parts.iter().map(|p| p.samples).sum();
        let total: u64 = ticks.iter().sum();
        assert_eq!(all, (total as f64 * 48_000.0 / t as f64).round() as u64);
        // One frame of 23.976 is 2002 samples exactly.
        assert_eq!(parts[0].samples, 2002);
        for p in &parts {
            let exact = p.ticks as f64 * 48_000.0 / t as f64;
            assert!((p.samples as f64 - exact).abs() <= 1.0);
        }
        // 25 fps in 19200 ticks a second: 768 ticks a frame, 1920 samples.
        assert_eq!(
            timings(&[768, 768], 19_200, AUDIO_RATE).iter().map(|p| p.samples).collect::<Vec<_>>(),
            [1920, 1920]
        );
    }

    #[test]
    fn frames_keep_their_grid_or_their_own_times() {
        let cfr = FrameIndex::cfr(r(1, 24000), r(24000, 1001), 0, 100);
        assert_eq!(grid(&cfr), r(1001, 24000));
        assert_eq!(part_ticks(&cfr, FrameRange::new(10, 34), 24_000), 24 * 1001);
        // Variable rate: the frames' own times in the file's ticks.
        let vfr = FrameIndex::from_pts(r(1, 1000), r(24, 1), vec![0, 42, 83, 100, 117, 133, 167, 200]).unwrap();
        assert!(vfr.is_vfr());
        assert_eq!(grid(&vfr), r(1, 1000));
        assert_eq!(part_ticks(&vfr, FrameRange::new(1, 5), 24_000), (133 - 42) * 24);
    }

    fn info(w: u32, h: u32, sar: Rational, rotation: i32) -> MediaInfo {
        let v: Value = serde_json::json!({
            "container": "mov", "durationSec": null, "startSec": 0.0, "sizeBytes": null, "bitRate": null,
            "video": {
                "index": 0, "codec": "h264", "profile": null, "pixFmt": "yuv420p", "bitDepth": 8,
                "width": w, "height": h, "sar": sar, "rFrameRate": r(24, 1), "avgFrameRate": null,
                "timeBase": r(1, 1000), "colorPrimaries": null, "colorTransfer": null, "colorSpace": null,
                "colorRange": null, "fieldOrder": null, "rotation": rotation, "hdr": "none"
            },
            "audio": [], "subtitles": []
        });
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn nvenc_is_capped_for_the_file_and_its_fastest_part() {
        use as_media::{CodecFamily, HwEncoder, Vendor};
        let nvenc = HwEncoder { name: "h264_nvenc".into(), family: CodecFamily::H264, vendor: Vendor::Nvidia };
        let caps = Caps { hw_encoders: vec![nvenc], ..Caps::default() };
        let mut film = info(1920, 1080, r(1, 1), 0);
        film.video.r_frame_rate = r(24000, 1001);
        let mut fast = info(1280, 720, r(1, 1), 0);
        fast.video.r_frame_rate = r(60000, 1001);
        let (slow_index, fast_index) = (
            FrameIndex::cfr(r(1, 24000), r(24000, 1001), 0, 1000),
            FrameIndex::cfr(r(1, 60000), r(60000, 1001), 0, 1000),
        );
        let sources = [
            MergeSource { info: &film, index: &slow_index, range: FrameRange::new(0, 500), audio_ordinal: None },
            MergeSource { info: &fast, index: &fast_index, range: FrameRange::new(0, 100), audio_ordinal: None },
        ];
        let preset = crate::builtin_presets().into_iter().find(|p| p.id == "h264-edit").unwrap();
        let args = plan_merge(&caps, &preset, &sources).unwrap().shape.encoding.args.join(" ");
        // Keyframes a second of the film most frames are of; the cap for the
        // file's 1080p picture at the 59.94 fps the fast part's encoder is told.
        let max = crate::clip::nvenc_maxrate(crate::preset::Gop::Editing, false, (1920, 1080), 60000.0 / 1001.0);
        assert!(args.contains("-g 24 "), "{args}");
        assert!(args.contains(&format!("-maxrate {:.0}k", max / 1000.0)), "{args}");
    }

    #[test]
    fn pictures_fit_the_frame() {
        // Anamorphic PAL 16:9 shows 1024×576 square pixels.
        assert_eq!(natural_size(&info(720, 576, r(64, 45), 0), None), (1024, 576));
        // A rotated phone clip stands upright.
        assert_eq!(natural_size(&info(1920, 1080, r(1, 1), 90), None), (1080, 1920));
        // Capped at the preset's height.
        assert_eq!(natural_size(&info(3840, 2160, r(1, 1), 0), Some(1080)), (1920, 1080));
        assert_eq!(natural_size(&info(1280, 720, r(1, 1), 0), Some(1080)), (1280, 720));
        // 4:3 in a 16:9 frame is pillarboxed; 16:9 SD is scaled up to fill it.
        assert_eq!(fit((640, 480), (1920, 1080)), (1440, 1080));
        assert_eq!(fit((1024, 576), (1920, 1080)), (1920, 1080));
        // Portrait in landscape.
        assert_eq!(fit((1080, 1920), (1920, 1080)), (608, 1080));
        assert_eq!(fit((1920, 1080), (1920, 1080)), (1920, 1080));
        // A crafted pixel shape is scaled down to 16K a side, whichever way up.
        assert_eq!(natural_size(&info(1920, 1080, r(i64::from(i32::MAX), 1), 0), None), (MAX_SIDE, 2));
        assert_eq!(natural_size(&info(1920, 1080, r(65535, 1), 0), Some(1080)), (MAX_SIDE, 2));
        assert_eq!(natural_size(&info(1920, 1080, r(65535, 1), 90), None), (2, 1920));
        assert_eq!(natural_size(&info(1920, 1080, r(20, 1), 0), None), (MAX_SIDE, 460));
        // One that isn't a positive number (an edited record) is square.
        let mut odd = info(1920, 1080, r(1, 1), 90);
        for sar in [Rational { num: 0, den: 1 }, Rational { num: 1, den: 0 }] {
            odd.video.sar = sar;
            assert_eq!(natural_size(&odd, None), (1080, 1920), "{sar:?}");
        }
        assert_eq!([even(f64::NAN), even(f64::INFINITY), even(-4.0), even(1e12)], [2, MAX_SIDE, 2, MAX_SIDE]);
    }

    #[test]
    fn a_clip_past_its_episode_is_refused_not_looked_up() {
        // Variable rate: its length comes from its frames' own times.
        let vfr = FrameIndex::from_pts(r(1, 1000), r(24, 1), vec![0, 42, 83, 100, 117, 133, 167, 200]).unwrap();
        assert!(vfr.is_vfr());
        let film = info(1920, 1080, r(1, 1), 0);
        let caps = Caps { sw_encoders: vec!["prores_ks".into()], ..Caps::default() };
        let prores = crate::builtin_presets().remove(0);
        let sources = |range| [MergeSource { info: &film, index: &vfr, range, audio_ordinal: None }];
        for range in [FrameRange::new(2, 20), FrameRange::new(8, 9), FrameRange::new(3, 3)] {
            let refused = |r: ExportResult<MergePlan>| matches!(r, Err(ExportError::BadRange(_)));
            assert!(refused(plan_merge(&caps, &prores, &sources(range))), "{range:?}");
            assert!(refused(plan_lossless_merge(&caps, &sources(range), &[ExactFormat::default()])), "{range:?}");
        }
        assert!(plan_merge(&caps, &prores, &sources(FrameRange::new(2, 8))).is_ok());
    }

    #[test]
    fn only_plain_names_reach_a_filter_graph() {
        let mut tagged = info(1920, 1080, r(1, 1), 0);
        tagged.video.color_primaries = Some("bt709[x];movie=a".into());
        tagged.video.color_transfer = Some("bt709".into());
        tagged.video.color_range = Some("tv:colorspace=gbr".into());
        let tags = ColorTags::of(&tagged);
        assert_eq!(
            (tags.primaries, tags.transfer.as_deref(), tags.space, tags.range),
            (None, Some("bt709"), None, None)
        );
        let caps = Caps { sw_encoders: vec!["prores_ks".into()], ..Caps::default() };
        let index = FrameIndex::cfr(r(1, 24), r(24, 1), 0, 100);
        let sources =
            [MergeSource { info: &tagged, index: &index, range: FrameRange::new(0, 10), audio_ordinal: None }];
        let plan = plan_merge(&caps, &crate::builtin_presets().remove(0), &sources).unwrap();
        let chain = plan.shape.picture(&tagged, &index).join(",");
        assert!(chain.ends_with(",setparams=color_trc=bt709"), "{chain}");

        let stream = |channels: u32, layout: &str| AudioStream {
            index: 1,
            ordinal: 0,
            codec: "pcm_s16le".into(),
            profile: None,
            channels,
            channel_layout: Some(layout.into()),
            sample_rate: 48_000,
            language: None,
            title: None,
            default: true,
            commentary: false,
        };
        assert_eq!(layout_of(&stream(2, "stereo[a];movie=x")), "stereo");
        assert_eq!(layout_of(&stream(6, "5.1:cl=mono")), "5.1");
        assert_eq!(layout_of(&stream(2, "")), "stereo");
        // What ffprobe prints passes as it is.
        for layout in ["3 channels (FL+FR+TFL)", "5.1(side)", "7.1(wide-side)", "ambisonic 1", "22.2"] {
            assert_eq!(layout_of(&stream(3, layout)), layout);
        }
    }

    #[test]
    fn metadata_and_lists_are_escaped_and_exact() {
        assert_eq!(escape_meta("a=b;c#d\\e\nf\r"), "a\\=b\\;c\\#d\\\\e\\\nf");
        let work = std::env::temp_dir().join("Scenes #1 it's");
        let part = |n: &str, ticks| JoinPart {
            path: work.join(n),
            frames: 1,
            timing: PartTiming { ticks, samples: 0 },
            title: "S01E01 = 1".into(),
        };
        let parts = [part("part-00001.mov", 1001), part("part-00002.mov", 1001), part("part-00003.mov", 1001)];
        let list = concat_script(&parts, 24_000).unwrap();
        let quoted = |n: &str| work.join(n).display().to_string().replace('\'', "'\\''");
        // 41708.33 µs each, rounded so the starts stay exact: 41708, 41709, 41708.
        assert_eq!(
            list,
            format!(
                "ffconcat version 1.0\nfile '{}'\nduration 41708us\nfile '{}'\nduration 41709us\nfile '{}'\nduration 41708us\n",
                quoted("part-00001.mov"),
                quoted("part-00002.mov"),
                quoted("part-00003.mov")
            )
        );
        assert!(list.contains("Scenes #1 it'\\''s"), "{list}");
        let meta = chapters_meta(&parts[..2], 24_000, Some("Walter · scenepack"));
        assert_eq!(
            meta,
            ";FFMETADATA1\ntitle=Walter · scenepack\n[CHAPTER]\nTIMEBASE=1/24000\nSTART=0\nEND=1001\ntitle=S01E01 \\= 1\n[CHAPTER]\nTIMEBASE=1/24000\nSTART=1001\nEND=2002\ntitle=S01E01 \\= 1\n"
        );
    }

    #[test]
    fn sound_takes_one_layout() {
        let stream = |channels: u32, layout: Option<&str>, lang: Option<&str>| AudioStream {
            index: 1,
            ordinal: 0,
            codec: "aac".into(),
            profile: None,
            channels,
            channel_layout: layout.map(String::from),
            sample_rate: 48_000,
            language: lang.map(String::from),
            title: None,
            default: true,
            commentary: false,
        };
        let with = |a: Vec<AudioStream>| MediaInfo { audio: a, ..info(640, 360, r(1, 1), 0) };
        let index = FrameIndex::cfr(r(1, 24), r(24, 1), 0, 100);
        let plan = |infos: &[MediaInfo], preset: &ExportPreset| {
            let sources: Vec<MergeSource> = infos
                .iter()
                .map(|i| MergeSource { info: i, index: &index, range: FrameRange::new(0, 10), audio_ordinal: None })
                .collect();
            sound_plan(preset, &sources)
        };
        let prores = crate::builtin_presets().remove(0);
        let five = with(vec![stream(6, Some("5.1(side)"), Some("eng"))]);
        let stereo = with(vec![stream(2, Some("stereo"), Some("en"))]);
        let silent = with(vec![]);
        assert_eq!(plan(&[five.clone(), five.clone()], &prores), (Some("5.1(side)".into()), Some("eng".into())));
        assert_eq!(plan(&[five.clone(), stereo.clone()], &prores), (Some("stereo".into()), Some("eng".into())));
        assert_eq!(plan(&[silent.clone(), with(vec![stream(1, None, None)])], &prores), (Some("mono".into()), None));
        assert_eq!(plan(&[silent.clone(), silent], &prores).0, None);
        let mut mobile = prores.clone();
        mobile.stereo = true;
        assert_eq!(plan(std::slice::from_ref(&five), &mobile).0.as_deref(), Some("stereo"));
        mobile.audio = AudioCodec::None;
        assert_eq!(plan(&[five], &mobile).0, None);
    }
}
