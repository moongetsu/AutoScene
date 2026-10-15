//! Scenepacks: a character's shots, filtered, ranked and picked by hand.
//! This is the one place that decides what a pack holds — for the grid, the
//! export plan and the export alike — so what is exported is what was shown.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use as_api::{
    CharacterShot, ClipRequest, ExportPlan, ExportRequest, MediaItem, MediaStatus, PackClip, PackExportRequest,
    PackKey, PackNaming, PackOrder, PackPreview, PackReason, PackSpec, PackState, PackTotals, PresetSize, ShotSize,
};
use as_core::{CharacterId, FrameIdx, FrameIndex, MediaId};
use as_db::PackRow;
use as_export::{ExportPreset, estimate_bytes};

use crate::cast::track_id;
use crate::naming::{NameParts, check_template, clip_timecode, render_name, safe_component};
use crate::project::episode_name;
use crate::{Project, ProjectError, ProjectResult};

// What makes a good clip, for "best first" and the limit. The weights add up
// to 1, so a score is 0..1.
/// How clearly the face shows who it is.
const W_QUALITY: f32 = 0.35;
/// How big the face is.
const W_SIZE: f32 = 0.30;
/// How much of the shot the character is on screen.
const W_COVERAGE: f32 = 0.20;
/// How well the shot's length cuts into an edit.
const W_LENGTH: f32 = 0.10;
/// Being alone in the shot.
const W_SOLO: f32 = 0.05;
/// A face this tall (share of the frame) or taller counts as full size.
const FULL_SIZE: f32 = 0.45;
/// Shot lengths that cut well into an edit, in seconds.
const GOOD_LENGTH: (f64, f64) = (1.0, 6.0);
/// Names a plan shows of a pack's files (the export dialog lists the first few).
const PLAN_NAMES: usize = 3;

/// How well a shot of `seconds` cuts into an edit: 1 inside the good range,
/// falling off outside it.
fn length_fit(seconds: f64) -> f32 {
    let (lo, hi) = GOOD_LENGTH;
    (if seconds < lo {
        seconds / lo
    } else if seconds > hi {
        hi / seconds
    } else {
        1.0
    }) as f32
}

/// A pack, made from the analysis as it stands.
struct Made {
    clips: Vec<PackClip>,
    totals: PackTotals,
    stamp: String,
    /// The episodes it was made from.
    sources: HashMap<MediaId, Source>,
}

/// An episode a pack can cut from, with its frame grid.
struct Source {
    media: MediaItem,
    /// `None` while it can't be cut from (its file is missing, or it isn't indexed).
    index: Option<Arc<FrameIndex>>,
}

impl Project {
    /// A character's pack settings as last used (the defaults the first time).
    pub fn pack_state(&self, c: CharacterId) -> ProjectResult<PackState> {
        let Some((spec, naming)) = self.db().pack_state(c)? else { return Ok(PackState::default()) };
        Ok(PackState { spec: with_defaults(&spec), naming: naming.map(|n| with_defaults(&n)).unwrap_or_default() })
    }

    /// Keep a character's pack settings.
    pub fn save_pack_state(&self, c: CharacterId, state: &PackState) -> ProjectResult<()> {
        let spec = serde_json::to_string(&state.spec).expect("a spec serialises");
        let naming = serde_json::to_string(&state.naming).expect("naming serialises");
        if !self.db().set_pack_state(c, &spec, Some(&naming))? {
            return Err(gone());
        }
        Ok(())
    }

    /// Pick (`Some(true)`), drop (`Some(false)`) or hand back to the filters
    /// (`None`) these shots of a character's pack.
    pub fn set_pack_overrides(&self, c: CharacterId, shots: &[PackKey], include: Option<bool>) -> ProjectResult<()> {
        // A pick is kept on the frame its shot starts at. When the shot was
        // found again since, an older pick lies inside it: it goes too.
        let now = shots_of(c, &self.db().pack_rows(c)?);
        let spans: Vec<(MediaId, FrameIdx, FrameIdx)> = shots
            .iter()
            .map(|k| {
                now.iter()
                    .find(|s| s.media == k.media_id && s.start == k.shot)
                    .map_or((k.media_id, k.shot, k.shot + 1), |s| (s.media, s.start, s.end))
            })
            .collect();
        if !self.db().set_pack_overrides(c, &spans, include)? {
            return Err(gone());
        }
        Ok(())
    }

    /// Every shot of the character, the ones `spec` (and the hand picks)
    /// take marked, in the pack's order.
    pub fn pack_preview(&self, c: CharacterId, spec: &PackSpec) -> ProjectResult<PackPreview> {
        let Made { clips, totals, stamp, .. } = self.make_pack(c, spec)?;
        Ok(PackPreview { clips, totals, stamp })
    }

    /// The first names the pack's files get (in its order), the character's
    /// folder and the pack's size per preset.
    pub fn pack_plan(
        &self,
        c: CharacterId,
        spec: &PackSpec,
        naming: &PackNaming,
        presets: &[ExportPreset],
    ) -> ProjectResult<ExportPlan> {
        let made = self.make_pack(c, spec)?;
        let clips = self.named_clips(c, &made, naming)?;
        let mut sizes = vec![0.0; presets.len()];
        for clip in &clips {
            let Some(Source { media, index: Some(index) }) = made.sources.get(&clip.media_id) else { continue };
            let Some(info) = &media.info else { continue };
            let seconds = index.t(clip.end) - index.t(clip.start);
            for (size, preset) in sizes.iter_mut().zip(presets) {
                *size += estimate_bytes(preset, info, media.audio_ordinal, u64::from(clip.end - clip.start), seconds);
            }
        }
        let folder = match naming.folder {
            true => Some(safe_component(&self.db().character_label(c)?.ok_or_else(gone)?, " ", 80)),
            false => None,
        };
        Ok(ExportPlan {
            names: clips.into_iter().filter_map(|c| c.name).take(PLAN_NAMES).collect(),
            sizes: presets.iter().zip(sizes).map(|(p, bytes)| PresetSize { preset_id: p.id.clone(), bytes }).collect(),
            folder,
        })
    }

    /// The export of a character's pack, made again from the analysis: what
    /// the grid showed (`req.stamp`), or an error when the pack changed since.
    /// Returns the request and the folder it goes to (inside `default_dir`
    /// unless the request names another, in the character's own folder if
    /// asked). The settings are kept for next time.
    pub fn pack_export_request(
        &self,
        c: CharacterId,
        req: &PackExportRequest,
        default_dir: &Path,
    ) -> ProjectResult<(ExportRequest, PathBuf)> {
        check_template(&req.naming.template).map_err(|e| ProjectError::Invalid(e.to_string()))?;
        let made = self.make_pack(c, &req.spec)?;
        if made.stamp != req.stamp {
            return Err(ProjectError::Invalid("The pack changed meanwhile. Check it and export again.".into()));
        }
        if made.totals.clips == 0 {
            return Err(ProjectError::Invalid("There's nothing in the pack to export.".into()));
        }
        let label = self.db().character_label(c)?.ok_or_else(gone)?;
        let clips = self.named_clips(c, &made, &req.naming)?;
        let base = req
            .out_dir
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map_or(default_dir.to_path_buf(), PathBuf::from);
        let out_dir = if req.naming.folder { base.join(safe_component(&label, " ", 80)) } else { base };
        self.save_pack_state(c, &PackState { spec: req.spec.clone(), naming: req.naming.clone() })?;
        let request = ExportRequest {
            preset_id: req.preset_id.clone(),
            out_dir: Some(out_dir.display().to_string()),
            clips,
            label: Some(format!("{label} · scenepack")),
        };
        Ok((request, out_dir))
    }

    /// The pack's clips (the ones it takes, in its order), named by `naming`.
    fn named_clips(&self, c: CharacterId, made: &Made, naming: &PackNaming) -> ProjectResult<Vec<ClipRequest>> {
        check_template(&naming.template).map_err(|e| ProjectError::Invalid(e.to_string()))?;
        let title = self.title()?;
        let label = self.db().character_label(c)?.ok_or_else(gone)?;
        let taken: Vec<&PackClip> = made.clips.iter().filter(|c| c.included).collect();
        let mut used = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(taken.len());
        for (i, clip) in taken.iter().enumerate() {
            // Only clips of episodes with a frame grid are taken.
            let Some(source @ Source { index: Some(index), .. }) = made.sources.get(&clip.shot.media_id) else {
                continue;
            };
            let fps = index.nominal_fps;
            let (tc, episode) = (clip_timecode(clip.start, fps), episode_name(&source.media));
            let parts = NameParts {
                series: &title,
                character: Some(&label),
                episode: &episode,
                index: (i + 1, taken.len()),
                tc: &tc,
                frame: clip.start,
            };
            let base = render_name(&naming.template, &parts).map_err(|e| ProjectError::Invalid(e.to_string()))?;
            // Two clips can't share a name (a template without {tc} or {index}).
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.to_lowercase()) {
                name = format!("{base}_{n}");
                n += 1;
            }
            out.push(ClipRequest { media_id: clip.shot.media_id, start: clip.start, end: clip.end, name: Some(name) });
        }
        Ok(out)
    }

    fn make_pack(&self, c: CharacterId, spec: &PackSpec) -> ProjectResult<Made> {
        let rows = self.db().pack_rows(c)?;
        let overrides = self.db().pack_overrides(c)?;
        // Every episode, in project order, with its frame grid when it can be cut from.
        let mut sources = HashMap::new();
        let mut order = HashMap::new();
        for (i, media) in self.media_list()?.into_iter().enumerate() {
            let index = match media.status {
                MediaStatus::Ready => self.frame_index(media.id).ok(),
                _ => None,
            };
            order.insert(media.id, i);
            sources.insert(media.id, Source { media, index });
        }
        let (clips, totals, stamp) = make(c, spec, &rows, &sources, &order, &overrides, |m, shot, end, track| {
            (
                Project::path_string(&self.thumb_path(m, track)),
                end.map(|end| Project::path_string(&self.sprite_path(m, shot, end))),
            )
        });
        Ok(Made { clips, totals, stamp, sources })
    }
}

/// Settings saved by another version: what is there, the defaults for the rest.
fn with_defaults<T: Default + serde::Serialize + serde::de::DeserializeOwned>(json: &str) -> T {
    let mut merged = serde_json::to_value(T::default()).expect("defaults serialise");
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(stored)) => {
            if let serde_json::Value::Object(fields) = &mut merged {
                fields.extend(stored);
            }
        }
        _ => tracing::warn!("unreadable pack settings; using the defaults"),
    }
    serde_json::from_value(merged).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "pack settings of an unknown shape; using the defaults");
        T::default()
    })
}

fn gone() -> ProjectError {
    ProjectError::NotFound("This character isn't in the cast any more.".into())
}

/// A shot of the character, worked out from its faces.
struct Shot {
    media: MediaId,
    start: FrameIdx,
    end: FrameIdx,
    tiny: bool,
    sprite: Option<(FrameIdx, u32)>,
    /// The character's clearest face in it.
    track: i64,
    quality: f32,
    /// The biggest the character's face is in it.
    face_h: f32,
    /// Frames the character is on screen.
    on_screen: u32,
    /// Other (shown) characters on screen in it.
    others: u32,
}

/// The shots of `c` from `rows` (ordered by media and shot, `c`'s clearest face first).
fn shots_of(c: CharacterId, rows: &[PackRow]) -> Vec<Shot> {
    let mut out: Vec<Shot> = Vec::new();
    let mut spans: Vec<(FrameIdx, FrameIdx)> = Vec::new();
    let mut others: Vec<CharacterId> = Vec::new();
    let mut flush = |shot: Option<Shot>, spans: &mut Vec<(FrameIdx, FrameIdx)>, others: &mut Vec<CharacterId>| {
        if let Some(mut s) = shot {
            spans.sort_unstable();
            let (mut covered, mut reach) = (0, s.start);
            for &(a, b) in spans.iter() {
                let (a, b) = (a.max(reach).max(s.start), b.min(s.end));
                if b > a {
                    covered += b - a;
                    reach = b;
                }
            }
            others.sort_unstable();
            others.dedup();
            s.on_screen = covered;
            s.others = others.len() as u32;
            out.push(s);
        }
        spans.clear();
        others.clear();
    };
    let mut current: Option<Shot> = None;
    for r in rows {
        let same = current.as_ref().is_some_and(|s| s.media == r.media_id && s.start == r.shot);
        if !same {
            flush(current.take(), &mut spans, &mut others);
            // The first row of a shot is the character's clearest face there.
            current = Some(Shot {
                media: r.media_id,
                start: r.shot,
                end: r.shot_end,
                tiny: r.tiny,
                sprite: r.sprite,
                track: r.track,
                quality: r.quality,
                face_h: 0.0,
                on_screen: 0,
                others: 0,
            });
        }
        let s = current.as_mut().expect("just set");
        if r.character_id == c {
            s.face_h = s.face_h.max(r.face_h);
            spans.push((r.start, r.end));
        } else if !r.hidden {
            others.push(r.character_id);
        }
    }
    flush(current.take(), &mut spans, &mut others);
    out
}

/// The pack itself — its clips, totals and stamp: pure, so every rule is
/// tested without a database. `pictures(media, shot, sprite_end, track)`
/// names the face and sprite files.
fn make(
    c: CharacterId,
    spec: &PackSpec,
    rows: &[PackRow],
    sources: &HashMap<MediaId, Source>,
    order: &HashMap<MediaId, usize>,
    overrides: &[(MediaId, FrameIdx, bool)],
    pictures: impl Fn(MediaId, FrameIdx, Option<FrameIdx>, i64) -> (String, Option<String>),
) -> (Vec<PackClip>, PackTotals, String) {
    let shots = shots_of(c, rows);
    // Picks by hand: on the shot that starts there, or — when shots were
    // found again since — on the shot that now holds that frame.
    let mut manual: HashMap<(MediaId, FrameIdx), bool> = HashMap::new();
    for &(m, f, include) in overrides {
        let at = shots
            .iter()
            .find(|s| s.media == m && s.start == f)
            .or_else(|| shots.iter().find(|s| s.media == m && s.start <= f && f < s.end));
        if let Some(s) = at {
            // An exact match wins over a moved one.
            if s.start == f || !manual.contains_key(&(m, s.start)) {
                manual.insert((m, s.start), include);
            }
        }
    }

    struct Cand {
        clip: PackClip,
        rule: Option<PackReason>,
        seconds_with_handles: f64,
    }
    let mut cands: Vec<Cand> = Vec::with_capacity(shots.len());
    for s in &shots {
        let source = sources.get(&s.media);
        let index = source.and_then(|x| x.index.as_ref());
        let seconds = index.map_or(0.0, |i| i.t(s.end.min(i.len())) - i.t(s.start.min(i.len())));
        let size = ShotSize::of(s.face_h);
        let coverage = if s.end > s.start { s.on_screen as f32 / (s.end - s.start) as f32 } else { 0.0 };
        let score = W_QUALITY * s.quality.clamp(0.0, 1.0)
            + W_SIZE * (s.face_h / FULL_SIZE).min(1.0)
            + W_COVERAGE * coverage.min(1.0)
            + W_LENGTH * length_fit(seconds)
            + W_SOLO * if s.others == 0 { 1.0 } else { 0.0 };
        let rule = if index.is_none() {
            Some(PackReason::Missing)
        } else if !spec.media.is_empty() && !spec.media.contains(&s.media) {
            Some(PackReason::Episode)
        } else if spec.skip_tiny && s.tiny {
            Some(PackReason::Tiny)
        } else if spec.min_sec.is_some_and(|min| seconds < min) {
            Some(PackReason::Short)
        } else if spec.max_sec.is_some_and(|max| seconds > max) {
            Some(PackReason::Long)
        } else if !spec.sizes.is_empty() && !spec.sizes.contains(&size) {
            Some(PackReason::Size)
        } else if spec.solo && s.others > 0 {
            Some(PackReason::NotSolo)
        } else {
            None
        };
        let (start, end) = match index {
            Some(i) => (s.start.saturating_sub(spec.handles), (s.end + spec.handles).min(i.len())),
            None => (s.start, s.end),
        };
        let seconds_with_handles = index.map_or(0.0, |i| i.t(end) - i.t(start));
        let (face, sprite) = pictures(s.media, s.start, s.sprite.map(|x| x.0), s.track);
        let shot = CharacterShot {
            media_id: s.media,
            start: s.start,
            end: s.end,
            sprite,
            cells: s.sprite.map_or(0, |x| x.1),
            track_id: track_id(s.track),
            face,
            face_height: s.face_h,
        };
        let clip = PackClip {
            shot,
            start,
            end,
            size,
            quality: s.quality,
            coverage,
            others: s.others,
            score,
            included: false,
            by_rules: rule.is_none(),
            reason: rule,
            manual: manual.get(&(s.media, s.start)).copied(),
        };
        cands.push(Cand { clip, rule, seconds_with_handles });
    }

    // The limit keeps the best of what the filters take; picks by hand
    // stay on top of it and don't take a place. A missing file can't be
    // picked back in: there is nothing to cut from.
    if let Some(limit) = spec.limit {
        let limit = limit as usize;
        let ahead = |a: usize, b: usize| cands[b].clip.score.total_cmp(&cands[a].clip.score).then(a.cmp(&b));
        let mut kept: Vec<usize> =
            (0..cands.len()).filter(|&i| cands[i].rule.is_none() && cands[i].clip.manual.is_none()).collect();
        kept.sort_by(|&a, &b| ahead(a, b));
        let past: Vec<usize> = kept.iter().skip(limit).copied().collect();
        // A pick by hand takes no place; the filters alone would take it
        // if it made the limit without the pick.
        let picked_past: Vec<usize> = (0..cands.len())
            .filter(|&i| cands[i].rule.is_none() && cands[i].clip.manual.is_some())
            .filter(|&i| kept.iter().filter(|&&k| ahead(k, i).is_lt()).count() >= limit)
            .collect();
        for i in past.into_iter().chain(picked_past) {
            cands[i].clip.by_rules = false;
            cands[i].rule = Some(PackReason::Limit);
        }
    }
    for cand in &mut cands {
        let missing = cand.rule == Some(PackReason::Missing);
        let clip = &mut cand.clip;
        clip.included = !missing && clip.manual.unwrap_or(cand.rule.is_none());
        clip.reason = match clip.manual {
            _ if missing => Some(PackReason::Missing),
            Some(false) => Some(PackReason::Removed),
            Some(true) => None,
            None => cand.rule,
        };
    }

    let episode = |c: &PackClip| (order.get(&c.shot.media_id).copied().unwrap_or(usize::MAX), c.shot.start);
    match spec.order {
        PackOrder::Episodes => cands.sort_by_key(|x| episode(&x.clip)),
        PackOrder::Best => cands.sort_by(|a, b| {
            b.clip.score.total_cmp(&a.clip.score).then_with(|| episode(&a.clip).cmp(&episode(&b.clip)))
        }),
    }

    let mut totals = PackTotals { clips: 0, frames: 0.0, seconds: 0.0 };
    let mut stamp = Fnv::new();
    for cand in cands.iter().filter(|x| x.clip.included) {
        totals.clips += 1;
        totals.frames += f64::from(cand.clip.end - cand.clip.start);
        totals.seconds += cand.seconds_with_handles;
        stamp.write(cand.clip.shot.media_id.to_string().as_bytes());
        stamp.write(&cand.clip.start.to_le_bytes());
        stamp.write(&cand.clip.end.to_le_bytes());
    }
    (cands.into_iter().map(|x| x.clip).collect(), totals, format!("{:016x}", stamp.0))
}

/// FNV-1a: a short fingerprint of what a pack takes.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use as_analysis::FaceDet;
    use as_analysis::tracks::{self, FaceTrack};
    use as_api::{CastEdit, Segment, SegmentKind, TrackFilter};

    use super::*;
    use crate::analysis::tests::project_with_media;
    use crate::cast::tests::seen;

    const FRAMES: u32 = 2400;

    /// `t` with a face `height` tall (share of the frame) in every detection.
    fn sized(mut t: FaceTrack, height: f32) -> FaceTrack {
        t.dets = t
            .dets
            .into_iter()
            .map(|d| FaceDet { bbox: [d.bbox[0], 0.3, d.bbox[0] + height * 0.6, 0.3 + height], ..d })
            .collect();
        t
    }

    struct Pack {
        dir: tempfile::TempDir,
        project: Arc<Project>,
        media: MediaId,
        walter: CharacterId,
        jesse: CharacterId,
    }

    /// The faces of one 24 fps episode (see [`pack`]); `split` is where the
    /// last shot starts.
    fn faces(split: FrameIdx) -> [FaceTrack; 5] {
        [
            sized(seen(1, 0, 0, 48, 0.1), 0.3),
            // Jesse: the only face 0.2 tall.
            sized(seen(2, 0, 0, 48, 0.6), 0.2),
            sized(seen(1, 48, 48, 50, 0.1), 0.3),
            sized(seen(1, 50, 50, split, 0.1), 0.5),
            sized(seen(1, split, 290, 350, 0.1), 0.05),
        ]
    }

    fn shots(split: FrameIdx) -> Vec<Segment> {
        let shot = |start, end| Segment { start, end, kind: SegmentKind::Shot, tiny: end - start < 3 };
        vec![shot(0, 48), shot(48, 50), shot(50, split), shot(split, 410)]
    }

    /// One 24 fps episode of 100 s: shots 0–48 (2 s, Walter with Jesse),
    /// 48–50 (a flash cut, Walter), 50–290 (10 s, Walter big all along) and
    /// 290–410 (5 s, Walter small for half of it).
    fn pack() -> Pack {
        let (dir, project, media) = project_with_media(FRAMES);
        project.db().store_segments(media, &shots(290)).unwrap();
        project.store_face_chunk(media, 0, FRAMES, &tracks::encode(&faces(290)), &[]).unwrap();
        project.finish_faces(media).unwrap();
        let tracks = project.cast_tracks(TrackFilter::Unassigned).unwrap();
        let is_jesse = |h: f32| (h - 0.2).abs() < 0.01;
        let pick = |jesse: bool| -> Vec<u32> {
            tracks.iter().filter(|t| is_jesse(t.face_height) == jesse).map(|t| t.id).collect()
        };
        assert_eq!((pick(false).len(), pick(true).len()), (4, 1));
        let new = |tracks, name: &str| {
            project.cast_edit(&CastEdit::NewCharacter { tracks, name: Some(name.into()) }).unwrap().unwrap()
        };
        let walter = new(pick(false), "Walter");
        let jesse = new(pick(true), "Jesse");
        Pack { dir, project, media, walter, jesse }
    }

    fn preview(p: &Pack, spec: &PackSpec) -> PackPreview {
        p.project.pack_preview(p.walter, spec).unwrap()
    }

    fn starts(v: &PackPreview, included: bool) -> Vec<FrameIdx> {
        v.clips.iter().filter(|c| c.included == included).map(|c| c.shot.start).collect()
    }

    fn clip(v: &PackPreview, start: FrameIdx) -> &PackClip {
        v.clips.iter().find(|c| c.shot.start == start).unwrap()
    }

    fn with(f: impl Fn(&mut PackSpec)) -> PackSpec {
        let mut s = PackSpec::default();
        f(&mut s);
        s
    }

    #[test]
    fn every_shot_of_the_character_in_episode_order() {
        let p = pack();
        let v = preview(&p, &PackSpec::default());
        assert_eq!(v.clips.iter().map(|c| c.shot.start).collect::<Vec<_>>(), [0, 48, 50, 290]);
        // Flash cuts stay out by default; every other shot is in.
        assert_eq!(starts(&v, true), [0, 50, 290]);
        assert_eq!(clip(&v, 48).reason, Some(PackReason::Tiny));
        // What each shot is like.
        let (a, b, c) = (clip(&v, 0), clip(&v, 50), clip(&v, 290));
        assert_eq!((a.size, b.size, c.size), (ShotSize::CloseUp, ShotSize::ExtremeCloseUp, ShotSize::Wide));
        assert_eq!((a.others, b.others), (1, 0), "Jesse is with him in shot 0");
        assert!((b.coverage - 1.0).abs() < 1e-6 && (c.coverage - 0.5).abs() < 1e-6, "{} {}", b.coverage, c.coverage);
        assert!(b.score > a.score && b.score > c.score, "big, alone, on screen all along: the best");
        // Totals: 48 + 240 + 120 frames at 24 fps.
        assert_eq!((v.totals.clips, v.totals.frames), (3, 408.0));
        assert!((v.totals.seconds - 17.0).abs() < 1e-9, "{}", v.totals.seconds);
        // Cards show the shot and the character's face.
        assert_eq!((b.shot.start, b.shot.end, b.start, b.end), (50, 290, 50, 290));
        assert!(b.shot.face.ends_with(".jpg"));
    }

    #[test]
    fn filters_leave_shots_out_and_say_why() {
        let p = pack();
        let v = preview(&p, &with(|s| s.min_sec = Some(3.0)));
        assert_eq!((starts(&v, true), clip(&v, 0).reason), (vec![50, 290], Some(PackReason::Short)));
        let v = preview(&p, &with(|s| s.max_sec = Some(6.0)));
        assert_eq!((starts(&v, true), clip(&v, 50).reason), (vec![0, 290], Some(PackReason::Long)));
        let v = preview(&p, &with(|s| s.sizes = vec![ShotSize::CloseUp, ShotSize::ExtremeCloseUp]));
        assert_eq!((starts(&v, true), clip(&v, 290).reason), (vec![0, 50], Some(PackReason::Size)));
        let v = preview(&p, &with(|s| s.solo = true));
        assert_eq!((starts(&v, true), clip(&v, 0).reason), (vec![50, 290], Some(PackReason::NotSolo)));
        let v = preview(&p, &with(|s| s.skip_tiny = false));
        assert_eq!(starts(&v, true), [0, 48, 50, 290]);
        let v = preview(&p, &with(|s| s.media = vec![MediaId::new()]));
        assert!(starts(&v, true).is_empty() && clip(&v, 0).reason == Some(PackReason::Episode));
        // A hidden character doesn't count as someone else in the shot.
        p.project.cast_edit(&CastEdit::SetHidden { character_id: p.jesse, hidden: true }).unwrap();
        assert_eq!(starts(&preview(&p, &with(|s| s.solo = true)), true), [0, 50, 290]);
    }

    #[test]
    fn handles_stay_inside_the_episode() {
        let p = pack();
        let v = preview(&p, &with(|s| (s.handles, s.skip_tiny) = (30, false)));
        let ranges: Vec<_> = v.clips.iter().map(|c| (c.start, c.end)).collect();
        assert_eq!(ranges, [(0, 78), (18, 80), (20, 320), (260, 440)]);
        // Handles longer than what is left end at the episode's last frame.
        let v = preview(&p, &with(|s| s.handles = 2100));
        assert_eq!((clip(&v, 0).start, clip(&v, 290).end), (0, FRAMES));
        // The filters look at the shot itself, not its handles.
        let v = preview(&p, &with(|s| (s.handles, s.min_sec) = (30, Some(3.0))));
        assert_eq!(starts(&v, true), [50, 290]);
    }

    #[test]
    fn best_first_and_a_limit_that_keeps_hand_picks() {
        let p = pack();
        let v = preview(&p, &with(|s| s.order = PackOrder::Best));
        assert_eq!(v.clips[0].shot.start, 50, "the best first");
        assert!(v.clips.windows(2).all(|w| w[0].score >= w[1].score));
        // At most one: the best. The flash cut picked by hand stays on top of it.
        p.project.set_pack_overrides(p.walter, &[PackKey { media_id: p.media, shot: 48 }], Some(true)).unwrap();
        let v = preview(&p, &with(|s| s.limit = Some(1)));
        assert_eq!(starts(&v, true), [48, 50]);
        assert_eq!((clip(&v, 0).reason, clip(&v, 290).reason), (Some(PackReason::Limit), Some(PackReason::Limit)));
        assert_eq!((clip(&v, 48).manual, clip(&v, 48).by_rules), (Some(true), false));
        // Dropped by hand: out, and says so; handing it back to the filters undoes it.
        let key = PackKey { media_id: p.media, shot: 50 };
        p.project.set_pack_overrides(p.walter, &[key], Some(false)).unwrap();
        let v = preview(&p, &PackSpec::default());
        assert_eq!((clip(&v, 50).included, clip(&v, 50).reason), (false, Some(PackReason::Removed)));
        p.project.set_pack_overrides(p.walter, &[key], None).unwrap();
        assert!(clip(&preview(&p, &PackSpec::default()), 50).included);
    }

    #[test]
    fn under_a_limit_a_pick_by_hand_knows_what_the_limit_would_say() {
        let p = pack();
        let limit = with(|s| s.limit = Some(1));
        // The shot at 0 is past the limit; picked, it is in, and the
        // filters alone would still leave it out.
        let key = PackKey { media_id: p.media, shot: 0 };
        p.project.set_pack_overrides(p.walter, &[key], Some(true)).unwrap();
        let v = preview(&p, &limit);
        assert_eq!((clip(&v, 0).included, clip(&v, 0).by_rules, clip(&v, 0).reason), (true, false, None));
        // Taken out again: back to the filters, past the limit — no pick left.
        p.project.set_pack_overrides(p.walter, &[key], None).unwrap();
        let v = preview(&p, &limit);
        assert_eq!((clip(&v, 0).manual, clip(&v, 0).reason), (None, Some(PackReason::Limit)));
        // The best shot picked by hand would make the limit anyway.
        p.project.set_pack_overrides(p.walter, &[PackKey { media_id: p.media, shot: 50 }], Some(true)).unwrap();
        assert!(clip(&preview(&p, &limit), 50).by_rules);
    }

    #[test]
    fn a_missing_file_says_so_even_for_a_pick() {
        let p = pack();
        p.project.set_pack_overrides(p.walter, &[PackKey { media_id: p.media, shot: 50 }], Some(false)).unwrap();
        p.project.set_pack_overrides(p.walter, &[PackKey { media_id: p.media, shot: 0 }], Some(true)).unwrap();
        std::fs::remove_file(p.project.media(p.media).unwrap().path).unwrap();
        let v = preview(&p, &PackSpec::default());
        assert!(v.clips.iter().all(|c| !c.included && c.reason == Some(PackReason::Missing)), "{:?}", v.clips);
    }

    #[test]
    fn hand_picks_follow_a_shot_found_again() {
        let p = pack();
        p.project.set_pack_overrides(p.walter, &[PackKey { media_id: p.media, shot: 290 }], Some(false)).unwrap();
        // Shots found again: the last one now starts at 280; a new face pass.
        p.project.db().store_segments(p.media, &shots(280)).unwrap();
        p.project.reset_faces(p.media).unwrap();
        p.project.store_face_chunk(p.media, 0, FRAMES, &tracks::encode(&faces(280)), &[]).unwrap();
        p.project.finish_faces(p.media).unwrap();
        let v = preview(&p, &PackSpec::default());
        assert_eq!(clip(&v, 280).reason, Some(PackReason::Removed), "the pick moved with its shot");
        // And it can be undone where the shot is now.
        p.project.set_pack_overrides(p.walter, &[PackKey { media_id: p.media, shot: 280 }], None).unwrap();
        let v = preview(&p, &PackSpec::default());
        assert_eq!((clip(&v, 280).manual, clip(&v, 280).included), (None, true));
        assert!(p.project.db().pack_overrides(p.walter).unwrap().is_empty());
    }

    #[test]
    fn an_export_takes_exactly_what_the_pack_showed() {
        let p = pack();
        let spec = with(|s| s.handles = 2);
        let naming = PackNaming::default();
        let v = preview(&p, &spec);
        let out = p.dir.path().join("Exports");
        let req = |spec: &PackSpec, stamp: &str| PackExportRequest {
            spec: spec.clone(),
            naming: naming.clone(),
            preset_id: "h264-edit".into(),
            out_dir: None,
            stamp: stamp.into(),
        };
        let (request, dir) = p.project.pack_export_request(p.walter, &req(&spec, &v.stamp), &out).unwrap();
        let shown: Vec<_> = v.clips.iter().filter(|c| c.included).map(|c| (c.shot.media_id, c.start, c.end)).collect();
        let exported: Vec<_> = request.clips.iter().map(|c| (c.media_id, c.start, c.end)).collect();
        assert_eq!(exported, shown);
        assert_eq!(dir, out.join("Walter"), "in the character's own folder");
        assert_eq!(request.label.as_deref(), Some("Walter · scenepack"));
        assert_eq!(
            request.clips.iter().map(|c| c.name.clone().unwrap()).collect::<Vec<_>>(),
            ["Show_Walter_E01_00-00-00-00", "Show_Walter_E01_00-00-02-00", "Show_Walter_E01_00-00-12-00"]
        );
        // The settings are kept for next time.
        assert_eq!(p.project.pack_state(p.walter).unwrap().spec, spec);
        // A pack that changed since it was shown isn't exported.
        let other = PackSpec { solo: true, ..spec.clone() };
        assert!(p.project.pack_export_request(p.walter, &req(&other, &v.stamp), &out).is_err());
        // Nor is a template that can't name files.
        let mut bad = req(&spec, &v.stamp);
        bad.naming.template = "{series} {nme}".into();
        assert!(p.project.pack_export_request(p.walter, &bad, &out).is_err());
    }

    #[test]
    fn settings_of_another_version_load_with_the_defaults_filled_in() {
        let spec: PackSpec = with_defaults(r#"{"solo":true,"pickedColour":"red"}"#);
        assert_eq!(spec, PackSpec { solo: true, ..PackSpec::default() });
        let naming: PackNaming = with_defaults("not json");
        assert_eq!(naming, PackNaming::default());
        // A field of the wrong shape can't be read: all defaults.
        let spec: PackSpec = with_defaults(r#"{"limit":"ten"}"#);
        assert_eq!(spec, PackSpec::default());
    }

    #[test]
    fn a_plan_names_and_sizes_the_pack() {
        let p = pack();
        let naming = PackNaming { template: "{character} {index}".into(), folder: false };
        let plan = p.project.pack_plan(p.walter, &PackSpec::default(), &naming, &as_export::builtin_presets()).unwrap();
        assert_eq!(plan.names, ["Walter_001", "Walter_002", "Walter_003"]);
        assert!(plan.sizes.iter().all(|s| s.bytes > 0.0));
    }
}
