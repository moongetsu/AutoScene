//! Naming anime characters from AniList (opt-in, library setting `anilist`).
//!
//! While the opt-in is on, the app looks names up by itself once an anime
//! project's characters are found: [`Library::anilist_find_series`] searches
//! the project's title ([`search_text`]) and keeps the series only when its
//! title is the one searched for ([`pick_series`]); the portraits of its
//! cast are matched, and the names wait in [`SUGGESTED`] for the person's
//! yes. By hand, a project goes through [`Library::anilist_search`] (with a
//! search text the person confirmed), [`Library::anilist_confirm`] and
//! [`Library::anilist_cast`]. Every request to the API waits its turn on one
//! process-wide [`Limiter`]: AniList allows 30 a minute (90 when it isn't
//! degraded), and pauses everyone who asks more often.
//!
//! What a project decided lives in its meta, as JSON (a migration would make
//! 1.0.3 refuse the project): [`MEDIA`] the confirmed series and its sequel
//! chain, [`REJECTED`] the series turned down, [`AUTO`] whether names are
//! given without asking, [`PICKED`] whether the app picked the series,
//! [`SUGGESTED`] the names waiting for a yes, [`REFUSED`] the names left out
//! and [`LOOKED`] whether the app looked by itself. Character images are
//! cached for the whole library by AniList character id
//! ([`AppPaths::anilist`]) and deleted with the opt-in.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use as_core::{CancelToken, ProjectId};
use parking_lot::Mutex;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{Library, Project, ProjectError, ProjectResult};

/// Project meta: the confirmed series first, then the rest of its sequel
/// chain (AniList media ids, a JSON array).
pub const MEDIA: &str = "anilist.media";
/// Project meta: series the person said aren't this one (a JSON array).
pub const REJECTED: &str = "anilist.rejected";
/// Project meta: whether names are given without asking (JSON `true`/`false`;
/// unset: only suggested).
pub const AUTO: &str = "anilist.auto";
/// Project meta: the names (and merges) a look-up found, waiting for the
/// person's yes ([`Suggested`]; only good while the cast is as it was).
pub const SUGGESTED: &str = "anilist.suggested";
/// Project meta: the app looked the names up by itself once (`true`): it
/// looks again only when the characters are found or grouped again.
pub const LOOKED: &str = "anilist.looked";
/// Project meta: names the person left out, as `[character, AniList
/// character]` pairs: never suggested for that character again.
pub const REFUSED: &str = "anilist.refused";
/// Project meta: [`MEDIA`] was picked by the app from the title (`true`),
/// not confirmed by the person: names from it are never given unasked.
pub const PICKED: &str = "anilist.picked";

const API: &str = "https://graphql.anilist.co";
/// At least this long between two requests to the API: 24 a minute, under
/// the 30 AniList allows while it is degraded.
pub(crate) const API_GAP: Duration = Duration::from_millis(2500);
/// Between two images from AniList's image server (not the API's limit).
const IMAGE_GAP: Duration = Duration::from_millis(250);
/// The shortest pause after AniList says "too many requests", or says none
/// are left: its limit counts a minute.
pub(crate) const MIN_PAUSE: Duration = Duration::from_secs(60);
/// The longest pause a `Retry-After` can ask for.
const MAX_PAUSE: Duration = Duration::from_secs(15 * 60);
/// How often one request is tried while AniList asks to wait.
const ATTEMPTS: usize = 3;
/// Search results asked for.
const RESULTS: u32 = 10;
/// Characters a page (AniList's most).
const PER_PAGE: u32 = 25;
/// Pages of one role read at most (1000 characters).
const MAX_PAGES: u32 = 40;
/// Seasons, films and specials of one chain followed at most.
const MAX_CHAIN: usize = 16;
/// The biggest answer read from the API, and the biggest image.
const MAX_ANSWER: u64 = 4 << 20;
const MAX_IMAGE: u64 = 8 << 20;

// ---------------------------------------------------------------------------
// Search text
// ---------------------------------------------------------------------------

/// Where a release name stops being a title: an episode or season number,
/// a resolution, a codec, a source or an audio tag.
static RELEASE_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)
        (?:^|\s) - \s* \d{1,4} (?:v\d)? (?:\s|$)
        | \b s\d{1,2} \s* e\d{1,4} \b
        | \b s\d{1,2} \b
        | \b (?:ep?|episode) \s* \d{1,4} \b
        | \b (?:2160p|1080p|720p|576p|480p|4k|uhd|x26[45]|h\.?26[45]|hevc|avc|av1|10-?bit|8-?bit|hdr|hdr10
            |bd|bdrip|blu-?ray|bdremux|remux|web|web-?dl|web-?rip|dvd|dvdrip|tv-?rip|aac|flac|opus|ac3|e-?ac-?3|ddp?5\.1
            |dual[\s-]audio|multi[\s-]?subs?) \b",
    )
    .expect("valid")
});
/// Fansub groups, checksums, years and tags. Not `【…】`: Japanese titles use
/// those (`【推しの子】`).
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]|\([^)]*\)|\{[^}]*\}").expect("valid"));
static VIDEO_EXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(?:mkv|mp4|m4v|avi|mov|webm|ts|m2ts)$").expect("valid"));

/// The search text a project title suggests: without fansub groups, tags in
/// brackets, the episode and the release's tags. The look-up searches it
/// while the opt-in is on; the search by hand only suggests it.
///
/// `[SubsPlease] Sousou no Frieren - 03 (1080p) [A1B2C3D4]` → `Sousou no Frieren`.
pub fn search_text(title: &str) -> String {
    let mut s = VIDEO_EXT.replace(title.trim(), "").replace('_', " ");
    // `Sousou.no.Frieren.S01E03.1080p`: dots for spaces.
    if !s.contains(' ') {
        s = s.replace('.', " ");
    }
    // `[Oshi no Ko] S2`: the title was in brackets.
    [clean(&BRACKETS.replace_all(&s, " ")), clean(&s.replace(['[', ']', '(', ')', '{', '}'], " "))]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| title.trim().to_string())
}

/// `s` up to its release tags, its spaces tidied.
fn clean(s: &str) -> String {
    let s = match RELEASE_TAIL.find(s) {
        Some(m) => &s[..m.start()],
        None => s,
    };
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.trim_matches(|c: char| c.is_whitespace() || "-–—|:.,~+".contains(c)).to_string()
}

/// A year in brackets, as titles carry one (`Hunter x Hunter (2011)`).
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[(\[]((?:19|20)\d{2})[)\]]").expect("valid"));

/// `s` for comparing titles: lower case, letters and digits, one space
/// between words (`Journey's End` → `journeys end`).
fn title_key(s: &str) -> String {
    let s: String = s
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\'' | '’'))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The series a project titled `title` plainly is, among AniList's answers
/// to its [`search_text`]: the first with a title (or synonym) that is the
/// search text itself — of the year the project's title names, when it
/// names one and such a series is among them — else, for a search text of
/// two words or more, the first answer when one of its titles starts with
/// it as whole words (`Sousou no` → `Sousou no Frieren`; one word, `Fate` or
/// `Love`, begins too many). Another season of the right series does no
/// harm: the cast spans the whole sequel chain. `None`: none plainly is.
pub fn pick_series<'a>(title: &str, found: &'a [Media]) -> Option<&'a Media> {
    let text = title_key(&search_text(title));
    if text.is_empty() {
        return None;
    }
    let titles = |m: &Media| -> Vec<String> {
        [&m.title.english, &m.title.romaji, &m.title.native]
            .into_iter()
            .flatten()
            .chain(&m.synonyms)
            .map(|t| title_key(t))
            .collect()
    };
    let exact: Vec<&Media> = found.iter().filter(|m| titles(m).contains(&text)).collect();
    let year = YEAR.captures(title).and_then(|c| c[1].parse::<u32>().ok());
    if let Some(m) = exact.iter().find(|m| year.is_some() && m.season_year == year) {
        return Some(m);
    }
    if let Some(m) = exact.first() {
        return Some(m);
    }
    if !text.contains(' ') {
        return None;
    }
    let first = found.first()?;
    titles(first).iter().any(|t| t.strip_prefix(&text).is_some_and(|rest| rest.starts_with(' '))).then_some(first)
}

// ---------------------------------------------------------------------------
// What AniList answers
// ---------------------------------------------------------------------------

/// A series, season, film or special on AniList.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub id: u32,
    /// Its MyAnimeList id.
    pub id_mal: Option<u32>,
    /// `TV`, `TV_SHORT`, `MOVIE`, `SPECIAL`, `OVA`, `ONA`, …
    pub format: Option<String>,
    pub episodes: Option<u32>,
    pub season_year: Option<u32>,
    pub title: Titles,
    pub synonyms: Vec<String>,
    /// Anime it is a sequel or prequel of (the chain one cast spans).
    pub sequels: Vec<u32>,
}

impl Media {
    /// Its English title, else its romaji or native one.
    pub fn name(&self) -> String {
        let t = &self.title;
        t.english.clone().or_else(|| t.romaji.clone()).or_else(|| t.native.clone()).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Titles {
    pub romaji: Option<String>,
    pub english: Option<String>,
    pub native: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Role {
    Main,
    Supporting,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Main => "MAIN",
            Self::Supporting => "SUPPORTING",
        }
    }
}

/// A character of a series' cast.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Character {
    pub id: u32,
    /// The biggest part they have anywhere in the chain.
    pub role: Role,
    /// Their full name.
    pub name: String,
    pub native: Option<String>,
    pub alternative: Vec<String>,
    /// AniList's large image of them.
    pub image: Option<String>,
}

/// A character with their image in the library's cache (`None`: AniList has
/// none, or it couldn't be loaded).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CastMember {
    pub character: Character,
    pub image: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMedia {
    id: u32,
    id_mal: Option<u32>,
    format: Option<String>,
    episodes: Option<u32>,
    season_year: Option<u32>,
    #[serde(default)]
    title: Titles,
    #[serde(default)]
    synonyms: Vec<Option<String>>,
    relations: Option<RawRelations>,
}

#[derive(Deserialize)]
struct RawRelations {
    #[serde(default)]
    edges: Vec<RawRelation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRelation {
    relation_type: Option<String>,
    node: Option<RawNode>,
}

#[derive(Deserialize)]
struct RawNode {
    id: u32,
    #[serde(rename = "type")]
    kind: Option<String>,
    format: Option<String>,
}

impl From<RawMedia> for Media {
    fn from(m: RawMedia) -> Self {
        let sequels = m
            .relations
            .map(|r| r.edges)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| matches!(e.relation_type.as_deref(), Some("SEQUEL" | "PREQUEL")))
            .filter_map(|e| e.node)
            .filter(|n| n.kind.as_deref() == Some("ANIME") && n.format.as_deref() != Some("MUSIC"))
            .map(|n| n.id)
            .collect();
        Self {
            id: m.id,
            id_mal: m.id_mal,
            format: m.format,
            episodes: m.episodes,
            season_year: m.season_year,
            title: m.title,
            synonyms: m.synonyms.into_iter().flatten().filter(|s| !s.trim().is_empty()).collect(),
            sequels,
        }
    }
}

#[derive(Deserialize)]
struct SearchData {
    #[serde(rename = "Page")]
    page: SearchPage,
}

#[derive(Deserialize)]
struct SearchPage {
    #[serde(default)]
    media: Vec<Option<RawMedia>>,
}

#[derive(Deserialize)]
struct MediaData {
    #[serde(rename = "Media")]
    media: Option<RawMedia>,
}

#[derive(Deserialize)]
struct CastData {
    #[serde(rename = "Media")]
    media: Option<CastMedia>,
}

#[derive(Deserialize)]
struct CastMedia {
    characters: Option<CastPage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CastPage {
    page_info: Option<PageInfo>,
    #[serde(default)]
    edges: Vec<Option<CastEdge>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    #[serde(default)]
    has_next_page: bool,
}

#[derive(Deserialize)]
struct CastEdge {
    role: Option<String>,
    node: Option<RawCharacter>,
}

#[derive(Deserialize)]
struct RawCharacter {
    id: u32,
    name: Option<RawName>,
    image: Option<RawImage>,
}

#[derive(Deserialize)]
struct RawName {
    full: Option<String>,
    native: Option<String>,
    #[serde(default)]
    alternative: Vec<Option<String>>,
}

#[derive(Deserialize)]
struct RawImage {
    large: Option<String>,
}

/// An answer: its `data`, or the `errors` that came instead.
#[derive(Deserialize)]
struct Answer<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<AnswerError>,
}

#[derive(Deserialize)]
struct AnswerError {
    message: Option<String>,
}

const MEDIA_FIELDS: &str = "id idMal format episodes seasonYear title { romaji english native } synonyms \
    relations { edges { relationType(version: 2) node { id type format } } }";

fn search_query() -> String {
    format!(
        "query ($search: String) {{ Page(page: 1, perPage: {RESULTS}) {{ media(search: $search, type: ANIME) {{ {MEDIA_FIELDS} }} }} }}"
    )
}

fn media_query() -> String {
    format!("query ($id: Int) {{ Media(id: $id, type: ANIME) {{ {MEDIA_FIELDS} }} }}")
}

fn cast_query() -> String {
    format!(
        "query ($id: Int, $role: CharacterRole, $page: Int) {{ Media(id: $id, type: ANIME) {{ id \
         characters(role: $role, page: $page, perPage: {PER_PAGE}, sort: [ROLE, RELEVANCE, ID]) {{ \
         pageInfo {{ hasNextPage }} edges {{ role node {{ id name {{ full native alternative }} image {{ large }} }} }} }} }} }}"
    )
}

// ---------------------------------------------------------------------------
// Rate limit
// ---------------------------------------------------------------------------

/// Time, so the limiter can be tested without waiting.
pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    /// Wait `d`, or less once `cancel` is cancelled.
    fn sleep(&self, d: Duration, cancel: &CancelToken);
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, d: Duration, cancel: &CancelToken) {
        // In slices: a cancel never waits out a minute's pause.
        let until = Instant::now() + d;
        while !cancel.is_cancelled() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            std::thread::sleep(left.min(Duration::from_millis(100)));
        }
    }
}

/// Requests one at a time, at least `gap` apart, none while AniList asked
/// to wait.
pub(crate) struct Limiter {
    gap: Duration,
    state: Mutex<LimiterState>,
}

#[derive(Default)]
struct LimiterState {
    /// The next request goes no earlier.
    next: Option<Instant>,
    /// AniList asked to wait until then.
    paused: Option<Instant>,
}

impl Limiter {
    pub(crate) fn new(gap: Duration) -> Self {
        Self { gap, state: Mutex::default() }
    }

    /// Wait for this request's turn (`Cancelled` once cancelled).
    pub(crate) fn acquire(&self, clock: &dyn Clock, cancel: &CancelToken) -> ProjectResult<()> {
        loop {
            if cancel.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let now = clock.now();
            let wait = {
                let mut s = self.state.lock();
                match s.next.max(s.paused).filter(|&at| at > now) {
                    Some(at) => at - now,
                    None => {
                        s.next = Some(now + self.gap);
                        return Ok(());
                    }
                }
            };
            // Another request may take the turn first: then this one waits again.
            clock.sleep(wait, cancel);
        }
    }

    /// No request for `d` from `now` (a longer pause already asked for stays).
    pub(crate) fn pause(&self, now: Instant, d: Duration) {
        let until = now + d;
        let mut s = self.state.lock();
        s.paused = s.paused.max(Some(until));
    }
}

/// The one limiter every request to the API waits on, in the whole process.
static API_LIMITER: LazyLock<Arc<Limiter>> = LazyLock::new(|| Arc::new(Limiter::new(API_GAP)));
static IMAGE_LIMITER: LazyLock<Arc<Limiter>> = LazyLock::new(|| Arc::new(Limiter::new(IMAGE_GAP)));

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// What came back for a request.
pub(crate) struct Reply {
    pub status: u16,
    /// `Retry-After`, in seconds.
    pub retry_after: Option<Duration>,
    /// `X-RateLimit-Remaining`.
    pub remaining: Option<u32>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// The network, so recorded answers can stand in for it.
pub(crate) trait Transport: Send + Sync {
    /// POST `body` (JSON) to the API.
    fn post(&self, body: &str) -> Result<Reply, String>;
    /// GET an image.
    fn get(&self, url: &str) -> Result<Reply, String>;
}

struct Ureq(ureq::Agent);

impl Ureq {
    fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            // A 429 is read like any answer: its headers say how long to wait.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("AutoScene/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self(agent)
    }

    fn reply(mut resp: ureq::http::Response<ureq::Body>, limit: u64) -> Result<Reply, String> {
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(|v| v.trim().to_string());
        let retry_after = header("retry-after").and_then(|v| v.parse::<u64>().ok()).map(Duration::from_secs);
        let remaining = header("x-ratelimit-remaining").and_then(|v| v.parse::<u32>().ok());
        let content_type = header("content-type");
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())?;
        Ok(Reply { status, retry_after, remaining, content_type, body })
    }
}

impl Transport for Ureq {
    fn post(&self, body: &str) -> Result<Reply, String> {
        let resp = self
            .0
            .post(API)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(body)
            .map_err(|e| e.to_string())?;
        Self::reply(resp, MAX_ANSWER)
    }

    fn get(&self, url: &str) -> Result<Reply, String> {
        Self::reply(self.0.get(url).call().map_err(|e| e.to_string())?, MAX_IMAGE)
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// AniList for one library: its requests, the media it has seen, and the
/// image cache.
pub struct AniList {
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    api: Arc<Limiter>,
    images: Arc<Limiter>,
    /// The library's character images (`<id>.<ext>`).
    dir: PathBuf,
    /// Media this session has read, by id: a confirmed series' chain often
    /// needs no request at all after its search.
    seen: Mutex<HashMap<u32, Media>>,
    /// Each anime's cast this session has read, by id: the look-up after
    /// every episode's analysis needs no request for it again.
    casts: Mutex<HashMap<u32, Vec<Character>>>,
    /// Character portraits embedded this session, by AniList character id
    /// (the engine is shared with the analysis: a look-up after every
    /// episode embeds only who is new).
    portraits: Mutex<HashMap<u32, Vec<f32>>>,
}

fn unreachable(e: impl std::fmt::Display) -> ProjectError {
    tracing::warn!("anilist: {e}");
    ProjectError::NotReady("AniList can't be reached right now. Check the internet connection and try again.".into())
}

fn busy() -> ProjectError {
    ProjectError::NotReady("AniList is busy. Try again in a few minutes.".into())
}

/// `Cancelled` once `cancel` is (the opt-in went off, say).
fn going(cancel: &CancelToken) -> ProjectResult<()> {
    if cancel.is_cancelled() { Err(ProjectError::Cancelled) } else { Ok(()) }
}

impl AniList {
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
            seen: Mutex::default(),
            casts: Mutex::default(),
            portraits: Mutex::default(),
        }
    }

    /// Where a character's image is cached, once loaded.
    pub fn image_dir(&self) -> &Path {
        &self.dir
    }

    /// Forget everything: the media seen and every cached image.
    pub(crate) fn forget(&self) -> std::io::Result<()> {
        self.seen.lock().clear();
        self.casts.lock().clear();
        self.portraits.lock().clear();
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// One request to the API, its turn waited for, tried again while
    /// AniList asks to wait.
    fn request<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
        cancel: &CancelToken,
    ) -> ProjectResult<T> {
        let body = serde_json::json!({ "query": query, "variables": variables }).to_string();
        for _ in 0..ATTEMPTS {
            self.api.acquire(&*self.clock, cancel)?;
            let reply = self.transport.post(&body).map_err(unreachable)?;
            if reply.remaining == Some(0) {
                self.api.pause(self.clock.now(), MIN_PAUSE);
            }
            if reply.status == 429 {
                let wait = reply.retry_after.unwrap_or_default().clamp(MIN_PAUSE, MAX_PAUSE);
                tracing::info!(wait_s = wait.as_secs(), "anilist: too many requests, pausing");
                self.api.pause(self.clock.now(), wait);
                continue;
            }
            return answer(reply);
        }
        Err(busy())
    }

    /// Anime matching `text`, best first. `text` is sent to AniList as it is.
    pub fn search(&self, text: &str, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ProjectError::Invalid("Type the name of the series to look for.".into()));
        }
        let data: SearchData = self.request(&search_query(), serde_json::json!({ "search": text }), cancel)?;
        let found: Vec<Media> = data.page.media.into_iter().flatten().map(Media::from).collect();
        going(cancel)?;
        self.seen.lock().extend(found.iter().map(|m| (m.id, m.clone())));
        Ok(found)
    }

    /// Anime `id` (read once a session).
    pub fn media(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Media> {
        if let Some(m) = self.seen.lock().get(&id) {
            return Ok(m.clone());
        }
        let data: MediaData = self.request(&media_query(), serde_json::json!({ "id": id }), cancel)?;
        let media = Media::from(data.media.ok_or_else(|| not_on_anilist(id))?);
        going(cancel)?;
        self.seen.lock().insert(id, media.clone());
        Ok(media)
    }

    /// `root` and every season, film or special its sequels and prequels
    /// lead to: `root` first, the rest by year.
    pub fn chain(&self, root: u32, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        let mut out = Vec::new();
        let mut queue = VecDeque::from([root]);
        let mut queued = HashSet::from([root]);
        while let Some(id) = queue.pop_front() {
            if out.len() == MAX_CHAIN {
                break;
            }
            let media = self.media(id, cancel)?;
            queue.extend(media.sequels.iter().copied().filter(|&n| queued.insert(n)));
            out.push(media);
        }
        out[1..].sort_by_key(|m| (m.season_year.unwrap_or(u32::MAX), m.id));
        Ok(out)
    }

    /// The main and supporting cast of anime `id`, page by page until AniList
    /// says there are no more: main first, in AniList's order.
    pub fn characters(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Vec<Character>> {
        if let Some(cast) = self.casts.lock().get(&id) {
            return Ok(cast.clone());
        }
        let query = cast_query();
        let mut out = Vec::new();
        for role in [Role::Main, Role::Supporting] {
            for page in 1..=MAX_PAGES {
                let vars = serde_json::json!({ "id": id, "page": page, "role": role.name() });
                let data: CastData = self.request(&query, vars, cancel)?;
                let cast = data.media.ok_or_else(|| not_on_anilist(id))?.characters;
                let Some(cast) = cast else { break };
                out.extend(cast.edges.into_iter().flatten().filter_map(|e| character(e, role)));
                if !cast.page_info.is_some_and(|p| p.has_next_page) {
                    break;
                }
            }
        }
        // Switched off meanwhile: nothing is kept.
        going(cancel)?;
        self.casts.lock().insert(id, out.clone());
        Ok(out)
    }

    /// The cast of every anime in `ids`, each character once, with their
    /// biggest part: main first.
    pub fn cast(&self, ids: &[u32], cancel: &CancelToken) -> ProjectResult<Vec<Character>> {
        let mut out: Vec<Character> = Vec::new();
        let mut at: HashMap<u32, usize> = HashMap::new();
        for &id in ids {
            for c in self.characters(id, cancel)? {
                match at.get(&c.id) {
                    Some(&i) => out[i].role = out[i].role.min(c.role),
                    None => {
                        at.insert(c.id, out.len());
                        out.push(c);
                    }
                }
            }
        }
        out.sort_by_key(|c| c.role);
        Ok(out)
    }

    /// A character's image, from the cache or loaded into it (`None`: they
    /// have none, or it isn't on AniList's image server).
    pub fn image(&self, c: &Character, cancel: &CancelToken) -> ProjectResult<Option<PathBuf>> {
        let Some(url) = c.image.as_deref().filter(|u| on_image_server(u)) else { return Ok(None) };
        if let Some(cached) = self.cached(c.id) {
            return Ok(Some(cached));
        }
        let mut reply = None;
        for _ in 0..ATTEMPTS {
            self.images.acquire(&*self.clock, cancel)?;
            let r = self.transport.get(url).map_err(unreachable)?;
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
            return Err(unreachable(format!("image of character {}: HTTP {}", c.id, reply.status)));
        }
        let Some(ext) = image_ext(url, reply.content_type.as_deref()) else {
            tracing::warn!(character = c.id, "anilist: the image isn't a picture");
            return Ok(None);
        };
        going(cancel)?;
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{}.{ext}", c.id));
        let part = self.dir.join(format!("{}.{ext}.part", c.id));
        std::fs::write(&part, &reply.body)?;
        std::fs::rename(&part, &path)?;
        Ok(Some(path))
    }

    /// A character's cached image.
    pub fn cached(&self, id: u32) -> Option<PathBuf> {
        ["png", "jpg", "webp", "gif"].iter().map(|ext| self.dir.join(format!("{id}.{ext}"))).find(|p| p.is_file())
    }

    /// Character `id`'s portrait as embedded this session.
    pub fn portrait(&self, id: u32) -> Option<Vec<f32>> {
        self.portraits.lock().get(&id).cloned()
    }

    /// Keep character `id`'s embedded portrait for this session (not once
    /// `cancel` is: the opt-in went off).
    pub fn keep_portrait(&self, id: u32, embedding: &[f32], cancel: &CancelToken) {
        if !cancel.is_cancelled() {
            self.portraits.lock().insert(id, embedding.to_vec());
        }
    }
}

fn not_on_anilist(id: u32) -> ProjectError {
    ProjectError::NotFound(format!("AniList has no anime {id}."))
}

/// The `data` of an answer, or why there is none.
fn answer<T: DeserializeOwned>(reply: Reply) -> ProjectResult<T> {
    let parsed: Result<Answer<T>, _> = serde_json::from_slice(&reply.body);
    let messages = |a: &Answer<T>| a.errors.iter().filter_map(|e| e.message.as_deref()).collect::<Vec<_>>().join("; ");
    match parsed {
        Ok(Answer { data: Some(data), ref errors }) if reply.status == 200 && errors.is_empty() => Ok(data),
        Ok(a) if reply.status == 404 => Err(ProjectError::NotFound(format!("AniList: {}", messages(&a)))),
        Ok(a) if reply.status >= 500 => Err(unreachable(format!("HTTP {}: {}", reply.status, messages(&a)))),
        Ok(a) => {
            tracing::warn!(status = reply.status, "anilist: {}", messages(&a));
            Err(ProjectError::Invalid(format!("AniList didn't take the request: {}", messages(&a))))
        }
        // Not AniList's answer (a proxy's page, a cut-off body).
        Err(e) => Err(unreachable(format!("HTTP {}: {e}", reply.status))),
    }
}

fn character(e: CastEdge, asked: Role) -> Option<Character> {
    // Only what was asked for: a page of one role holds only that role.
    let role = match e.role.as_deref() {
        Some("MAIN") => Role::Main,
        Some("SUPPORTING") => Role::Supporting,
        _ => return None,
    };
    if role != asked {
        return None;
    }
    let node = e.node?;
    let name = node.name.unwrap_or(RawName { full: None, native: None, alternative: Vec::new() });
    let nonempty = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let native = nonempty(name.native);
    let full = nonempty(name.full).or_else(|| native.clone())?;
    Some(Character {
        id: node.id,
        role,
        name: full,
        native,
        alternative: name.alternative.into_iter().filter_map(nonempty).collect(),
        image: nonempty(node.image.and_then(|i| i.large)),
    })
}

/// Only AniList's own image server is asked for images, over https.
fn on_image_server(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    host == "anilist.co" || host.ends_with(".anilist.co")
}

fn image_ext(url: &str, content_type: Option<&str>) -> Option<&'static str> {
    let by_type = match content_type.map(|t| t.split(';').next().unwrap_or_default().trim().to_ascii_lowercase()) {
        Some(t) if t == "image/png" => Some("png"),
        Some(t) if t == "image/jpeg" || t == "image/jpg" => Some("jpg"),
        Some(t) if t == "image/webp" => Some("webp"),
        Some(t) if t == "image/gif" => Some("gif"),
        Some(t) if !t.starts_with("image/") && t != "application/octet-stream" => return None,
        _ => None,
    };
    by_type.or_else(|| {
        let path = url.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
        match path.rsplit('.').next() {
            Some("png") => Some("png"),
            Some("jpg" | "jpeg") => Some("jpg"),
            Some("webp") => Some("webp"),
            Some("gif") => Some("gif"),
            _ => None,
        }
    })
}

// ---------------------------------------------------------------------------
// A project's decisions
// ---------------------------------------------------------------------------

impl Project {
    fn anilist_ids(&self, key: &str) -> ProjectResult<Vec<u32>> {
        Ok(self.db().meta(key)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
    }

    fn set_anilist_ids(&self, key: &str, ids: &[u32]) -> ProjectResult<()> {
        if ids.is_empty() {
            return Ok(self.db().delete_meta(key)?);
        }
        Ok(self.db().set_meta(key, &serde_json::to_string(ids).expect("ids serialize"))?)
    }

    /// The confirmed series first, then the rest of its chain (empty: none
    /// confirmed).
    pub fn anilist_media(&self) -> ProjectResult<Vec<u32>> {
        self.anilist_ids(MEDIA)
    }

    /// Series the person said aren't this one.
    pub fn anilist_rejected(&self) -> ProjectResult<Vec<u32>> {
        self.anilist_ids(REJECTED)
    }

    /// Whether names from AniList are given without asking (off: only
    /// suggested).
    pub fn anilist_auto(&self) -> ProjectResult<bool> {
        Ok(self.db().meta(AUTO)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or(false))
    }

    pub fn set_anilist_auto(&self, on: bool) -> ProjectResult<()> {
        Ok(self.db().set_meta(AUTO, if on { "true" } else { "false" })?)
    }

    /// Keep `chain` (the confirmed series first) as this project's series,
    /// confirmed by the person or `picked` by the app from the title; none
    /// of it stays rejected.
    pub fn confirm_anilist_media(&self, chain: &[u32], picked: bool) -> ProjectResult<()> {
        let rejected: Vec<u32> = self.anilist_rejected()?.into_iter().filter(|id| !chain.contains(id)).collect();
        self.set_anilist_ids(REJECTED, &rejected)?;
        // Names found in another series' cast don't wait any longer.
        if self.anilist_media()? != chain {
            self.set_anilist_suggested(None)?;
        }
        if picked {
            self.db().set_meta(PICKED, "true")?;
        } else {
            self.db().delete_meta(PICKED)?;
        }
        self.set_anilist_ids(MEDIA, chain)
    }

    /// Whether the series was picked by the app from the title, not
    /// confirmed by the person.
    pub fn anilist_picked(&self) -> ProjectResult<bool> {
        Ok(self.db().meta(PICKED)?.as_deref() == Some("true"))
    }

    /// Say series `id` isn't this one (and forget it if it was confirmed).
    pub fn reject_anilist_media(&self, id: u32) -> ProjectResult<()> {
        let mut rejected = self.anilist_rejected()?;
        if !rejected.contains(&id) {
            rejected.push(id);
        }
        self.set_anilist_ids(REJECTED, &rejected)?;
        if self.anilist_media()?.contains(&id) {
            self.set_anilist_ids(MEDIA, &[])?;
            self.db().delete_meta(PICKED)?;
            self.set_anilist_suggested(None)?;
        }
        Ok(())
    }

    /// The names a look-up found, still waiting for the person's yes: those
    /// for characters still shown and unnamed (and not left out before), the
    /// merges of characters both still shown. `None` without any, or once
    /// another series is confirmed.
    pub fn anilist_suggested(&self) -> ProjectResult<Option<Suggested>> {
        let Some(mut s) = self.db().meta(SUGGESTED)?.and_then(|v| serde_json::from_str::<Suggested>(&v).ok()) else {
            return Ok(None);
        };
        if s.media != self.anilist_media()? {
            return Ok(None);
        }
        let characters = self.db().characters()?;
        let shown = |id: as_core::CharacterId| characters.iter().find(|c| c.id == id && !c.hidden);
        let refused = self.anilist_refused()?;
        s.names.retain(|n| {
            shown(n.character_id).is_some_and(|c| c.name.is_none())
                && !refused.contains(&(n.character_id, n.anilist_id))
        });
        s.merges.retain(|m| {
            m.from != m.into
                && shown(m.from).is_some()
                && shown(m.into).is_some()
                && !refused.contains(&(m.from, m.anilist_id))
                && !refused.contains(&(m.into, m.anilist_id))
        });
        Ok((!s.names.is_empty() || !s.merges.is_empty()).then_some(s))
    }

    /// Keep `s` waiting for the person's yes (`None`: nothing waits).
    pub fn set_anilist_suggested(&self, s: Option<&Suggested>) -> ProjectResult<()> {
        match s {
            Some(s) => Ok(self.db().set_meta(SUGGESTED, &serde_json::to_string(s).expect("suggestions serialize"))?),
            None => Ok(self.db().delete_meta(SUGGESTED)?),
        }
    }

    /// Whether the app looked the names up by itself.
    pub fn anilist_looked(&self) -> ProjectResult<bool> {
        Ok(self.db().meta(LOOKED)?.as_deref() == Some("true"))
    }

    pub fn set_anilist_looked(&self) -> ProjectResult<()> {
        Ok(self.db().set_meta(LOOKED, "true")?)
    }

    /// Names the person left out: `(character, AniList character)`.
    pub fn anilist_refused(&self) -> ProjectResult<Vec<(as_core::CharacterId, u32)>> {
        Ok(self.db().meta(REFUSED)?.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default())
    }

    /// Give the waiting names and merges ([`Self::anilist_suggested`]) the
    /// person saw and kept: names `give` and merges `merge`, each only while it
    /// still waits as it was shown (a look-up that ran since gives nothing
    /// the person didn't see). Those `left_out` (a name, or a merge as its
    /// `from` and portrait) are never suggested again. Merges first, then
    /// names; returns the merges and the names given, and nothing waits
    /// afterwards.
    pub fn accept_anilist_suggested(
        &self,
        give: &[(as_core::CharacterId, u32)],
        merge: &[(as_core::CharacterId, as_core::CharacterId)],
        left_out: &[(as_core::CharacterId, u32)],
    ) -> ProjectResult<(usize, usize)> {
        let Some(s) = self.anilist_suggested()? else {
            return Err(ProjectError::NotReady("These names no longer wait: the series or the cast changed.".into()));
        };
        let mut refused = self.anilist_refused()?;
        let waiting = |pair: &(as_core::CharacterId, u32)| {
            s.names.iter().any(|n| (n.character_id, n.anilist_id) == *pair)
                || s.merges.iter().any(|m| (m.from, m.anilist_id) == *pair)
        };
        for pair in left_out.iter().filter(|p| waiting(p)) {
            if !refused.contains(pair) {
                refused.push(*pair);
            }
        }
        self.db().set_meta(REFUSED, &serde_json::to_string(&refused).expect("pairs serialize"))?;
        let mut merged = 0;
        for m in s.merges.iter().filter(|m| merge.contains(&(m.from, m.into))) {
            self.cast_edit(&as_api::CastEdit::Merge { from: m.from, into: m.into })?;
            merged += 1;
        }
        let mut named = 0;
        for n in s.names.iter().filter(|n| give.contains(&(n.character_id, n.anilist_id))) {
            if matches!(self.db().character_exists(n.character_id), Ok(true)) {
                self.cast_edit(&as_api::CastEdit::Rename { character_id: n.character_id, name: Some(n.name.clone()) })?;
                named += 1;
            }
        }
        self.set_anilist_suggested(None)?;
        Ok((merged, named))
    }
}

// ---------------------------------------------------------------------------
// Matching AniList portraits to the cast
// ---------------------------------------------------------------------------

/// A portrait scoring at least this against a character's look (the
/// quality-weighted mean of its own faces) may suggest the name. CCIP reads
/// hair, eyes and colours first, so portrait art lands high on everyone who
/// shares them: the bar only keeps out the plainly unlike, [`LOOK_ALIKES`]
/// decides. Provisional until E5's bench tunes it.
const SUGGEST_MIN_COS: f32 = 0.70;

/// Portraits within this of the best one are the character's look-alikes
/// (a father, a brother, a younger self score as high as the person
/// themself): of those, only one with a bigger part than all the others
/// names the character, and none does when two share the biggest part.
/// Provisional until E5's bench tunes it.
const LOOK_ALIKES: f32 = 0.08;

/// Two cast characters whose best portrait is the same are proposed for
/// merging only when their looks are this alike too; otherwise the weaker
/// is left unnamed (a portrait names one character).
const MERGE_MIN_COS: f32 = 0.80;

/// AniList says this character's name: the cast character, the match score
/// (cosine of the portrait to the character's look) and the portrait file
/// it came from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NameSuggestion {
    pub character_id: as_core::CharacterId,
    pub anilist_id: u32,
    pub name: String,
    pub score: f32,
    pub image: Option<PathBuf>,
}

/// Two cast characters matched the same AniList one: `from` merges `into`
/// the other (the stronger match keeps its id).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MergeProposal {
    pub from: as_core::CharacterId,
    pub into: as_core::CharacterId,
    pub anilist_id: u32,
}

/// A project's series on AniList ([`Library::anilist_find_series`]): its
/// title, its chain (the series first), and whether the app picked it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Series {
    pub name: String,
    pub media: Vec<u32>,
    pub picked: bool,
}

/// What a look-up found for a project's cast, waiting for the person's yes
/// ([`SUGGESTED`]): from the cast of series `series` (the chain `media`, the
/// series first, as confirmed when it was made).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Suggested {
    pub media: Vec<u32>,
    pub series: String,
    pub names: Vec<NameSuggestion>,
    pub merges: Vec<MergeProposal>,
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::NEG_INFINITY;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The portrait that names a character whose look scores `scores` against
/// the portraits of `roles` (`strongest`: each portrait's best score over
/// every character): the best one at the bar, or among the look-alikes
/// within [`LOOK_ALIKES`] of it the one with the biggest part when no
/// other shares it and no other character matches it better. When another
/// does, this one is either a look-alike of that part (a father scoring
/// high on his son's portrait) or a split of the same person: it is named
/// only when its own best portrait is the one look-alike left. `None` when
/// nothing is close enough or the look-alikes can't be told apart.
fn best_portrait(scores: &[f32], roles: &[Role], strongest: &[f32]) -> Option<(usize, f32)> {
    let (best, top) =
        scores.iter().copied().enumerate().filter(|(_, s)| s.is_finite()).max_by(|a, b| a.1.total_cmp(&b.1))?;
    if top < SUGGEST_MIN_COS {
        return None;
    }
    let close: Vec<usize> = (0..scores.len()).filter(|&i| scores[i] >= top - LOOK_ALIKES).collect();
    if close.len() == 1 {
        return Some((best, top));
    }
    let part = close.iter().map(|&i| roles[i]).min()?;
    let [i] = close.iter().copied().filter(|&i| roles[i] == part).collect::<Vec<_>>()[..] else {
        return None;
    };
    if scores[i] >= strongest[i] {
        return (scores[i] >= SUGGEST_MIN_COS).then_some((i, scores[i]));
    }
    match close.iter().copied().filter(|&j| j != i).collect::<Vec<_>>()[..] {
        [j] if j == best => Some((best, top)),
        _ => None,
    }
}

/// A cast character's match: the portrait that names it, its score and the
/// character's look.
struct Match {
    character: as_core::CharacterId,
    portrait: usize,
    score: f32,
    look: Vec<f32>,
}

/// The proposals the characters' matches make: each portrait names at most
/// one character — the strongest match; another one matching it is
/// proposed for merging into that one when their looks agree.
fn proposals(portraits: &[(CastMember, Vec<f32>)], matches: Vec<Match>) -> (Vec<NameSuggestion>, Vec<MergeProposal>) {
    let mut by_portrait: HashMap<usize, Vec<Match>> = HashMap::new();
    for m in matches {
        by_portrait.entry(m.portrait).or_default().push(m);
    }
    let mut names = Vec::new();
    let mut merges = Vec::new();
    for (p, mut hit) in by_portrait {
        hit.sort_by(|a, b| b.score.total_cmp(&a.score));
        let named = &hit[0];
        let (member, _) = &portraits[p];
        names.push(NameSuggestion {
            character_id: named.character,
            anilist_id: member.character.id,
            name: member.character.name.clone(),
            score: named.score,
            image: member.image.clone(),
        });
        for other in &hit[1..] {
            if cosine(&other.look, &named.look) >= MERGE_MIN_COS {
                merges.push(MergeProposal {
                    from: other.character,
                    into: named.character,
                    anilist_id: member.character.id,
                });
            }
        }
    }
    names.sort_by(|a, b| b.score.total_cmp(&a.score));
    merges.sort_by_key(|m| m.anilist_id);
    (names, merges)
}

/// A character's look: the quality-weighted mean of its `dim`-long face
/// embeddings (the portraits' length), unit length (`None` without one).
fn look(faces: &[as_db::OwnFace], dim: usize) -> Option<Vec<f32>> {
    let mut sum = vec![0.0f32; dim];
    for f in faces {
        let Some(e) = as_analysis::tracks::embedding_from_bytes(&f.emb).filter(|e| e.len() == dim) else { continue };
        let w = if f.quality.is_finite() { f.quality.max(1e-3) } else { 1e-3 };
        sum.iter_mut().zip(&e).for_each(|(s, x)| *s += w * x);
    }
    let n = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
    (n > 0.0 && n.is_finite()).then(|| sum.into_iter().map(|x| x / n).collect())
}

impl Project {
    /// Match AniList portraits (already embedded — the engine embeds, this
    /// matches) to this project's cast: one name suggestion per portrait,
    /// for the character whose look matches it best, and a merge proposal
    /// when a second character with the same look lands on it too. Returns
    /// the cast revision the match was made on:
    /// [`Self::apply_anilist_suggestions`] only applies it while that holds.
    pub fn anilist_suggest(
        &self,
        portraits: &[(CastMember, Vec<f32>)],
    ) -> ProjectResult<(i64, Vec<NameSuggestion>, Vec<MergeProposal>)> {
        let rev = self.db().cast_rev()?;
        let roles: Vec<Role> = portraits.iter().map(|(m, _)| m.character.role).collect();
        let Some(dim) = portraits.first().map(|(_, emb)| emb.len()) else {
            return Ok((rev, Vec::new(), Vec::new()));
        };
        // Every unnamed, shown character's look and its score on each portrait.
        let mut cast = Vec::new();
        for c in self.db().characters()? {
            if c.hidden || c.name.is_some() {
                continue;
            }
            let Some(look) = look(&self.db().own_faces(c.id)?, dim) else { continue };
            let scores: Vec<f32> = portraits.iter().map(|(_, emb)| cosine(&look, emb)).collect();
            cast.push((c.id, look, scores));
        }
        let strongest: Vec<f32> = (0..portraits.len())
            .map(|p| cast.iter().map(|(_, _, s)| s[p]).filter(|s| s.is_finite()).fold(f32::NEG_INFINITY, f32::max))
            .collect();
        let mut matches = Vec::new();
        for (character, look, scores) in cast {
            if let Some((portrait, score)) = best_portrait(&scores, &roles, &strongest) {
                matches.push(Match { character, portrait, score, look });
            }
        }
        let (names, merges) = proposals(portraits, matches);
        Ok((rev, names, merges))
    }

    /// Apply suggestions made on cast revision `rev` (merges first, then
    /// the surviving characters' names). A regroup or decision since makes
    /// them stale: nothing is applied.
    pub fn apply_anilist_suggestions(
        &self,
        rev: i64,
        names: &[NameSuggestion],
        merges: &[MergeProposal],
    ) -> ProjectResult<usize> {
        if self.db().cast_rev()? != rev {
            return Err(ProjectError::NotReady("The cast changed; match the portraits again.".into()));
        }
        let mut applied = 0;
        for m in merges {
            self.cast_edit(&as_api::CastEdit::Merge { from: m.from, into: m.into })?;
            applied += 1;
        }
        for n in names {
            if matches!(self.db().character_exists(n.character_id), Ok(true)) {
                self.cast_edit(&as_api::CastEdit::Rename { character_id: n.character_id, name: Some(n.name.clone()) })?;
                applied += 1;
            }
        }
        self.set_anilist_suggested(None)?;
        Ok(applied)
    }
}

// ---------------------------------------------------------------------------
// The library's per-project actions
// ---------------------------------------------------------------------------

impl Library {
    fn anilist_allowed(&self) -> ProjectResult<()> {
        if self.names_from_anilist() {
            Ok(())
        } else {
            Err(ProjectError::Invalid("Turn on names from AniList in Settings first.".into()))
        }
    }

    /// The search text project `id`'s title suggests (nothing is sent).
    pub fn anilist_search_text(&self, id: ProjectId) -> ProjectResult<String> {
        Ok(search_text(&self.summary(id)?.title))
    }

    /// Look `text` up on AniList for project `id` (the text the person
    /// confirmed: it is sent as it is). Series the project rejected aren't
    /// among the results.
    pub fn anilist_search(&self, id: ProjectId, text: &str, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        self.anilist_allowed()?;
        let rejected = self.project(id)?.anilist_rejected()?;
        let found = self.anilist().search(text, cancel)?;
        Ok(found.into_iter().filter(|m| !rejected.contains(&m.id)).collect())
    }

    /// Confirm series `media` for project `id`: it and its sequel chain are
    /// kept. Returns the chain, `media` first.
    pub fn anilist_confirm(&self, id: ProjectId, media: u32, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        self.anilist_allowed()?;
        let project = self.project(id)?;
        let chain = self.anilist().chain(media, cancel)?;
        project.confirm_anilist_media(&chain.iter().map(|m| m.id).collect::<Vec<_>>(), false)?;
        Ok(chain)
    }

    /// Say series `media` isn't project `id`'s.
    pub fn anilist_reject(&self, id: ProjectId, media: u32) -> ProjectResult<()> {
        self.project(id)?.reject_anilist_media(media)
    }

    /// Whether project `id` is one AniList names: the opt-in is on, it was
    /// made as anime, and someone shown is still unnamed.
    pub fn anilist_wanted(&self, id: ProjectId) -> ProjectResult<bool> {
        if !self.names_from_anilist() {
            return Ok(false);
        }
        let project = self.project(id)?;
        if project.content()? != as_api::ContentKind::Anime {
            return Ok(false);
        }
        Ok(project.db().characters()?.iter().any(|c| !c.hidden && c.name.is_none()))
    }

    /// Project `id`'s series on AniList, found without asking: the one
    /// confirmed, or else the one its title plainly names ([`pick_series`]),
    /// then kept with its sequel chain (as picked by the app, see
    /// [`PICKED`]). `None` when none plainly is (the person searches by hand
    /// then). The project isn't held open while AniList answers.
    pub fn anilist_find_series(&self, id: ProjectId, cancel: &CancelToken) -> ProjectResult<Option<Series>> {
        self.anilist_allowed()?;
        let confirmed = |lib: &Self| -> ProjectResult<Option<Series>> {
            let project = lib.project(id)?;
            let media = project.anilist_media()?;
            let picked = project.anilist_picked()?;
            drop(project);
            let Some(&root) = media.first() else { return Ok(None) };
            Ok(Some(Series { name: lib.anilist().media(root, cancel)?.name(), media, picked }))
        };
        if let Some(series) = confirmed(self)? {
            return Ok(Some(series));
        }
        let title = self.summary(id)?.title;
        let text = search_text(&title);
        if text.is_empty() {
            return Ok(None);
        }
        let rejected = self.project(id)?.anilist_rejected()?;
        // Picked among all the answers: with the turned-down ones left out,
        // the next answer would be taken for the title's.
        let found = self.anilist().search(&text, cancel)?;
        let Some(media) = pick_series(&title, &found) else {
            tracing::info!(text, "anilist: no series plainly matches the title");
            return Ok(None);
        };
        let chain = self.anilist().chain(media.id, cancel)?;
        // Another season of a series the person turned down is still it.
        if chain.iter().any(|m| rejected.contains(&m.id)) {
            tracing::info!(series = media.id, "anilist: the title's series was turned down");
            return Ok(None);
        }
        going(cancel)?;
        // The person confirmed one meanwhile: theirs it is.
        if let Some(series) = confirmed(self)? {
            return Ok(Some(series));
        }
        let ids: Vec<u32> = chain.iter().map(|m| m.id).collect();
        self.project(id)?.confirm_anilist_media(&ids, true)?;
        tracing::info!(series = media.id, chain = chain.len(), "anilist: series found by the title");
        Ok(Some(Series { name: media.name(), media: ids, picked: true }))
    }

    /// Give the names and merges waiting for project `id` that the person
    /// kept (see [`Project::accept_anilist_suggested`]).
    pub fn anilist_accept(
        &self,
        id: ProjectId,
        give: &[(as_core::CharacterId, u32)],
        merge: &[(as_core::CharacterId, as_core::CharacterId)],
        left_out: &[(as_core::CharacterId, u32)],
    ) -> ProjectResult<(usize, usize)> {
        self.anilist_allowed()?;
        self.project(id)?.accept_anilist_suggested(give, merge, left_out)
    }

    /// The names waiting for project `id`'s yes (none while the opt-in is
    /// off).
    pub fn anilist_suggested(&self, id: ProjectId) -> ProjectResult<Option<Suggested>> {
        if !self.names_from_anilist() {
            return Ok(None);
        }
        self.project(id)?.anilist_suggested()
    }

    /// The person said no to the waiting names: they stop waiting.
    pub fn anilist_dismiss(&self, id: ProjectId) -> ProjectResult<()> {
        self.project(id)?.set_anilist_suggested(None)
    }

    /// AniList character `id`'s portrait as embedded this session.
    pub fn anilist_portrait(&self, id: u32) -> Option<Vec<f32>> {
        self.anilist().portrait(id)
    }

    /// Keep AniList character `id`'s embedded portrait for the session (not
    /// once `cancel` is).
    pub fn keep_anilist_portrait(&self, id: u32, embedding: &[f32], cancel: &CancelToken) {
        self.anilist().keep_portrait(id, embedding, cancel);
    }

    /// Give project `id`'s characters the suggested names (see
    /// [`Project::anilist_suggest`]): `rev` must still be the cast's — one
    /// made stale by a regroup or a decision is not applied. Returns whose
    /// names were given (merges first, then names).
    pub fn anilist_apply(
        &self,
        id: ProjectId,
        rev: i64,
        names: &[NameSuggestion],
        merges: &[MergeProposal],
    ) -> ProjectResult<usize> {
        self.anilist_allowed()?;
        self.project(id)?.apply_anilist_suggestions(rev, names, merges)
    }

    /// The main and supporting cast of project `id`'s confirmed series, their
    /// images loaded into the library's cache.
    pub fn anilist_cast(&self, id: ProjectId, cancel: &CancelToken) -> ProjectResult<Vec<CastMember>> {
        self.anilist_allowed()?;
        let media = self.project(id)?.anilist_media()?;
        if media.is_empty() {
            return Err(ProjectError::NotReady("Confirm the series on AniList first.".into()));
        }
        let client = self.anilist();
        let cast = client.cast(&media, cancel)?;
        let mut out = Vec::with_capacity(cast.len());
        for character in cast {
            let image = match client.image(&character, cancel) {
                Ok(image) => image,
                Err(ProjectError::Cancelled) => return Err(ProjectError::Cancelled),
                // One image that won't load leaves the rest.
                Err(ProjectError::NotReady(_)) => None,
                Err(e) => return Err(e),
            };
            // Switched off meanwhile: nothing stays behind.
            if !self.names_from_anilist() {
                client.forget()?;
                return Err(ProjectError::Cancelled);
            }
            out.push(CastMember { character, image });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use as_api::{AppSettings, ContentKind, NewProject, ProjectKind};
    use as_core::AppPaths;

    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/anilist");

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(FIXTURES).join(name)).unwrap()
    }

    /// Time that only moves when slept.
    struct FakeClock {
        now: Mutex<Instant>,
        slept: Mutex<Duration>,
    }

    impl FakeClock {
        fn new() -> Arc<Self> {
            Arc::new(Self { now: Mutex::new(Instant::now()), slept: Mutex::default() })
        }

        fn slept(&self) -> Duration {
            *self.slept.lock()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock()
        }

        fn sleep(&self, d: Duration, _: &CancelToken) {
            *self.now.lock() += d;
            *self.slept.lock() += d;
        }
    }

    /// A piece of the request body, then the reply (status, body,
    /// `Retry-After`), and whether it is used only once.
    type Canned = (String, u16, Vec<u8>, Option<u64>, bool);

    /// Recorded answers, by what the request asks for.
    #[derive(Default)]
    struct Recorded {
        /// First match wins.
        answers: Mutex<Vec<Canned>>,
        bodies: Mutex<Vec<String>>,
        images: AtomicUsize,
        /// When each request was sent, by the fake clock.
        at: Mutex<Vec<Instant>>,
        clock: Mutex<Option<Arc<FakeClock>>>,
    }

    impl Recorded {
        fn answer(&self, matching: &str, status: u16, body: Vec<u8>) {
            self.answers.lock().push((matching.into(), status, body, None, false));
        }

        fn answer_once(&self, matching: &str, status: u16, body: Vec<u8>, retry_after: Option<u64>) {
            // Before the lasting answers: it comes first.
            self.answers.lock().insert(0, (matching.into(), status, body, retry_after, true));
        }

        fn frieren(&self) {
            self.answer("\"search\":", 200, fixture("search_frieren.json"));
            self.answer("\"id\":182255}", 200, fixture("media_182255.json"));
            self.answer("\"id\":209939}", 200, fixture("media_209939.json"));
            self.answer("\"page\":1,\"role\":\"MAIN\"", 200, fixture("characters_154587_main_1.json"));
            for p in 1..=3 {
                self.answer(
                    &format!("\"page\":{p},\"role\":\"SUPPORTING\""),
                    200,
                    fixture(&format!("characters_154587_supporting_{p}.json")),
                );
            }
        }
    }

    impl Transport for Recorded {
        fn post(&self, body: &str) -> Result<Reply, String> {
            self.bodies.lock().push(body.to_string());
            if let Some(c) = &*self.clock.lock() {
                self.at.lock().push(c.now());
            }
            let mut answers = self.answers.lock();
            let i = answers
                .iter()
                .position(|(m, ..)| body.contains(m.as_str()))
                .ok_or_else(|| format!("no answer for {body}"))?;
            let (_, status, reply, retry_after, once) = answers[i].clone();
            if once {
                answers.remove(i);
            }
            Ok(Reply {
                status,
                retry_after: retry_after.map(Duration::from_secs),
                remaining: Some(20),
                content_type: Some("application/json".into()),
                body: reply,
            })
        }

        fn get(&self, url: &str) -> Result<Reply, String> {
            self.images.fetch_add(1, Ordering::Relaxed);
            assert!(on_image_server(url), "{url}");
            Ok(Reply {
                status: 200,
                retry_after: None,
                remaining: None,
                content_type: Some("image/png".into()),
                body: b"\x89PNG".to_vec(),
            })
        }
    }

    fn client(dir: &Path) -> (AniList, Arc<Recorded>, Arc<FakeClock>) {
        let net = Arc::new(Recorded::default());
        let clock = FakeClock::new();
        *net.clock.lock() = Some(clock.clone());
        let client = AniList::with(
            dir.join("anilist"),
            net.clone(),
            clock.clone(),
            Arc::new(Limiter::new(API_GAP)),
            Arc::new(Limiter::new(IMAGE_GAP)),
        );
        (client, net, clock)
    }

    #[test]
    fn search_text_drops_fansub_groups_and_release_tags() {
        for (title, want) in [
            ("[SubsPlease] Sousou no Frieren - 03 (1080p) [A1B2C3D4]", "Sousou no Frieren"),
            ("[Erai-raws] Kusuriya no Hitorigoto - 12 [1080p][Multiple Subtitle].mkv", "Kusuriya no Hitorigoto"),
            ("Sousou.no.Frieren.S01E03.1080p.WEB.H264-GROUP", "Sousou no Frieren"),
            ("Frieren_-_Beyond_Journeys_End_-_05v2_[BD_1080p_HEVC]", "Frieren - Beyond Journeys End"),
            ("[Judas] Mob Psycho 100 S2 [BD 1080p][HEVC x265 10bit]", "Mob Psycho 100"),
            (
                "Re:Zero kara Hajimeru Isekai Seikatsu 2nd Season (2020)",
                "Re:Zero kara Hajimeru Isekai Seikatsu 2nd Season",
            ),
            ("86 - Eighty Six", "86 - Eighty Six"),
            ("Dr. Stone", "Dr. Stone"),
            ("Spy x Family Episode 7 [Dual Audio]", "Spy x Family"),
            ("【推しの子】 第2期", "【推しの子】 第2期"),
            ("[Oshi no Ko] S2 - 05 [1080p]", "Oshi no Ko"),
            ("  Sousou no Frieren  ", "Sousou no Frieren"),
            // Nothing left but tags: what the brackets hold.
            ("[SubsPlease]", "SubsPlease"),
            ("[1080p]", "[1080p]"),
        ] {
            assert_eq!(search_text(title), want, "{title}");
        }
    }

    #[test]
    fn recorded_search_reads_titles_and_the_sequel_chain() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.frieren();
        let found = client.search("Sousou no Frieren", &CancelToken::new()).unwrap();
        let first = &found[0];
        assert_eq!(first.id, 154587);
        assert_eq!(first.id_mal, Some(52991));
        assert_eq!(first.format.as_deref(), Some("TV"));
        assert_eq!((first.episodes, first.season_year), (Some(28), Some(2023)));
        assert_eq!(first.title.romaji.as_deref(), Some("Sousou no Frieren"));
        assert_eq!(first.title.native.as_deref(), Some("葬送のフリーレン"));
        assert!(first.synonyms.iter().any(|s| s == "Frieren at the Funeral"));
        // Its side stories, music and manga aren't part of the chain.
        assert_eq!(first.sequels, [182255]);
        // The text went as it was, for anime only.
        let body: serde_json::Value = serde_json::from_str(&net.bodies.lock()[0]).unwrap();
        assert_eq!(body["variables"]["search"], "Sousou no Frieren");
        assert!(body["query"].as_str().unwrap().contains("type: ANIME"));

        // Every season was among the results: the chain needs no request.
        let chain = client.chain(154587, &CancelToken::new()).unwrap();
        assert_eq!(chain.iter().map(|m| m.id).collect::<Vec<_>>(), [154587, 182255, 209939]);
        assert_eq!(net.bodies.lock().len(), 1);
    }

    #[test]
    fn a_chain_is_followed_by_request_when_it_wasnt_found() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.frieren();
        net.answer("\"id\":154587}", 200, first_season());
        // Confirmed from the second season, nothing searched this session.
        let chain = client.chain(182255, &CancelToken::new()).unwrap();
        // The confirmed one first, the rest by year.
        assert_eq!(chain.iter().map(|m| m.id).collect::<Vec<_>>(), [182255, 154587, 209939]);
        assert_eq!(net.bodies.lock().len(), 3);
        // Read once a session.
        client.chain(209939, &CancelToken::new()).unwrap();
        assert_eq!(net.bodies.lock().len(), 3);
    }

    /// The first season's answer to a `Media(id:)` request, from the search's.
    fn first_season() -> Vec<u8> {
        let search: serde_json::Value = serde_json::from_slice(&fixture("search_frieren.json")).unwrap();
        serde_json::json!({ "data": { "Media": search["data"]["Page"]["media"][0] } }).to_string().into_bytes()
    }

    #[test]
    fn the_cast_is_read_page_by_page_until_there_are_no_more() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.frieren();
        let cast = client.characters(154587, &CancelToken::new()).unwrap();
        // 1 page of main characters, 3 of supporting ones.
        assert_eq!(net.bodies.lock().len(), 4);
        assert_eq!(cast.len(), 3 + 25 + 25 + 4);
        assert_eq!(cast.iter().take(3).map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Frieren", "Fern", "Stark"]);
        assert!(cast[..3].iter().all(|c| c.role == Role::Main) && cast[3..].iter().all(|c| c.role == Role::Supporting));
        let frieren = &cast[0];
        assert_eq!(frieren.id, 176754);
        assert_eq!(frieren.native.as_deref(), Some("フリーレン"));
        assert_eq!(
            frieren.image.as_deref(),
            Some("https://s4.anilist.co/file/anilistcdn/character/large/b176754-PCnpqIOkjhFk.png")
        );
        assert!(cast.iter().any(|c| c.alternative == ["Chief of Sword Village"]));
        let body: serde_json::Value = serde_json::from_str(&net.bodies.lock()[0]).unwrap();
        assert!(body["query"].as_str().unwrap().contains("sort: [ROLE, RELEVANCE, ID]"));
        assert!(body["query"].as_str().unwrap().contains("perPage: 25"));
    }

    #[test]
    fn a_cast_over_a_chain_names_everyone_once_with_their_biggest_part() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.frieren();
        // The second season: Frieren as supporting, and someone new.
        let second = serde_json::json!({ "data": { "Media": { "id": 182255, "characters": {
            "pageInfo": { "hasNextPage": false },
            "edges": [
                { "role": "SUPPORTING", "node": { "id": 176754, "name": { "full": "Frieren", "native": null, "alternative": [] }, "image": null } },
                { "role": "SUPPORTING", "node": { "id": 9, "name": { "full": " ", "native": "新人", "alternative": ["", null] }, "image": { "large": null } } },
                { "role": "BACKGROUND", "node": { "id": 10, "name": { "full": "Farmer" }, "image": null } }
            ] } } } });
        let empty = serde_json::json!({ "data": { "Media": { "id": 182255, "characters": { "pageInfo": { "hasNextPage": false }, "edges": [] } } } });
        net.answers.lock().insert(
            0,
            (
                "\"id\":182255,\"page\":1,\"role\":\"SUPPORTING\"".into(),
                200,
                second.to_string().into_bytes(),
                None,
                false,
            ),
        );
        net.answers.lock().insert(
            0,
            ("\"id\":182255,\"page\":1,\"role\":\"MAIN\"".into(), 200, empty.to_string().into_bytes(), None, false),
        );
        let cast = client.cast(&[154587, 182255], &CancelToken::new()).unwrap();
        assert_eq!(cast.iter().filter(|c| c.id == 176754).count(), 1);
        assert_eq!(cast.iter().find(|c| c.id == 176754).unwrap().role, Role::Main);
        let new = cast.iter().find(|c| c.id == 9).unwrap();
        assert_eq!((new.name.as_str(), new.alternative.len(), new.image.as_deref()), ("新人", 0, None));
        assert!(cast.iter().all(|c| c.id != 10), "background characters aren't asked for");
    }

    #[test]
    fn errors_say_what_anilist_said() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.answer("\"search\":", 400, fixture("error_400.json"));
        let e = client.search("x", &CancelToken::new()).unwrap_err();
        assert!(matches!(&e, ProjectError::Invalid(m) if m.contains("is not defined")), "{e:?}");
        assert!(matches!(client.search("  ", &CancelToken::new()), Err(ProjectError::Invalid(_))));

        net.answer(
            "\"id\":1}",
            404,
            br#"{"errors":[{"message":"Not Found.","status":404}],"data":{"Media":null}}"#.to_vec(),
        );
        assert!(matches!(client.media(1, &CancelToken::new()), Err(ProjectError::NotFound(_))));
        net.answer("\"id\":2}", 502, b"<html>Bad gateway</html>".to_vec());
        assert!(matches!(client.media(2, &CancelToken::new()), Err(ProjectError::NotReady(_))));
        net.answer("\"id\":3}", 200, b"{\"data\":{\"Media\":null}}".to_vec());
        assert!(matches!(client.media(3, &CancelToken::new()), Err(ProjectError::NotFound(_))));
    }

    #[test]
    fn requests_are_two_and_a_half_seconds_apart() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, clock) = client(dir.path());
        net.frieren();
        client.characters(154587, &CancelToken::new()).unwrap();
        let at = net.at.lock().clone();
        assert_eq!(at.len(), 4);
        for pair in at.windows(2) {
            assert!(pair[1] - pair[0] >= API_GAP, "{:?}", pair[1] - pair[0]);
        }
        assert_eq!(clock.slept(), API_GAP * 3);
    }

    #[test]
    fn too_many_requests_pause_every_request_for_at_least_a_minute() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, clock) = client(dir.path());
        net.frieren();
        // A short Retry-After still pauses a minute; a long one is honoured.
        net.answer_once(
            "\"search\":",
            429,
            b"{\"errors\":[{\"message\":\"Too Many Requests.\"}],\"data\":null}".to_vec(),
            Some(5),
        );
        let found = client.search("Sousou no Frieren", &CancelToken::new()).unwrap();
        assert_eq!(found[0].id, 154587);
        let at = net.at.lock().clone();
        assert_eq!(at.len(), 2);
        assert!(at[1] - at[0] >= MIN_PAUSE);

        net.answer_once("\"id\":182255}", 429, Vec::new(), Some(120));
        let before = clock.now();
        client.seen.lock().clear();
        client.media(182255, &CancelToken::new()).unwrap();
        assert!(clock.now() - before >= Duration::from_secs(120));

        // AniList asking again and again: given up after a few tries.
        for _ in 0..ATTEMPTS {
            net.answer_once("\"id\":209939}", 429, Vec::new(), None);
        }
        client.seen.lock().clear();
        assert!(matches!(client.media(209939, &CancelToken::new()), Err(ProjectError::NotReady(_))));
    }

    #[test]
    fn limiter_shares_one_pace_and_its_pauses() {
        let clock = FakeClock::new();
        let limiter = Limiter::new(API_GAP);
        let cancel = CancelToken::new();
        let start = clock.now();
        limiter.acquire(&*clock, &cancel).unwrap();
        assert_eq!(clock.now(), start, "the first request waits for nothing");
        limiter.acquire(&*clock, &cancel).unwrap();
        assert_eq!(clock.now() - start, API_GAP);

        // A pause holds back the next request; a shorter one asked for later
        // doesn't shorten it.
        limiter.pause(clock.now(), Duration::from_secs(60));
        limiter.pause(clock.now(), Duration::from_secs(1));
        let paused_at = clock.now();
        limiter.acquire(&*clock, &cancel).unwrap();
        assert_eq!(clock.now() - paused_at, Duration::from_secs(60));

        // Time that went by on its own counts.
        *clock.now.lock() += Duration::from_secs(10);
        let before = clock.slept();
        limiter.acquire(&*clock, &cancel).unwrap();
        assert_eq!(clock.slept(), before);

        cancel.cancel();
        assert!(matches!(limiter.acquire(&*clock, &cancel), Err(ProjectError::Cancelled)));
    }

    #[test]
    fn no_requests_left_pause_the_next() {
        struct Empty;
        impl Transport for Empty {
            fn post(&self, _: &str) -> Result<Reply, String> {
                Ok(Reply {
                    status: 200,
                    retry_after: None,
                    remaining: Some(0),
                    content_type: None,
                    body: first_season(),
                })
            }
            fn get(&self, _: &str) -> Result<Reply, String> {
                Err("no images".into())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let clock = FakeClock::new();
        let api = Arc::new(Limiter::new(API_GAP));
        let client =
            AniList::with(dir.path().into(), Arc::new(Empty), clock.clone(), api, Arc::new(Limiter::new(IMAGE_GAP)));
        client.media(154587, &CancelToken::new()).unwrap();
        client.seen.lock().clear();
        client.media(154587, &CancelToken::new()).unwrap();
        assert_eq!(clock.slept(), MIN_PAUSE);
    }

    #[test]
    fn images_come_only_from_anilist_and_are_cached() {
        assert!(on_image_server("https://s4.anilist.co/file/anilistcdn/character/large/b1.png"));
        assert!(!on_image_server("http://s4.anilist.co/x.png"), "never in the clear");
        assert!(!on_image_server("https://anilist.co.evil.example/x.png"));
        assert!(!on_image_server("https://evil.example/anilist.co/x.png"));
        assert_eq!(image_ext("https://s4.anilist.co/a/b1-x.jpg", Some("image/jpeg; charset=binary")), Some("jpg"));
        assert_eq!(image_ext("https://s4.anilist.co/a/b1-x.png", None), Some("png"));
        assert_eq!(image_ext("https://s4.anilist.co/a/b1-x.png", Some("text/html")), None);

        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        let frieren = Character {
            id: 176754,
            role: Role::Main,
            name: "Frieren".into(),
            native: None,
            alternative: Vec::new(),
            image: Some("https://s4.anilist.co/file/anilistcdn/character/large/b176754-PCnpqIOkjhFk.png".into()),
        };
        let path = client.image(&frieren, &CancelToken::new()).unwrap().unwrap();
        assert_eq!(path, dir.path().join("anilist").join("176754.png"));
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG");
        assert_eq!(client.image(&frieren, &CancelToken::new()).unwrap(), Some(path));
        assert_eq!(net.images.load(Ordering::Relaxed), 1, "loaded once");
        let elsewhere = Character { id: 2, image: Some("https://example.com/2.png".into()), ..frieren };
        assert_eq!(client.image(&elsewhere, &CancelToken::new()).unwrap(), None);
        assert_eq!(net.images.load(Ordering::Relaxed), 1);
    }

    fn library(dir: &Path) -> Library {
        let lib = Library::open(&AppPaths::at(dir.join("data"))).unwrap();
        let s = lib.settings().unwrap();
        lib.update_settings(&AppSettings { library_root: dir.join("Library").display().to_string(), ..s }).unwrap();
        lib
    }

    fn anime(lib: &Library, title: &str) -> ProjectId {
        let new =
            NewProject { title: title.into(), kind: ProjectKind::Series, content: ContentKind::Anime, files: vec![] };
        lib.create_project(&new).unwrap().0.id
    }

    #[test]
    fn nothing_is_sent_while_it_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let id = anime(&lib, "[SubsPlease] Sousou no Frieren - 03 (1080p)");
        assert!(!lib.settings().unwrap().anilist, "off by default");
        assert_eq!(lib.anilist_search_text(id).unwrap(), "Sousou no Frieren");
        let cancel = CancelToken::new();
        assert!(matches!(lib.anilist_search(id, "Sousou no Frieren", &cancel), Err(ProjectError::Invalid(_))));
        assert!(matches!(lib.anilist_confirm(id, 154587, &cancel), Err(ProjectError::Invalid(_))));
        assert!(matches!(lib.anilist_cast(id, &cancel), Err(ProjectError::Invalid(_))));
    }

    #[test]
    fn a_project_keeps_its_series_in_meta_without_a_migration() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let id = anime(&lib, "Frieren");
        let project = lib.project(id).unwrap();
        assert!(project.anilist_media().unwrap().is_empty());
        assert!(!project.anilist_auto().unwrap(), "names are only suggested by default");

        project.reject_anilist_media(170068).unwrap();
        project.reject_anilist_media(182255).unwrap();
        project.reject_anilist_media(182255).unwrap();
        assert_eq!(project.anilist_rejected().unwrap(), [170068, 182255]);
        project.confirm_anilist_media(&[154587, 182255, 209939], false).unwrap();
        assert_eq!(project.anilist_media().unwrap(), [154587, 182255, 209939]);
        assert_eq!(project.anilist_rejected().unwrap(), [170068], "a confirmed season isn't rejected");
        project.set_anilist_auto(true).unwrap();
        assert_eq!(project.db().meta(MEDIA).unwrap().as_deref(), Some("[154587,182255,209939]"));
        assert_eq!(project.db().meta(AUTO).unwrap().as_deref(), Some("true"));

        // Kept across a reopen, in a project 1.0.3 still opens.
        let dir_of = project.dir.clone();
        drop(project);
        drop(lib);
        let lib = Library::open(&AppPaths::at(dir.path().join("data"))).unwrap();
        let project = lib.project(id).unwrap();
        assert_eq!(project.anilist_media().unwrap(), [154587, 182255, 209939]);
        assert!(project.anilist_auto().unwrap());
        project.reject_anilist_media(182255).unwrap();
        assert!(project.anilist_media().unwrap().is_empty(), "rejecting a confirmed season forgets the series");
        let version: i64 = rusqlite::Connection::open(dir_of.join(crate::project::PROJECT_DB))
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 15);
    }

    #[test]
    fn switching_it_off_deletes_the_images() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let images = AppPaths::at(dir.path().join("data")).anilist();
        assert_eq!(lib.anilist().image_dir(), images);
        let on = AppSettings { anilist: true, ..lib.settings().unwrap() };
        assert!(lib.update_settings(&on).unwrap().anilist);
        std::fs::create_dir_all(&images).unwrap();
        std::fs::write(images.join("176754.png"), b"x").unwrap();
        lib.anilist().seen.lock().insert(1, Media::from(serde_json::from_str::<RawMedia>(r#"{"id":1}"#).unwrap()));

        // Other settings saved while it is on keep them.
        let again = AppSettings { audio_language: "jpn".into(), ..lib.settings().unwrap() };
        lib.update_settings(&again).unwrap();
        assert!(images.join("176754.png").is_file());

        let off = AppSettings { anilist: false, ..lib.settings().unwrap() };
        assert!(!lib.update_settings(&off).unwrap().anilist);
        assert!(!images.exists());
        assert!(lib.anilist().seen.lock().is_empty());
        // Kept across a reopen.
        drop(lib);
        let lib = Library::open(&AppPaths::at(dir.path().join("data"))).unwrap();
        assert!(!lib.settings().unwrap().anilist);
    }

    fn member(id: u32, name: &str) -> CastMember {
        CastMember {
            character: Character {
                id,
                role: Role::Main,
                name: name.into(),
                native: None,
                alternative: vec![],
                image: None,
            },
            image: None,
        }
    }

    #[test]
    fn a_look_alike_with_a_smaller_part_does_not_take_the_name() {
        use Role::{Main, Supporting};
        let roles = [Supporting, Supporting, Main, Supporting];
        // The hero's father and brother share his hair and score a little
        // higher on his look than his own portrait: the main part wins.
        let hero = [0.962, 0.962, 0.935, 0.63];
        assert_eq!(best_portrait(&hero, &roles, &hero), Some((2, 0.935)));
        // The father's own group, the hero there and closer to the hero's
        // portrait: not the hero, and father and brother can't be told
        // apart — no name.
        let strongest = [0.95, 0.962, 0.935, 0.63];
        assert_eq!(best_portrait(&[0.95, 0.90, 0.90, 0.5], &roles, &strongest), None);
        // Without the brother as a look-alike the father keeps his own.
        assert_eq!(best_portrait(&[0.95, 0.5, 0.90, 0.5], &roles, &strongest), Some((0, 0.95)));
        // A second group of the hero, the first closer to his portrait: not
        // the brother whose portrait comes next — no name.
        assert_eq!(best_portrait(&[0.5, 0.80, 0.857, 0.5], &roles, &[0.95, 0.962, 0.935, 0.63]), None);
        // A clear best needs no part to decide.
        let clear = [0.91, 0.78, 0.75];
        assert_eq!(best_portrait(&clear, &[Supporting, Main, Main], &clear), Some((0, 0.91)));
        // Look-alikes of the same part can't be told apart: no name.
        let same = [0.84, 0.82, 0.80];
        assert_eq!(best_portrait(&same, &[Supporting, Supporting, Supporting], &same), None);
        assert_eq!(best_portrait(&[0.90, 0.88], &[Main, Main], &[0.90, 0.88]), None);
        // The bigger part names only at the bar itself.
        let low = [0.72, SUGGEST_MIN_COS - 0.05];
        assert_eq!(best_portrait(&low, &[Supporting, Main], &low), None);
        // Under the bar, nothing at all, or a spoilt score: no name from it.
        assert_eq!(best_portrait(&[SUGGEST_MIN_COS - 0.01], &[Main], &[1.0]), None);
        assert_eq!(best_portrait(&[], &[], &[]), None);
        assert_eq!(best_portrait(&[f32::NAN, 0.91], &[Main, Supporting], &[1.0, 0.91]), Some((1, 0.91)));
    }

    #[test]
    fn a_look_weighs_faces_by_quality_and_skips_other_sizes() {
        let face = |v: &[f32], quality: f32| as_db::OwnFace {
            id: 0,
            pinned: false,
            quality,
            emb: as_analysis::tracks::embedding_to_bytes(v),
        };
        let faces = [face(&[1.0, 0.0], 0.9), face(&[0.0, 1.0], 0.1), face(&[0.0, 0.0, 1.0], 1.0)];
        let l = look(&faces, 2).unwrap();
        assert!((l[0] - 0.9939).abs() < 1e-3 && (l[1] - 0.1104).abs() < 1e-3, "{l:?}");
        // No face of the portraits' length: no look.
        assert!(look(&faces[2..], 2).is_none());
        assert!(look(&[], 2).is_none());
    }

    #[test]
    fn a_portrait_names_one_character_and_merges_only_alike_looks() {
        let portraits: Vec<(CastMember, Vec<f32>)> =
            vec![(member(10, "Frieren"), vec![]), (member(11, "Fern"), vec![])];
        let (a, b, c, d) = (
            as_core::CharacterId::new(),
            as_core::CharacterId::new(),
            as_core::CharacterId::new(),
            as_core::CharacterId::new(),
        );
        let m = |character, portrait, score, look: [f32; 2]| Match { character, portrait, score, look: look.to_vec() };
        // a and b both match Frieren's portrait and look alike (cosine
        // 0.98): a, the stronger, is named and b proposed for merging into
        // it. d matches it too but looks unlike a (cosine 0.6): unnamed,
        // not merged. c is Fern.
        let (names, merges) = proposals(
            &portraits,
            vec![
                m(a, 0, 0.9, [1.0, 0.0]),
                m(b, 0, 0.85, [0.98, 0.199]),
                m(d, 0, 0.8, [0.6, 0.8]),
                m(c, 1, 0.75, [0.0, 1.0]),
            ],
        );
        assert_eq!(names.len(), 2);
        let frieren = names.iter().find(|n| n.anilist_id == 10).unwrap();
        assert_eq!((frieren.character_id, frieren.name.as_str()), (a, "Frieren"));
        assert!(names.iter().any(|n| n.anilist_id == 11 && n.character_id == c));
        assert_eq!(merges.len(), 1);
        assert_eq!((merges[0].from, merges[0].into, merges[0].anilist_id), (b, a, 10));
    }

    #[test]
    fn a_series_is_picked_only_when_the_title_plainly_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.frieren();
        let found = client.search("Sousou no Frieren", &CancelToken::new()).unwrap();
        let picked = |title: &str| pick_series(title, &found).map(|m| m.id);
        // A title of the series, a release name, its English title (no
        // matter the apostrophe), the first answer's title begun.
        assert_eq!(picked("Sousou no Frieren"), Some(154587));
        assert_eq!(picked("[SubsPlease] Sousou no Frieren - 03 (1080p) [A1B2C3D4]"), Some(154587));
        assert_eq!(picked("Frieren - Beyond Journeys End"), Some(154587));
        assert_eq!(picked("Sousou no"), Some(154587));
        // One word begins too many titles: only one that is the title.
        assert_eq!(picked("Frieren"), None);
        assert_eq!(picked("Sousou no Frieren 2nd Season"), Some(182255));
        // A year doesn't outweigh the title.
        assert_eq!(picked("Sousou no Frieren (2026)"), Some(154587));
        // Not whole words, another series, nothing.
        assert_eq!(picked("Frie"), None);
        assert_eq!(picked("Naruto"), None);
        assert_eq!(picked(""), None);
        assert_eq!(pick_series("Sousou no Frieren", &[]), None);

        // Two series of one title: the year the title names picks.
        let series = |id, year| Media {
            id,
            id_mal: None,
            format: Some("TV".into()),
            episodes: None,
            season_year: Some(year),
            title: Titles { english: Some("Hunter x Hunter".into()), romaji: None, native: None },
            synonyms: vec![],
            sequels: vec![],
        };
        let both = [series(136, 1999), series(11061, 2011)];
        assert_eq!(pick_series("Hunter x Hunter (2011)", &both).map(|m| m.id), Some(11061));
        assert_eq!(pick_series("Hunter x Hunter", &both).map(|m| m.id), Some(136));
    }

    #[test]
    fn a_cast_is_read_once_a_session() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.frieren();
        let first = client.characters(154587, &CancelToken::new()).unwrap();
        let asked = net.bodies.lock().len();
        assert_eq!(client.characters(154587, &CancelToken::new()).unwrap(), first);
        assert_eq!(net.bodies.lock().len(), asked, "no request the second time");
        client.forget().unwrap();
        client.characters(154587, &CancelToken::new()).unwrap();
        assert!(net.bodies.lock().len() > asked, "asked again once forgotten");
    }

    #[test]
    fn the_title_finds_the_series_and_names_wait_for_a_yes() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.frieren();
        let lib = library(dir.path()).with_anilist(client);
        let id = anime(&lib, "[SubsPlease] Sousou no Frieren - 03 (1080p)");
        let cancel = lib.anilist_token();
        assert!(matches!(lib.anilist_find_series(id, &cancel), Err(ProjectError::Invalid(_))), "off: nothing sent");
        assert!(net.bodies.lock().is_empty());
        lib.update_settings(&AppSettings { anilist: true, ..lib.settings().unwrap() }).unwrap();
        assert!(!lib.anilist_wanted(id).unwrap(), "no one to name yet");

        let series = lib.anilist_find_series(id, &cancel).unwrap().unwrap();
        assert_eq!(series.name, "Frieren: Beyond Journey’s End");
        assert_eq!(series.media, [154587, 182255, 209939]);
        assert!(series.picked, "picked from the title");
        let project = lib.project(id).unwrap();
        assert_eq!(project.anilist_media().unwrap(), series.media);
        assert!(project.anilist_picked().unwrap());
        let body: serde_json::Value = serde_json::from_str(&net.bodies.lock()[0]).unwrap();
        assert_eq!(body["variables"]["search"], "Sousou no Frieren", "the title's search text");
        // Kept: no search again.
        let asked = net.bodies.lock().len();
        assert_eq!(lib.anilist_find_series(id, &cancel).unwrap().as_ref(), Some(&series));
        assert_eq!(net.bodies.lock().len(), asked);
        // Confirmed by the person: no longer picked.
        project.confirm_anilist_media(&series.media, false).unwrap();
        assert!(!lib.anilist_find_series(id, &cancel).unwrap().unwrap().picked);

        // Three unnamed characters, one hidden.
        let db = rusqlite::Connection::open(project.dir.join(crate::project::PROJECT_DB)).unwrap();
        let [a, b, c] = [(); 3].map(|()| as_core::CharacterId::new());
        for (n, (ch, hidden)) in [(a, 0), (b, 0), (c, 1)].into_iter().enumerate() {
            db.execute(
                "INSERT INTO characters (id, number, name, hidden, created_at) VALUES (?1, ?2, NULL, ?3, 0)",
                rusqlite::params![ch.0.to_string(), n as i64 + 1, hidden],
            )
            .unwrap();
        }
        let name = |character_id, anilist_id: u32, name: &str| NameSuggestion {
            character_id,
            anilist_id,
            name: name.into(),
            score: 0.9,
            image: None,
        };
        let waiting = Suggested {
            media: series.media.clone(),
            series: series.name.clone(),
            names: vec![name(a, 176754, "Frieren"), name(b, 176755, "Fern"), name(c, 176756, "Stark")],
            merges: vec![MergeProposal { from: c, into: a, anilist_id: 176754 }],
        };
        project.set_anilist_suggested(Some(&waiting)).unwrap();
        // The hidden character's name and merge don't wait.
        let back = lib.anilist_suggested(id).unwrap().unwrap();
        assert_eq!(back.names.iter().map(|n| n.character_id).collect::<Vec<_>>(), [a, b]);
        assert!(back.merges.is_empty());
        lib.anilist_dismiss(id).unwrap();
        assert!(lib.anilist_suggested(id).unwrap().is_none(), "said no to");
        // A name given meanwhile: only the other waits (the cast changing
        // doesn't make the rest go).
        project.set_anilist_suggested(Some(&waiting)).unwrap();
        project.cast_edit(&as_api::CastEdit::Rename { character_id: a, name: Some("Frieren".into()) }).unwrap();
        let back = lib.anilist_suggested(id).unwrap().unwrap();
        assert_eq!(back.names.iter().map(|n| n.character_id).collect::<Vec<_>>(), [b]);
        project.cast_edit(&as_api::CastEdit::Rename { character_id: a, name: None }).unwrap();
        // Another series confirmed: names from the old one's cast go.
        project.confirm_anilist_media(&[154587, 182255, 209939], false).unwrap();
        assert!(lib.anilist_suggested(id).unwrap().is_some(), "the same series again keeps them");
        project.set_anilist_ids(MEDIA, &[154587]).unwrap();
        assert!(lib.anilist_suggested(id).unwrap().is_none(), "made from another chain");
        project.confirm_anilist_media(&series.media, false).unwrap();

        // Given, but one left out: never suggested for them again.
        project.set_anilist_suggested(Some(&waiting)).unwrap();
        // Only what was shown and kept is given: a name no longer waiting
        // as shown (another portrait for b) isn't.
        assert_eq!(lib.anilist_accept(id, &[(a, 176754), (b, 999)], &[], &[(b, 176755)]).unwrap(), (0, 1));
        let named = |ch| project.db().characters().unwrap().into_iter().find(|r| r.id == ch).unwrap().name;
        assert_eq!((named(a).as_deref(), named(b)), (Some("Frieren"), None));
        assert!(project.db().meta(SUGGESTED).unwrap().is_none());
        assert_eq!(project.anilist_refused().unwrap(), [(b, 176755)]);
        project.set_anilist_suggested(Some(&waiting)).unwrap();
        assert!(lib.anilist_suggested(id).unwrap().is_none(), "Fern was left out for b, a is named");
        assert!(matches!(lib.anilist_accept(id, &[], &[], &[]), Err(ProjectError::NotReady(_))));
        // A merge left out doesn't come back; one resting on a name left out
        // doesn't wait either.
        let merging = Suggested {
            names: vec![],
            merges: vec![
                MergeProposal { from: b, into: a, anilist_id: 5 },
                MergeProposal { from: a, into: b, anilist_id: 176755 },
            ],
            ..waiting.clone()
        };
        project.set_anilist_suggested(Some(&merging)).unwrap();
        let back = lib.anilist_suggested(id).unwrap().unwrap();
        assert_eq!(back.merges.iter().map(|m| m.from).collect::<Vec<_>>(), [b], "b isn't Fern");
        assert_eq!(lib.anilist_accept(id, &[], &[], &[(b, 5)]).unwrap(), (0, 0));
        project.set_anilist_suggested(Some(&merging)).unwrap();
        assert!(lib.anilist_suggested(id).unwrap().is_none());

        // Hidden while the opt-in is off; a series turned down goes with
        // its names.
        let fresh = Suggested { names: vec![name(b, 1, "Himmel")], ..waiting.clone() };
        project.set_anilist_suggested(Some(&fresh)).unwrap();
        lib.update_settings(&AppSettings { anilist: false, ..lib.settings().unwrap() }).unwrap();
        assert!(lib.anilist_suggested(id).unwrap().is_none());
        lib.update_settings(&AppSettings { anilist: true, ..lib.settings().unwrap() }).unwrap();
        assert!(lib.anilist_suggested(id).unwrap().is_some());
        project.reject_anilist_media(154587).unwrap();
        assert!(lib.anilist_suggested(id).unwrap().is_none());
        assert!(!project.anilist_picked().unwrap());

        // A season of a series turned down isn't picked by the title.
        assert!(project.anilist_media().unwrap().is_empty());
        let cancel = lib.anilist_token();
        assert_eq!(lib.anilist_find_series(id, &cancel).unwrap(), None);
        assert!(project.anilist_media().unwrap().is_empty());
    }

    #[test]
    fn switching_it_off_stops_what_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.frieren();
        let lib = library(dir.path()).with_anilist(client);
        lib.update_settings(&AppSettings { anilist: true, ..lib.settings().unwrap() }).unwrap();
        let id = anime(&lib, "Sousou no Frieren");
        let running = lib.anilist_token();
        lib.update_settings(&AppSettings { anilist: false, ..lib.settings().unwrap() }).unwrap();
        lib.update_settings(&AppSettings { anilist: true, ..lib.settings().unwrap() }).unwrap();
        // Switched on again meanwhile: what ran before still stops.
        assert!(running.is_cancelled());
        assert!(matches!(lib.anilist_find_series(id, &running), Err(ProjectError::Cancelled)));
        assert!(net.bodies.lock().is_empty(), "nothing sent");
        assert!(lib.project(id).unwrap().anilist_media().unwrap().is_empty());
        // A new piece of work goes.
        assert!(lib.anilist_find_series(id, &lib.anilist_token()).unwrap().is_some());
        // Portraits are kept only while it is on.
        lib.anilist().keep_portrait(1, &[1.0], &running);
        assert_eq!(lib.anilist().portrait(1), None);
        lib.anilist().keep_portrait(1, &[1.0], &lib.anilist_token());
        assert_eq!(lib.anilist().portrait(1), Some(vec![1.0]));
        lib.update_settings(&AppSettings { anilist: false, ..lib.settings().unwrap() }).unwrap();
        assert_eq!(lib.anilist().portrait(1), None);
    }
}
