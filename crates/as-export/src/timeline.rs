//! NLE project and timeline cut list exporters.
//!
//! Generates industry standard interchange files (CMX 3600 EDL and DaVinci CSV)
//! directly from frame-accurate cut lists without re-encoding video.

use as_core::{FrameRange, Rational, Timecode};

/// A clip entry in a project timeline cut list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineClip {
    pub index: usize,
    pub name: String,
    pub source_file: String,
    pub source_range: FrameRange,
    pub record_range: FrameRange,
    pub time_base: Rational,
}

/// Generates a CMX 3600 Edit Decision List (EDL) string from timeline clips.
///
/// Format:
/// ```text
/// TITLE: <title>
/// FCM: NON-DROP FRAME
///
/// 001  AX       V     C        00:00:00:00 00:00:02:12 00:00:00:00 00:00:02:12
/// * FROM CLIP NAME: clip_name.mp4
/// ```
pub fn export_cmx3600_edl(title: &str, clips: &[TimelineClip], time_base: Rational) -> String {
    let mut out = String::new();
    out.push_str(&format!("TITLE: {}\n", title.trim()));
    out.push_str("FCM: NON-DROP FRAME\n\n");

    for (idx, clip) in clips.iter().enumerate() {
        let event_num = idx + 1;
        let reel = "AX      ";
        let track = "V     C        ";

        let src_in = Timecode::from_frame(clip.source_range.start as u64, time_base, false);
        let src_out = Timecode::from_frame(clip.source_range.end as u64, time_base, false);
        let rec_in = Timecode::from_frame(clip.record_range.start as u64, time_base, false);
        let rec_out = Timecode::from_frame(clip.record_range.end as u64, time_base, false);

        out.push_str(&format!(
            "{event_num:03}  {reel} {track}{src_in} {src_out} {rec_in} {rec_out}\n"
        ));
        out.push_str(&format!("* FROM CLIP NAME: {}\n\n", clip.source_file));
    }

    out
}

/// Generates a DaVinci Resolve Cut Sheet CSV string from timeline clips.
pub fn export_davinci_csv(clips: &[TimelineClip], time_base: Rational) -> String {
    let mut out = String::new();
    out.push_str("Index,Clip Name,Source File,Source In,Source Out,Record In,Record Out,Duration (Frames)\n");

    for clip in clips {
        let src_in = Timecode::from_frame(clip.source_range.start as u64, time_base, false);
        let src_out = Timecode::from_frame(clip.source_range.end as u64, time_base, false);
        let rec_in = Timecode::from_frame(clip.record_range.start as u64, time_base, false);
        let rec_out = Timecode::from_frame(clip.record_range.end as u64, time_base, false);
        let duration = clip.source_range.len();

        out.push_str(&format!(
            "{},\"{}\",\"{}\",{},{},{},{},{}\n",
            clip.index, clip.name, clip.source_file, src_in, src_out, rec_in, rec_out, duration
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_cmx3600_edl_accurately() {
        let fps = Rational::new(24, 1);
        let clips = vec![
            TimelineClip {
                index: 1,
                name: "Shot 1".into(),
                source_file: "episode_01.mkv".into(),
                source_range: FrameRange::new(0, 48),
                record_range: FrameRange::new(0, 48),
                time_base: fps,
            },
            TimelineClip {
                index: 2,
                name: "Shot 2".into(),
                source_file: "episode_01.mkv".into(),
                source_range: FrameRange::new(100, 148),
                record_range: FrameRange::new(48, 96),
                time_base: fps,
            },
        ];

        let edl = export_cmx3600_edl("My Project", &clips, fps);
        assert!(edl.contains("TITLE: My Project"));
        assert!(edl.contains("001  AX       V     C        00:00:00:00 00:00:02:00 00:00:00:00 00:00:02:00"));
        assert!(edl.contains("* FROM CLIP NAME: episode_01.mkv"));
        assert!(edl.contains("002  AX       V     C        00:00:04:04 00:00:06:04 00:00:02:00 00:00:04:00"));
    }

    #[test]
    fn generates_davinci_csv_accurately() {
        let fps = Rational::new(24, 1);
        let clips = vec![TimelineClip {
            index: 1,
            name: "Intro".into(),
            source_file: "ep1.mp4".into(),
            source_range: FrameRange::new(0, 24),
            record_range: FrameRange::new(0, 24),
            time_base: fps,
        }];

        let csv = export_davinci_csv(&clips, fps);
        assert!(csv.contains("Index,Clip Name,Source File"));
        assert!(csv.contains("1,\"Intro\",\"ep1.mp4\",00:00:00:00,00:00:01:00,00:00:00:00,00:00:01:00,24"));
    }
}
