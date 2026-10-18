//! Artist photos from fanart.tv, for "Find photo online" on an artist's page.
//! fanart.tv knows artists by their MusicBrainz ID, so the artist is looked up on
//! MusicBrainz first (by the ID the files are tagged with, else by name), then each
//! match's photos are offered with a small preview. The one the user picks is
//! downloaded in full. Nothing is asked until the user searches.
//!
//! fanart.tv needs a project API key, compiled into release builds from
//! `VCMP_FANARTTV_API_KEY`. Without one there is no online search. A user's own
//! personal key, when they gave one, goes with it as `client_key`, as fanart.tv's
//! terms ask apps to allow; fanart.tv then shows them newer photos sooner.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use serde::Deserialize;

use crate::musicbrainz::{ArtistMatch, Searcher, Transport};

const API_URL: &str = "https://webservice.fanart.tv/v3/music";
/// The most photos the picker offers.
const PICKER_PHOTOS: usize = 8;

/// VCMP's fanart.tv project key, when this build has one.
pub fn api_key() -> Option<&'static str> {
    option_env!("VCMP_FANARTTV_API_KEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

/// The artist to find photos of, as the library knows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistQuery {
    pub name: String,
    /// The MusicBrainz artist ID the files are tagged with, if any.
    pub mbid: Option<String>,
}

/// A photo the user can pick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Photo {
    /// The full image, about 1000 pixels square.
    pub url: String,
    /// The artist it shows, as MusicBrainz names them, and what tells them apart.
    pub artist: String,
    pub disambiguation: Option<String>,
    /// The preview image, as downloaded.
    pub preview: Vec<u8>,
}

/// Why a search found nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchError {
    /// fanart.tv refused a key (its status): VCMP's, or the user's personal key.
    Refused(u16),
    /// MusicBrainz or fanart.tv could not be asked, or answered something else.
    Failed(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::Refused(status) => write!(f, "fanart.tv refused the API key ({status})"),
            SearchError::Failed(error) => f.write_str(error),
        }
    }
}

/// What a [`PhotoPicker`] reports about a job, which it names by number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhotoEvent {
    /// A photo for the search, as soon as its preview is downloaded.
    Found(Photo),
    /// The search ended. `Err` when the services could not be asked or fanart.tv
    /// refused a key; photos found before that were reported.
    Searched(Result<(), SearchError>),
    /// The picked photo's image, or why it could not be downloaded.
    Downloaded(Result<Vec<u8>, String>),
}

/// The photos of `artist` that could be offered, each sent to `found` as soon as its
/// preview is in. `client_key` is the user's personal key, sent with VCMP's
/// `api_key`. Setting `stop` ends the search before its next request.
pub fn photos<T: Transport>(
    searcher: &mut Searcher<T>,
    api_key: &str,
    client_key: Option<&str>,
    artist: &ArtistQuery,
    stop: &AtomicBool,
    found: impl FnMut(Photo),
) -> Result<(), SearchError> {
    // Error messages end up in the log; the keys don't.
    let scrub = |error: String| {
        let error = error.replace(api_key, "<api key>");
        match client_key {
            Some(key) => error.replace(key, "<personal key>"),
            None => error,
        }
    };
    search(searcher, api_key, client_key, artist, stop, found).map_err(|error| match error {
        SearchError::Failed(error) => SearchError::Failed(scrub(error)),
        refused => refused,
    })
}

fn search<T: Transport>(
    searcher: &mut Searcher<T>,
    api_key: &str,
    client_key: Option<&str>,
    artist: &ArtistQuery,
    stop: &AtomicBool,
    mut found: impl FnMut(Photo),
) -> Result<(), SearchError> {
    let mut keys = vec![("api_key", api_key)];
    if let Some(key) = client_key {
        keys.push(("client_key", key));
    }
    let mut artists = Vec::new();
    if let Some(mbid) = &artist.mbid {
        artists.push(ArtistMatch {
            id: mbid.clone(),
            name: artist.name.clone(),
            disambiguation: None,
        });
    }
    if !stop.load(Ordering::Relaxed) {
        for candidate in searcher
            .artists(&artist.name)
            .map_err(SearchError::Failed)?
        {
            if !artists.iter().any(|known| known.id == candidate.id) {
                artists.push(candidate);
            }
        }
    }
    let (mut offered, mut problem) = (0, None);
    for candidate in artists {
        if stop.load(Ordering::Relaxed) || offered >= PICKER_PHOTOS {
            break;
        }
        let url = format!("{API_URL}/{}", candidate.id);
        let (status, body) = searcher.fetch(&url, &keys).map_err(SearchError::Failed)?;
        let images = match status {
            200 => {
                let mut images = serde_json::from_str::<ArtistImages>(&body)
                    .map_err(|error| {
                        SearchError::Failed(format!("unexpected answer from fanart.tv: {error}"))
                    })?
                    .artistthumb;
                // The best liked first.
                images.sort_by_key(|image| std::cmp::Reverse(image.likes()));
                images
            }
            // fanart.tv has no images of this artist.
            404 => continue,
            401 | 403 => return Err(SearchError::Refused(status)),
            other => {
                problem = Some(format!("fanart.tv answered {other}"));
                continue;
            }
        };
        for image in images {
            if stop.load(Ordering::Relaxed) || offered >= PICKER_PHOTOS {
                break;
            }
            match preview(searcher, &image.url) {
                Ok(preview) => {
                    offered += 1;
                    found(Photo {
                        url: image.url,
                        artist: candidate.name.clone(),
                        disambiguation: candidate.disambiguation.clone(),
                        preview,
                    });
                }
                // One image that can't be fetched doesn't end the search.
                Err(error) => problem = Some(error),
            }
        }
    }
    match problem {
        Some(problem) if offered == 0 => Err(SearchError::Failed(problem)),
        _ => Ok(()),
    }
}

/// fanart.tv's small version of an image, or the image itself when there is none.
fn preview<T: Transport>(searcher: &mut Searcher<T>, url: &str) -> Result<Vec<u8>, String> {
    let small = url.replacen("/fanart/", "/preview/", 1);
    if small != url
        && let Ok((200, bytes)) = searcher.fetch_bytes(&small)
    {
        return Ok(bytes);
    }
    download(searcher, url)
}

/// The full image.
pub fn download<T: Transport>(searcher: &mut Searcher<T>, url: &str) -> Result<Vec<u8>, String> {
    match searcher.fetch_bytes(url)? {
        (200, bytes) => Ok(bytes),
        (status, _) => Err(format!("fanart.tv answered {status} for the image")),
    }
}

enum Job {
    Search {
        id: u64,
        artist: ArtistQuery,
        client_key: Option<String>,
        stop: Arc<AtomicBool>,
    },
    Download {
        id: u64,
        url: String,
    },
}

/// Searches photos for the user to pick from, and downloads the picked one, on a
/// worker thread, one job at a time. Events go to `on_event` on the worker thread. A
/// search runs while it is the latest one: starting another or calling
/// [`PhotoPicker::cancel`] ends it without a `Searched` event.
pub struct PhotoPicker {
    jobs: Sender<Job>,
    current: Mutex<Option<Arc<AtomicBool>>>,
}

impl PhotoPicker {
    /// The real services, with this build's key; `None` without one.
    pub fn start(on_event: impl Fn(u64, PhotoEvent) + Send + 'static) -> Option<Self> {
        Some(Self::with_searcher(
            Searcher::online(),
            api_key()?,
            on_event,
        ))
    }

    pub fn with_searcher<T: Transport>(
        mut searcher: Searcher<T>,
        api_key: &str,
        on_event: impl Fn(u64, PhotoEvent) + Send + 'static,
    ) -> Self {
        let api_key = api_key.to_string();
        let (jobs, inbox) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("vcmp-photo-picker".into())
            .spawn(move || {
                for job in inbox {
                    match job {
                        Job::Search {
                            id,
                            artist,
                            client_key,
                            stop,
                        } => {
                            let result = photos(
                                &mut searcher,
                                &api_key,
                                client_key.as_deref(),
                                &artist,
                                &stop,
                                |photo| on_event(id, PhotoEvent::Found(photo)),
                            );
                            if !stop.load(Ordering::Relaxed) {
                                on_event(id, PhotoEvent::Searched(result));
                            }
                        }
                        Job::Download { id, url } => {
                            on_event(id, PhotoEvent::Downloaded(download(&mut searcher, &url)));
                        }
                    }
                }
            })
            .expect("spawning the photo picker thread");
        Self {
            jobs,
            current: Mutex::default(),
        }
    }

    /// Searches photos of `artist`, with the user's personal key when they gave
    /// one, ending the search before it.
    pub fn search(&self, id: u64, artist: ArtistQuery, client_key: Option<String>) {
        let stop = Arc::new(AtomicBool::new(false));
        self.replace_current(Some(stop.clone()));
        let _ = self.jobs.send(Job::Search {
            id,
            artist,
            client_key,
            stop,
        });
    }

    pub fn download(&self, id: u64, url: String) {
        let _ = self.jobs.send(Job::Download { id, url });
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
struct ArtistImages {
    #[serde(default)]
    artistthumb: Vec<Image>,
}

#[derive(Deserialize)]
struct Image {
    url: String,
    #[serde(default)]
    likes: String,
}

impl Image {
    fn likes(&self) -> u32 {
        self.likes.parse().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::musicbrainz::tests::Scripted;

    fn artists(items: &[(&str, &str, u32)]) -> Result<(u16, String), String> {
        let items: Vec<String> = items
            .iter()
            .map(|(id, name, score)| format!(r#"{{"id":"{id}","name":"{name}","score":{score}}}"#))
            .collect();
        Ok((200, format!(r#"{{"artists":[{}]}}"#, items.join(","))))
    }

    fn thumbs(items: &[(&str, &str)]) -> Result<(u16, String), String> {
        let items: Vec<String> = items
            .iter()
            .map(|(url, likes)| format!(r#"{{"id":"1","url":"{url}","likes":"{likes}"}}"#))
            .collect();
        Ok((200, format!(r#"{{"artistthumb":[{}]}}"#, items.join(","))))
    }

    fn image(bytes: &str) -> Result<(u16, String), String> {
        Ok((200, bytes.into()))
    }

    const A: &str = "https://assets.fanart.tv/fanart/music/a/artistthumb/a.jpg";
    const B: &str = "https://assets.fanart.tv/fanart/music/a/artistthumb/b.jpg";

    #[test]
    fn photos_of_the_tagged_artist_come_first_best_liked_first() {
        let (transport, requests) = Scripted::new([
            artists(&[("other", "Band", 100), ("unsure", "Band", 50)]),
            thumbs(&[(A, "1"), (B, "7")]),
            image("preview b"),
            image("preview a"),
            Ok((404, r#"{"status":"error"}"#.into())),
        ]);
        let mut searcher = Searcher::new(transport, Duration::ZERO);
        let mut found = Vec::new();
        let query = ArtistQuery {
            name: "Band".into(),
            mbid: Some("tagged".into()),
        };
        photos(
            &mut searcher,
            "key",
            None,
            &query,
            &AtomicBool::new(false),
            |photo| found.push(photo),
        )
        .unwrap();
        let previews: Vec<&[u8]> = found.iter().map(|photo| &photo.preview[..]).collect();
        assert_eq!(previews, [&b"preview b"[..], b"preview a"]);
        assert_eq!(found[0].url, B);
        let requests: Vec<String> = requests
            .lock()
            .unwrap()
            .iter()
            .map(|(request, _)| request.clone())
            .collect();
        assert!(requests[0].contains("/artist/") && requests[0].contains(r#"artist:"Band""#));
        assert!(requests[1].ends_with("/music/tagged "), "{}", requests[1]);
        assert!(
            requests[2].contains("/preview/music/"),
            "previews are small"
        );
        assert!(
            requests[4].ends_with("/music/other "),
            "the unsure hit is skipped"
        );
    }

    #[test]
    fn a_refused_key_and_no_photos_are_told_apart() {
        let (transport, _) =
            Scripted::new([artists(&[("a", "Band", 100)]), Ok((401, "{}".into()))]);
        let mut searcher = Searcher::new(transport, Duration::ZERO);
        let query = ArtistQuery {
            name: "Band".into(),
            mbid: None,
        };
        let result = photos(
            &mut searcher,
            "key",
            None,
            &query,
            &AtomicBool::new(false),
            |_| {},
        );
        assert_eq!(result, Err(SearchError::Refused(401)));

        let (transport, _) =
            Scripted::new([artists(&[("a", "Band", 100)]), Ok((404, "{}".into()))]);
        let mut searcher = Searcher::new(transport, Duration::ZERO);
        let mut found = 0;
        photos(
            &mut searcher,
            "key",
            None,
            &query,
            &AtomicBool::new(false),
            |_| found += 1,
        )
        .unwrap();
        assert_eq!(found, 0);
    }

    #[test]
    fn a_missing_preview_falls_back_to_the_full_image() {
        let (transport, _) = Scripted::new([
            artists(&[]),
            thumbs(&[(A, "1")]),
            Ok((404, String::new())),
            image("full a"),
        ]);
        let mut searcher = Searcher::new(transport, Duration::ZERO);
        let query = ArtistQuery {
            name: "Band".into(),
            mbid: Some("tagged".into()),
        };
        let mut found = Vec::new();
        photos(
            &mut searcher,
            "key",
            None,
            &query,
            &AtomicBool::new(false),
            |photo| found.push(photo),
        )
        .unwrap();
        assert_eq!(found[0].preview, b"full a");
    }

    /// A request's parameters, as names and values.
    type Parameters = Vec<(String, String)>;

    /// Answers every lookup with 404 and keeps each request's parameters.
    struct Keys(Arc<Mutex<Vec<Parameters>>>);

    impl Transport for Keys {
        fn get(&mut self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String), String> {
            if url.contains("musicbrainz") {
                return artists(&[]);
            }
            let pairs = query
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
            self.0.lock().unwrap().push(pairs);
            Ok((404, String::new()))
        }

        fn head(&mut self, _: &str) -> Result<u16, String> {
            Ok(404)
        }

        fn get_bytes(&mut self, _: &str) -> Result<(u16, Vec<u8>), String> {
            Ok((404, Vec::new()))
        }
    }

    #[test]
    fn a_personal_key_goes_with_the_project_key() {
        let query = ArtistQuery {
            name: "Band".into(),
            mbid: Some("tagged".into()),
        };
        let sent = |client_key: Option<&str>| {
            let requests = Arc::new(Mutex::new(Vec::new()));
            let mut searcher = Searcher::new(Keys(requests.clone()), Duration::ZERO);
            photos(
                &mut searcher,
                "project",
                client_key,
                &query,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            requests.lock().unwrap().remove(0)
        };
        let pair = |key: &str, value: &str| (key.to_string(), value.to_string());
        assert_eq!(sent(None), [pair("api_key", "project")]);
        assert_eq!(
            sent(Some("mine")),
            [pair("api_key", "project"), pair("client_key", "mine")]
        );
    }

    #[test]
    fn keys_are_scrubbed_from_errors() {
        struct Failing;
        impl Transport for Failing {
            fn get(&mut self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String), String> {
                if url.contains("musicbrainz") {
                    return artists(&[]);
                }
                Err(format!("could not reach {url}?{}", query[1].1))
            }
            fn head(&mut self, _: &str) -> Result<u16, String> {
                Ok(404)
            }
            fn get_bytes(&mut self, _: &str) -> Result<(u16, Vec<u8>), String> {
                Ok((404, Vec::new()))
            }
        }
        let mut searcher = Searcher::new(Failing, Duration::ZERO);
        let query = ArtistQuery {
            name: "Band".into(),
            mbid: Some("tagged".into()),
        };
        let error = photos(
            &mut searcher,
            "project",
            Some("mine"),
            &query,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        let SearchError::Failed(message) = error else {
            panic!("{error:?}");
        };
        assert!(!message.contains("mine"), "{message}");
        assert!(message.contains("<personal key>"), "{message}");
    }
}
