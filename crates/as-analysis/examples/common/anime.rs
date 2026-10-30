//! The anime bench's files beside the labels (`bench/anime/FORMATS.md`):
//! the gallery and its scoring levels, shot presence, repeated spans, and
//! the shots of a `bb.segments.tsv`.

#![allow(dead_code)]

use std::collections::HashMap;

use crate::labels::tsv_rows;

/// What a name is scored as: a look (the code itself), a figure (a
/// transformation or mask/persona is a figure of its own, every other look
/// is its person's) or a person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Look,
    Figure,
    Person,
}

impl Level {
    /// `LEVEL=look|figure|person`, the look by default.
    pub fn from_env() -> Self {
        match std::env::var("LEVEL").as_deref() {
            Err(_) | Ok("look") => Self::Look,
            Ok("figure") => Self::Figure,
            Ok("person") => Self::Person,
            Ok(other) => panic!("LEVEL={other}: look, figure or person"),
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Look => "look",
            Self::Figure => "figure",
            Self::Person => "person",
        }
    }
}

/// `gallery.tsv`: per look code, its person and type.
pub struct Gallery {
    codes: HashMap<String, (String, String)>,
}

impl Gallery {
    pub fn read(path: &str) -> Self {
        Self::of(tsv_rows(path), path)
    }

    /// The gallery of a file's rows (`path` names it in errors).
    pub fn of(rows: Vec<Vec<String>>, path: &str) -> Self {
        const TYPES: [&str; 7] = ["base", "outfit", "hairstyle", "age", "chibi", "transformation", "mask"];
        let mut codes = HashMap::new();
        for v in rows {
            assert!(v.len() >= 3, "{path}: `code person type note`");
            assert!(TYPES.contains(&v[2].as_str()), "{path}: unknown type {}", v[2]);
            assert!(codes.insert(v[0].clone(), (v[1].clone(), v[2].clone())).is_none(), "{path}: {} twice", v[0]);
        }
        Self { codes }
    }

    pub fn contains(&self, code: &str) -> bool {
        self.codes.contains_key(code)
    }

    /// `code` at `level`; `None` when it isn't in the gallery.
    pub fn at<'a>(&'a self, code: &'a str, level: Level) -> Option<&'a str> {
        let (person, kind) = self.codes.get(code)?;
        Some(match level {
            Level::Look => code,
            Level::Figure if matches!(kind.as_str(), "transformation" | "mask") => code,
            Level::Figure | Level::Person => person,
        })
    }

    /// Every code of the gallery.
    pub fn codes(&self) -> impl Iterator<Item = &str> {
        self.codes.keys().map(String::as_str)
    }
}

/// A name at `level`: a gallery code mapped, anything else (no gallery, a
/// name outside it, `?e`, `n`, `x`, …) as it is.
pub fn at_level(name: &str, gallery: Option<&Gallery>, level: Level) -> String {
    gallery.and_then(|g| g.at(name, level)).unwrap_or(name).to_string()
}

/// A shot of `<project>.presence.tsv`: every code visible in it.
pub struct Presence {
    pub start: u32,
    pub end: u32,
    pub codes: Vec<String>,
}

pub fn read_presence(path: &str) -> Vec<Presence> {
    tsv_rows(path)
        .into_iter()
        .map(|v| Presence {
            start: v[0].parse().unwrap(),
            end: v[1].parse().unwrap(),
            codes: v.get(2).map_or(Vec::new(), |c| {
                c.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
            }),
        })
        .collect()
}

/// `<project>.spans.tsv`: OP/ED, eyecatch, recap and preview.
#[derive(Default)]
pub struct Spans(pub Vec<(u32, u32, String)>);

impl Spans {
    pub fn read(path: &str) -> Self {
        const KINDS: [&str; 5] = ["op", "ed", "eyecatch", "recap", "preview"];
        Self(
            tsv_rows(path)
                .into_iter()
                .map(|v| {
                    assert!(KINDS.contains(&v[2].as_str()), "{path}: unknown span kind {}", v[2]);
                    (v[0].parse().unwrap(), v[1].parse().unwrap(), v[2].clone())
                })
                .collect(),
        )
    }

    /// The kind of span `start..end` lies mostly (more than half) inside.
    pub fn mostly_in(&self, start: u32, end: u32) -> Option<&str> {
        let len = end.saturating_sub(start).max(1);
        self.0
            .iter()
            .map(|(a, b, k)| (end.min(*b).saturating_sub(start.max(*a)), k))
            .filter(|&(o, _)| 2 * o > len)
            .map(|(_, k)| k.as_str())
            .next()
    }

    /// Whether `frame` is inside a span.
    pub fn holds(&self, frame: u32) -> bool {
        self.0.iter().any(|(a, b, _)| (*a..*b).contains(&frame))
    }
}

/// The shots of a `bb.segments.tsv` (`start end kind`, no header): the
/// rows of kind `shot`, in order.
pub fn read_shots(path: &str) -> Vec<(u32, u32)> {
    let mut shots: Vec<(u32, u32)> = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .filter_map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            (v.len() >= 3 && v[2].trim() == "shot").then(|| (v[0].parse().unwrap(), v[1].parse().unwrap()))
        })
        .collect();
    shots.sort_unstable();
    shots
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::labels::parse_tsv;

    /// A gallery of two people: Tanjiro (base, outfit, age, chibi, mask)
    /// and Nezuko (base, hairstyle, transformation).
    pub fn gallery() -> Gallery {
        let text = "code\tperson\ttype\tnote\n\
                    # Tanjiro\n\
                    TAN\ttanjiro\tbase\tchecked haori\n\
                    TAN_U\ttanjiro\toutfit\tuniform only\n\
                    TAN_Y\ttanjiro\tage\tas a child\n\
                    TAN_C\ttanjiro\tchibi\t\n\
                    TAN_M\ttanjiro\tmask\tfox mask\n\
                    NEZ\tnezuko\tbase\tbamboo muzzle\n\
                    NEZ_H\tnezuko\thairstyle\tshort hair\n\
                    NEZ_D\tnezuko\ttransformation\tdemon form\n";
        Gallery::of(parse_tsv(text), "gallery")
    }

    #[test]
    fn transformations_and_masks_are_figures_of_their_own() {
        let g = gallery();
        for (code, figure, person) in [
            ("TAN", "tanjiro", "tanjiro"),
            ("TAN_U", "tanjiro", "tanjiro"),
            ("TAN_Y", "tanjiro", "tanjiro"),
            ("TAN_C", "tanjiro", "tanjiro"),
            ("TAN_M", "TAN_M", "tanjiro"),
            ("NEZ_H", "nezuko", "nezuko"),
            ("NEZ_D", "NEZ_D", "nezuko"),
        ] {
            assert_eq!(g.at(code, Level::Look), Some(code));
            assert_eq!(g.at(code, Level::Figure), Some(figure), "{code}");
            assert_eq!(g.at(code, Level::Person), Some(person), "{code}");
        }
        assert_eq!(g.at("ZEN", Level::Figure), None);
        // Names outside the gallery stay as they are, at every level.
        for name in ["?e", "?u", "n", "x", "ZEN"] {
            assert_eq!(at_level(name, Some(&g), Level::Person), name);
        }
        assert_eq!(at_level("NEZ_D", None, Level::Person), "NEZ_D");
    }

    #[test]
    #[should_panic(expected = "unknown type")]
    fn a_gallery_knows_its_types() {
        Gallery::of(parse_tsv("code\tperson\ttype\nTAN\ttanjiro\tdemon\n"), "gallery");
    }
}
