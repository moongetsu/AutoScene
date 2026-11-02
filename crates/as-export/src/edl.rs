//! Edit lists: where each clip of an export sits in its episode, for other
//! apps. A CMX 3600 EDL lets an editor (DaVinci Resolve, Premiere Pro, Avid)
//! build the cut again from the episodes themselves, with their handles; a
//! cut list (CSV) gives the same timecodes to a spreadsheet.
//!
//! The clips follow each other on a timeline from 01:00:00:00. Every
//! timecode of an EDL counts at its one rate, drop frame at 29.97 and 59.94
//! (`;` before the frames), as the header's `FCM` says. An episode's own
//! start timecode is taken as the frame it labels, so one that counts
//! otherwise (non-drop at 29.97) still gives the right frames, and a NOTE
//! under its clip says so. A cut list gives the source timecodes as the
//! episode counts them, as an editor shows the clip. Out points are
//! exclusive, as CMX counts them: the frame after the clip.
//!
//! Timecode starts over at midnight, and an event can't run across it (its
//! out point would come before its in point): an EDL refuses a clip that
//! runs to its episode's midnight, and clips that run to the timeline's.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use as_core::{MediaId, Rational, Timecode};

/// One clip of a list.
#[derive(Clone, Debug, PartialEq)]
pub struct EdlEvent {
    /// Its episode's tape name (see [`reel_names`]).
    pub reel: String,
    /// Its episode's file name.
    pub clip_name: String,
    /// Its episode's full path.
    pub source_path: String,
    /// Its episode's frame rate. The clips of a list share one: the list
    /// counts at it.
    pub fps: Rational,
    /// The episode's start timecode, as its file writes it; `None` when it
    /// carries none: it's counted from 00:00:00:00, and the list says so.
    pub src_start: Option<Timecode>,
    /// The clip's first frame in its episode.
    pub start: u64,
    /// The frame after its last (exclusive).
    pub end: u64,
    /// The clip has sound.
    pub audio: bool,
}

impl EdlEvent {
    /// The frame the episode's start timecode labels, counted from
    /// 00:00:00:00 (0 without one).
    fn origin(&self) -> u64 {
        self.src_start.map_or(0, |tc| tc.to_frame(self.fps))
    }
}

/// Why an EDL can't be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdlError {
    /// The clip at this index (from 0) runs to midnight in its episode's
    /// timecode, or past it.
    PastMidnight(usize),
    /// The clips run to the timeline's midnight: 23 hours from 01:00:00:00.
    TooLong,
}

/// `events` as a CMX 3600 EDL named `title` (in printable ASCII, at most 70
/// characters): a numbered line per clip with its reel, its source in and
/// out and its record in and out, then its file name (`FROM CLIP NAME`,
/// which Premiere Pro matches on) and path (`SOURCE FILE`, which Resolve
/// and Avid read). Lines end in CRLF. Past 999 clips the numbers take four
/// digits, which Resolve and Premiere Pro read and strict CMX readers may
/// not. A clip that runs to its episode's midnight is refused, and so are
/// clips that run to the timeline's.
pub fn write_cmx3600(title: &str, events: &[EdlEvent]) -> Result<String, EdlError> {
    if let Some(i) = events.iter().position(|e| e.origin() + e.end >= Clock::new(e.fps).day) {
        return Err(EdlError::PastMidnight(i));
    }
    if let Some(first) = events.first() {
        let record = Clock::new(first.fps);
        if record.hour() + events.iter().map(|e| e.end.saturating_sub(e.start)).sum::<u64>() >= record.day {
            return Err(EdlError::TooLong);
        }
    }
    let title: String = title
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n'))
        .map(|c| if c == ' ' || c.is_ascii_graphic() { c } else { '_' })
        .take(70)
        .collect();
    let drop_frame = events.first().is_some_and(|e| Clock::new(e.fps).drop_frame);
    let fcm = if drop_frame { "DROP FRAME" } else { "NON-DROP FRAME" };
    let mut lines = vec![format!("TITLE: {}", title.trim()), format!("FCM: {fcm}"), String::new()];
    let one_line = |s: &str| s.replace(['\r', '\n'], "");
    for r in rows(events, false) {
        let channels = if r.event.audio { "AA/V" } else { "V" };
        let (n, reel) = (r.n, &r.event.reel);
        lines.push(format!(
            "{n:03}  {reel:<8} {channels:<5} C        {} {} {} {}",
            r.src_in, r.src_out, r.rec_in, r.rec_out
        ));
        lines.push(format!("* FROM CLIP NAME: {}", one_line(&r.event.clip_name)));
        lines.push(format!("* SOURCE FILE: {}", one_line(&r.event.source_path)));
        match r.event.src_start {
            None => lines.push("* NOTE: SOURCE TIMECODE UNKNOWN, COUNTED FROM 00:00:00:00".into()),
            Some(tc) if tc.drop_frame != r.src_in.drop_frame => {
                lines.push("* NOTE: SOURCE TIMECODE IS NON-DROP FRAME, GIVEN HERE IN DROP FRAME".into())
            }
            Some(_) => {}
        }
        lines.push(String::new());
    }
    Ok(crlf(lines))
}

/// `events` as a cut list: a CSV (RFC 4180, CRLF, UTF-8 without a BOM) with
/// a row per clip and the timecodes the EDL gives it, but for the source's:
/// those count as the episode's start timecode does (non-drop at 29.97 when
/// it's written so), as an editor shows the clip, and start over at its
/// midnight. Drop Frame tells how the rest count. A clip whose episode has
/// no start timecode says so in Note. It's for reading and sorting: an
/// editor builds the cut from the EDL.
pub fn write_csv(events: &[EdlEvent]) -> String {
    let mut lines = vec![
        "#,Reel,Clip Name,Source File,Source In,Source Out,Record In,Record Out,Duration,Frame Rate,Drop Frame,Note"
            .to_owned(),
    ];
    for r in rows(events, true) {
        let row = [
            r.n.to_string(),
            not_a_formula(&r.event.reel),
            not_a_formula(&r.event.clip_name),
            not_a_formula(&r.event.source_path),
            r.src_in.to_string(),
            r.src_out.to_string(),
            r.rec_in.to_string(),
            r.rec_out.to_string(),
            r.duration.to_string(),
            rate_label(r.event.fps),
            (if r.rec_in.drop_frame { "Yes" } else { "No" }).to_owned(),
            (if r.event.src_start.is_none() { "Source timecode unknown, counted from 00:00:00:00" } else { "" })
                .to_owned(),
        ];
        lines.push(row.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
    }
    crlf(lines)
}

/// A tape name (reel) per episode, from its name (`S01E03`, or its file's
/// stem): upper case, only `A-Z`, `0-9` and `_` (anything else becomes
/// `_`), at most 8 characters, `AX` when nothing is left. An episode keeps
/// its first; one taken by another episode ends in `_2`, `_3`… within the
/// 8 (`EPISODE1`, then `EPISOD_2`).
pub fn reel_names(names: &[(MediaId, String)]) -> HashMap<MediaId, String> {
    let mut reels = HashMap::new();
    let mut taken = HashSet::new();
    for (id, name) in names {
        if reels.contains_key(id) {
            continue;
        }
        let base: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c.to_ascii_uppercase() } else { '_' })
            .take(8)
            .collect();
        let base = if base.is_empty() { "AX".to_owned() } else { base };
        let reel = if taken.contains(&base) {
            (2u32..)
                .map(|n| {
                    let tail = format!("_{n}");
                    base.chars().take(8usize.saturating_sub(tail.len())).collect::<String>() + &tail
                })
                .find(|r| !taken.contains(r))
                .expect("unbounded")
        } else {
            base
        };
        taken.insert(reel.clone());
        reels.insert(*id, reel);
    }
    reels
}

/// Timecodes at one rate: drop frame where the rate drops labels, starting
/// over after a day.
#[derive(Clone, Copy)]
struct Clock {
    fps: Rational,
    drop_frame: bool,
    day: u64,
}

impl Clock {
    /// How a list counts at `fps`: drop frame where the rate drops labels.
    fn new(fps: Rational) -> Self {
        Self::counting(fps, true)
    }

    /// Counting with drop frame (where the rate drops labels) or without.
    fn counting(fps: Rational, drop_frame: bool) -> Self {
        // `from_frame` drops labels only at the rates that do (29.97, 59.94).
        let drop_frame = Timecode::from_frame(0, fps, drop_frame).drop_frame;
        Self { fps, drop_frame, day: Timecode::day_frames(fps, drop_frame) }
    }

    fn at(self, f: u64) -> Timecode {
        Timecode::from_frame(f % self.day, self.fps, self.drop_frame)
    }

    /// The frame 01:00:00:00 labels, where the timeline starts.
    fn hour(self) -> u64 {
        Timecode { hours: 1, minutes: 0, seconds: 0, frames: 0, drop_frame: self.drop_frame }.to_frame(self.fps)
    }
}

/// A clip of a list with its timecodes.
struct Row<'a> {
    n: usize,
    event: &'a EdlEvent,
    src_in: Timecode,
    src_out: Timecode,
    rec_in: Timecode,
    rec_out: Timecode,
    duration: Timecode,
}

/// `events` one after another on the timeline, counted at the first's rate;
/// with `own_count`, each source as its start timecode counts.
fn rows(events: &[EdlEvent], own_count: bool) -> Vec<Row<'_>> {
    let Some(first) = events.first() else { return Vec::new() };
    let record = Clock::new(first.fps);
    let mut at = record.hour();
    events
        .iter()
        .enumerate()
        .map(|(i, event)| {
            let source = match event.src_start {
                Some(tc) if own_count => Clock::counting(event.fps, tc.drop_frame),
                _ => Clock::new(event.fps),
            };
            let from = event.origin();
            let len = event.end.saturating_sub(event.start);
            let row = Row {
                n: i + 1,
                event,
                src_in: source.at(from + event.start),
                src_out: source.at(from + event.end),
                rec_in: record.at(at),
                rec_out: record.at(at + len),
                duration: record.at(len),
            };
            at += len;
            row
        })
        .collect()
}

/// Each line ended with CRLF.
fn crlf(lines: Vec<String>) -> String {
    lines.into_iter().map(|l| l + "\r\n").collect()
}

/// A frame rate as editors write it: `23.976`, `29.97`, `25`.
fn rate_label(fps: Rational) -> String {
    format!("{:.3}", fps.to_f64()).trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A text cell a spreadsheet won't run: one that starts like a formula
/// (`=`, `+`, `-`, `@`, a tab or a CR) gets a `'` first.
fn not_a_formula(s: &str) -> String {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_owned() }
}

/// A CSV field: quoted, with its quotes doubled, when it holds a comma, a
/// quote or a line break, or starts or ends with a space.
fn csv_field(s: &str) -> Cow<'_, str> {
    if s.contains([',', '"', '\r', '\n']) || s.starts_with(' ') || s.ends_with(' ') {
        Cow::Owned(format!("\"{}\"", s.replace('"', "\"\"")))
    } else {
        Cow::Borrowed(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NTSC: Rational = Rational { num: 30000, den: 1001 };
    const PAL: Rational = Rational { num: 25, den: 1 };

    fn event(reel: &str, fps: Rational, src_start: Option<&str>, start: u64, end: u64) -> EdlEvent {
        EdlEvent {
            reel: reel.into(),
            clip_name: format!("{reel}.mkv"),
            source_path: format!(r"D:\Video\{reel}.mkv"),
            fps,
            src_start: src_start.map(|tc| Timecode::parse(tc, fps).unwrap()),
            start,
            end,
            audio: true,
        }
    }

    /// The event lines of an EDL (each clip's first line).
    fn event_lines(edl: &str) -> Vec<&str> {
        edl.split("\r\n").filter(|l| l.starts_with(|c: char| c.is_ascii_digit())).collect()
    }

    #[test]
    fn drop_frame_record_starts_at_one_hour() {
        let zero = Some("00:00:00;00");
        let events = [event("S01E01", NTSC, zero, 1800, 1830), event("S01E02", NTSC, zero, 0, 17982)];
        let edl = write_cmx3600("Walter · scenepack\r\n", &events).unwrap();
        assert_eq!(
            edl,
            "TITLE: Walter _ scenepack\r\n\
             FCM: DROP FRAME\r\n\
             \r\n\
             001  S01E01   AA/V  C        00:01:00;02 00:01:01;02 01:00:00;00 01:00:01;00\r\n\
             * FROM CLIP NAME: S01E01.mkv\r\n\
             * SOURCE FILE: D:\\Video\\S01E01.mkv\r\n\
             \r\n\
             002  S01E02   AA/V  C        00:00:00;00 00:10:00;00 01:00:01;00 01:10:01;00\r\n\
             * FROM CLIP NAME: S01E02.mkv\r\n\
             * SOURCE FILE: D:\\Video\\S01E02.mkv\r\n\
             \r\n"
        );
        // 01:00:00;00 at 29.97 drop frame is frame 107892, not 108000.
        assert_eq!(Clock::new(NTSC).hour(), 107_892);
        assert_eq!(Clock::new(Rational::new(24000, 1001)).hour(), 86_400);
        // Without drop frame, a whole-number rate counts every frame.
        let edl = write_cmx3600("Cut", &[event("A", PAL, Some("00:00:00:00"), 0, 25)]).unwrap();
        assert!(edl.contains("FCM: NON-DROP FRAME\r\n"), "{edl}");
        assert_eq!(event_lines(&edl), ["001  A        AA/V  C        00:00:00:00 00:00:01:00 01:00:00:00 01:00:01:00"]);
    }

    #[test]
    fn source_in_adds_the_start_timecode() {
        let edl = write_cmx3600("Cut", &[event("A", NTSC, Some("01:00:00;00"), 1800, 1801)]).unwrap();
        assert_eq!(event_lines(&edl), ["001  A        AA/V  C        01:01:00;02 01:01:00;03 01:00:00;00 01:00:00;01"]);
        assert!(!edl.contains("NOTE"), "{edl}");
        let edl = write_cmx3600("Cut", &[event("A", PAL, Some("10:00:00:00"), 50, 75), event("B", PAL, None, 50, 75)])
            .unwrap();
        let lines = event_lines(&edl);
        assert_eq!(lines[0], "001  A        AA/V  C        10:00:02:00 10:00:03:00 01:00:00:00 01:00:01:00");
        // An episode without a start timecode counts from zero, and the EDL says so under its clip.
        assert_eq!(lines[1], "002  B        AA/V  C        00:00:02:00 00:00:03:00 01:00:01:00 01:00:02:00");
        assert!(
            edl.ends_with(
                "* SOURCE FILE: D:\\Video\\B.mkv\r\n* NOTE: SOURCE TIMECODE UNKNOWN, COUNTED FROM 00:00:00:00\r\n\r\n"
            ),
            "{edl}"
        );
        // A start labelled without drop frame at 29.97 is the same frame, written as the EDL counts, and
        // the EDL says so.
        let ndf = [event("A", NTSC, Some("01:00:00:00"), 0, 1)];
        let edl = write_cmx3600("Cut", &ndf).unwrap();
        assert_eq!(
            event_lines(&edl)[0],
            "001  A        AA/V  C        01:00:03;18 01:00:03;19 01:00:00;00 01:00:00;01"
        );
        assert!(
            edl.ends_with(
                "* SOURCE FILE: D:\\Video\\A.mkv\r\n* NOTE: SOURCE TIMECODE IS NON-DROP FRAME, GIVEN HERE IN DROP FRAME\r\n\r\n"
            ),
            "{edl}"
        );
        // A cut list gives it as the episode counts, as an editor shows the clip.
        let csv = write_csv(&ndf);
        assert!(csv.ends_with(",01:00:00:00,01:00:00:01,01:00:00;00,01:00:00;01,00:00:00;01,29.97,Yes,\r\n"), "{csv}");
    }

    #[test]
    fn out_points_are_exclusive() {
        let zero = Some("00:00:00:00");
        let events = [event("A", PAL, zero, 0, 1), event("A", PAL, zero, 100, 125), event("B", PAL, None, 50, 75)];
        let csv = write_csv(&events);
        let rows: Vec<&str> = csv.split("\r\n").skip(1).filter(|r| !r.is_empty()).collect();
        // One frame runs from its frame to the next; the next clip starts where the last ends. A clip whose
        // episode has no start timecode says so, as the EDL's NOTE does.
        assert_eq!(
            rows,
            [
                r"1,A,A.mkv,D:\Video\A.mkv,00:00:00:00,00:00:00:01,01:00:00:00,01:00:00:01,00:00:00:01,25,No,",
                r"2,A,A.mkv,D:\Video\A.mkv,00:00:04:00,00:00:05:00,01:00:00:01,01:00:01:01,00:00:01:00,25,No,",
                r#"3,B,B.mkv,D:\Video\B.mkv,00:00:02:00,00:00:03:00,01:00:01:01,01:00:02:01,00:00:01:00,25,No,"Source timecode unknown, counted from 00:00:00:00""#,
            ]
        );
        assert!(csv.starts_with(
            "#,Reel,Clip Name,Source File,Source In,Source Out,Record In,Record Out,Duration,Frame Rate,Drop Frame,Note\r\n"
        ));
        assert_eq!(rate_label(NTSC), "29.97");
        assert_eq!(rate_label(Rational::new(24000, 1001)), "23.976");
        assert_eq!(rate_label(Rational::new(60000, 1001)), "59.94");
        assert_eq!(rate_label(Rational::new(30, 1)), "30");
    }

    #[test]
    fn fifty_nine_ninety_four_drops_four() {
        let fps = Rational::new(60000, 1001);
        let zero = Some("00:00:00;00");
        let events = [event("A", fps, zero, 0, 3600), event("A", fps, zero, 35964, 35965)];
        let edl = write_cmx3600("Cut", &events).unwrap();
        assert!(edl.contains("FCM: DROP FRAME\r\n"), "{edl}");
        // Minute 1 starts at ;04, minute 10 drops none.
        assert_eq!(
            event_lines(&edl),
            [
                "001  A        AA/V  C        00:00:00;00 00:01:00;04 01:00:00;00 01:01:00;04",
                "002  A        AA/V  C        00:10:00;00 00:10:00;01 01:01:00;04 01:01:00;05",
            ]
        );
        let csv = write_csv(&events[..1]);
        assert!(csv.ends_with(",00:01:00;04,59.94,Yes,\r\n"), "{csv}");
    }

    #[test]
    fn edl_events_never_run_past_midnight() {
        let edl = |events: &[EdlEvent]| write_cmx3600("Cut", events);
        // A clip may end on the day's last frame; a frame more and its out point is midnight, 00:00:00:00.
        let last = |end| [event("A", PAL, Some("23:59:59:00"), 0, end)];
        assert_eq!(
            event_lines(&edl(&last(24)).unwrap()),
            ["001  A        AA/V  C        23:59:59:00 23:59:59:24 01:00:00:00 01:00:00:24"]
        );
        assert_eq!(edl(&last(25)), Err(EdlError::PastMidnight(0)));
        // Across it at 29.97 drop frame, and wholly past it: the index says which clip.
        let across = [event("A", NTSC, Some("00:00:00;00"), 0, 30), event("B", NTSC, Some("23:59:59;00"), 0, 60)];
        assert_eq!(edl(&across), Err(EdlError::PastMidnight(1)));
        assert_eq!(edl(&[event("A", PAL, Some("23:59:59:00"), 50, 75)]), Err(EdlError::PastMidnight(0)));
        // A cut list gives them as they are, starting over.
        let csv = write_csv(&[event("A", PAL, Some("23:59:59:00"), 0, 60)]);
        assert!(csv.contains(",23:59:59:00,00:00:01:10,01:00:00:00,01:00:02:10,"), "{csv}");
        assert!(write_csv(&across).contains(",23:59:59;00,00:00:01;00,"));
        // The timeline holds the 23 hours from 01:00:00:00 to its last frame before midnight.
        let hours_23 = 25 * 3600 * 23;
        let full = edl(&[event("A", PAL, None, 0, hours_23 - 1)]).unwrap();
        assert!(event_lines(&full)[0].ends_with(" 01:00:00:00 23:59:59:24"), "{full}");
        assert_eq!(
            edl(&[event("A", PAL, None, 0, hours_23 - 25), event("A", PAL, None, 0, 25)]),
            Err(EdlError::TooLong)
        );
    }

    #[test]
    fn reels_are_eight_characters_and_unique() {
        let ids: Vec<MediaId> = (0..7).map(|_| MediaId::new()).collect();
        let names = [
            (ids[0], "Episode1 (Director's Cut)".to_owned()),
            (ids[1], "episode1".to_owned()),
            (ids[2], "EPISODE1".to_owned()),
            (ids[0], "Episode1 again".to_owned()),
            (ids[3], "S01E03".to_owned()),
            (ids[4], String::new()),
            (ids[5], "Café".to_owned()),
            (ids[6], "ÄÖÜ".to_owned()),
        ];
        let reels = reel_names(&names);
        assert_eq!(reels.len(), 7);
        let reel = |i: usize| reels[&ids[i]].as_str();
        // Cut to 8; the same name of another episode gets a number within the 8.
        assert_eq!([reel(0), reel(1), reel(2)], ["EPISODE1", "EPISOD_2", "EPISOD_3"]);
        assert_eq!(reel(3), "S01E03");
        assert_eq!(reel(4), "AX");
        assert_eq!(reel(5), "CAF_");
        assert_eq!(reel(6), "___");
        // From the tenth on, the number takes a character more.
        let many: Vec<(MediaId, String)> = (0..12).map(|_| (MediaId::new(), "Episode1".to_owned())).collect();
        let reels = reel_names(&many);
        assert_eq!([&reels[&many[8].0], &reels[&many[9].0]], ["EPISOD_9", "EPISO_10"]);
        let unique: HashSet<&String> = reels.values().collect();
        assert_eq!(unique.len(), 12);
        assert!(reels.values().all(|r| r.len() <= 8));
    }

    #[test]
    fn csv_fields_are_quoted_and_quotes_doubled() {
        assert_eq!(csv_field(r#"a,"b""#), r#""a,""b""""#);
        assert_eq!(csv_field("two\r\nlines"), "\"two\r\nlines\"");
        assert_eq!(csv_field(" padded"), "\" padded\"");
        assert_eq!(csv_field("plain text"), "plain text");
        let mut e = event("A", PAL, Some("00:00:00:00"), 0, 25);
        e.clip_name = r#"Show, "Pilot".mkv"#.into();
        let csv = write_csv(&[e]);
        assert!(csv.contains(r#",A,"Show, ""Pilot"".mkv",D:\Video\A.mkv,"#), "{csv}");
    }

    #[test]
    fn csv_cells_never_start_a_formula() {
        let mut e = event("A", PAL, Some("00:00:00:00"), 0, 25);
        e.clip_name = "=HYPERLINK(\"x\")".into();
        e.source_path = "@SUM(1)".into();
        e.reel = "-1".into();
        let csv = write_csv(&[e]);
        let row = csv.split("\r\n").nth(1).unwrap();
        assert!(row.starts_with(r#"1,'-1,"'=HYPERLINK(""x"")",'@SUM(1),"#), "{row}");
        for text in ["+1", "\tx", "\rx"] {
            assert!(not_a_formula(text).starts_with('\''), "{text:?}");
        }
        assert_eq!(not_a_formula("S01E03"), "S01E03");
    }

    #[test]
    fn events_past_999_keep_counting() {
        let events: Vec<EdlEvent> = (0..1001).map(|i| event("A", PAL, Some("00:00:00:00"), i, i + 1)).collect();
        let edl = write_cmx3600("Cut", &events).unwrap();
        let lines = event_lines(&edl);
        assert_eq!(lines.len(), 1001);
        assert!(lines[998].starts_with("999  A "), "{}", lines[998]);
        assert!(lines[999].starts_with("1000  A "), "{}", lines[999]);
        assert!(lines[1000].starts_with("1001  A "), "{}", lines[1000]);
        assert!(write_csv(&events).contains("\r\n1001,A,"));
    }
}
