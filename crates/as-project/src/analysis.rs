//! Shot analysis bookkeeping. The engine measures signals; the project keeps
//! them as checkpoints (one row per chunk), knows where a run resumes, and
//! turns finished signals into segments — re-runnable without decoding.

use std::path::PathBuf;

use as_analysis::signals::{self, FrameSignals};
use as_analysis::{CHUNK, FusionParams, SegmentKind as Kind, TransitionKind, fuse};
use as_api::{ContentKind, MediaInfo, Segment, SegmentKind, StageState};
use as_core::{FrameIdx, MediaId};

use crate::{Project, ProjectError, ProjectResult};

/// What the engine needs to measure a media's shots.
#[derive(Clone, Debug)]
pub struct ShotsPlan {
    pub path: PathBuf,
    pub info: MediaInfo,
    /// `FrameIndex::to_bytes`.
    pub index: Vec<u8>,
    pub frames: u32,
    /// First frame without a checkpoint (`frames` = everything is measured).
    pub resume: FrameIdx,
    /// Frames per checkpoint.
    pub chunk: u32,
}

impl Project {
    /// Live action or anime (fusion and, from M3, the detectors depend on it).
    pub fn content(&self) -> ProjectResult<ContentKind> {
        let v = self.db().meta("content")?.unwrap_or_default();
        Ok(serde_json::from_value(serde_json::Value::String(v)).unwrap_or(ContentKind::Live))
    }

    /// Where the shot pass of `id` resumes. Checkpoints that don't line up
    /// (unreadable, from an older format, a gap) are dropped with everything
    /// after them.
    pub fn shots_plan(&self, id: MediaId) -> ProjectResult<ShotsPlan> {
        let (m, info, index) = self.ready_source(id)?;
        let frames = index.len();
        let mut resume = 0;
        for (start, data) in self.db().shot_chunks(id)? {
            let len = signals::decode(&data).map(|s| s.len() as u32);
            let fits = start == resume
                && len.is_some_and(|l| l > 0 && (l == CHUNK || start + l == frames) && start + l <= frames);
            if !fits {
                tracing::info!(file = %m.file_name, start, "dropping shot checkpoints that don't line up");
                self.db().drop_shot_chunks_from(id, start)?;
                break;
            }
            resume = start + len.unwrap_or(0);
        }
        Ok(ShotsPlan { path: PathBuf::from(&m.path), info, index: index.to_bytes(), frames, resume, chunk: CHUNK })
    }

    pub fn store_shot_chunk(&self, id: MediaId, start: FrameIdx, data: &[u8]) -> ProjectResult<()> {
        Ok(self.db().put_shot_chunk(id, start, data)?)
    }

    pub fn set_shots_state(&self, id: MediaId, state: StageState, error: Option<&str>) -> ProjectResult<()> {
        Ok(self.db().set_shots_state(id, state, error)?)
    }

    /// Fuse the checkpointed signals of `id` into segments and store them.
    /// Returns the number of shots.
    pub fn finish_shots(&self, id: MediaId) -> ProjectResult<usize> {
        let frames = self.frame_index(id)?.len() as usize;
        let mut all: Vec<FrameSignals> = Vec::with_capacity(frames);
        for (_, data) in self.db().shot_chunks(id)? {
            all.extend(signals::decode(&data).ok_or_else(|| ProjectError::Invalid("unreadable shot signals".into()))?);
        }
        if all.len() != frames {
            return Err(ProjectError::Invalid(format!("shot signals cover {} of {frames} frames", all.len())));
        }
        let params = match self.content()? {
            ContentKind::Anime => FusionParams::anime(),
            ContentKind::Live | ContentKind::Mixed => FusionParams::live(),
        };
        let segments: Vec<Segment> = fuse(&all, &params)
            .into_iter()
            .map(|s| Segment {
                start: s.start,
                end: s.end,
                kind: match s.kind {
                    Kind::Shot => SegmentKind::Shot,
                    Kind::Transition(TransitionKind::Dissolve) => SegmentKind::Dissolve,
                    Kind::Transition(TransitionKind::FadeOut) => SegmentKind::FadeOut,
                    Kind::Transition(TransitionKind::FadeIn) => SegmentKind::FadeIn,
                    Kind::Black => SegmentKind::Black,
                    Kind::Flash => SegmentKind::Flash,
                },
                tiny: s.tiny,
            })
            .collect();
        self.db().store_segments(id, &segments)?;
        Ok(segments.iter().filter(|s| s.kind == SegmentKind::Shot).count())
    }

    /// Segments of `id` (empty until its shots are detected).
    pub fn segments(&self, id: MediaId) -> ProjectResult<Vec<Segment>> {
        Ok(self.db().segments(id)?)
    }

    /// Forget the shot analysis of `id` so it runs again from scratch.
    pub fn reset_shots(&self, id: MediaId) -> ProjectResult<()> {
        Ok(self.db().reset_shots(id)?)
    }

    /// Indexed media still waiting for (or interrupted in) shot detection.
    pub fn pending_shots(&self) -> ProjectResult<Vec<MediaId>> {
        Ok(self.db().pending_shots()?)
    }
}

#[cfg(test)]
mod tests {
    use as_analysis::signals::encode;
    use as_api::{AppSettings, HdrKind, NewProject, ProjectKind, ProxyState, VideoStream};
    use as_core::{AppPaths, FrameIndex, Rational};

    use super::*;

    fn info() -> MediaInfo {
        MediaInfo {
            container: "matroska,webm".into(),
            duration_sec: Some(10.0),
            start_sec: 0.0,
            size_bytes: Some(1.0),
            bit_rate: None,
            video: VideoStream {
                index: 0,
                codec: "h264".into(),
                profile: None,
                pix_fmt: "yuv420p".into(),
                bit_depth: 8,
                width: 1920,
                height: 1080,
                sar: Rational::new(1, 1),
                r_frame_rate: Rational::new(24, 1),
                avg_frame_rate: None,
                time_base: Rational::new(1, 1000),
                color_primaries: None,
                color_transfer: None,
                color_space: None,
                color_range: None,
                field_order: None,
                rotation: 0,
                hdr: HdrKind::None,
            },
            audio: vec![],
            subtitles: vec![],
        }
    }

    /// A project with one media "indexed" as `frames` frames at 24 fps.
    fn project_with_media(frames: u32) -> (tempfile::TempDir, std::sync::Arc<Project>, MediaId) {
        let dir = tempfile::tempdir().unwrap();
        let lib = crate::Library::open(&AppPaths::at(dir.path().join("data"))).unwrap();
        lib.update_settings(&AppSettings {
            library_root: dir.path().join("Library").display().to_string(),
            export_preset_id: "h264-edit".into(),
            audio_language: String::new(),
        })
        .unwrap();
        let file = dir.path().join("Show - 01.mkv");
        std::fs::write(&file, b"x").unwrap();
        let (_, project, ids) = lib
            .create_project(&NewProject {
                title: "Show".into(),
                kind: ProjectKind::Series,
                content: ContentKind::Live,
                files: vec![file.display().to_string()],
            })
            .unwrap();
        let index = FrameIndex::cfr(Rational::new(1, 24), Rational::new(24, 1), 0, frames);
        let rec = as_db::IngestRecord {
            info: info(),
            frames,
            duration_sec: frames as f64 / 24.0,
            fps: Rational::new(24, 1),
            vfr: false,
            interlaced: false,
            proxy: ProxyState::NotNeeded,
            poster: None,
            index_blob: index.to_bytes(),
            audio_ordinal: None,
        };
        project.db().record_ingest(ids[0], &rec).unwrap();
        (dir, project, ids[0])
    }

    #[test]
    fn resume_point_follows_the_checkpoints() {
        let frames = CHUNK * 2 + 100;
        let (_dir, project, id) = project_with_media(frames);
        let chunk = |n: u32| encode(&vec![FrameSignals::default(); n as usize]);
        assert_eq!(project.pending_shots().unwrap(), vec![id]);
        assert_eq!(project.shots_plan(id).unwrap().resume, 0);
        project.store_shot_chunk(id, 0, &chunk(CHUNK)).unwrap();
        assert_eq!(project.shots_plan(id).unwrap().resume, CHUNK);
        // A chunk after a gap (and everything after it) is dropped.
        project.store_shot_chunk(id, CHUNK * 2, &chunk(100)).unwrap();
        assert_eq!(project.shots_plan(id).unwrap().resume, CHUNK);
        assert_eq!(project.db().shot_chunks(id).unwrap().len(), 1);
        // Unreadable data too.
        project.store_shot_chunk(id, CHUNK, b"garbage").unwrap();
        assert_eq!(project.shots_plan(id).unwrap().resume, CHUNK);
        project.store_shot_chunk(id, CHUNK, &chunk(CHUNK)).unwrap();
        project.store_shot_chunk(id, CHUNK * 2, &chunk(100)).unwrap();
        assert_eq!(project.shots_plan(id).unwrap().resume, frames);

        // All-black signals: one black segment, no shots.
        assert_eq!(project.finish_shots(id).unwrap(), 0);
        assert_eq!(
            project.segments(id).unwrap(),
            [Segment { start: 0, end: frames, kind: SegmentKind::Black, tiny: false }]
        );
        let m = project.media(id).unwrap();
        assert_eq!((m.shots, m.shot_count), (StageState::Ready, Some(0)));
        assert!(project.pending_shots().unwrap().is_empty());

        project.reset_shots(id).unwrap();
        assert_eq!(project.shots_plan(id).unwrap().resume, 0);
        assert_eq!(project.pending_shots().unwrap(), vec![id]);
    }
}
