//! Artist photos from fanart.tv, for "Find photo online" on an artist's page.
//! fanart.tv knows artists by their MusicBrainz ID, so the artist is looked up on
//! MusicBrainz first (by the ID the files are tagged with, else by name), then each
//! match's photos are offered with a small preview. The one the user picks is
//! downloaded in full. Nothing is asked until the user searches.
//!
//! fanart.tv needs a project API key, compiled into release builds from
//! `VCMP_FANARTTV_API_KEY`. Without one there is no online search.

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

/// What a [`PhotoPicker`] reports about a job, which it names by number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhotoEvent {
    /// A photo for the search, as soon as its preview is downloaded.
    Found(Photo),
    /// The search ended. `Err` when the services could not be asked; photos found
    /// before that were reported.
    Searched(Result<(), String>),
    /// The picked photo's image, or why it could not be downloaded.
    Downloaded(Result<Vec<u8>, String>),
}

/// The photos of `artist` that could be offered, each sent to `found` as soon as its
/// preview is in. Setting `stop` ends the search before its next request.
pub fn photos<T: Transport>(
    searcher: &mut Searcher<T>,
    api_key: &str,
    artist: &ArtistQuery,
    stop: &AtomicBool,
    mut found: impl FnMut(Photo),
) -> Result<(), String> {
    let mut artists = Vec::new();
    if let Some(mbid) = &artist.mbid {
        artists.push(ArtistMatch {
            id: mbid.clone(),
            name: artist.name.clone(),
            disambiguation: None,
        });
    }
    if !stop.load(Ordering::Relaxed) {
        for candidate in searcher.artists(&artist.name)? {
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
        let (status, body) = searcher.fetch(&url, &[("api_key", api_key)])?;
        let images = match status {
            200 => {
                let mut images = serde_json::from_str::<ArtistImages>(&body)
                    .map_err(|error| format!("unexpected answer from fanart.tv: {error}"))?
                    .artistthumb;
                // The best liked first.
                images.sort_by_key(|image| std::cmp::Reverse(image.likes()));
                images
            }
            // fanart.tv has no images of this artist.
            404 => continue,
            401 | 403 => return Err(format!("fanart.tv refused the API key ({status})")),
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
        Some(problem) if offered == 0 => Err(problem),
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
                        Job::Search { id, artist, stop } => {
                            let result = photos(&mut searcher, &api_key, &artist, &stop, |photo| {
                                on_event(id, PhotoEvent::Found(photo))
                            });
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

    /// Searches photos of `artist`, ending the search before it.
    pub fn search(&self, id: u64, artist: ArtistQuery) {
        let stop = Arc::new(AtomicBool::new(false));
        self.replace_current(Some(stop.clone()));
        let _ = self.jobs.send(Job::Search { id, artist, stop });
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
            &query,
            &AtomicBool::new(false),
            |_| {},
        );
        assert!(result.unwrap_err().contains("refused"));

        let (transport, _) =
            Scripted::new([artists(&[("a", "Band", 100)]), Ok((404, "{}".into()))]);
        let mut searcher = Searcher::new(transport, Duration::ZERO);
        let mut found = 0;
        photos(
            &mut searcher,
            "key",
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
            &query,
            &AtomicBool::new(false),
            |photo| found.push(photo),
        )
        .unwrap();
        assert_eq!(found[0].preview, b"full a");
    }
}
