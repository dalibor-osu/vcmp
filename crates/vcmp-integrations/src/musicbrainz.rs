//! Album covers from MusicBrainz and the Cover Art Archive, for albums whose files
//! carry no MusicBrainz release ID. The Discord presence shows them; Discord fetches
//! the image itself.
//!
//! Lookups run one at a time on a worker thread. Requests are spaced at least a
//! second apart, as MusicBrainz asks, and carry VCMP's User-Agent. Matching is
//! strict, because a wrong cover is worse than none: the search score must be high,
//! the titles must agree once normalised, and the credited artist must contain the
//! album artist (or the other way round).

use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use unicode_normalization::UnicodeNormalization;

pub const USER_AGENT: &str = concat!(
    "VCMP/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/dalibor-osu/vcmp)"
);

const SEARCH_URL: &str = "https://musicbrainz.org/ws/2";
/// MusicBrainz allows one request a second; the margin keeps clock jitter under it.
const INTERVAL: Duration = Duration::from_millis(1100);
/// How often a request answered "busy" (503) is tried again, waiting longer each time.
const BUSY_RETRIES: u32 = 3;
/// Search scores run to 100; below this the hit is too unsure to show.
const MIN_SCORE: u32 = 90;
/// How many matching results are checked for a cover before giving up.
const CANDIDATES: usize = 3;

/// The album to look up, as the library knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumQuery {
    pub artist: String,
    pub title: String,
    /// Tracks the library has; a matching release has at least as many.
    pub track_count: u32,
}

/// A front cover on the Cover Art Archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cover {
    /// The cover of one release, by its MBID.
    Release(String),
    /// The cover the archive picks for a release group, by its MBID.
    ReleaseGroup(String),
}

impl Cover {
    /// The 250-pixel image, which is what Discord shows.
    pub fn url(&self) -> String {
        match self {
            Cover::Release(id) => format!("https://coverartarchive.org/release/{id}/front-250"),
            Cover::ReleaseGroup(id) => {
                format!("https://coverartarchive.org/release-group/{id}/front-250")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Found(Cover),
    /// Nothing matched well enough, or no match has a cover.
    NotFound,
    /// The services could not be asked (offline, an error); worth trying later.
    Failed(String),
}

/// The HTTP side of a lookup, replaced in tests.
pub trait Transport: Send + 'static {
    /// GETs `url` with `query` parameters, returning the status and the body.
    fn get(&mut self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String), String>;
    /// HEADs `url` without following redirects, returning the status.
    fn head(&mut self, url: &str) -> Result<u16, String>;
}

/// The real network.
pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .into();
        Self { agent }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for Http {
    fn get(&mut self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String), String> {
        let mut request = self.agent.get(url).header("Accept", "application/json");
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let mut response = request.call().map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| error.to_string())?;
        Ok((status, body))
    }

    fn head(&mut self, url: &str) -> Result<u16, String> {
        let response = self
            .agent
            .head(url)
            .call()
            .map_err(|error| error.to_string())?;
        Ok(response.status().as_u16())
    }
}

/// Looks albums up through a transport, spacing its requests.
pub struct Searcher<T> {
    transport: T,
    interval: Duration,
    last_request: Option<Instant>,
}

impl Searcher<Http> {
    /// Searches the real services at the pace MusicBrainz allows.
    pub fn online() -> Self {
        Self::new(Http::new(), INTERVAL)
    }
}

impl<T: Transport> Searcher<T> {
    pub fn new(transport: T, interval: Duration) -> Self {
        Self {
            transport,
            interval,
            last_request: None,
        }
    }

    pub fn find(&mut self, album: &AlbumQuery) -> Lookup {
        match self.search(album) {
            Ok(Some(cover)) => Lookup::Found(cover),
            Ok(None) => Lookup::NotFound,
            Err(error) => Lookup::Failed(error),
        }
    }

    /// Releases as tagged, then with the title and artist simplified, then release
    /// groups; the first match with a cover wins.
    fn search(&mut self, album: &AlbumQuery) -> Result<Option<Cover>, String> {
        if normalize(&album.title).is_empty() || normalize(&album.artist).is_empty() {
            return Ok(None);
        }
        let simple = AlbumQuery {
            artist: main_artist(&album.artist),
            title: plain_title(&album.title),
            track_count: album.track_count,
        };
        let mut tries = vec![album.clone()];
        if simple != *album {
            tries.push(simple.clone());
        }
        let mut checked = Vec::new();
        for query in &tries {
            let found: ReleaseSearch = self.ask("release", &search_query("release", query))?;
            let matches = found.releases.into_iter().filter(|release| {
                matches(query, release.score, &release.title, &release.artist_credit)
                    && release
                        .track_count
                        .is_none_or(|count| count >= query.track_count)
            });
            for release in matches.take(CANDIDATES) {
                let group = release
                    .release_group
                    .map(|group| Cover::ReleaseGroup(group.id));
                for cover in [Some(Cover::Release(release.id)), group]
                    .into_iter()
                    .flatten()
                {
                    if self.has_cover(&cover, &mut checked)? {
                        return Ok(Some(cover));
                    }
                }
            }
        }
        let found: GroupSearch =
            self.ask("release-group", &search_query("releasegroup", &simple))?;
        let matches = found
            .release_groups
            .into_iter()
            .filter(|group| matches(&simple, group.score, &group.title, &group.artist_credit));
        for group in matches.take(CANDIDATES) {
            let cover = Cover::ReleaseGroup(group.id);
            if self.has_cover(&cover, &mut checked)? {
                return Ok(Some(cover));
            }
        }
        Ok(None)
    }

    /// Runs a MusicBrainz search, waiting and trying again while it is busy.
    fn ask<R: DeserializeOwned>(&mut self, entity: &str, query: &str) -> Result<R, String> {
        let url = format!("{SEARCH_URL}/{entity}/");
        for attempt in 1..=BUSY_RETRIES + 1 {
            self.wait_turn();
            let (status, body) = self
                .transport
                .get(&url, &[("query", query), ("fmt", "json"), ("limit", "10")])?;
            match status {
                200 => {
                    return serde_json::from_str(&body)
                        .map_err(|error| format!("unexpected answer from MusicBrainz: {error}"));
                }
                503 => thread::sleep(self.interval * 2 * attempt),
                other => return Err(format!("MusicBrainz answered {other}")),
            }
        }
        Err("MusicBrainz stayed busy".into())
    }

    /// Whether the archive has a front image; it redirects to the image when it does.
    fn has_cover(&mut self, cover: &Cover, checked: &mut Vec<Cover>) -> Result<bool, String> {
        if checked.contains(cover) {
            return Ok(false);
        }
        checked.push(cover.clone());
        self.wait_turn();
        match self.transport.head(&cover.url())? {
            200 | 302 | 307 => Ok(true),
            404 => Ok(false),
            other => Err(format!("the Cover Art Archive answered {other}")),
        }
    }

    fn wait_turn(&mut self) {
        if let Some(last) = self.last_request {
            let next = last + self.interval;
            let now = Instant::now();
            if next > now {
                thread::sleep(next - now);
            }
        }
        self.last_request = Some(Instant::now());
    }
}

/// Looks albums up on a worker thread, one at a time, and hands each outcome with
/// its key to `on_result` (on the worker thread). Requests asked for while another
/// runs wait their turn. Dropping the finder lets the worker finish its current
/// lookup and end.
pub struct CoverFinder<K> {
    jobs: Sender<(K, AlbumQuery)>,
}

impl<K: Send + 'static> CoverFinder<K> {
    pub fn start(on_result: impl Fn(K, Lookup) + Send + 'static) -> Self {
        Self::with_searcher(Searcher::online(), on_result)
    }

    pub fn with_searcher<T: Transport>(
        mut searcher: Searcher<T>,
        on_result: impl Fn(K, Lookup) + Send + 'static,
    ) -> Self {
        let (jobs, inbox) = mpsc::channel::<(K, AlbumQuery)>();
        thread::Builder::new()
            .name("vcmp-cover-lookup".into())
            .spawn(move || {
                for (key, album) in inbox {
                    let lookup = searcher.find(&album);
                    on_result(key, lookup);
                }
            })
            .expect("spawning the cover lookup thread");
        Self { jobs }
    }

    pub fn find(&self, key: K, album: AlbumQuery) {
        let _ = self.jobs.send((key, album));
    }
}

#[derive(Deserialize)]
struct ReleaseSearch {
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    id: String,
    title: String,
    #[serde(default)]
    score: u32,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<Credit>,
    #[serde(rename = "track-count")]
    track_count: Option<u32>,
    #[serde(rename = "release-group")]
    release_group: Option<GroupRef>,
}

#[derive(Deserialize)]
struct GroupRef {
    id: String,
}

#[derive(Deserialize)]
struct GroupSearch {
    #[serde(rename = "release-groups", default)]
    release_groups: Vec<ReleaseGroup>,
}

#[derive(Deserialize)]
struct ReleaseGroup {
    id: String,
    title: String,
    #[serde(default)]
    score: u32,
    #[serde(rename = "artist-credit", default)]
    artist_credit: Vec<Credit>,
}

#[derive(Deserialize)]
struct Credit {
    name: String,
    #[serde(default)]
    joinphrase: String,
}

fn matches(query: &AlbumQuery, score: u32, title: &str, credit: &[Credit]) -> bool {
    let credit: String = credit
        .iter()
        .map(|part| format!("{}{}", part.name, part.joinphrase))
        .collect();
    let (artist, credit) = (normalize(&query.artist), normalize(&credit));
    score >= MIN_SCORE
        && normalize(title) == normalize(&query.title)
        && !credit.is_empty()
        && (credit.contains(&artist) || artist.contains(&credit))
}

/// `release:"title" AND artist:"artist"`, with Lucene's special characters escaped.
fn search_query(field: &str, album: &AlbumQuery) -> String {
    format!(
        r#"{field}:"{}" AND artist:"{}""#,
        escape(&album.title),
        escape(&album.artist)
    )
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if r#"+-&|!(){}[]^"~*?:\/"#.contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// Compatibility-normalised, lowercased, letters and digits only: "Ⅱ" and "II",
/// "ＳＴＡＲ☆" and "Star" compare equal.
fn normalize(text: &str) -> String {
    text.nfkc()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// The title without trailing notes in brackets: "Album (Deluxe Edition)",
/// "Single【初回盤】" and "Album [Remastered]" become "Album" and "Single".
fn plain_title(title: &str) -> String {
    const PAIRS: [(char, char); 5] = [
        ('(', ')'),
        ('[', ']'),
        ('（', '）'),
        ('【', '】'),
        ('［', '］'),
    ];
    let mut plain = title.trim();
    while let Some(last) = plain.chars().last() {
        let Some((open, _)) = PAIRS.iter().find(|(_, close)| *close == last) else {
            break;
        };
        match plain.rfind(*open) {
            Some(start) if !plain[..start].trim().is_empty() => plain = plain[..start].trim_end(),
            _ => break,
        }
    }
    plain.to_string()
}

/// The first of several credited artists: "A, B", "A & B", "A feat. B" become "A".
fn main_artist(artist: &str) -> String {
    let lower = artist.to_ascii_lowercase();
    let end = [",", ";", "&", " feat.", " feat ", " ft.", " ft "]
        .iter()
        .filter_map(|separator| lower.find(separator))
        .min()
        .unwrap_or(artist.len());
    let main = artist[..end].trim();
    if main.is_empty() { artist.trim() } else { main }.to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Each request made, and when.
    type Requests = Arc<Mutex<Vec<(String, Instant)>>>;

    /// Answers requests from a script, in order, and records them.
    struct Scripted {
        answers: VecDeque<Result<(u16, String), String>>,
        requests: Requests,
    }

    impl Scripted {
        fn new(
            answers: impl IntoIterator<Item = Result<(u16, String), String>>,
        ) -> (Self, Requests) {
            let requests = Arc::default();
            let transport = Self {
                answers: answers.into_iter().collect(),
                requests: Arc::clone(&requests),
            };
            (transport, requests)
        }

        fn answer(&mut self, request: String) -> Result<(u16, String), String> {
            self.requests
                .lock()
                .unwrap()
                .push((request.clone(), Instant::now()));
            self.answers
                .pop_front()
                .unwrap_or_else(|| panic!("no answer scripted for {request}"))
        }
    }

    impl Transport for Scripted {
        fn get(&mut self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String), String> {
            let query = query
                .iter()
                .find(|(key, _)| *key == "query")
                .map_or("", |(_, value)| value);
            self.answer(format!("GET {url} {query}"))
        }

        fn head(&mut self, url: &str) -> Result<u16, String> {
            self.answer(format!("HEAD {url}")).map(|(status, _)| status)
        }
    }

    fn album(artist: &str, title: &str, tracks: u32) -> AlbumQuery {
        AlbumQuery {
            artist: artist.into(),
            title: title.into(),
            track_count: tracks,
        }
    }

    fn release(id: &str, title: &str, artist: &str, score: u32, tracks: u32) -> String {
        format!(
            r#"{{"id":"{id}","title":"{title}","score":{score},"track-count":{tracks},
                "artist-credit":[{{"name":"{artist}","joinphrase":""}}],
                "release-group":{{"id":"group-of-{id}"}}}}"#
        )
    }

    fn releases(items: &[String]) -> Result<(u16, String), String> {
        Ok((200, format!(r#"{{"releases":[{}]}}"#, items.join(","))))
    }

    fn groups(items: &[(&str, &str, &str, u32)]) -> Result<(u16, String), String> {
        let items: Vec<String> = items
            .iter()
            .map(|(id, title, artist, score)| {
                format!(
                    r#"{{"id":"{id}","title":"{title}","score":{score},
                        "artist-credit":[{{"name":"{artist}"}}]}}"#
                )
            })
            .collect();
        Ok((
            200,
            format!(r#"{{"release-groups":[{}]}}"#, items.join(",")),
        ))
    }

    fn status(code: u16) -> Result<(u16, String), String> {
        Ok((code, String::new()))
    }

    fn find(
        album: &AlbumQuery,
        answers: impl IntoIterator<Item = Result<(u16, String), String>>,
    ) -> (Lookup, Vec<String>) {
        let (transport, requests) = Scripted::new(answers);
        let lookup = Searcher::new(transport, Duration::ZERO).find(album);
        let requests = requests
            .lock()
            .unwrap()
            .iter()
            .map(|(r, _)| r.clone())
            .collect();
        (lookup, requests)
    }

    #[test]
    fn a_matching_release_with_a_cover_is_found() {
        let (lookup, requests) = find(
            &album("Koven", "Butterfly Effect", 13),
            [
                releases(&[release("r1", "Butterfly Effect", "Koven", 100, 13)]),
                status(307),
            ],
        );
        assert_eq!(lookup, Lookup::Found(Cover::Release("r1".into())));
        assert_eq!(
            requests,
            [
                r#"GET https://musicbrainz.org/ws/2/release/ release:"Butterfly Effect" AND artist:"Koven""#,
                "HEAD https://coverartarchive.org/release/r1/front-250",
            ]
        );
    }

    #[test]
    fn unsure_or_different_results_are_not_matches() {
        let (lookup, requests) = find(
            &album("Koven", "Butterfly Effect", 13),
            [
                releases(&[
                    release("low", "Butterfly Effect", "Koven", 80, 13),
                    release("title", "Butterfly Effect II", "Koven", 100, 13),
                    release("artist", "Butterfly Effect", "Someone Else", 100, 13),
                    release("short", "Butterfly Effect", "Koven", 100, 4),
                ]),
                groups(&[("g-low", "Butterfly Effect", "Koven", 50)]),
            ],
        );
        assert_eq!(lookup, Lookup::NotFound);
        assert_eq!(requests.len(), 2, "no cover was checked: {requests:?}");
    }

    #[test]
    fn a_release_without_art_falls_back_to_its_group() {
        let (lookup, _) = find(
            &album("μ's", "これから", 2),
            [
                releases(&[release("r1", "これから", "μ's", 100, 2)]),
                status(404),
                status(307),
            ],
        );
        assert_eq!(
            lookup,
            Lookup::Found(Cover::ReleaseGroup("group-of-r1".into()))
        );
    }

    #[test]
    fn edition_notes_and_extra_artists_are_dropped_on_the_second_try() {
        let (lookup, requests) = find(
            &album("Neko Hacker, Someone", "Stray (Deluxe Edition)", 14),
            [
                releases(&[]),
                releases(&[release("r1", "Stray", "Neko Hacker", 95, 14)]),
                status(307),
            ],
        );
        assert_eq!(lookup, Lookup::Found(Cover::Release("r1".into())));
        assert!(
            requests[1].ends_with(r#"release:"Stray" AND artist:"Neko Hacker""#),
            "{}",
            requests[1]
        );
    }

    #[test]
    fn release_groups_are_the_last_resort() {
        let (lookup, requests) = find(
            &album("Joe Ford", "DIVERGENCE II", 32),
            [
                releases(&[]),
                groups(&[("g1", "Divergence II", "Joe Ford", 100)]),
                status(307),
            ],
        );
        assert_eq!(lookup, Lookup::Found(Cover::ReleaseGroup("g1".into())));
        assert!(
            requests[1].contains("release-group/ releasegroup:"),
            "{}",
            requests[1]
        );
    }

    #[test]
    fn a_busy_musicbrainz_is_asked_again() {
        let (lookup, _) = find(
            &album("Koven", "Butterfly Effect", 13),
            [
                status(503),
                releases(&[release("r1", "Butterfly Effect", "Koven", 100, 13)]),
                status(307),
            ],
        );
        assert_eq!(lookup, Lookup::Found(Cover::Release("r1".into())));
    }

    #[test]
    fn network_errors_fail_the_lookup() {
        let (lookup, _) = find(
            &album("Koven", "Butterfly Effect", 13),
            [Err("offline".to_string())],
        );
        assert_eq!(lookup, Lookup::Failed("offline".into()));
    }

    #[test]
    fn a_name_of_symbols_only_is_not_searched() {
        let (lookup, requests) = find(&album("!!!", "...", 1), []);
        assert_eq!((lookup, requests.len()), (Lookup::NotFound, 0));
    }

    #[test]
    fn requests_are_spaced() {
        let interval = Duration::from_millis(40);
        let (transport, requests) = Scripted::new([
            releases(&[release("r1", "Album", "Band", 100, 1)]),
            status(404),
            status(307),
        ]);
        Searcher::new(transport, interval).find(&album("Band", "Album", 1));
        let times: Vec<Instant> = requests.lock().unwrap().iter().map(|(_, at)| *at).collect();
        assert_eq!(times.len(), 3);
        for pair in times.windows(2) {
            assert!(pair[1] - pair[0] >= interval, "{:?}", pair[1] - pair[0]);
        }
    }

    #[test]
    fn the_finder_answers_on_its_thread_with_the_key() {
        let (transport, _) = Scripted::new([
            releases(&[release("r1", "Album", "Band", 100, 1)]),
            status(307),
        ]);
        let (sender, results) = mpsc::channel();
        let finder = CoverFinder::with_searcher(
            Searcher::new(transport, Duration::ZERO),
            move |key, lookup| {
                sender.send((key, lookup)).unwrap();
            },
        );
        finder.find(7, album("Band", "Album", 1));
        assert_eq!(
            results.recv_timeout(Duration::from_secs(5)).unwrap(),
            (7, Lookup::Found(Cover::Release("r1".into())))
        );
    }

    #[test]
    fn names_compare_by_letters_and_digits() {
        assert_eq!(
            normalize("Neko Hacker Ⅱ: Stray"),
            normalize("neko hacker II stray")
        );
        assert_eq!(normalize("THE BRILLIANT STAR☆"), "thebrilliantstar");
        assert_eq!(normalize("ビタミンSUMMER！"), normalize("ビタミンsummer!"));
    }

    #[test]
    fn titles_lose_trailing_notes_only() {
        assert_eq!(
            plain_title("Butterfly Effect (Deluxe Edition)"),
            "Butterfly Effect"
        );
        assert_eq!(
            plain_title("Chance Day, Chance Way！【第8話盤】"),
            "Chance Day, Chance Way！"
        );
        assert_eq!(plain_title("Album (Deluxe) [Remastered]"), "Album");
        assert_eq!(plain_title("A (B) C"), "A (B) C");
        assert_eq!(plain_title("(What's the Story)"), "(What's the Story)");
    }

    #[test]
    fn the_main_artist_comes_first() {
        assert_eq!(main_artist("Neko Hacker, Neko Hacker"), "Neko Hacker");
        assert_eq!(main_artist("A & B"), "A");
        assert_eq!(main_artist("A feat. B"), "A");
        assert_eq!(main_artist("μ's"), "μ's");
        assert_eq!(main_artist(", B"), ", B");
    }

    #[test]
    fn lucene_characters_are_escaped() {
        assert_eq!(
            search_query("release", &album("AC/DC", "Who Made Who?", 9)),
            r#"release:"Who Made Who\?" AND artist:"AC\/DC""#
        );
    }

    #[test]
    fn covers_come_from_the_cover_art_archive() {
        assert_eq!(
            Cover::Release("abc".into()).url(),
            "https://coverartarchive.org/release/abc/front-250"
        );
        assert_eq!(
            Cover::ReleaseGroup("def".into()).url(),
            "https://coverartarchive.org/release-group/def/front-250"
        );
    }
}
