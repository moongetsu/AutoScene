//! Naming the characters of animated films from their franchise's Fandom
//! wiki (behind the same opt-in as AniList and TVmaze, see
//! [`crate::anilist`]): AniList knows anime, TVmaze series, but a film's own
//! characters — the film's designs of them, a picture each — are on the
//! film's page in its franchise's wiki.
//!
//! The film and its wiki are found through Wikidata ([`Fandom::search`]):
//! the film's own article in an English wiki, else an article of what it is
//! based on, its series or its franchise, else a wiki named after one of
//! those when such a wiki exists. On the film's page the sections about its
//! characters or its cast name them — in a cast line only the links after
//! "as", not the actors — and each character page's main picture shows
//! them. Both answer without a key; Fandom's pictures come only to a request
//! that says it is from Fandom. Wikidata's data is CC0, Fandom's CC BY-SA:
//! the app says where the names come from.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use as_core::CancelToken;
use parking_lot::Mutex;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::anilist::{
    ATTEMPTS, Character, Clock, Limiter, MAX_PAUSE, Media, Names, Reply, Role, SystemClock, Titles, Transport, Ureq,
    going, image_ext, store_picture, title_key,
};
use crate::{ProjectError, ProjectResult};

const WIKIDATA: &str = "https://www.wikidata.org/w/api.php";
/// At least this long between two requests to Wikidata or a wiki.
const API_GAP: Duration = Duration::from_millis(400);
/// Between two pictures from Fandom's image server.
const IMAGE_GAP: Duration = Duration::from_millis(250);
/// The shortest pause after "too many requests".
const MIN_PAUSE: Duration = Duration::from_secs(10);
/// Search results read.
const RESULTS: u32 = 8;
/// The pictures' width (the image server scales them).
const PICTURE_WIDTH: u32 = 600;
/// Pages asked for at once (the wikis' most).
const PER_QUERY: usize = 50;
/// A film's characters read at most.
const MAX_CHARACTERS: usize = 300;
/// What a film is on Wikidata (`P31`): film, animated film, animated
/// feature film, feature film, television film, animated short film, short
/// film, film adaptation.
const FILM_CLASSES: [&str; 8] =
    ["Q11424", "Q202866", "Q29168811", "Q24869", "Q506240", "Q17517379", "Q24862", "Q1257444"];
/// What a film is based on, the series it belongs to, its franchise: where
/// its wiki is when the film has no article of its own.
const RELATED: [&str; 3] = ["P144", "P179", "P8345"];
/// Words of a description of something about films that isn't one.
const NOT_A_FILM: [&str; 9] =
    ["series", "franchise", "soundtrack", "album", "song", "character", "game", "novel", "book"];
/// Wikis about films in general, or about something else that has pages on
/// films: never a film's characters' wiki.
const GENERIC_WIKIS: [&str; 10] = [
    "movies",
    "moviepedia",
    "logos",
    "youtube",
    "gamicus",
    "video-games",
    "dubbing",
    "allthetropes",
    "lyrics",
    "soundtrack",
];
/// Title words no wiki is named after.
const COMMON_WORDS: [&str; 8] = ["movie", "movies", "film", "films", "with", "from", "part", "chapter"];
/// A film's format ([`Media::format`]) begins with this.
pub const FILM: &str = "Film";

// ---------------------------------------------------------------------------
// What Wikidata and the wikis answer
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SearchAnswer {
    query: Option<SearchQuery>,
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    search: Vec<SearchHit>,
}

#[derive(Deserialize)]
struct SearchHit {
    title: String,
}

#[derive(Deserialize)]
struct Entities {
    #[serde(default)]
    entities: HashMap<String, Entity>,
}

#[derive(Default, Deserialize)]
struct Entity {
    #[serde(default)]
    labels: HashMap<String, Text>,
    #[serde(default)]
    aliases: HashMap<String, Vec<Text>>,
    #[serde(default)]
    descriptions: HashMap<String, Text>,
    #[serde(default)]
    claims: HashMap<String, Vec<Claim>>,
}

#[derive(Deserialize)]
struct Text {
    value: String,
}

#[derive(Deserialize)]
struct Claim {
    mainsnak: Snak,
}

#[derive(Deserialize)]
struct Snak {
    datavalue: Option<DataValue>,
}

#[derive(Deserialize)]
struct DataValue {
    value: serde_json::Value,
}

impl Entity {
    /// The values of property `p`: item ids (`Q…`), strings, or a time's
    /// `+YYYY-…`.
    fn values(&self, p: &str) -> Vec<String> {
        let one = |v: &serde_json::Value| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(o) => {
                o.get("id").or_else(|| o.get("time")).and_then(|x| x.as_str()).map(str::to_owned)
            }
            _ => None,
        };
        self.claims.get(p).into_iter().flatten().filter_map(|c| one(&c.mainsnak.datavalue.as_ref()?.value)).collect()
    }

    fn label(&self) -> Option<String> {
        ["en", "mul"]
            .iter()
            .find_map(|l| self.labels.get(*l))
            .map(|t| t.value.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// A film by what it is an instance of, else by a description that
    /// says film and nothing else (not a film series, a franchise, a
    /// soundtrack).
    fn is_film(&self) -> bool {
        if self.values("P31").iter().any(|c| FILM_CLASSES.contains(&c.as_str())) {
            return true;
        }
        let d = self.descriptions.get("en").map(|d| title_key(&d.value)).unwrap_or_default();
        let words: Vec<&str> = d.split(' ').collect();
        let film = words.iter().any(|w| matches!(*w, "film" | "movie"));
        let other = words.iter().any(|w| NOT_A_FILM.contains(w));
        film && !other
    }

    /// Every title it has, in every language.
    fn titles(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in self.labels.values().chain(self.aliases.values().flatten()) {
            let t = t.value.trim();
            if !t.is_empty() && !out.iter().any(|o| o == t) {
                out.push(t.to_string());
            }
        }
        out
    }

    /// Its articles in English Fandom wikis: `(wiki, article)`.
    fn articles(&self) -> Vec<(String, String)> {
        self.values("P6262")
            .into_iter()
            .filter_map(|f| {
                let (wiki, article) = f.split_once(':')?;
                let english = !wiki.is_empty() && !wiki.contains('.') && valid_wiki(wiki);
                english.then(|| (wiki.to_string(), article.replace('_', " ")))
            })
            .collect()
    }
}

/// A wiki's name as a host's first label: letters, digits and dashes.
fn valid_wiki(w: &str) -> bool {
    !w.is_empty() && w.len() <= 63 && w.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[derive(Deserialize)]
struct ParseAnswer {
    parse: Option<Parsed>,
}

#[derive(Deserialize)]
struct Parsed {
    title: Option<String>,
    #[serde(default)]
    sections: Vec<Section>,
    wikitext: Option<Wikitext>,
}

#[derive(Deserialize)]
struct Section {
    index: String,
    /// Where it is in the outline (`4.1`: the first under the fourth).
    #[serde(default)]
    number: String,
    line: String,
}

#[derive(Deserialize)]
struct Wikitext {
    #[serde(rename = "*")]
    text: String,
}

#[derive(Deserialize)]
struct PagesAnswer {
    query: Option<Pages>,
}

#[derive(Deserialize)]
struct Pages {
    #[serde(default)]
    pages: HashMap<String, Page>,
    #[serde(default)]
    normalized: Vec<Moved>,
    #[serde(default)]
    redirects: Vec<Moved>,
}

#[derive(Deserialize)]
struct Moved {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct Page {
    pageid: Option<u32>,
    title: String,
    thumbnail: Option<Thumbnail>,
    /// Its lead section (`rvsection=0`): the infobox.
    revisions: Option<Vec<Revision>>,
    /// A file page's picture.
    imageinfo: Option<Vec<ImageInfo>>,
}

impl Page {
    fn lead(&self) -> Option<&str> {
        self.revisions.as_ref()?.first()?.slots.as_ref()?.main.as_ref()?.content.as_deref()
    }
}

#[derive(Deserialize)]
struct Revision {
    slots: Option<Slots>,
}

#[derive(Deserialize)]
struct Slots {
    main: Option<Slot>,
}

#[derive(Deserialize)]
struct Slot {
    #[serde(rename = "*")]
    content: Option<String>,
}

#[derive(Deserialize)]
struct ImageInfo {
    thumburl: Option<String>,
}

#[derive(Deserialize)]
struct Thumbnail {
    source: String,
}

#[derive(Deserialize)]
struct SiteInfo {
    query: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Reading a film's page
// ---------------------------------------------------------------------------

/// What a section of a film's page lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lists {
    /// Its characters: every page a line links to.
    Characters,
    /// Its cast: actors and the characters they play.
    Cast,
}

/// The sections of a film's page that name its characters, by the words of
/// their headings: those about its characters or its cast ("Cast and crew"
/// too; not "Casting", "Broadcast" or "Character design") — each read with
/// what is under it, so what is under one read isn't read again — but
/// nothing under one about its crew, or about how the film differs from the
/// series (its characters there are the series').
fn character_sections(sections: &[Section]) -> Vec<(&Section, Lists)> {
    let mut out: Vec<(&Section, Lists)> = Vec::new();
    let mut passed: Vec<&str> = Vec::new();
    let under = |s: &Section, of: &str| s.number.strip_prefix(of).is_some_and(|rest| rest.starts_with('.'));
    for s in sections {
        if out.iter().any(|(o, _)| under(s, &o.number)) || passed.iter().any(|p| under(s, p)) {
            continue;
        }
        let words: Vec<String> = title_key(&s.line).split(' ').map(str::to_owned).collect();
        let has = |w: &str| words.iter().any(|x| x == w);
        if has("cast") {
            out.push((s, Lists::Cast));
        } else if (has("characters") || has("character")) && !has("design") {
            out.push((s, Lists::Characters));
        } else if has("differences") || has("difference") || has("crew") {
            passed.push(&s.number);
        }
    }
    out
}

/// `text` without what isn't a list's own words: references, comments and
/// templates (`{{WP|Jack Black}}`).
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut depth = 0usize;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("{{") {
            depth += 1;
            rest = r;
        } else if depth > 0 && rest.starts_with("}}") {
            depth -= 1;
            rest = &rest[2..];
        } else if depth == 0 && rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
        } else if depth == 0 && rest.starts_with("<ref") {
            let open_end = rest.find('>').unwrap_or(rest.len() - 1);
            rest = if rest[..=open_end].ends_with("/>") {
                &rest[open_end + 1..]
            } else {
                rest.find("</ref>").map_or("", |end| &rest[end + 6..])
            };
        } else {
            let c = rest.chars().next().unwrap_or_default();
            if depth == 0 {
                out.push(c);
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// What stands between an actor and their character in a cast line.
const CAST_SEPARATORS: [&str; 5] = [" as ", ": ", " - ", " – ", " — "];

/// The pages a section's wikitext links to, with the text shown for them.
/// In a cast line (`[[Actor]] as [[Character]]`, `[[Actor]]: [[Character]]`,
/// `[[Actor]] - [[Character]]`) only the characters after it: the first
/// link, and those that follow it in a list (`,`, `/`, `and`); a cast line
/// without one names no character.
fn character_links(wikitext: &str, lists: Lists) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for line in plain(wikitext).lines() {
        let line = match lists {
            Lists::Characters => line,
            Lists::Cast => {
                let at = CAST_SEPARATORS.iter().filter_map(|s| line.find(s).map(|i| (i, s.len()))).min();
                match at {
                    Some((i, len)) => &line[i + len..],
                    None => continue,
                }
            }
        };
        let mut rest = line;
        let mut first = true;
        while let Some(start) = rest.find("[[") {
            let between = rest[..start].trim().trim_matches('\'').trim();
            if lists == Lists::Cast
                && !(between.is_empty() || matches!(between, "," | "/" | "&" | "and" | ", and" | "or"))
            {
                break;
            }
            if lists == Lists::Cast && first && !between.is_empty() {
                break;
            }
            first = false;
            let after = &rest[start + 2..];
            let Some(end) = after.find("]]") else { break };
            let inner = &after[..end];
            rest = &after[end + 2..];
            let (target, shown) = match inner.split_once('|') {
                Some((t, s)) => (t.trim(), Some(s.trim().to_string()).filter(|s| !s.is_empty())),
                None => (inner.trim(), None),
            };
            let target = target.split('#').next().unwrap_or_default().trim();
            // Files, categories, other wikis.
            if target.is_empty() || target.contains(':') {
                continue;
            }
            out.push((target.replace('_', " "), shown));
        }
    }
    out
}

/// A character's name from its page title or the text shown for it: a
/// film's version of them is a page of its own (`Tikki/Movie`, `Mime
/// (movie)`); its name is theirs. Of two names in one (`Marinette
/// Dupain-Cheng/Ladybug`: her and her other self), the first: each self is
/// a character of its own (see [`identities`]).
fn character_name(title: &str, shown: Option<&str>) -> String {
    let name = shown.filter(|s| s.len() <= 60 && !s.contains("[[")).unwrap_or(title);
    let mut name = name.replace("''", "").trim().to_string();
    for suffix in ["/movie", "/film", " (movie)", " (film)"] {
        if name.to_lowercase().ends_with(suffix) {
            name.truncate(name.len() - suffix.len());
        }
    }
    let name = name.split('/').next().unwrap_or_default();
    name.trim().to_string()
}

/// `name` without a page's disambiguation that only names the film or its
/// franchise (`Dolly (Toy Story)` in Toy Story 4): `film` is the film's
/// title words.
fn without_film(name: &str, film: &HashSet<String>) -> String {
    let trimmed = name.trim_end();
    let Some(inner) = trimmed.strip_suffix(')').and_then(|s| s.rsplit_once(" (")) else { return name.to_string() };
    let words: Vec<String> = title_key(inner.1).split(' ').map(str::to_owned).collect();
    let film_words = words.iter().all(|w| film.contains(w) || matches!(w.as_str(), "the" | "character" | "franchise"));
    if film_words && !words.is_empty() && !inner.0.trim().is_empty() {
        inner.0.trim().to_string()
    } else {
        name.to_string()
    }
}

/// The selves a character page's infobox shows a picture of each, `(file,
/// caption)`, when it shows more than one — a hero and their everyday self,
/// a villain's other form: its image is a gallery (`File.png|Caption` a
/// line) or tabs (`Caption=[[File:File.png]]`, `|-|` between). Empty when
/// it shows one picture: the page's own picture is theirs.
fn identities(lead: &str) -> Vec<(String, String)> {
    let Some(infobox) = lead.find("{{Infobox").or_else(|| lead.find("{{infobox")).map(|i| &lead[i..]) else {
        return Vec::new();
    };
    // The infobox's image, right after `|image =`.
    let Some(image) = infobox.find("|image").map(|i| &infobox[i + "|image".len()..]) else { return Vec::new() };
    let Some(value) = image.trim_start().strip_prefix('=').map(str::trim_start) else { return Vec::new() };
    let file = |f: &str| {
        let f = f.trim();
        let f = f.strip_prefix("File:").or_else(|| f.strip_prefix("Image:")).unwrap_or(f);
        f.trim().replace('_', " ")
    };
    let caption = |c: &str| c.replace(['[', ']', '\''], "").trim().to_string();
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(gallery) = value.strip_prefix("<gallery") {
        let Some(body) = gallery.split_once('>').map(|(_, b)| b) else { return Vec::new() };
        let Some((body, _)) = body.split_once("</gallery>") else { return Vec::new() };
        for line in body.lines() {
            let (f, c) = line.split_once('|').unwrap_or((line, ""));
            if !f.trim().is_empty() {
                out.push((file(f), caption(c)));
            }
        }
    } else if let Some(tabber) = value.strip_prefix("<tabber>") {
        let Some((body, _)) = tabber.split_once("</tabber>") else { return Vec::new() };
        for tab in body.split("|-|") {
            let Some((c, rest)) = tab.split_once('=') else { continue };
            let Some(f) = rest.split_once("[[").and_then(|(_, r)| r.split(['|', ']']).next()) else { continue };
            out.push((file(f), caption(c)));
        }
    }
    if out.len() < 2 {
        return Vec::new();
    }
    out
}

/// The name of self `n` of a character named `name` whose infobox captions
/// it `caption`: the first self is the character (their full name, when the
/// caption is the start of it: `Marinette` of Marinette Dupain-Cheng),
/// another one goes by its caption (`Ladybug`), else by its picture's name.
fn identity_name(name: &str, n: usize, caption: &str, file: &str) -> String {
    let full = name.to_lowercase();
    let short = caption.to_lowercase();
    if n == 0 && (short.is_empty() || full == short || full.starts_with(&format!("{short} "))) {
        return name.to_string();
    }
    if !caption.is_empty() {
        return caption.to_string();
    }
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    character_name(stem.replace(" Square", "").trim(), None)
}

/// A stable id for self `n` of page `page` of wiki `wiki` (FNV-1a): page
/// ids are only unique within one wiki.
fn character_id(wiki: &str, page: u32, n: usize) -> u32 {
    let key = if n == 0 { format!("{wiki}:{page}") } else { format!("{wiki}:{page}:{n}") };
    let mut h: u32 = 0x811c_9dc5;
    for b in key.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// `s` for a URL's query: unreserved characters as they are, the rest
/// percent-encoded as UTF-8.
fn query_part(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A wiki name a title suggests (`Kung Fu Panda` → `kungfupanda`).
fn wiki_name(title: &str) -> Option<String> {
    let head = title.split([':', '(', '–', '—']).next().unwrap_or_default();
    let words: Vec<&str> = head.split_whitespace().collect();
    // A sequel's number or "The Movie" isn't the franchise's name.
    let words: Vec<&str> = match words.as_slice() {
        [rest @ .., last] if last.chars().all(|c| c.is_ascii_digit() || "IVX".contains(c)) => rest.to_vec(),
        _ => words,
    };
    let name: String = words.concat().chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    (name.len() >= 3).then_some(name)
}

/// Only Fandom's own image server is asked for pictures, over https.
fn on_image_server(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    host == "static.wikia.nocookie.net"
}

fn unreachable(e: impl std::fmt::Display) -> ProjectError {
    tracing::warn!("fandom: {e}");
    ProjectError::NotReady(
        "Wikidata or Fandom can't be reached right now. Check the internet connection and try again.".into(),
    )
}

fn busy() -> ProjectError {
    ProjectError::NotReady("Wikidata or Fandom is busy. Try again in a few minutes.".into())
}

fn no_wiki(title: &str) -> ProjectError {
    ProjectError::NotFound(format!("No Fandom wiki was found for {title}."))
}

static API_LIMITER: LazyLock<Arc<Limiter>> = LazyLock::new(|| Arc::new(Limiter::new(API_GAP)));
static IMAGE_LIMITER: LazyLock<Arc<Limiter>> = LazyLock::new(|| Arc::new(Limiter::new(IMAGE_GAP)));

/// A film found on Wikidata, with the wiki that knows its characters and
/// its page there, when known.
#[derive(Clone, Debug)]
struct Film {
    media: Media,
    wiki: Option<String>,
    page: Option<String>,
}

/// Wikidata and Fandom for one library: requests, the films and casts read
/// this session, and the picture cache.
pub struct Fandom {
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    api: Arc<Limiter>,
    images: Arc<Limiter>,
    /// The library's character pictures (`<id>.<ext>`).
    dir: PathBuf,
    /// Films this session has read, by id (Wikidata's `Q` number).
    films: Mutex<HashMap<u32, Film>>,
    casts: Mutex<HashMap<u32, Vec<Character>>>,
    portraits: Mutex<HashMap<u32, Vec<f32>>>,
    /// Whether a wiki exists, as asked this session.
    wikis: Mutex<HashMap<String, bool>>,
}

impl Fandom {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self::with(dir, Arc::new(Ureq::new()), Arc::new(SystemClock), API_LIMITER.clone(), IMAGE_LIMITER.clone())
    }

    pub(crate) fn with(
        dir: PathBuf,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
        api: Arc<Limiter>,
        images: Arc<Limiter>,
    ) -> Self {
        Self {
            transport,
            clock,
            api,
            images,
            dir,
            films: Mutex::default(),
            casts: Mutex::default(),
            portraits: Mutex::default(),
            wikis: Mutex::default(),
        }
    }

    /// Where a character's picture is cached, once loaded.
    pub fn image_dir(&self) -> &Path {
        &self.dir
    }

    /// One GET, its turn waited for, tried again while asked to wait.
    /// `None`: there is no such thing.
    fn request<T: DeserializeOwned>(&self, url: &str, cancel: &CancelToken) -> ProjectResult<Option<T>> {
        for _ in 0..ATTEMPTS {
            self.api.acquire(&*self.clock, cancel)?;
            let reply = self.transport.get(url).map_err(unreachable)?;
            match reply.status {
                429 => {
                    let wait = reply.retry_after.unwrap_or_default().clamp(MIN_PAUSE, MAX_PAUSE);
                    tracing::info!(wait_s = wait.as_secs(), "fandom: too many requests, pausing");
                    self.api.pause(self.clock.now(), wait);
                }
                404 => return Ok(None),
                200 => {
                    return serde_json::from_slice(&reply.body)
                        .map(Some)
                        .map_err(|e| unreachable(format!("{url}: not the answer: {e}")));
                }
                status => return Err(unreachable(format!("{url}: HTTP {status}"))),
            }
        }
        Err(busy())
    }

    fn wiki_api(wiki: &str, query: &str) -> String {
        format!("https://{wiki}.fandom.com/api.php?{query}&format=json")
    }

    /// Wikidata's items `ids` (labels, aliases, descriptions, claims).
    fn entities(&self, ids: &[String], cancel: &CancelToken) -> ProjectResult<HashMap<String, Entity>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(PER_QUERY) {
            let url = format!(
                "{WIKIDATA}?action=wbgetentities&ids={}&props=labels%7Caliases%7Cdescriptions%7Cclaims&format=json",
                query_part(&chunk.join("|"))
            );
            let answer: Option<Entities> = self.request(&url, cancel)?;
            out.extend(answer.map(|a| a.entities).unwrap_or_default());
        }
        Ok(out)
    }

    /// Whether wiki `wiki` exists (asked once a session; one that doesn't
    /// answer as a wiki doesn't).
    fn wiki_exists(&self, wiki: &str, cancel: &CancelToken) -> ProjectResult<bool> {
        if let Some(&known) = self.wikis.lock().get(wiki) {
            return Ok(known);
        }
        let answer: ProjectResult<Option<SiteInfo>> =
            self.request(&Self::wiki_api(wiki, "action=query&meta=siteinfo"), cancel);
        let exists = match answer {
            Err(ProjectError::Cancelled) => return Err(ProjectError::Cancelled),
            Ok(answer) => answer.is_some_and(|a| a.query.is_some()),
            Err(_) => false,
        };
        self.wikis.lock().insert(wiki.to_string(), exists);
        Ok(exists)
    }

    /// The wiki that knows film `e`'s characters, and the film's page there
    /// when Wikidata names it (see the module's doc); `None` when there's
    /// none.
    fn wiki_of(
        &self,
        e: &Entity,
        related: &HashMap<String, Entity>,
        cancel: &CancelToken,
    ) -> ProjectResult<Option<(String, Option<String>)>> {
        let titles: Vec<String> = std::iter::once(e.label().unwrap_or_default())
            .chain(RELATED.iter().flat_map(|p| e.values(p)).filter_map(|q| related.get(&q)?.label()))
            .collect();
        let words: HashSet<String> = titles
            .iter()
            .flat_map(|t| title_key(t).split(' ').map(str::to_owned).collect::<Vec<_>>())
            .filter(|w| w.len() >= 4 && !COMMON_WORDS.contains(&w.as_str()))
            .collect();
        let named = |wiki: &str| words.iter().any(|w| wiki.to_lowercase().contains(w.as_str()));
        let generic = |wiki: &str| GENERIC_WIKIS.contains(&wiki.to_lowercase().as_str());
        let own = e.articles();
        // The film's own article in a wiki named after it or its franchise.
        if let Some((wiki, article)) = own.iter().find(|(w, _)| named(w) && !generic(w)) {
            return Ok(Some((wiki.clone(), Some(article.clone()))));
        }
        // What it is based on, its series, its franchise: their wikis named
        // after them (never a wiki about something else that has a page on it).
        for p in RELATED {
            for q in e.values(p) {
                for (wiki, _) in related.get(&q).map(Entity::articles).unwrap_or_default() {
                    if named(&wiki) && !generic(&wiki) {
                        return Ok(Some((wiki, None)));
                    }
                }
            }
        }
        // Its own article in a studio's wiki (Disney's), not a wiki of films
        // in general.
        if let Some((wiki, article)) = own.iter().find(|(w, _)| !generic(w)) {
            return Ok(Some((wiki.clone(), Some(article.clone()))));
        }
        // A wiki named after the franchise, the series, the film.
        let mut tried = HashSet::new();
        for title in titles.iter().skip(1).chain(titles.first()) {
            let Some(wiki) = wiki_name(title) else { continue };
            if tried.insert(wiki.clone()) && self.wiki_exists(&wiki, cancel)? {
                return Ok(Some((wiki, None)));
            }
        }
        Ok(None)
    }

    /// Films matching `text` (sent to Wikidata as it is), best first, each
    /// with the wiki that knows its characters; films without one are left
    /// out.
    pub fn search(&self, text: &str, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ProjectError::Invalid("Type the name of the film to look for.".into()));
        }
        let url = format!(
            "{WIKIDATA}?action=query&list=search&srsearch={}&srnamespace=0&srlimit={RESULTS}&format=json",
            query_part(text)
        );
        let answer: Option<SearchAnswer> = self.request(&url, cancel)?;
        let ids: Vec<String> =
            answer.and_then(|a| a.query).map(|q| q.search).unwrap_or_default().into_iter().map(|h| h.title).collect();
        let films = self.films_of(&ids, cancel)?;
        going(cancel)?;
        let found: Vec<Media> = films.iter().map(|f| f.media.clone()).collect();
        self.films.lock().extend(films.into_iter().map(|f| (f.media.id, f)));
        Ok(found)
    }

    /// The films among items `ids`, in their order, with their wikis.
    fn films_of(&self, ids: &[String], cancel: &CancelToken) -> ProjectResult<Vec<Film>> {
        let entities = self.entities(ids, cancel)?;
        let films: Vec<(&String, &Entity)> =
            ids.iter().filter_map(|q| Some((q, entities.get(q)?))).filter(|(_, e)| e.is_film()).collect();
        let related_ids: Vec<String> = films
            .iter()
            .flat_map(|(_, e)| RELATED.iter().flat_map(|p| e.values(p)))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let related = if related_ids.is_empty() { HashMap::new() } else { self.entities(&related_ids, cancel)? };
        let mut out = Vec::new();
        for (q, e) in films {
            let Some(id) = q.strip_prefix('Q').and_then(|n| n.parse::<u32>().ok()) else { continue };
            let Some(label) = e.label() else { continue };
            let Some((wiki, page)) = self.wiki_of(e, &related, cancel)? else {
                tracing::info!(film = q.as_str(), "fandom: no wiki for the film");
                continue;
            };
            let year = e.values("P577").iter().filter_map(|t| t.get(1..5)?.parse::<u32>().ok()).min();
            let media = Media {
                id,
                id_mal: None,
                format: Some(format!("{FILM} · {wiki}.fandom.com")),
                episodes: None,
                season_year: year,
                title: Titles { english: Some(label), romaji: None, native: None },
                synonyms: e.titles(),
                sequels: Vec::new(),
            };
            out.push(Film { media, wiki: Some(wiki), page });
        }
        Ok(out)
    }

    fn film(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Film> {
        if let Some(f) = self.films.lock().get(&id) {
            return Ok(f.clone());
        }
        let film = self.films_of(&[format!("Q{id}")], cancel)?.pop().ok_or_else(|| not_a_film(id))?;
        going(cancel)?;
        self.films.lock().insert(id, film.clone());
        Ok(film)
    }

    /// Film `id` (read once a session).
    pub fn media(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Media> {
        Ok(self.film(id, cancel)?.media)
    }

    /// The film's page in its wiki: the one Wikidata names, else the one
    /// the wiki finds titled as the film is (a wiki about something else
    /// has none).
    fn page_of(&self, film: &Film, wiki: &str, cancel: &CancelToken) -> ProjectResult<Option<String>> {
        if let Some(page) = &film.page {
            return Ok(Some(page.clone()));
        }
        let label = film.media.name();
        let query = format!("action=query&list=search&srsearch={}&srnamespace=0&srlimit=5", query_part(&label));
        let answer: Option<SearchAnswer> = self.request(&Self::wiki_api(wiki, &query), cancel)?;
        let hits: Vec<String> =
            answer.and_then(|a| a.query).map(|q| q.search).unwrap_or_default().into_iter().map(|h| h.title).collect();
        let keys: Vec<String> =
            std::iter::once(label).chain(film.media.synonyms.iter().cloned()).map(|t| title_key(&t)).collect();
        Ok(hits.iter().find(|h| keys.contains(&title_key(h))).cloned())
    }

    /// Film `id`'s characters, in the order its page names them, each once,
    /// with their pictures' addresses — each self of one ([`identities`]) a
    /// character of its own: a hero and their everyday self look nothing
    /// alike, and are named apart.
    pub fn characters(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Vec<Character>> {
        if let Some(cast) = self.casts.lock().get(&id) {
            return Ok(cast.clone());
        }
        let film = self.film(id, cancel)?;
        let wiki = film.wiki.clone().ok_or_else(|| no_wiki(&film.media.name()))?;
        let page = self.page_of(&film, &wiki, cancel)?.ok_or_else(|| no_wiki(&film.media.name()))?;
        let query = format!("action=parse&page={}&prop=sections&redirects=1", query_part(&page));
        let parsed: Option<ParseAnswer> = self.request(&Self::wiki_api(&wiki, &query), cancel)?;
        let parsed = parsed.and_then(|p| p.parse).ok_or_else(|| no_wiki(&film.media.name()))?;
        let page = parsed.title.unwrap_or(page);
        let film_words: HashSet<String> = std::iter::once(film.media.name())
            .chain(film.media.synonyms.iter().cloned())
            .flat_map(|t| title_key(&t).split(' ').map(str::to_owned).collect::<Vec<_>>())
            .collect();
        // Each page once, with the text the page shows for it first.
        let mut links: Vec<(String, Option<String>)> = Vec::new();
        for (s, lists) in character_sections(&parsed.sections) {
            let query = format!(
                "action=parse&page={}&section={}&prop=wikitext&redirects=1",
                query_part(&page),
                query_part(&s.index)
            );
            let text: Option<ParseAnswer> = self.request(&Self::wiki_api(&wiki, &query), cancel)?;
            let text = text.and_then(|p| p.parse?.wikitext).map(|w| w.text).unwrap_or_default();
            for (target, shown) in character_links(&text, lists) {
                if !links.iter().any(|(t, _)| *t == target) && links.len() < MAX_CHARACTERS {
                    links.push((target, shown));
                }
            }
        }
        let mut out: Vec<Character> = Vec::new();
        for chunk in links.chunks(PER_QUERY) {
            let titles = chunk.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join("|");
            let query = format!(
                "action=query&prop=pageimages&piprop=thumbnail&pithumbsize={PICTURE_WIDTH}&redirects=1&titles={}",
                query_part(&titles)
            );
            let answer: Option<PagesAnswer> = self.request(&Self::wiki_api(&wiki, &query), cancel)?;
            let Some(pages) = answer.and_then(|a| a.query) else { continue };
            // A link's title as the wiki answers for it (normalized, followed).
            let moved: HashMap<&str, &str> =
                pages.normalized.iter().chain(&pages.redirects).map(|m| (m.from.as_str(), m.to.as_str())).collect();
            let by_title: HashMap<&str, &Page> = pages.pages.values().map(|p| (p.title.as_str(), p)).collect();
            let found: Vec<(&Page, Option<&str>)> = chunk
                .iter()
                .filter_map(|(target, shown)| {
                    let mut title = target.as_str();
                    for _ in 0..2 {
                        title = moved.get(title).copied().unwrap_or(title);
                    }
                    Some((*by_title.get(title)?, shown.as_deref()))
                })
                .collect();
            let selves = self.selves(&wiki, found.iter().map(|(p, _)| p.title.as_str()), cancel)?;
            for (page, shown) in found {
                let Some(pageid) = page.pageid else { continue };
                let name = without_film(&character_name(&page.title, shown), &film_words);
                if name.is_empty() || name.to_lowercase().starts_with("list of") {
                    continue;
                }
                let pictures: Vec<(String, String)> = match selves.get(page.title.as_str()) {
                    Some(each) => each
                        .iter()
                        .enumerate()
                        .map(|(n, (file, caption, url))| (identity_name(&name, n, caption, file), url.clone()))
                        .collect(),
                    None => page.thumbnail.iter().map(|t| (name.clone(), t.source.clone())).collect(),
                };
                for (n, (name, url)) in pictures.into_iter().enumerate() {
                    let id = character_id(&wiki, pageid, n);
                    if out.iter().any(|c| c.id == id) {
                        continue;
                    }
                    // A film doesn't say who leads; another self is kin of the first.
                    let image = Some(url);
                    let kin = (n > 0).then(|| character_id(&wiki, pageid, 0));
                    out.push(Character {
                        id,
                        role: Role::Main,
                        name,
                        native: None,
                        alternative: Vec::new(),
                        image,
                        kin,
                    });
                }
            }
        }
        going(cancel)?;
        self.casts.lock().insert(id, out.clone());
        Ok(out)
    }

    /// The selves of each of `pages` whose infobox shows more than one
    /// ([`identities`]), by page title: `(file, caption, picture's address)`,
    /// those whose picture the wiki has.
    #[allow(clippy::type_complexity)]
    fn selves<'a>(
        &self,
        wiki: &str,
        pages: impl Iterator<Item = &'a str>,
        cancel: &CancelToken,
    ) -> ProjectResult<HashMap<String, Vec<(String, String, String)>>> {
        let titles: Vec<&str> = pages.collect();
        if titles.is_empty() {
            return Ok(HashMap::new());
        }
        let query = format!(
            "action=query&prop=revisions&rvprop=content&rvslots=main&rvsection=0&titles={}",
            query_part(&titles.join("|"))
        );
        let answer: Option<PagesAnswer> = self.request(&Self::wiki_api(wiki, &query), cancel)?;
        let leads: Vec<(String, Vec<(String, String)>)> = answer
            .and_then(|a| a.query)
            .map(|q| q.pages.into_values().filter_map(|p| Some((p.title.clone(), identities(p.lead()?)))).collect())
            .unwrap_or_default();
        let files: Vec<String> =
            leads.iter().flat_map(|(_, each)| each.iter().map(|(f, _)| format!("File:{f}"))).collect();
        let mut urls: HashMap<String, String> = HashMap::new();
        for chunk in files.chunks(PER_QUERY) {
            let query = format!(
                "action=query&prop=imageinfo&iiprop=url&iiurlwidth={PICTURE_WIDTH}&titles={}",
                query_part(&chunk.join("|"))
            );
            let answer: Option<PagesAnswer> = self.request(&Self::wiki_api(wiki, &query), cancel)?;
            let Some(pages) = answer.and_then(|a| a.query) else { continue };
            let moved: HashMap<&str, &str> =
                pages.normalized.iter().map(|m| (m.from.as_str(), m.to.as_str())).collect();
            let by_title: HashMap<&str, &Page> = pages.pages.values().map(|p| (p.title.as_str(), p)).collect();
            for file in chunk {
                let title = moved.get(file.as_str()).copied().unwrap_or(file.as_str());
                let url = by_title.get(title).and_then(|p| p.imageinfo.as_ref()?.first()?.thumburl.clone());
                if let Some(url) = url {
                    urls.insert(file.clone(), url);
                }
            }
        }
        Ok(leads
            .into_iter()
            .filter_map(|(title, each)| {
                let each: Vec<(String, String, String)> = each
                    .into_iter()
                    .filter_map(|(f, c)| Some((f.clone(), c, urls.get(&format!("File:{f}"))?.clone())))
                    .collect();
                (each.len() >= 2).then_some((title, each))
            })
            .collect())
    }

    /// A character's picture, from the cache or loaded into it (`None`: they
    /// have none, or it isn't on Fandom's image server).
    pub fn image(&self, c: &Character, cancel: &CancelToken) -> ProjectResult<Option<PathBuf>> {
        let Some(url) = c.image.as_deref().filter(|u| on_image_server(u)) else { return Ok(None) };
        if let Some(cached) = self.cached(c.id) {
            return Ok(Some(cached));
        }
        let mut reply: Option<Reply> = None;
        for _ in 0..ATTEMPTS {
            self.images.acquire(&*self.clock, cancel)?;
            // The image server serves pages that come from Fandom only.
            let r = self.transport.get_from(url, "https://www.fandom.com/").map_err(unreachable)?;
            if r.status == 429 {
                let wait = r.retry_after.unwrap_or_default().clamp(MIN_PAUSE, MAX_PAUSE);
                self.images.pause(self.clock.now(), wait);
                continue;
            }
            reply = Some(r);
            break;
        }
        let reply = reply.ok_or_else(busy)?;
        if reply.status == 404 {
            return Ok(None);
        }
        if reply.status != 200 {
            return Err(unreachable(format!("picture of character {}: HTTP {}", c.id, reply.status)));
        }
        let Some(ext) = image_ext(url, reply.content_type.as_deref()) else {
            tracing::warn!(character = c.id, "fandom: the picture isn't a picture");
            return Ok(None);
        };
        Ok(Some(store_picture(&self.dir, c.id, ext, &reply.body, cancel)?))
    }

    /// A character's cached picture.
    pub fn cached(&self, id: u32) -> Option<PathBuf> {
        ["png", "jpg", "webp", "gif"].iter().map(|ext| self.dir.join(format!("{id}.{ext}"))).find(|p| p.is_file())
    }
}

fn not_a_film(id: u32) -> ProjectError {
    ProjectError::NotFound(format!("Wikidata has no film Q{id} with a Fandom wiki."))
}

/// A film's search text without the year at its end (`Toy Story 1995` →
/// `Toy Story`); `None` when it ends in none (or is only one).
pub fn without_year(text: &str) -> Option<String> {
    let (head, year) = text.trim().rsplit_once(' ')?;
    let year_like =
        year.len() == 4 && (year.starts_with("19") || year.starts_with("20")) && year.parse::<u32>().is_ok();
    (year_like && !head.trim().is_empty()).then(|| head.trim_end().to_string())
}

/// The film a project titled `title` plainly is, among `found`: one titled
/// so (see [`crate::anilist::pick_series`]), else — for a title of two
/// words or more — the first whose titles, in any language, hold every one
/// of its words (`Miraculous Awakening`: "Miraculous: Ladybug & Cat Noir,
/// The Movie", "Ladybug & Cat Noir: The Awakening").
pub fn pick_film<'a>(title: &str, found: &'a [Media]) -> Option<&'a Media> {
    if let Some(m) = crate::anilist::pick_series(title, found) {
        return Some(m);
    }
    let text = title_key(&crate::anilist::search_text(title));
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() < 2 {
        return None;
    }
    found.iter().find(|m| {
        let have: HashSet<String> = m
            .synonyms
            .iter()
            .chain(m.title.english.iter())
            .flat_map(|t| title_key(t).split(' ').map(str::to_owned).collect::<Vec<_>>())
            .collect();
        words.iter().all(|w| have.contains(*w))
    })
}

impl Names for Fandom {
    fn search(&self, text: &str, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        Fandom::search(self, text, cancel)
    }

    fn media(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Media> {
        Fandom::media(self, id, cancel)
    }

    /// A film is its own chain.
    fn chain(&self, root: u32, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        Ok(vec![Fandom::media(self, root, cancel)?])
    }

    fn cast(&self, ids: &[u32], cancel: &CancelToken) -> ProjectResult<Vec<Character>> {
        let mut out: Vec<Character> = Vec::new();
        for &id in ids {
            for c in self.characters(id, cancel)? {
                if !out.iter().any(|o| o.id == c.id) {
                    out.push(c);
                }
            }
        }
        Ok(out)
    }

    fn image(&self, c: &Character, cancel: &CancelToken) -> ProjectResult<Option<PathBuf>> {
        Fandom::image(self, c, cancel)
    }

    fn portrait(&self, id: u32) -> Option<Vec<f32>> {
        self.portraits.lock().get(&id).cloned()
    }

    fn keep_portrait(&self, id: u32, embedding: &[f32], cancel: &CancelToken) {
        if !cancel.is_cancelled() {
            self.portraits.lock().insert(id, embedding.to_vec());
        }
    }

    fn forget(&self) -> std::io::Result<()> {
        self.films.lock().clear();
        self.casts.lock().clear();
        self.portraits.lock().clear();
        self.wikis.lock().clear();
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use as_api::{AppSettings, ContentKind, NewProject, ProjectKind};
    use as_core::AppPaths;

    use super::*;
    use crate::Library;
    use crate::anilist::Source;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/fandom");

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(FIXTURES).join(name)).unwrap()
    }

    /// Time that only moves when slept.
    #[derive(Default)]
    struct FakeClock {
        now: Mutex<Option<Instant>>,
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock().get_or_insert_with(Instant::now)
        }

        fn sleep(&self, d: Duration, _: &CancelToken) {
            let now = self.now();
            *self.now.lock() = Some(now + d);
        }
    }

    /// A piece of the URL and the reply (status, body).
    type Canned = (String, u16, Vec<u8>);

    /// Recorded answers by a piece of the URL (first match wins), and every
    /// URL asked for.
    #[derive(Default)]
    struct Recorded {
        answers: Mutex<Vec<Canned>>,
        asked: Mutex<Vec<String>>,
        referers: Mutex<Vec<String>>,
        pictures: AtomicUsize,
    }

    impl Recorded {
        fn answer(&self, matching: &str, status: u16, body: Vec<u8>) {
            self.answers.lock().push((matching.into(), status, body));
        }

        fn ladybug(&self) {
            self.answer("wikidata.org/w/api.php?action=query&list=search", 200, fixture("wikidata_search.json"));
            self.answer("ids=Q57958005", 200, fixture("wikidata_film.json"));
            self.answer("ids=Q6470744", 200, fixture("wikidata_related.json"));
            self.answer(
                "miraculousladybug.fandom.com/api.php?action=query&list=search",
                200,
                fixture("fandom_search.json"),
            );
            self.answer("prop=sections", 200, fixture("fandom_sections.json"));
            self.answer("section=6&", 200, fixture("fandom_section_6.json"));
            self.answer("section=12&", 200, fixture("fandom_section_12.json"));
            self.answer("prop=pageimages", 200, fixture("fandom_pictures.json"));
            self.answer("rvsection=0", 200, fixture("fandom_leads.json"));
            self.answer("prop=imageinfo", 200, fixture("fandom_files.json"));
        }
    }

    impl Transport for Recorded {
        fn post(&self, body: &str) -> Result<Reply, String> {
            Err(format!("nothing is posted: {body}"))
        }

        fn get(&self, url: &str) -> Result<Reply, String> {
            self.asked.lock().push(url.to_string());
            let answers = self.answers.lock();
            let (_, status, body) = answers
                .iter()
                .find(|(m, ..)| url.contains(m.as_str()))
                .ok_or_else(|| format!("no answer for {url}"))?;
            let content_type = Some("application/json".into());
            Ok(Reply { status: *status, retry_after: None, remaining: None, content_type, body: body.clone() })
        }

        fn get_from(&self, url: &str, referer: &str) -> Result<Reply, String> {
            self.referers.lock().push(referer.to_string());
            self.pictures.fetch_add(1, Ordering::Relaxed);
            let body = b"\x89PNG".to_vec();
            let content_type = Some("image/png".into());
            assert!(on_image_server(url), "{url}");
            Ok(Reply { status: 200, retry_after: None, remaining: None, content_type, body })
        }
    }

    fn client(dir: &Path) -> (Fandom, Arc<Recorded>) {
        let net = Arc::new(Recorded::default());
        let clock = Arc::new(FakeClock::default());
        let (api, images) = (Arc::new(Limiter::new(API_GAP)), Arc::new(Limiter::new(IMAGE_GAP)));
        (Fandom::with(dir.join("fandom"), net.clone(), clock, api, images), net)
    }

    const LADYBUG: u32 = 57958005;

    #[test]
    fn a_film_is_found_with_the_wiki_of_what_it_is_based_on() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net) = client(dir.path());
        net.ladybug();
        let found = client.search("Miraculous Awakening", &CancelToken::new()).unwrap();
        assert_eq!(found.len(), 1);
        let film = &found[0];
        assert_eq!((film.id, film.name().as_str()), (LADYBUG, "Ladybug & Cat Noir: The Movie"));
        // The series' English wiki named after it: not a dub's wiki, not YouTube's.
        assert_eq!(film.format.as_deref(), Some("Film · miraculousladybug.fandom.com"));
        assert_eq!((film.season_year, film.episodes), (Some(2023), None));
        assert!(film.synonyms.iter().any(|t| t == "Ladybug & Cat Noir: The Awakening"));
        let asked = net.asked.lock().clone();
        assert!(asked[0].contains("srsearch=Miraculous%20Awakening"), "{}", asked[0]);
        assert_eq!(asked.len(), 3, "the search, the film, what it is based on");
        // Picked by the title: every word of it is among the film's titles.
        assert_eq!(pick_film("Miraculous Awakening", &found).map(|m| m.id), Some(LADYBUG));
        assert_eq!(pick_film("Ladybug & Cat Noir: The Movie (2023)", &found).map(|m| m.id), Some(LADYBUG));
        assert_eq!(pick_film("Miraculous", &found), None, "one word picks nothing");
        assert_eq!(pick_film("Miraculous Panda", &found), None);
        // Read once a session; a film is its own chain.
        assert_eq!(Names::chain(&client, LADYBUG, &CancelToken::new()).unwrap(), std::slice::from_ref(film));
        assert_eq!(net.asked.lock().len(), 3);
        assert!(matches!(client.search(" ", &CancelToken::new()), Err(ProjectError::Invalid(_))));
    }

    #[test]
    fn a_films_characters_are_those_its_page_names_with_their_pictures() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net) = client(dir.path());
        net.ladybug();
        let cast = Names::cast(&client, &[LADYBUG], &CancelToken::new()).unwrap();
        let names: Vec<&str> = cast.iter().map(|c| c.name.as_str()).collect();
        // As the page shows them, the film's versions under their own names,
        // and a hero apart from their everyday self.
        let want = ["Marinette Dupain-Cheng", "Ladybug", "Adrien Agreste", "Cat Noir", "Gabriel Agreste", "Hawk Moth"];
        assert_eq!(names[..6], want);
        assert!(names.contains(&"Tom Dupain") && names.contains(&"Sabine Cheng") && names.contains(&"Émilie Agreste"));
        assert!(names.contains(&"Nino Lahiffe") && names.contains(&"Bubbler"));
        assert!(names.iter().all(|n| !n.contains('/')), "{names:?}");
        assert_eq!(cast.len(), 25);
        // Ladybug is Marinette's other self: they don't compete for a group.
        assert_eq!((cast[0].kin, cast[1].kin), (None, Some(cast[0].id)));
        assert!(cast.iter().all(|c| c.role == Role::Main && c.image.as_deref().is_some_and(on_image_server)));
        let mut ids: Vec<u32> = cast.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 25, "one id per self");
        // The characters' and the cast's sections, each with what is under
        // it; nothing under "Differences between the movie and the show".
        let asked = net.asked.lock().clone();
        let sections: Vec<&String> = asked.iter().filter(|u| u.contains("prop=wikitext")).collect();
        assert_eq!(sections.len(), 2, "{sections:?}");
        assert!(sections[0].contains("section=6&") && sections[1].contains("section=12&"));
        let asked_before = asked.len();
        client.characters(LADYBUG, &CancelToken::new()).unwrap();
        assert_eq!(net.asked.lock().len(), asked_before, "read once a session");

        // Pictures come from Fandom's image server, saying they're for Fandom.
        let path = client.image(&cast[0], &CancelToken::new()).unwrap().unwrap();
        assert!(path.starts_with(dir.path().join("fandom")) && path.extension().unwrap() == "png");
        assert_eq!(net.referers.lock().as_slice(), ["https://www.fandom.com/"]);
        assert_eq!(client.image(&cast[0], &CancelToken::new()).unwrap(), Some(path.clone()));
        assert_eq!(net.pictures.load(Ordering::Relaxed), 1);
        for url in ["https://example.com/a.png", "http://static.wikia.nocookie.net/a.png"] {
            let elsewhere = Character { id: 1, image: Some(url.into()), ..cast[0].clone() };
            assert_eq!(client.image(&elsewhere, &CancelToken::new()).unwrap(), None);
        }
        Names::forget(&client).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn a_cast_line_names_the_character_not_the_actor() {
        // Disney's, Kung Fu Panda's, Pixar's, Toy Story's ways of a cast line.
        let cast = "==Cast==\n*[[Stephanie Beatriz]] as [[Mirabel Madrigal]]\n**Noemi Flores as Young Mirabel\n\
                    * {{WP|Jack Black}} as [[Po]] the panda, who meets [[the Chameleon]]\n\
                    *[[Amy Poehler]]: [[Joy]]<ref>[[2013]] interview</ref>\n\
                    *[[Jack Angel]] - [[Rocky Gibraltar]], [[Shifu|Master Shifu]] and [[Tigress]]\n\
                    *[[Tom Hanks]] <!-- [[Woody]] --> narrates\n\
                    *[[Cristina Vee]] as Marinette Dupain-Cheng/Ladybug";
        let links = character_links(cast, Lists::Cast);
        let targets: Vec<&str> = links.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(targets, ["Mirabel Madrigal", "Po", "Joy", "Rocky Gibraltar", "Shifu", "Tigress"]);
        assert_eq!(links[4].1.as_deref(), Some("Master Shifu"));
        // A list of characters: every page a line links to, but no file,
        // category or anything in a reference or template.
        let characters = "* [[Shifu|Master Shifu]] and [[File:Shifu.png|thumb]] [[Category:Masters]] [[Po#Early life|Po]]\n\
                          * {{Main|[[Tai Lung]]}} [[Zhen]]<ref name=\"a\"/> and [[Li]]";
        let links = character_links(characters, Lists::Characters);
        let targets: Vec<&str> = links.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(targets, ["Shifu", "Po", "Zhen", "Li"]);
        let film: HashSet<String> = ["toy", "story", "4"].map(str::to_owned).into();
        assert_eq!(without_film("Dolly (Toy Story)", &film), "Dolly");
        assert_eq!(without_film("Buzz Lightyear (character)", &film), "Buzz Lightyear");
        assert_eq!(without_film("Bo Peep (Lamp)", &film), "Bo Peep (Lamp)", "not the film's words");
        assert_eq!(character_name("''Bonnie''", None), "Bonnie");
        assert_eq!(character_name("Tikki/Movie", None), "Tikki");
        assert_eq!(character_name("Mime (movie)", None), "Mime");
        assert_eq!(character_name("Shifu", Some("Master Shifu")), "Master Shifu");
        assert_eq!(
            character_name("Marinette Dupain-Cheng/Movie", Some("Marinette Dupain-Cheng/Ladybug")),
            "Marinette Dupain-Cheng"
        );
        assert_eq!(character_id("a", 1, 0), character_id("a", 1, 0));
        assert_ne!(character_id("a", 1, 0), character_id("b", 1, 0), "page ids are per wiki");
        assert_ne!(character_id("a", 1, 0), character_id("a", 1, 1), "each self its own");
    }

    #[test]
    fn a_films_page_is_read_where_it_names_its_characters() {
        let section = |index: &str, number: &str, line: &str| Section {
            index: index.into(),
            number: number.into(),
            line: line.into(),
        };
        let sections = [
            section("1", "1", "Plot"),
            section("2", "2", "Cast and crew"),
            section("3", "2.1", "Voice cast"),
            section("4", "3", "Casting"),
            section("5", "4", "Broadcast"),
            section("6", "5", "Character design"),
            section("7", "6", "Characters"),
            section("8", "7", "Differences from the series"),
            section("9", "7.1", "Characters"),
            section("10", "8", "Production crew"),
            section("11", "8.1", "Main cast"),
        ];
        let read: Vec<(&str, Lists)> =
            character_sections(&sections).iter().map(|(s, l)| (s.line.as_str(), *l)).collect();
        assert_eq!(read, [("Cast and crew", Lists::Cast), ("Characters", Lists::Characters)]);
    }

    #[test]
    fn a_film_is_a_film_by_what_it_is_or_says() {
        let entity = |json: &str| serde_json::from_str::<Entity>(json).unwrap();
        let film = entity(r#"{"claims": {"P31": [{"mainsnak": {"datavalue": {"value": {"id": "Q202866"}}}}]}}"#);
        assert!(film.is_film());
        assert!(entity(r#"{"descriptions": {"en": {"value": "2021 film directed by Jared Bush"}}}"#).is_film());
        for other in ["CGI-animated film series by Pixar", "soundtrack album of the 2019 film", "media franchise"] {
            let json = format!(r#"{{"descriptions": {{"en": {{"value": "{other}"}}}}}}"#);
            assert!(!entity(&json).is_film(), "{other}");
        }
        let text = crate::anilist::search_text("Toy.Story.1995.1080p.BluRay");
        assert_eq!(without_year(&text).as_deref(), Some("Toy Story"));
        assert_eq!(
            without_year("Blade Runner 2049").as_deref(),
            Some("Blade Runner"),
            "tried only when that finds nothing"
        );
        assert_eq!((without_year("1917"), without_year("Toy Story 3")), (None, None));
    }

    #[test]
    fn a_wiki_of_films_in_general_is_never_a_films_wiki() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net) = client(dir.path());
        // Toy Story 3: articles in Moviepedia and Disney's wiki; its
        // franchise has a wiki named after it.
        let film = br#"{"entities": {"Q1": {"labels": {"en": {"value": "Toy Story 3"}},
            "descriptions": {"en": {"value": "2010 film by Lee Unkrich"}},
            "claims": {"P8345": [{"mainsnak": {"datavalue": {"value": {"id": "Q2"}}}}],
                       "P6262": [{"mainsnak": {"datavalue": {"value": "movies:Toy_Story_3"}}},
                                 {"mainsnak": {"datavalue": {"value": "disney:Toy_Story_3"}}}]}}}}"#;
        let franchise = br#"{"entities": {"Q2": {"labels": {"en": {"value": "Toy Story"}},
            "claims": {"P6262": [{"mainsnak": {"datavalue": {"value": "toystory:Toy_Story_(franchise)"}}}]}}}}"#;
        net.answer("ids=Q1", 200, film.to_vec());
        net.answer("ids=Q2", 200, franchise.to_vec());
        let media = client.media(1, &CancelToken::new()).unwrap();
        assert_eq!(media.format.as_deref(), Some("Film · toystory.fandom.com"));
        // Without the franchise's wiki: the studio's, never Moviepedia.
        let (client, net) = self::client(dir.path());
        net.answer("ids=Q1", 200, film.to_vec());
        net.answer("ids=Q2", 200, br#"{"entities": {"Q2": {"labels": {"en": {"value": "Toy Story"}}}}}"#.to_vec());
        let media = client.media(1, &CancelToken::new()).unwrap();
        assert_eq!(media.format.as_deref(), Some("Film · disney.fandom.com"));
        assert!(net.asked.lock().iter().all(|u| !u.contains("siteinfo")), "no guessing needed");
        // A wiki that won't answer as a wiki is none, asked once.
        let (client, net) = self::client(dir.path());
        net.answer("siteinfo", 503, b"".to_vec());
        assert!(!client.wiki_exists("down", &CancelToken::new()).unwrap());
        assert!(!client.wiki_exists("down", &CancelToken::new()).unwrap());
        assert_eq!(net.asked.lock().len(), 1);
    }

    #[test]
    fn each_self_an_infobox_shows_is_a_character_of_its_own() {
        let gallery = "{{Icons|Ladybug}}\n{{Infobox character\n|name = Marinette<br />(a.k.a. Ladybug)\n\
                       |image = <gallery>\nMarinette Dupain-Cheng (Movie) Square.png|Marinette\n\
                       File:Ladybug (Movie) Square.png|[[Ladybug]]\n</gallery>\n|only = [[The Movie]]\n}}";
        let selves = identities(gallery);
        let want =
            [("Marinette Dupain-Cheng (Movie) Square.png", "Marinette"), ("Ladybug (Movie) Square.png", "Ladybug")];
        assert_eq!(selves, want.map(|(f, c)| (f.to_string(), c.to_string())));
        assert_eq!(identity_name("Marinette Dupain-Cheng", 0, "Marinette", "x.png"), "Marinette Dupain-Cheng");
        assert_eq!(identity_name("Marinette Dupain-Cheng", 1, "Ladybug", "x.png"), "Ladybug");
        assert_eq!(identity_name("Plagg", 1, "", "Plagg (movie cat form) Square.png"), "Plagg (movie cat form)");
        assert_eq!(identity_name("Raymond", 0, "Gargoyle", "Gargoyle.png"), "Gargoyle", "not the start of the name");
        let tabs = "{{Infobox character|image=<tabber>Adrien=[[File:Adrien.png|200px]]\n|-|Cat Noir=[[File:Cat_Noir.png]]</tabber>\n}}";
        let want = [("Adrien.png", "Adrien"), ("Cat Noir.png", "Cat Noir")];
        assert_eq!(identities(tabs), want.map(|(f, c)| (f.to_string(), c.to_string())));
        // One picture, or a gallery that isn't the infobox's: one self.
        assert!(identities("{{Infobox character\n|image = Tom (Movie) Square.png\n}}").is_empty());
        let later = "{{Infobox character\n|image = Tom.png\n|age = 40\n}}\n<gallery>\nA.png|A\nB.png|B\n</gallery>";
        assert!(identities(later).is_empty());
        assert!(identities("<gallery>\nA.png|A\nB.png|B\n</gallery>").is_empty(), "no infobox");
        let one = "{{Infobox character\n|image = <gallery>\nA.png|A\n</gallery>\n}}";
        assert!(identities(one).is_empty());
    }

    #[test]
    fn without_an_article_the_wiki_named_after_the_franchise_is_tried() {
        assert_eq!(wiki_name("Kung Fu Panda 4").as_deref(), Some("kungfupanda"));
        assert_eq!(wiki_name("Frozen II").as_deref(), Some("frozen"));
        assert_eq!(wiki_name("Ladybug & Cat Noir: The Movie").as_deref(), Some("ladybugcatnoir"));
        assert_eq!(wiki_name("Up").as_deref(), None, "too short to be a wiki's name");
        let dir = tempfile::tempdir().unwrap();
        let (client, net) = client(dir.path());
        let film = br#"{"entities": {"Q1": {"labels": {"en": {"value": "Kung Fu Panda 4"}},
            "descriptions": {"en": {"value": "2024 film"}},
            "claims": {"P179": [{"mainsnak": {"datavalue": {"value": {"id": "Q2"}}}}],
                       "P6262": [{"mainsnak": {"datavalue": {"value": "no.norske-dubber:Kung_Fu_Panda_4"}}}]}}}}"#;
        let series = br#"{"entities": {"Q2": {"labels": {"en": {"value": "Kung Fu Panda"}}, "claims": {}}}}"#;
        net.answer("ids=Q1", 200, film.to_vec());
        net.answer("ids=Q2", 200, series.to_vec());
        net.answer("kungfupanda.fandom.com/api.php?action=query&meta=siteinfo", 200, br#"{"query": {}}"#.to_vec());
        let media = client.media(1, &CancelToken::new()).unwrap();
        assert_eq!(media.format.as_deref(), Some("Film · kungfupanda.fandom.com"));
        // No wiki at all: no film.
        net.answers.lock().retain(|(m, ..)| !m.contains("siteinfo"));
        net.answer("siteinfo", 404, b"".to_vec());
        assert!(matches!(client.media(2, &CancelToken::new()), Err(ProjectError::NotFound(_))), "not a film");
        let (client, net) = self::client(dir.path());
        net.answer("ids=Q1", 200, film.to_vec());
        net.answer("ids=Q2", 200, series.to_vec());
        net.answer("siteinfo", 404, b"".to_vec());
        assert!(matches!(client.media(1, &CancelToken::new()), Err(ProjectError::NotFound(_))));
    }

    #[test]
    fn an_animated_film_project_is_named_from_its_films_wiki() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net) = client(dir.path());
        net.ladybug();
        let lib = Library::open(&AppPaths::at(dir.path().join("data"))).unwrap().with_fandom(client);
        let root = dir.path().join("Library").display().to_string();
        lib.update_settings(&AppSettings { library_root: root, anilist: true, ..lib.settings().unwrap() }).unwrap();
        let new = NewProject {
            title: "Miraculous Awakening".into(),
            kind: ProjectKind::Movie,
            content: ContentKind::Animation,
            files: vec![],
        };
        let id = lib.create_project(&new).unwrap().0.id;
        assert_eq!(lib.name_source(id).unwrap(), Source::Fandom);
        let cancel = lib.anilist_token();
        let film = lib.anilist_find_series(id, &cancel).unwrap().unwrap();
        assert_eq!(
            (film.name.as_str(), film.media.as_slice(), film.picked),
            ("Ladybug & Cat Noir: The Movie", &[LADYBUG][..], true)
        );
        let project = lib.project(id).unwrap();
        assert_eq!(project.db().meta("fandom.media").unwrap().as_deref(), Some("[57958005]"));
        assert_eq!(project.db().meta("tvmaze.media").unwrap(), None);
        let (source, cast) = lib.anilist_cast(id, &cancel).unwrap();
        assert_eq!((source, cast.len()), (Source::Fandom, 25));
        assert!(cast.iter().all(|m| m.image.as_ref().is_some_and(|p| p.starts_with(dir.path().join("fandom")))));
        lib.update_settings(&AppSettings { anilist: false, ..lib.settings().unwrap() }).unwrap();
        assert!(!dir.path().join("fandom").exists(), "switched off: the pictures go");
    }
}
