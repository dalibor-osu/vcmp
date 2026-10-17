//! Album covers from MusicBrainz and the Cover Art Archive, for albums whose files
//! carry no MusicBrainz release ID or no art. The Discord presence shows the first
//! match ([`CoverFinder`]; Discord fetches the image itself). "Find cover online" on
//! an album page offers every match with a cover and downloads the one the user
//! picks ([`CoverPicker`]).
//!
//! Lookups run one at a time on a worker thread. Requests are spaced at least a
//! second apart, as MusicBrainz asks, also across the two workers, and carry VCMP's
//! User-Agent. Matching is strict, because a wrong cover is worse than none: the
//! search score must be high, the titles must agree once normalised, and the
//! credited artist must contain the album artist (or the other way round).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
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
/// The most covers the picker offers.
const PICKER_COVERS: usize = 8;
/// The most images fetched while looking for the picker's covers, so that a search
/// ends within about fifteen seconds at MusicBrainz's pace.
const PICKER_TRIES: usize = 12;
/// Image downloads that take longer than this fail.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// The 250-pixel image, which is what Discord and the picker show.
    pub fn url(&self) -> String {
        self.image_url("250")
    }

    /// The front image at `size` pixels: 250, 500 or 1200.
    fn image_url(&self, size: &str) -> String {
        match self {
            Cover::Release(id) => {
                format!("https://coverartarchive.org/release/{id}/front-{size}")
            }
            Cover::ReleaseGroup(id) => {
                format!("https://coverartarchive.org/release-group/{id}/front-{size}")
            }
        }
    }
}

/// A cover the user can pick for an album, with what tells it apart from the others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub cover: Cover,
    /// The release's or release group's title, as MusicBrainz has it.
    pub title: String,
    /// The year it came out, when MusicBrainz knows it.
    pub year: Option<i32>,
    /// The release's country (an ISO 3166 code), when it came out in one; `None` for
    /// release groups and worldwide releases.
    pub country: Option<String>,
    /// The 250-pixel image, as downloaded.
    pub preview: Vec<u8>,
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
    /// GETs an image at `url`, following redirects, returning the status and the bytes.
    fn get_bytes(&mut self, url: &str) -> Result<(u16, Vec<u8>), String>;
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

    fn get_bytes(&mut self, url: &str) -> Result<(u16, Vec<u8>), String> {
        // The archive redirects to the image on archive.org.
        let mut response = self
            .agent
            .get(url)
            .config()
            .max_redirects(5)
            .timeout_global(Some(IMAGE_TIMEOUT))
            .build()
            .call()
            .map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|error| error.to_string())?;
        Ok((status, body))
    }
}

/// When the last request went out.
type LastRequest = Arc<Mutex<Option<Instant>>>;

/// The last request of every searcher on the real network, so that the Discord
/// lookup and a cover search running together still keep to MusicBrainz's pace.
fn online_last_request() -> LastRequest {
    static LAST: OnceLock<LastRequest> = OnceLock::new();
    LAST.get_or_init(LastRequest::default).clone()
}

/// Looks albums up through a transport, spacing its requests.
pub struct Searcher<T> {
    transport: T,
    interval: Duration,
    last_request: LastRequest,
}

impl Searcher<Http> {
    /// Searches the real services at the pace MusicBrainz allows.
    pub fn online() -> Self {
        Self {
            transport: Http::new(),
            interval: INTERVAL,
            last_request: online_last_request(),
        }
    }
}

impl<T: Transport> Searcher<T> {
    pub fn new(transport: T, interval: Duration) -> Self {
        Self {
            transport,
            interval,
            last_request: LastRequest::default(),
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

    /// The covers that could be the album's, for the user to pick from: the front
    /// covers of the matching releases (first the one the files are tagged with, if
    /// any), of a release's group when the release has none, and of the matching
    /// release groups. Matching is [`Searcher::find`]'s, and an image shared by
    /// several releases is offered once. Each cover goes to `found` as soon as its
    /// preview is downloaded. Setting `stop` ends the search before its next request.
    ///
    /// Fails when MusicBrainz can't be asked, and when nothing was found because the
    /// archive answered with errors.
    pub fn candidates(
        &mut self,
        album: &AlbumQuery,
        tagged_release: Option<&str>,
        stop: &AtomicBool,
        found: impl FnMut(Candidate),
    ) -> Result<(), String> {
        let mut picker = Picker {
            found,
            stop,
            previews: Vec::new(),
            checked: Vec::new(),
            tries: 0,
            problem: None,
        };
        if let Some(release) = tagged_release {
            let cover = Cover::Release(release.to_string());
            self.offer(&mut picker, cover, &album.title, None, None)?;
        }
        if normalize(&album.title).is_empty() || normalize(&album.artist).is_empty() {
            return picker.finish();
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
        for query in &tries {
            if picker.done() {
                return picker.finish();
            }
            let found: ReleaseSearch = self.ask("release", &search_query("release", query))?;
            let matches = found.releases.into_iter().filter(|release| {
                matches(query, release.score, &release.title, &release.artist_credit)
                    && release
                        .track_count
                        .is_none_or(|count| count >= query.track_count)
            });
            for release in matches {
                let year = year(release.date.as_deref());
                let country = release
                    .country
                    .filter(|country| !country.is_empty() && country != "XW");
                let cover = Cover::Release(release.id);
                if !self.offer(&mut picker, cover, &release.title, year, country)?
                    && let Some(group) = release.release_group
                {
                    let title = group.title.as_deref().unwrap_or(&release.title);
                    self.offer(
                        &mut picker,
                        Cover::ReleaseGroup(group.id),
                        title,
                        None,
                        None,
                    )?;
                }
            }
        }
        if picker.done() {
            return picker.finish();
        }
        let found: GroupSearch =
            self.ask("release-group", &search_query("releasegroup", &simple))?;
        let matches = found
            .release_groups
            .into_iter()
            .filter(|group| matches(&simple, group.score, &group.title, &group.artist_credit));
        for group in matches {
            let year = year(group.first_release_date.as_deref());
            self.offer(
                &mut picker,
                Cover::ReleaseGroup(group.id),
                &group.title,
                year,
                None,
            )?;
        }
        picker.finish()
    }

    /// Downloads a picked cover at up to 1200 pixels, the most the now playing view
    /// shows (the archive serves smaller originals as they are), or at 500 when it
    /// has no 1200-pixel version.
    pub fn download(&mut self, cover: &Cover) -> Result<Vec<u8>, String> {
        for size in ["1200", "500"] {
            self.wait_turn();
            let (status, image) = self.transport.get_bytes(&cover.image_url(size))?;
            match status {
                200 => return Ok(image),
                404 => {}
                other => return Err(format!("the Cover Art Archive answered {other}")),
            }
        }
        Err("the Cover Art Archive has no image for this cover".into())
    }

    /// Downloads a cover's preview and offers it to the picker, unless the picker has
    /// enough, checked it already or holds the same image. Returns whether the
    /// archive has the cover.
    fn offer(
        &mut self,
        picker: &mut Picker<impl FnMut(Candidate)>,
        cover: Cover,
        title: &str,
        year: Option<i32>,
        country: Option<String>,
    ) -> Result<bool, String> {
        if picker.done() || picker.checked.contains(&cover) {
            return Ok(false);
        }
        picker.checked.push(cover.clone());
        picker.tries += 1;
        self.wait_turn();
        let (status, preview) = self.transport.get_bytes(&cover.url())?;
        match status {
            200 => {
                if !picker.previews.contains(&preview) {
                    picker.previews.push(preview.clone());
                    (picker.found)(Candidate {
                        cover,
                        title: title.to_string(),
                        year,
                        country,
                        preview,
                    });
                }
                Ok(true)
            }
            404 => Ok(false),
            other => {
                // One image the archive can't serve doesn't end the search.
                picker.problem = Some(format!("the Cover Art Archive answered {other}"));
                Ok(false)
            }
        }
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

    /// Waits until a request may go out. The lock is held while waiting, so searchers
    /// sharing it take turns.
    fn wait_turn(&mut self) {
        let mut last = self
            .last_request
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(last) = *last {
            let next = last + self.interval;
            let now = Instant::now();
            if next > now {
                thread::sleep(next - now);
            }
        }
        *last = Some(Instant::now());
    }
}

/// A running candidate search: what it offered and checked so far.
struct Picker<'a, F> {
    found: F,
    stop: &'a AtomicBool,
    previews: Vec<Vec<u8>>,
    checked: Vec<Cover>,
    tries: usize,
    /// The last error the archive answered with.
    problem: Option<String>,
}

impl<F> Picker<'_, F> {
    fn done(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
            || self.previews.len() >= PICKER_COVERS
            || self.tries >= PICKER_TRIES
    }

    fn finish(self) -> Result<(), String> {
        match self.problem {
            Some(problem) if self.previews.is_empty() => Err(problem),
            _ => Ok(()),
        }
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

/// What a [`CoverPicker`] reports about a job, which it names by number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// A cover for the search, as soon as its preview is downloaded.
    Found(Candidate),
    /// The search ended. `Err` when the services could not be asked; covers found
    /// before that were reported.
    Searched(Result<(), String>),
    /// The picked cover's image, or why it could not be downloaded.
    Downloaded(Result<Vec<u8>, String>),
}

enum PickerJob {
    Search {
        id: u64,
        album: AlbumQuery,
        tagged_release: Option<String>,
        stop: Arc<AtomicBool>,
    },
    Download {
        id: u64,
        cover: Cover,
    },
}

/// Searches covers for the user to pick from, and downloads the picked one, on a
/// worker thread, one job at a time. Events go to `on_event` on the worker thread. A
/// search runs while it is the latest one: starting another or calling
/// [`CoverPicker::cancel`] ends it before its next request, without a `Searched`
/// event. Nothing is requested until a search is started.
pub struct CoverPicker {
    jobs: Sender<PickerJob>,
    current: Mutex<Option<Arc<AtomicBool>>>,
}

impl CoverPicker {
    pub fn start(on_event: impl Fn(u64, PickerEvent) + Send + 'static) -> Self {
        Self::with_searcher(Searcher::online(), on_event)
    }

    pub fn with_searcher<T: Transport>(
        mut searcher: Searcher<T>,
        on_event: impl Fn(u64, PickerEvent) + Send + 'static,
    ) -> Self {
        let (jobs, inbox) = mpsc::channel::<PickerJob>();
        thread::Builder::new()
            .name("vcmp-cover-picker".into())
            .spawn(move || {
                for job in inbox {
                    match job {
                        PickerJob::Search {
                            id,
                            album,
                            tagged_release,
                            stop,
                        } => {
                            let result = searcher.candidates(
                                &album,
                                tagged_release.as_deref(),
                                &stop,
                                |candidate| on_event(id, PickerEvent::Found(candidate)),
                            );
                            if !stop.load(Ordering::Relaxed) {
                                on_event(id, PickerEvent::Searched(result));
                            }
                        }
                        PickerJob::Download { id, cover } => {
                            on_event(id, PickerEvent::Downloaded(searcher.download(&cover)));
                        }
                    }
                }
            })
            .expect("spawning the cover picker thread");
        Self {
            jobs,
            current: Mutex::default(),
        }
    }

    /// Searches covers for `album`, ending the search before it.
    pub fn search(&self, id: u64, album: AlbumQuery, tagged_release: Option<String>) {
        let stop = Arc::new(AtomicBool::new(false));
        self.replace_current(Some(stop.clone()));
        let _ = self.jobs.send(PickerJob::Search {
            id,
            album,
            tagged_release,
            stop,
        });
    }

    pub fn download(&self, id: u64, cover: Cover) {
        let _ = self.jobs.send(PickerJob::Download { id, cover });
    }

    /// Ends the running search.
    pub fn cancel(&self) {
        self.replace_current(None);
    }

    fn replace_current(&self, next: Option<Arc<AtomicBool>>) {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = std::mem::replace(&mut *current, next) {
            previous.store(true, Ordering::Relaxed);
        }
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
    date: Option<String>,
    country: Option<String>,
}

#[derive(Deserialize)]
struct GroupRef {
    id: String,
    title: Option<String>,
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
    #[serde(rename = "first-release-date")]
    first_release_date: Option<String>,
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

/// The year of a MusicBrainz date ("1997-07-01", "1997-07" or "1997").
fn year(date: Option<&str>) -> Option<i32> {
    date?.get(..4)?.parse().ok()
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

        fn get_bytes(&mut self, url: &str) -> Result<(u16, Vec<u8>), String> {
            self.answer(format!("IMAGE {url}"))
                .map(|(status, body)| (status, body.into_bytes()))
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

    /// An image the archive serves; the bytes stand in for the picture.
    fn image(bytes: &str) -> Result<(u16, String), String> {
        Ok((200, bytes.into()))
    }

    fn dated_release(id: &str, title: &str, date: &str, country: &str) -> String {
        format!(
            r#"{{"id":"{id}","title":"{title}","score":100,"track-count":2,
                "date":"{date}","country":"{country}",
                "artist-credit":[{{"name":"Band","joinphrase":""}}],
                "release-group":{{"id":"group-of-{id}","title":"{title}"}}}}"#
        )
    }

    /// Runs a candidate search; returns what it found, how it ended and the requests.
    fn candidates(
        album: &AlbumQuery,
        tagged: Option<&str>,
        answers: impl IntoIterator<Item = Result<(u16, String), String>>,
    ) -> (Vec<Candidate>, Result<(), String>, Vec<String>) {
        let (transport, requests) = Scripted::new(answers);
        let mut found = Vec::new();
        let result = Searcher::new(transport, Duration::ZERO).candidates(
            album,
            tagged,
            &AtomicBool::new(false),
            |candidate| found.push(candidate),
        );
        let requests = requests
            .lock()
            .unwrap()
            .iter()
            .map(|(r, _)| r.clone())
            .collect();
        (found, result, requests)
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
    fn the_picker_gets_every_matching_cover_with_its_details() {
        let (found, result, requests) = candidates(
            &album("Band", "Album", 2),
            None,
            [
                releases(&[
                    dated_release("jp", "Album", "2019-03-06", "JP"),
                    dated_release("xw", "Album", "2020", "XW"),
                    release("other", "Another Album", "Band", 100, 2),
                ]),
                image("jp cover"),
                image("xw cover"),
                groups(&[("g", "Album", "Band", 100)]),
                image("group cover"),
            ],
        );
        assert_eq!(result, Ok(()));
        let summary: Vec<_> = found
            .iter()
            .map(|c| {
                (
                    c.cover.clone(),
                    c.year,
                    c.country.clone(),
                    c.preview.clone(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    Cover::Release("jp".into()),
                    Some(2019),
                    Some("JP".into()),
                    b"jp cover".to_vec()
                ),
                // A worldwide release names no country.
                (
                    Cover::Release("xw".into()),
                    Some(2020),
                    None,
                    b"xw cover".to_vec()
                ),
                (
                    Cover::ReleaseGroup("g".into()),
                    None,
                    None,
                    b"group cover".to_vec()
                ),
            ]
        );
        assert_eq!(
            requests[1],
            "IMAGE https://coverartarchive.org/release/jp/front-250"
        );
    }

    #[test]
    fn the_same_image_is_offered_once_and_missing_art_falls_back_to_the_group() {
        let (found, result, requests) = candidates(
            &album("Band", "Album", 2),
            None,
            [
                releases(&[
                    dated_release("a", "Album", "2001", "GB"),
                    dated_release("b", "Album", "2002", "US"),
                    dated_release("c", "Album", "2003", "DE"),
                ]),
                image("cover"),
                image("cover"),
                status(404),
                image("group of c"),
                // The group search finds groups already checked.
                groups(&[("group-of-c", "Album", "Band", 100)]),
            ],
        );
        assert_eq!(result, Ok(()));
        let covers: Vec<_> = found.into_iter().map(|c| c.cover).collect();
        assert_eq!(
            covers,
            [
                Cover::Release("a".into()),
                Cover::ReleaseGroup("group-of-c".into())
            ]
        );
        assert_eq!(requests.len(), 6, "{requests:?}");
    }

    #[test]
    fn the_tagged_release_comes_first() {
        let (found, _, requests) = candidates(
            &album("Band", "Album", 2),
            Some("tagged"),
            [image("tagged cover"), releases(&[]), groups(&[])],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].cover, Cover::Release("tagged".into()));
        assert_eq!(found[0].title, "Album", "the library's title stands in");
        assert_eq!(
            requests[0],
            "IMAGE https://coverartarchive.org/release/tagged/front-250"
        );
    }

    #[test]
    fn the_picker_finds_nothing_or_fails_in_plain_terms() {
        let (found, result, _) = candidates(
            &album("Band", "Album", 2),
            None,
            [releases(&[]), groups(&[])],
        );
        assert!(found.is_empty());
        assert_eq!(result, Ok(()), "nothing found is not a failure");

        let (found, result, _) = candidates(
            &album("Band", "Album", 2),
            None,
            [Err("offline".to_string())],
        );
        assert!(found.is_empty());
        assert_eq!(result, Err("offline".into()));

        // An archive error with nothing else found fails; with covers found, it
        // doesn't.
        let (_, result, _) = candidates(
            &album("Band", "Album", 2),
            None,
            [
                releases(&[dated_release("a", "Album", "2001", "GB")]),
                status(503),
                status(404),
                groups(&[]),
            ],
        );
        assert_eq!(result, Err("the Cover Art Archive answered 503".into()));
        let (found, result, _) = candidates(
            &album("Band", "Album", 2),
            None,
            [
                releases(&[
                    dated_release("a", "Album", "2001", "GB"),
                    dated_release("b", "Album", "2002", "GB"),
                ]),
                status(503),
                status(404),
                image("b"),
                groups(&[]),
            ],
        );
        assert_eq!((found.len(), result), (1, Ok(())));
    }

    #[test]
    fn the_picker_stops_at_its_limits() {
        let many: Vec<String> = (0..20)
            .map(|n| dated_release(&format!("r{n}"), "Album", "2001", "GB"))
            .collect();
        let mut answers = vec![releases(&many)];
        answers.extend((0..20).map(|n| image(&format!("cover {n}"))));
        let (found, result, requests) = candidates(&album("Band", "Album", 2), None, answers);
        assert_eq!(result, Ok(()));
        assert_eq!(found.len(), PICKER_COVERS);
        assert_eq!(
            requests.len(),
            1 + PICKER_COVERS,
            "no group search once full"
        );

        let mut answers = vec![releases(&many)];
        answers.extend((0..20).map(|_| status(404)));
        let (found, _, requests) = candidates(&album("Band", "Album", 2), None, answers);
        assert!(found.is_empty());
        assert_eq!(requests.len(), 1 + PICKER_TRIES);
    }

    #[test]
    fn a_stopped_search_asks_nothing_more() {
        let (transport, requests) = Scripted::new([]);
        let stop = AtomicBool::new(true);
        let result = Searcher::new(transport, Duration::ZERO).candidates(
            &album("Band", "Album", 2),
            Some("tagged"),
            &stop,
            |_| panic!("nothing is offered"),
        );
        assert_eq!(result, Ok(()));
        assert!(requests.lock().unwrap().is_empty());
    }

    #[test]
    fn a_picked_cover_downloads_large_or_at_the_next_size() {
        let (transport, requests) = Scripted::new([status(404), image("500 px")]);
        let image = Searcher::new(transport, Duration::ZERO)
            .download(&Cover::ReleaseGroup("g".into()))
            .unwrap();
        assert_eq!(image, b"500 px");
        let requests: Vec<String> = requests
            .lock()
            .unwrap()
            .iter()
            .map(|(r, _)| r.clone())
            .collect();
        assert_eq!(
            requests,
            [
                "IMAGE https://coverartarchive.org/release-group/g/front-1200",
                "IMAGE https://coverartarchive.org/release-group/g/front-500",
            ]
        );

        let (transport, _) = Scripted::new([Err("offline".to_string())]);
        assert_eq!(
            Searcher::new(transport, Duration::ZERO).download(&Cover::Release("r".into())),
            Err("offline".into())
        );
    }

    #[test]
    fn the_picker_worker_reports_by_job_and_drops_cancelled_searches() {
        let (transport, _) = Scripted::new([
            releases(&[dated_release("r1", "Album", "2001", "GB")]),
            image("cover"),
            groups(&[]),
            image("large"),
        ]);
        let (sender, events) = mpsc::channel();
        let picker = CoverPicker::with_searcher(
            Searcher::new(transport, Duration::ZERO),
            move |id, event| {
                sender.send((id, event)).unwrap();
            },
        );
        picker.search(3, album("Band", "Album", 2), None);
        let next = || events.recv_timeout(Duration::from_secs(5)).unwrap();
        let (id, event) = next();
        assert!(
            matches!(event, PickerEvent::Found(_)) && id == 3,
            "{event:?}"
        );
        assert_eq!(next(), (3, PickerEvent::Searched(Ok(()))));
        picker.download(3, Cover::Release("r1".into()));
        assert_eq!(next(), (3, PickerEvent::Downloaded(Ok(b"large".to_vec()))));

        // Cancelled before it ran: no requests (none are scripted), no events.
        picker.cancel();
        picker.search(4, album("Band", "Album", 2), None);
        picker.cancel();
        assert!(
            events.recv_timeout(Duration::from_millis(200)).is_err(),
            "a cancelled search reports nothing"
        );
    }

    #[test]
    fn online_searchers_share_one_pace() {
        let first = Searcher::online();
        let second = Searcher::online();
        assert!(Arc::ptr_eq(&first.last_request, &second.last_request));
    }

    #[test]
    fn years_come_from_dates() {
        assert_eq!(year(Some("1997-07-01")), Some(1997));
        assert_eq!(year(Some("2004")), Some(2004));
        assert_eq!(year(Some("")), None);
        assert_eq!(year(None), None);
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
        assert_eq!(
            Cover::Release("abc".into()).image_url("1200"),
            "https://coverartarchive.org/release/abc/front-1200"
        );
    }
}
