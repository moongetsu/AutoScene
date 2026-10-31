//! Naming animated characters from TVmaze (the same opt-in as AniList's
//! names, see [`crate::anilist`]): AniList knows only anime, TVmaze knows the
//! casts of animated series — Western ones too — with a picture of each
//! character. A project made as animation ([`Source::TvMaze`]) looks its
//! series up here the way an anime project does on AniList: a show is a
//! chain of one (its seasons share the cast), its characters all have the
//! same part (TVmaze doesn't say who leads). The pictures are matched with
//! the anime embedder against pictures of the project's characters (see
//! [`crate::Project::look_pictures`]): the faces the project was grouped by
//! can't tell cartoon characters apart.
//!
//! TVmaze's API is free, without a key; it allows 20 requests every 10
//! seconds from one address. Its data is CC BY-SA: the app says where the
//! names come from.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use as_core::CancelToken;
use parking_lot::Mutex;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::anilist::{
    ATTEMPTS, Character, Clock, Limiter, MAX_PAUSE, Media, Names, Reply, Role, SystemClock, Titles, Transport, Ureq,
    going, image_ext, store_picture,
};
use crate::{ProjectError, ProjectResult};

const API: &str = "https://api.tvmaze.com";
/// At least this long between two requests to the API: under the 20 every
/// 10 seconds TVmaze allows.
const API_GAP: Duration = Duration::from_millis(550);
/// Between two pictures from TVmaze's image server.
const IMAGE_GAP: Duration = Duration::from_millis(250);
/// The shortest pause after TVmaze says "too many requests" (it counts ten
/// seconds).
const MIN_PAUSE: Duration = Duration::from_secs(10);
/// Search results whose other titles (AKAs) are read: a show's English
/// title is often one of those (TVmaze names a French show in French).
const AKA_RESULTS: usize = 5;
/// The type of an animated show ([`Media::format`]); TVmaze lists
/// scripted, reality and other shows too.
pub const ANIMATION: &str = "Animation";

/// A show TVmaze answers with.
#[derive(Deserialize)]
struct RawShow {
    id: u32,
    name: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    premiered: Option<String>,
}

#[derive(Deserialize)]
struct SearchHit {
    show: Option<RawShow>,
}

#[derive(Deserialize)]
struct Aka {
    name: Option<String>,
    country: Option<Country>,
}

#[derive(Deserialize)]
struct Country {
    code: Option<String>,
}

#[derive(Deserialize)]
struct Credit {
    character: Option<RawCharacter>,
}

#[derive(Deserialize)]
struct RawCharacter {
    id: u32,
    name: Option<String>,
    image: Option<RawImage>,
}

#[derive(Deserialize)]
struct RawImage {
    medium: Option<String>,
    original: Option<String>,
}

/// A show as a series: its English other title (if it has one) to show,
/// its own and every other title to pick it by.
fn media(show: RawShow, akas: Vec<Aka>) -> Media {
    let nonempty = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let name = nonempty(show.name);
    let english = akas
        .iter()
        .find(|a| a.country.as_ref().and_then(|c| c.code.as_deref()).is_some_and(|c| c == "US" || c == "GB"))
        .and_then(|a| nonempty(a.name.clone()));
    let mut synonyms: Vec<String> = akas.into_iter().filter_map(|a| nonempty(a.name)).collect();
    synonyms.dedup();
    Media {
        id: show.id,
        id_mal: None,
        format: nonempty(show.kind),
        episodes: None,
        season_year: show.premiered.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()),
        title: Titles { english: english.or_else(|| name.clone()), romaji: None, native: name },
        synonyms,
        sequels: Vec::new(),
    }
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

/// Only TVmaze's own image server is asked for pictures, over https.
fn on_image_server(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    host == "static.tvmaze.com"
}

fn unreachable(e: impl std::fmt::Display) -> ProjectError {
    tracing::warn!("tvmaze: {e}");
    ProjectError::NotReady("TVmaze can't be reached right now. Check the internet connection and try again.".into())
}

fn busy() -> ProjectError {
    ProjectError::NotReady("TVmaze is busy. Try again in a few minutes.".into())
}

static API_LIMITER: LazyLock<Arc<Limiter>> = LazyLock::new(|| Arc::new(Limiter::new(API_GAP)));
static IMAGE_LIMITER: LazyLock<Arc<Limiter>> = LazyLock::new(|| Arc::new(Limiter::new(IMAGE_GAP)));

/// TVmaze for one library: its requests, the shows and casts it has read,
/// and the picture cache.
pub struct TvMaze {
    transport: Arc<dyn Transport>,
    clock: Arc<dyn Clock>,
    api: Arc<Limiter>,
    images: Arc<Limiter>,
    /// The library's character pictures (`<id>.<ext>`).
    dir: PathBuf,
    /// Shows this session has read, by id.
    seen: Mutex<HashMap<u32, Media>>,
    /// Each show's cast this session has read, by id.
    casts: Mutex<HashMap<u32, Vec<Character>>>,
    /// Character pictures embedded this session, by TVmaze character id.
    portraits: Mutex<HashMap<u32, Vec<f32>>>,
}

impl TvMaze {
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

    /// Where a character's picture is cached, once loaded.
    pub fn image_dir(&self) -> &Path {
        &self.dir
    }

    /// One GET of the API (`path` and query), its turn waited for, tried
    /// again while TVmaze asks to wait. `None`: TVmaze has no such thing.
    fn request<T: DeserializeOwned>(&self, path: &str, cancel: &CancelToken) -> ProjectResult<Option<T>> {
        let url = format!("{API}{path}");
        for _ in 0..ATTEMPTS {
            self.api.acquire(&*self.clock, cancel)?;
            let reply = self.transport.get(&url).map_err(unreachable)?;
            match reply.status {
                429 => {
                    let wait = reply.retry_after.unwrap_or_default().clamp(MIN_PAUSE, MAX_PAUSE);
                    tracing::info!(wait_s = wait.as_secs(), "tvmaze: too many requests, pausing");
                    self.api.pause(self.clock.now(), wait);
                }
                404 => return Ok(None),
                200 => {
                    return serde_json::from_slice(&reply.body)
                        .map(Some)
                        .map_err(|e| unreachable(format!("{path}: not TVmaze's answer: {e}")));
                }
                status => return Err(unreachable(format!("{path}: HTTP {status}"))),
            }
        }
        Err(busy())
    }

    /// Show `id`'s other titles (none when it has none).
    fn akas(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Vec<Aka>> {
        Ok(self.request(&format!("/shows/{id}/akas"), cancel)?.unwrap_or_default())
    }

    /// Shows matching `text`, best first, with their other titles (the
    /// first few's). `text` is sent to TVmaze as it is.
    pub fn search(&self, text: &str, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ProjectError::Invalid("Type the name of the series to look for.".into()));
        }
        let hits: Vec<SearchHit> =
            self.request(&format!("/search/shows?q={}", query_part(text)), cancel)?.unwrap_or_default();
        let mut found = Vec::new();
        for (i, show) in hits.into_iter().filter_map(|h| h.show).enumerate() {
            // Other titles that won't load leave the show its own.
            let akas = match (i < AKA_RESULTS).then(|| self.akas(show.id, cancel)) {
                Some(Err(ProjectError::Cancelled)) => return Err(ProjectError::Cancelled),
                Some(Ok(akas)) => akas,
                Some(Err(_)) | None => Vec::new(),
            };
            found.push(media(show, akas));
        }
        going(cancel)?;
        self.seen.lock().extend(found.iter().map(|m| (m.id, m.clone())));
        Ok(found)
    }

    /// Show `id` (read once a session).
    pub fn media(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Media> {
        if let Some(m) = self.seen.lock().get(&id) {
            return Ok(m.clone());
        }
        let show: RawShow = self.request(&format!("/shows/{id}"), cancel)?.ok_or_else(|| not_on_tvmaze(id))?;
        let media = media(show, self.akas(id, cancel)?);
        going(cancel)?;
        self.seen.lock().insert(id, media.clone());
        Ok(media)
    }

    /// Show `id`'s cast, in TVmaze's order, each character once (a part
    /// voiced by several people is listed once per voice).
    pub fn characters(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Vec<Character>> {
        if let Some(cast) = self.casts.lock().get(&id) {
            return Ok(cast.clone());
        }
        let credits: Vec<Credit> =
            self.request(&format!("/shows/{id}/cast"), cancel)?.ok_or_else(|| not_on_tvmaze(id))?;
        let mut out: Vec<Character> = Vec::new();
        for c in credits.into_iter().filter_map(|c| c.character) {
            // Of two names in one (`Marinette Dupain-Cheng / Ladybug`: her and
            // her other self, in one picture), the first: never both in one.
            let name = c.name.as_deref().and_then(|n| n.split('/').next()).map(str::trim).unwrap_or_default();
            let Some(name) = Some(name.to_string()).filter(|n| !n.is_empty()) else { continue };
            if out.iter().any(|o| o.id == c.id) {
                continue;
            }
            let image = c.image.and_then(|i| i.original.or(i.medium)).filter(|u| !u.trim().is_empty());
            // TVmaze doesn't say who leads: everyone has the same part.
            let kin = None;
            out.push(Character { id: c.id, role: Role::Main, name, native: None, alternative: Vec::new(), image, kin });
        }
        going(cancel)?;
        self.casts.lock().insert(id, out.clone());
        Ok(out)
    }

    /// A character's picture, from the cache or loaded into it (`None`: they
    /// have none, or it isn't on TVmaze's image server).
    pub fn image(&self, c: &Character, cancel: &CancelToken) -> ProjectResult<Option<PathBuf>> {
        let Some(url) = c.image.as_deref().filter(|u| on_image_server(u)) else { return Ok(None) };
        if let Some(cached) = self.cached(c.id) {
            return Ok(Some(cached));
        }
        let mut reply: Option<Reply> = None;
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
            return Err(unreachable(format!("picture of character {}: HTTP {}", c.id, reply.status)));
        }
        let Some(ext) = image_ext(url, reply.content_type.as_deref()) else {
            tracing::warn!(character = c.id, "tvmaze: the picture isn't a picture");
            return Ok(None);
        };
        Ok(Some(store_picture(&self.dir, c.id, ext, &reply.body, cancel)?))
    }

    /// A character's cached picture.
    pub fn cached(&self, id: u32) -> Option<PathBuf> {
        ["jpg", "png", "webp", "gif"].iter().map(|ext| self.dir.join(format!("{id}.{ext}"))).find(|p| p.is_file())
    }
}

fn not_on_tvmaze(id: u32) -> ProjectError {
    ProjectError::NotFound(format!("TVmaze has no show {id}."))
}

impl Names for TvMaze {
    fn search(&self, text: &str, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        TvMaze::search(self, text, cancel)
    }

    fn media(&self, id: u32, cancel: &CancelToken) -> ProjectResult<Media> {
        TvMaze::media(self, id, cancel)
    }

    /// A show is its whole chain: its seasons share its cast.
    fn chain(&self, root: u32, cancel: &CancelToken) -> ProjectResult<Vec<Media>> {
        Ok(vec![TvMaze::media(self, root, cancel)?])
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
        TvMaze::image(self, c, cancel)
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
        self.seen.lock().clear();
        self.casts.lock().clear();
        self.portraits.lock().clear();
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
    use as_core::{AppPaths, ProjectId};

    use super::*;
    use crate::Library;
    use crate::anilist::{Source, pick_series};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/tvmaze");

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(FIXTURES).join(name)).unwrap()
    }

    /// Time that only moves when slept.
    #[derive(Default)]
    struct FakeClock {
        now: Mutex<Option<Instant>>,
        slept: Mutex<Duration>,
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock().get_or_insert_with(Instant::now)
        }

        fn sleep(&self, d: Duration, _: &CancelToken) {
            let now = self.now();
            *self.now.lock() = Some(now + d);
            *self.slept.lock() += d;
        }
    }

    /// A piece of the URL, the reply (status, body), and whether it is used
    /// only once.
    type Canned = (String, u16, Vec<u8>, bool);

    /// Recorded answers by a piece of the URL (first match wins; `once`
    /// ones are used up), and every URL asked for.
    #[derive(Default)]
    struct Recorded {
        answers: Mutex<Vec<Canned>>,
        asked: Mutex<Vec<String>>,
        pictures: AtomicUsize,
    }

    impl Recorded {
        fn answer(&self, matching: &str, status: u16, body: Vec<u8>) {
            self.answers.lock().push((matching.into(), status, body, false));
        }

        fn miraculous(&self) {
            self.answer("/search/shows?q=", 200, fixture("search_miraculous.json"));
            self.answer("/shows/5557/akas", 200, fixture("akas_5557.json"));
            self.answer("/akas", 200, b"[]".to_vec());
            self.answer("/shows/5557/cast", 200, fixture("cast_5557.json"));
            self.answer("/shows/5557", 200, fixture("show_5557.json"));
        }
    }

    impl Transport for Recorded {
        fn post(&self, body: &str) -> Result<Reply, String> {
            Err(format!("TVmaze is never posted to: {body}"))
        }

        fn get(&self, url: &str) -> Result<Reply, String> {
            self.asked.lock().push(url.to_string());
            let json = Some("application/json".to_string());
            if on_image_server(url) {
                self.pictures.fetch_add(1, Ordering::Relaxed);
                let body = b"\xFF\xD8jpeg".to_vec();
                let content_type = Some("image/jpeg".into());
                return Ok(Reply { status: 200, retry_after: None, remaining: None, content_type, body });
            }
            assert!(url.starts_with("https://api.tvmaze.com/"), "{url}");
            let mut answers = self.answers.lock();
            let i = answers
                .iter()
                .position(|(m, ..)| url.contains(m.as_str()))
                .ok_or_else(|| format!("no answer for {url}"))?;
            let (_, status, body, once) = answers[i].clone();
            if once {
                answers.remove(i);
            }
            Ok(Reply { status, retry_after: None, remaining: None, content_type: json, body })
        }
    }

    fn client(dir: &Path) -> (TvMaze, Arc<Recorded>, Arc<FakeClock>) {
        let net = Arc::new(Recorded::default());
        let clock = Arc::new(FakeClock::default());
        let api = Arc::new(Limiter::new(API_GAP));
        let images = Arc::new(Limiter::new(IMAGE_GAP));
        (TvMaze::with(dir.join("tvmaze"), net.clone(), clock.clone(), api, images), net, clock)
    }

    #[test]
    fn a_search_reads_shows_with_their_other_titles() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.miraculous();
        let found = client.search("Miraculous: Tales & Co é", &CancelToken::new()).unwrap();
        assert_eq!(found.iter().map(|m| m.id).collect::<Vec<_>>(), [69077, 68577, 62953, 85305, 5557]);
        // The text went as it was, encoded; the first five's other titles were read.
        let asked = net.asked.lock().clone();
        assert_eq!(asked[0], "https://api.tvmaze.com/search/shows?q=Miraculous%3A%20Tales%20%26%20Co%20%C3%A9");
        assert_eq!(asked.len(), 6);
        // TVmaze names the French show in French: its English title shows,
        // every title picks it.
        let show = &found[4];
        assert_eq!(show.name(), "Miraculous: Tales of Ladybug and Cat Noir");
        assert_eq!(show.title.native.as_deref(), Some("Miraculous: Les aventures de Ladybug et Chat Noir"));
        assert_eq!((show.format.as_deref(), show.season_year, show.episodes), (Some("Animation"), Some(2015), None));
        assert!(show.synonyms.iter().any(|s| s == "Miraculous: Tales of Ladybug & Cat Noir"));
        assert!(show.sequels.is_empty());
        assert_eq!(found[0].name(), "Miraculous Brothers", "no other title: its own");
        // Picked by a title of it, not by the franchise's first word.
        let picked = |title: &str| pick_series(title, &found).map(|m| m.id);
        assert_eq!(picked("Miraculous.Tales.of.Ladybug.and.Cat.Noir.S01E03.1080p"), Some(5557));
        assert_eq!(picked("Miraculous: Les aventures de Ladybug et Chat Noir"), Some(5557));
        assert_eq!(picked("Miraculous Awakening"), None);
        // Read once a session; a show is its own chain.
        assert_eq!(Names::chain(&client, 5557, &CancelToken::new()).unwrap(), [found[4].clone()]);
        assert_eq!(net.asked.lock().len(), 6);
        assert!(matches!(client.search("  ", &CancelToken::new()), Err(ProjectError::Invalid(_))));
    }

    #[test]
    fn a_show_not_searched_for_is_read_with_its_titles() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.answer("/shows/1", 404, b"{}".to_vec());
        net.miraculous();
        let show = client.media(5557, &CancelToken::new()).unwrap();
        assert_eq!(show.name(), "Miraculous: Tales of Ladybug and Cat Noir");
        assert_eq!(net.asked.lock().len(), 2, "the show and its titles");
        assert!(matches!(client.media(1, &CancelToken::new()), Err(ProjectError::NotFound(_))));
    }

    #[test]
    fn a_cast_lists_each_character_once_with_their_picture() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.miraculous();
        let cast = Names::cast(&client, &[5557, 5557], &CancelToken::new()).unwrap();
        let names: Vec<&str> = cast.iter().map(|c| c.name.as_str()).collect();
        // Voiced by three, listed once; a character without a name left out.
        let want = ["Marinette Dupain-Cheng", "Adrien Agreste", "Gabriel Agreste"];
        assert_eq!(names[..3], want);
        assert_eq!(names[3..], ["Sabrina Raincomprix", "Tikki", "Copycat"]);
        assert!(cast.iter().all(|c| c.role == Role::Main), "TVmaze doesn't say who leads");
        assert_eq!(cast[0].id, 1063644);
        let picture = "https://static.tvmaze.com/uploads/images/original_untouched/535/1338734.jpg";
        assert_eq!(cast[0].image.as_deref(), Some(picture));
        assert_eq!(net.asked.lock().len(), 1, "read once a session");
    }

    #[test]
    fn pictures_come_only_from_tvmazes_image_server_and_go_with_the_opt_in() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.miraculous();
        let marinette = client.characters(5557, &CancelToken::new()).unwrap().remove(0);
        let path = client.image(&marinette, &CancelToken::new()).unwrap().unwrap();
        assert_eq!(path, dir.path().join("tvmaze").join("1063644.jpg"));
        assert_eq!(std::fs::read(&path).unwrap(), b"\xFF\xD8jpeg");
        assert_eq!(client.image(&marinette, &CancelToken::new()).unwrap(), Some(path.clone()));
        assert_eq!(net.pictures.load(Ordering::Relaxed), 1, "loaded once");
        let elsewhere = [
            "https://example.com/1.jpg",
            "http://static.tvmaze.com/1.jpg",
            "https://static.tvmaze.com.example.com/1.jpg",
        ];
        for url in elsewhere {
            let c = Character { id: 2, image: Some(url.into()), ..marinette.clone() };
            assert_eq!(client.image(&c, &CancelToken::new()).unwrap(), None, "{url}");
        }
        assert_eq!(net.pictures.load(Ordering::Relaxed), 1);
        Names::keep_portrait(&client, 1063644, &[1.0], &CancelToken::new());
        assert_eq!(Names::portrait(&client, 1063644), Some(vec![1.0]));
        let off = CancelToken::new();
        off.cancel();
        Names::keep_portrait(&client, 7, &[1.0], &off);
        assert_eq!(Names::portrait(&client, 7), None, "not once switched off");
        Names::forget(&client).unwrap();
        assert!(!path.exists());
        assert_eq!(Names::portrait(&client, 1063644), None);
    }

    #[test]
    fn too_many_requests_wait_their_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, clock) = client(dir.path());
        net.answers.lock().push(("/shows/5557/cast".into(), 429, b"{}".to_vec(), true));
        net.miraculous();
        assert_eq!(client.characters(5557, &CancelToken::new()).unwrap().len(), 6);
        assert!(*clock.slept.lock() >= MIN_PAUSE, "paused after the 429");
        // Every answer a 429: busy, not an endless wait.
        let (client, net, _) = self::client(dir.path());
        net.answer("/shows/5557", 429, b"{}".to_vec());
        assert!(matches!(client.media(5557, &CancelToken::new()), Err(ProjectError::NotReady(_))));
        // A page that isn't TVmaze's answer.
        let (client, net, _) = self::client(dir.path());
        net.answer("/shows/5557", 200, b"<html>".to_vec());
        assert!(matches!(client.media(5557, &CancelToken::new()), Err(ProjectError::NotReady(_))));
    }

    fn library(dir: &Path, tvmaze: TvMaze) -> Library {
        let lib = Library::open(&AppPaths::at(dir.join("data"))).unwrap().with_tvmaze(tvmaze);
        let s = lib.settings().unwrap();
        let root = dir.join("Library").display().to_string();
        lib.update_settings(&AppSettings { library_root: root, anilist: true, ..s }).unwrap();
        lib
    }

    fn project(lib: &Library, title: &str, content: ContentKind) -> ProjectId {
        let new = NewProject { title: title.into(), kind: ProjectKind::Series, content, files: vec![] };
        lib.create_project(&new).unwrap().0.id
    }

    /// One unnamed character in project `id`.
    fn someone(lib: &Library, id: ProjectId) {
        let db = rusqlite::Connection::open(lib.project(id).unwrap().dir.join(crate::project::PROJECT_DB)).unwrap();
        db.execute(
            "INSERT INTO characters (id, number, name, hidden, created_at) VALUES (?1, 1, NULL, 0, 0)",
            rusqlite::params![as_core::CharacterId::new().0.to_string()],
        )
        .unwrap();
    }

    #[test]
    fn an_animation_project_is_named_from_tvmaze_keeping_its_decisions_apart() {
        let (series, film) = (ProjectKind::Series, ProjectKind::Movie);
        assert_eq!(Source::of(ContentKind::Animation, series), Some(Source::TvMaze));
        assert_eq!(Source::of(ContentKind::Animation, film), Some(Source::Fandom));
        assert_eq!(Source::of(ContentKind::Anime, series), Some(Source::AniList));
        assert_eq!(Source::of(ContentKind::Anime, film), Some(Source::AniList));
        assert_eq!((Source::of(ContentKind::Live, film), Source::of(ContentKind::Mixed, series)), (None, None));
        // Only anime's own faces were embedded the way the portraits are.
        assert!(Source::AniList.faces_are_looks());
        assert!(!Source::TvMaze.faces_are_looks() && !Source::Fandom.faces_are_looks());
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.miraculous();
        let lib = library(dir.path(), client);
        let id = project(&lib, "Miraculous Tales of Ladybug and Cat Noir S02", ContentKind::Animation);
        let live = project(&lib, "Breaking Bad", ContentKind::Live);
        assert_eq!(lib.name_source(id).unwrap(), Source::TvMaze);
        assert!(!lib.anilist_wanted(id).unwrap(), "no one to name yet");
        someone(&lib, id);
        someone(&lib, live);
        assert!(lib.anilist_wanted(id).unwrap());
        assert!(!lib.anilist_wanted(live).unwrap(), "live action has no source");

        // Found by the title, kept in TVmaze's own meta.
        let cancel = lib.anilist_token();
        let series = lib.anilist_find_series(id, &cancel).unwrap().unwrap();
        assert_eq!(series.name, "Miraculous: Tales of Ladybug and Cat Noir");
        assert_eq!((series.media.as_slice(), series.picked), (&[5557][..], true));
        let p = lib.project(id).unwrap();
        assert_eq!(p.anilist_media().unwrap(), [5557]);
        assert_eq!(p.db().meta("tvmaze.media").unwrap().as_deref(), Some("[5557]"));
        assert_eq!(p.db().meta(crate::anilist::MEDIA).unwrap(), None, "AniList's decisions untouched");
        assert!(net.asked.lock()[0].ends_with("q=Miraculous%20Tales%20of%20Ladybug%20and%20Cat%20Noir"));

        // Its cast with their pictures, in TVmaze's own cache.
        let (source, cast) = lib.anilist_cast(id, &cancel).unwrap();
        assert_eq!((source, cast.len()), (Source::TvMaze, 6));
        let cache = dir.path().join("tvmaze");
        assert!(cast.iter().all(|m| m.image.as_ref().is_some_and(|p| p.starts_with(&cache))));
        // Turned down: the search by hand leaves it out, the title no longer picks it.
        p.reject_anilist_media(5557).unwrap();
        assert!(p.anilist_media().unwrap().is_empty());
        assert_eq!(p.db().meta("tvmaze.rejected").unwrap().as_deref(), Some("[5557]"));
        assert!(lib.anilist_search(id, "Miraculous", &cancel).unwrap().iter().all(|m| m.id != 5557));
        assert_eq!(lib.anilist_find_series(id, &cancel).unwrap(), None);

        // Live action now: none of it applies, nothing is asked or kept.
        let db = rusqlite::Connection::open(p.dir.join(crate::project::PROJECT_DB)).unwrap();
        db.execute("UPDATE meta SET value = 'live' WHERE key = 'content'", []).unwrap();
        assert!(!lib.anilist_wanted(id).unwrap());
        assert!(matches!(p.anilist_rejected(), Err(ProjectError::Invalid(_))));
        assert!(matches!(lib.name_source(id), Err(ProjectError::Invalid(_))));
        let asked = net.asked.lock().len();
        assert!(matches!(lib.anilist_confirm(id, 5557, &cancel), Err(ProjectError::Invalid(_))));
        assert!(matches!(lib.anilist_search(id, "Miraculous", &cancel), Err(ProjectError::Invalid(_))));
        assert!(matches!(lib.anilist_cast(id, &cancel), Err(ProjectError::Invalid(_))));
        assert_eq!(net.asked.lock().len(), asked);
        assert_eq!(p.db().meta(crate::anilist::MEDIA).unwrap(), None);
        // Switched off: TVmaze's pictures go too.
        lib.update_settings(&AppSettings { anilist: false, ..lib.settings().unwrap() }).unwrap();
        assert!(!cache.exists());
    }

    #[test]
    fn only_an_animated_series_is_picked_by_the_title() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.miraculous();
        let lib = library(dir.path(), client);
        let cancel = lib.anilist_token();
        // TVmaze's "Miraculous Brothers" is a Korean drama, not animation.
        let drama = project(&lib, "Miraculous Brothers", ContentKind::Animation);
        assert_eq!(lib.anilist_find_series(drama, &cancel).unwrap(), None);
        assert!(lib.project(drama).unwrap().anilist_media().unwrap().is_empty());
        // An animated film isn't TVmaze's (it lists series only).
        let new = NewProject {
            title: "Miraculous: Tales of Ladybug and Cat Noir".into(),
            kind: ProjectKind::Movie,
            content: ContentKind::Animation,
            files: vec![],
        };
        let film = lib.create_project(&new).unwrap().0.id;
        assert_eq!(lib.name_source(film).unwrap(), Source::Fandom);
    }

    #[test]
    fn other_titles_that_wont_load_leave_a_show_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let (client, net, _) = client(dir.path());
        net.answer("/shows/5557/akas", 500, b"{}".to_vec());
        net.miraculous();
        let found = client.search("Miraculous", &CancelToken::new()).unwrap();
        assert_eq!(found[4].name(), "Miraculous: Les aventures de Ladybug et Chat Noir");
        assert!(found[4].synonyms.is_empty());
    }

    #[test]
    fn query_text_is_percent_encoded() {
        assert_eq!(query_part("Abc-1.2_x~"), "Abc-1.2_x~");
        assert_eq!(query_part("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }
}
