//! Language tags on audio/subtitle streams.
//!
//! Containers use ISO 639-2 codes (bibliographic `ger` or terminology `deu`),
//! sometimes ISO 639-1 (`de`) or a plain name. [`canonical`] folds all of them
//! to one ISO 639-2/T code so "prefer English" matches `eng`, `en` and
//! `English` alike.

/// `(639-2/T, 639-2/B alias, 639-1, English name)`.
const LANGS: &[(&str, &str, &str, &str)] = &[
    ("eng", "", "en", "english"),
    ("jpn", "", "ja", "japanese"),
    ("deu", "ger", "de", "german"),
    ("fra", "fre", "fr", "french"),
    ("spa", "", "es", "spanish"),
    ("ita", "", "it", "italian"),
    ("por", "", "pt", "portuguese"),
    ("rus", "", "ru", "russian"),
    ("kor", "", "ko", "korean"),
    ("zho", "chi", "zh", "chinese"),
    ("hin", "", "hi", "hindi"),
    ("ara", "", "ar", "arabic"),
    ("tur", "", "tr", "turkish"),
    ("pol", "", "pl", "polish"),
    ("nld", "dut", "nl", "dutch"),
    ("swe", "", "sv", "swedish"),
    ("nor", "", "no", "norwegian"),
    ("dan", "", "da", "danish"),
    ("fin", "", "fi", "finnish"),
    ("ces", "cze", "cs", "czech"),
    ("hun", "", "hu", "hungarian"),
    ("ell", "gre", "el", "greek"),
    ("heb", "", "he", "hebrew"),
    ("tha", "", "th", "thai"),
    ("vie", "", "vi", "vietnamese"),
    ("ind", "", "id", "indonesian"),
    ("ukr", "", "uk", "ukrainian"),
    ("ron", "rum", "ro", "romanian"),
    ("tam", "", "ta", "tamil"),
    ("tel", "", "te", "telugu"),
];

/// ISO 639-2/T code for a stream language tag, or the lower-cased tag itself
/// when it isn't one we know. `None` for empty/undetermined tags.
pub fn canonical(tag: &str) -> Option<String> {
    let t = tag.trim().to_ascii_lowercase();
    // "en-US", "pt_BR" → "en", "pt"
    let t = t.split(['-', '_']).next().unwrap_or_default().to_string();
    if t.is_empty() || matches!(t.as_str(), "und" | "unk" | "mis" | "zxx" | "mul") {
        return None;
    }
    Some(
        LANGS
            .iter()
            .find(|(t2, b, one, name)| t == *t2 || (!b.is_empty() && t == *b) || t == *one || t == *name)
            .map_or(t, |(t2, ..)| (*t2).to_string()),
    )
}

/// Do two language tags name the same language?
pub fn same(a: &str, b: &str) -> bool {
    matches!((canonical(a), canonical(b)), (Some(x), Some(y)) if x == y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_codes_and_names() {
        assert_eq!(canonical("ger").as_deref(), Some("deu"));
        assert_eq!(canonical("DE").as_deref(), Some("deu"));
        assert_eq!(canonical("en-US").as_deref(), Some("eng"));
        assert_eq!(canonical("English").as_deref(), Some("eng"));
        assert_eq!(canonical("und"), None);
        assert_eq!(canonical("tlh").as_deref(), Some("tlh"));
        assert!(same("chi", "zh"));
        assert!(!same("eng", "hin"));
        assert!(!same("und", "und"));
    }
}
