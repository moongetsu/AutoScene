//! File names in and out: recognising episode numbers in the names people
//! actually have (scene releases, fansubs, Plex layouts, Japanese titles),
//! natural ordering, and safe names for the files we write.

use std::cmp::Ordering;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

/// Containers worth offering for import.
pub const VIDEO_EXTENSIONS: &[&str] =
    &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "ts", "m2ts", "mts", "mpg", "mpeg", "vob", "flv", "ogv", "3gp"];

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
}

fn re(s: &str) -> Regex {
    Regex::new(s).expect("static regex")
}

// Patterns that carry their own season, tried on the raw file stem.
static SXXEYY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:^|[^a-z])s(\d{1,2})[ ._-]?e(\d{1,4})"));
static NXNN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:^|[^0-9a-z])(\d{1,2})x(\d{2,3})(?:[^0-9a-z]|$)"));
static SEASON_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)season[ ._-]*(\d{1,2}).*?(?:episode|ep)[ ._-]*(\d{1,4})"));
static JAPANESE: LazyLock<Regex> = LazyLock::new(|| re(r"第\s*(\d{1,4})\s*[話话集回]"));

// Episode-only patterns, tried on the stem with tags removed.
static TAGS: LazyLock<Regex> = LazyLock::new(|| re(r"\[[^\]]*\]|\([^)]*\)|\{[^}]*\}|【[^】]*】"));
static EPISODE_WORD: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:^|[^a-z])(?:episode|ep|e)[ ._-]?(\d{1,4})(?:v\d)?(?:[^0-9]|$)"));
static DASH_NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|\s)-\s*(\d{1,4})(?:v\d)?(?:\s|$)"));
static TRAILING_NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[\s_.-])(\d{1,3})(?:v\d)?$"));

static FOLDER_SEASON: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:season|staffel|saison|temporada|stagione|series)[ ._-]*(\d{1,2})|^s(\d{1,2})$"));

/// Season and episode recognised in a file name, using the parent folder
/// (`Season 2`, `S02`, `Staffel 2`…) when the name itself has no season.
pub fn parse_episode(file_name: &str, parent_dir: Option<&str>) -> (Option<u32>, Option<u32>) {
    let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or(file_name);
    let num = |m: Option<regex::Match>| m.and_then(|m| m.as_str().parse::<u32>().ok());

    for r in [&*SXXEYY, &*NXNN, &*SEASON_EPISODE] {
        if let Some(c) = r.captures(stem) {
            return (num(c.get(1)), num(c.get(2)));
        }
    }

    let clean = TAGS.replace_all(stem, " ").replace(['_', '.'], " ");
    let clean = clean.trim();
    let episode = JAPANESE
        .captures(stem)
        .or_else(|| EPISODE_WORD.captures(clean))
        .or_else(|| DASH_NUMBER.captures(clean))
        .or_else(|| TRAILING_NUMBER.captures(clean))
        .and_then(|c| num(c.get(1)));
    let season =
        episode.and(parent_dir.and_then(|d| FOLDER_SEASON.captures(d.trim()).and_then(|c| num(c.get(1).or(c.get(2))))));
    (season, episode)
}

/// "file10" after "file9": compares digit runs as numbers, the rest case-insensitively.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Episode order: numbered episodes by (season, episode), then the rest by name.
pub fn episode_cmp(a: (Option<u32>, Option<u32>, &str), b: (Option<u32>, Option<u32>, &str)) -> Ordering {
    let key = |(s, e, _): (Option<u32>, Option<u32>, &str)| (e.is_none(), s.unwrap_or(0), e.unwrap_or(0));
    key(a).cmp(&key(b)).then_with(|| natural_cmp(a.2, b.2))
}

/// A string safe to use as one path component on Windows and macOS.
/// `sep` replaces whitespace (use `" "` to keep spaces).
pub fn safe_component(s: &str, sep: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_sep = false;
    for c in s.chars() {
        let bad = matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control();
        if c.is_whitespace() || bad {
            pending_sep = !out.is_empty();
            continue;
        }
        if pending_sep {
            out.push_str(sep);
            pending_sep = false;
        }
        out.push(c);
        if out.chars().count() >= max_chars {
            break;
        }
    }
    // Windows refuses names ending in a dot or space, and reserved device names.
    let out = out.trim_end_matches(['.', ' ']).to_string();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"];
    if out.is_empty() || reserved.iter().any(|r| r.eq_ignore_ascii_case(&out)) { format!("_{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(name: &str) -> (Option<u32>, Option<u32>) {
        parse_episode(name, None)
    }

    #[test]
    fn scene_and_plex_names() {
        assert_eq!(ep("Breaking.Bad.S02E09.720p.BluRay.x264.mkv"), (Some(2), Some(9)));
        assert_eq!(ep("the office s3e15.mp4"), (Some(3), Some(15)));
        assert_eq!(ep("Show - S01.E02 - Pilot.mkv"), (Some(1), Some(2)));
        assert_eq!(ep("Show S01E02E03.mkv"), (Some(1), Some(2)));
        assert_eq!(ep("Friends 4x12 The Embryos.avi"), (Some(4), Some(12)));
        assert_eq!(ep("Season 2 Episode 7.mp4"), (Some(2), Some(7)));
        assert_eq!(ep("Dark (2017) - S01E01 - Secrets [1080p].mkv"), (Some(1), Some(1)));
    }

    #[test]
    fn fansub_and_anime_names() {
        assert_eq!(ep("[SubsPlease] Sousou no Frieren - 03 (1080p) [ABCD1234].mkv"), (None, Some(3)));
        assert_eq!(ep("[Erai-raws] Kimetsu no Yaiba - 12v2 [1080p][Multiple Subtitle].mkv"), (None, Some(12)));
        assert_eq!(ep("Steins;Gate - 24 [BD 1080p].mkv"), (None, Some(24)));
        assert_eq!(ep("One Piece - 1071 [720p].mp4"), (None, Some(1071)));
        assert_eq!(ep("進撃の巨人 第05話.mp4"), (None, Some(5)));
        assert_eq!(ep("show_name_ep_05.mp4"), (None, Some(5)));
        assert_eq!(ep("Episode 10.mkv"), (None, Some(10)));
        assert_eq!(ep("Frieren 03.mkv"), (None, Some(3)));
    }

    #[test]
    fn numbers_that_are_not_episodes() {
        assert_eq!(ep("Blade Runner 2049 (2017) 2160p HDR.mkv"), (None, None));
        assert_eq!(ep("Movie.1920x1080.x264.mkv"), (None, None));
        assert_eq!(ep("Interstellar.2014.1080p.BluRay.x265-RARBG.mp4"), (None, None));
        assert_eq!(ep("Spirited Away.mkv"), (None, None));
    }

    #[test]
    fn season_from_folder() {
        assert_eq!(parse_episode("Show - 05.mkv", Some("Season 2")), (Some(2), Some(5)));
        assert_eq!(parse_episode("05.mkv", Some("S03")), (Some(3), Some(5)));
        assert_eq!(parse_episode("Folge 05.mkv", Some("Staffel 1")), (Some(1), Some(5)));
        // The file's own season wins over the folder.
        assert_eq!(parse_episode("S04E01.mkv", Some("Season 2")), (Some(4), Some(1)));
        // No episode number: the folder alone doesn't invent one.
        assert_eq!(parse_episode("Extras.mkv", Some("Season 2")), (None, None));
    }

    #[test]
    fn natural_ordering() {
        let mut v = vec!["ep10.mkv", "ep2.mkv", "Ep1.mkv", "ep02b.mkv", "extra.mkv"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["Ep1.mkv", "ep2.mkv", "ep02b.mkv", "ep10.mkv", "extra.mkv"]);
        assert_eq!(natural_cmp("a01", "a1"), Ordering::Greater);
    }

    #[test]
    fn episode_ordering_puts_unnumbered_last() {
        let mut v = [(None, None, "NCOP"), (Some(1), Some(2), "b"), (None, Some(1), "a"), (Some(1), Some(1), "c")];
        v.sort_by(|a, b| episode_cmp(*a, *b));
        assert_eq!(v.iter().map(|x| x.2).collect::<Vec<_>>(), vec!["a", "c", "b", "NCOP"]);
    }

    #[test]
    fn safe_components() {
        assert_eq!(safe_component("Frieren: Beyond Journey's End", "_", 80), "Frieren_Beyond_Journey's_End");
        assert_eq!(safe_component("  What?  If  ", " ", 80), "What If");
        assert_eq!(safe_component("con", " ", 80), "_con");
        assert_eq!(safe_component("...", " ", 80), "_");
        assert_eq!(safe_component("abcdef", "_", 3), "abc");
    }
}
