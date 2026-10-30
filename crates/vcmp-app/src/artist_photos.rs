//! The artist page's photo, behind the `PhotoSearch` global: "Find photo online"
//! looks the artist up on MusicBrainz and offers their photos from fanart.tv
//! (`PhotoPicker` in `vcmp-integrations`), "Choose photo…" takes an image file, and
//! "Remove photo" goes back to the newest album's cover. The photo is kept by the
//! library and shows at once on the artist page and on the artist cards. Nothing goes
//! to the network until the user asks, and only in builds with VCMP's fanart.tv key.
//! The user's own fanart.tv personal key, set in Settings, goes with each search.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use vcmp_integrations::fanart::{ArtistQuery, Photo, PhotoEvent, PhotoPicker, SearchError};
use vcmp_library::{ArtistId, Library};

use crate::albums::{TILE_PIXELS, initial};
use crate::library_view::LibraryView;
use crate::{AlbumTile, AppWindow, ArtworkState, PhotoSearch, PhotoSearchStatus};

/// Image files "Choose photo…" offers.
const PHOTO_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];

/// The user's fanart.tv personal API key, in the library's settings; absent or empty
/// while they have given none.
pub const SETTING_FANART_KEY: &str = "integrations.fanart_personal_key";

/// Where photo searches go: MusicBrainz and fanart.tv, or a fake in tests. Answers
/// come back through `ArtistPhotos::handle`, tagged with the search's number.
pub trait PhotoSource {
    /// Searches with the user's personal key, when they gave one.
    fn search(&self, search: u64, artist: ArtistQuery, personal_key: Option<String>);
    fn download(&self, search: u64, url: String);
    /// Stops the running search.
    fn cancel(&self);
}

impl PhotoSource for PhotoPicker {
    fn search(&self, search: u64, artist: ArtistQuery, personal_key: Option<String>) {
        PhotoPicker::search(self, search, artist, personal_key);
    }

    fn download(&self, search: u64, url: String) {
        PhotoPicker::download(self, search, url);
    }

    fn cancel(&self) {
        PhotoPicker::cancel(self);
    }
}

pub struct ArtistPhotos {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    view: Rc<LibraryView>,
    source: Option<Box<dyn PhotoSource>>,
    // The search the picker shows; answers to earlier ones are dropped.
    search: Cell<u64>,
    // The artist it is for, the photos found (in the picker's order) and the one
    // being downloaded.
    artist: Cell<Option<ArtistId>>,
    photos: RefCell<Vec<String>>,
    tiles: Rc<VecModel<AlbumTile>>,
    chosen: RefCell<Option<String>>,
}

thread_local! {
    /// The service answers are delivered to, on the UI thread.
    static PHOTOS: RefCell<Weak<ArtistPhotos>> = RefCell::default();
}

impl ArtistPhotos {
    /// Connects `PhotoSearch` to the library, and to `source` for online photos
    /// (`None`: this build can't search). Keep it alive as long as the window.
    pub fn connect(
        window: &AppWindow,
        library: Rc<Library>,
        view: Rc<LibraryView>,
        source: Option<Box<dyn PhotoSource>>,
    ) -> Rc<Self> {
        let tiles = Rc::new(VecModel::default());
        let state = window.global::<PhotoSearch>();
        state.set_photos(ModelRc::from(tiles.clone()));
        state.set_available(source.is_some());
        match library.setting(SETTING_FANART_KEY) {
            Ok(key) => state.set_personal_key(key.unwrap_or_default().into()),
            Err(error) => log::warn!("could not read the fanart.tv personal key: {error}"),
        }
        let photos = Rc::new(Self {
            window: window.as_weak(),
            library,
            view,
            source,
            search: Cell::new(0),
            artist: Cell::new(None),
            photos: RefCell::default(),
            tiles,
            chosen: RefCell::default(),
        });
        PHOTOS.with(|current| *current.borrow_mut() = Rc::downgrade(&photos));
        let weak = Rc::downgrade(&photos);
        state.on_find({
            let weak = weak.clone();
            move || {
                if let Some(photos) = weak.upgrade() {
                    photos.find();
                }
            }
        });
        state.on_choose({
            let weak = weak.clone();
            move |index| {
                if let (Some(photos), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    photos.choose(index);
                }
            }
        });
        state.on_cancel({
            let weak = weak.clone();
            move || {
                if let Some(photos) = weak.upgrade() {
                    photos.cancel();
                }
            }
        });
        state.on_choose_file({
            let weak = weak.clone();
            move || {
                if let Some(photos) = weak.upgrade() {
                    photos.choose_file();
                }
            }
        });
        state.on_set_personal_key({
            let weak = weak.clone();
            move |key| {
                if let Some(photos) = weak.upgrade() {
                    photos.set_personal_key(&key);
                }
            }
        });
        state.on_remove(move || {
            if let Some(photos) = weak.upgrade() {
                photos.remove();
            }
        });
        photos
    }

    /// Keeps the user's fanart.tv personal key for the searches to come; an empty one
    /// forgets it.
    pub fn set_personal_key(&self, key: &str) {
        let key = key.trim();
        if let Err(error) = self.library.set_setting(SETTING_FANART_KEY, key) {
            log::warn!("could not save the fanart.tv personal key: {error}");
        }
        if let Some(window) = self.window.upgrade() {
            window.global::<PhotoSearch>().set_personal_key(key.into());
        }
    }

    /// Runs `action` with the service on the UI thread, from any thread. False once
    /// the event loop is gone.
    pub fn post(action: impl FnOnce(&ArtistPhotos) + Send + 'static) -> bool {
        slint::invoke_from_event_loop(move || {
            if let Some(photos) = PHOTOS.with(|current| current.borrow().upgrade()) {
                action(&photos);
            }
        })
        .is_ok()
    }

    /// Opens the picker for the artist the artist page shows and starts searching.
    pub fn find(&self) {
        let (Some(window), Some(source), Some(artist)) = (
            self.window.upgrade(),
            self.source.as_ref(),
            self.view.shown_artist_entry(),
        ) else {
            return;
        };
        let search = self.next_search();
        self.artist.set(Some(artist.id));
        self.photos.borrow_mut().clear();
        self.tiles.set_vec(Vec::new());
        *self.chosen.borrow_mut() = None;
        let state = window.global::<PhotoSearch>();
        state.set_artist_name(artist.name.as_str().into());
        state.set_status(PhotoSearchStatus::Searching);
        state.set_file_failed(false);
        state.set_open(true);
        let mbid = self.library.artist_mbid(artist.id).unwrap_or_else(|error| {
            log::warn!(
                "could not read the MusicBrainz ID of {:?}: {error}",
                artist.name
            );
            None
        });
        let personal_key = Some(state.get_personal_key().to_string()).filter(|key| !key.is_empty());
        source.search(
            search,
            ArtistQuery {
                name: artist.name,
                mbid,
            },
            personal_key,
        );
    }

    /// Downloads photo `index` of the picker to make it the artist's.
    pub fn choose(&self, index: usize) {
        let (Some(window), Some(source)) = (self.window.upgrade(), self.source.as_ref()) else {
            return;
        };
        let state = window.global::<PhotoSearch>();
        if !state.get_open() || state.get_status() == PhotoSearchStatus::Downloading {
            return;
        }
        let Some(url) = self.photos.borrow().get(index).cloned() else {
            return;
        };
        *self.chosen.borrow_mut() = Some(url.clone());
        state.set_status(PhotoSearchStatus::Downloading);
        source.download(self.search.get(), url);
    }

    /// Closes the picker and stops its search; a download under way is ignored.
    pub fn cancel(&self) {
        if let Some(source) = &self.source {
            source.cancel();
        }
        self.close();
    }

    /// Takes an answer from the photo source.
    pub fn handle(&self, search: u64, event: PhotoEvent) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PhotoSearch>();
        if search != self.search.get() || !state.get_open() {
            return;
        }
        match event {
            PhotoEvent::Found(photo) => self.add(photo),
            PhotoEvent::Searched(result) => {
                if let Err(error) = &result {
                    log::info!("the photo search failed: {error}");
                }
                // A download started meanwhile keeps its status.
                if state.get_status() == PhotoSearchStatus::Searching {
                    let found = !self.photos.borrow().is_empty();
                    state.set_status(match (result, found) {
                        (_, true) => PhotoSearchStatus::Found,
                        (Ok(()), false) => PhotoSearchStatus::NothingFound,
                        (Err(SearchError::Refused(_)), false) => PhotoSearchStatus::Refused,
                        (Err(SearchError::Failed(_)), false) => PhotoSearchStatus::Failed,
                    });
                }
            }
            PhotoEvent::Downloaded(Ok(image)) => self.apply(&image),
            PhotoEvent::Downloaded(Err(error)) => {
                log::info!("the photo download failed: {error}");
                state.set_status(PhotoSearchStatus::DownloadFailed);
            }
        }
    }

    /// Makes an image file the photo of the artist the artist page shows.
    pub fn use_file(&self, data: &[u8]) {
        let (Some(window), Some(artist)) = (self.window.upgrade(), self.view.shown_artist_entry())
        else {
            return;
        };
        let stored = self.library.set_artist_photo(artist.id, data, None);
        if let Err(error) = &stored {
            log::info!(
                "could not use the file as the photo of {:?}: {error}",
                artist.name
            );
        }
        window
            .global::<PhotoSearch>()
            .set_file_failed(stored.is_err());
        if stored.is_ok() {
            self.view.refresh_artist_pictures();
        }
    }

    fn choose_file(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let picked = slint::spawn_local(async move {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("Images", PHOTO_EXTENSIONS)
                .pick_file()
                .await
            else {
                return;
            };
            let data = file.read().await;
            if let Some(photos) = weak.upgrade() {
                photos.use_file(&data);
            }
        });
        if let Err(error) = picked {
            log::error!("could not open the image chooser: {error}");
        }
    }

    fn remove(&self) {
        let Some(artist) = self.view.shown_artist_entry() else {
            return;
        };
        if let Err(error) = self.library.clear_artist_photo(artist.id) {
            log::error!("could not remove the photo of {:?}: {error}", artist.name);
        }
        if let Some(window) = self.window.upgrade() {
            window.global::<PhotoSearch>().set_file_failed(false);
        }
        self.view.refresh_artist_pictures();
    }

    fn add(&self, photo: Photo) {
        let Some(preview) = decode(&photo.preview) else {
            log::info!("skipped a photo preview that is not an image");
            return;
        };
        self.tiles.push(AlbumTile {
            title: photo.artist.as_str().into(),
            artist: photo.disambiguation.unwrap_or_default().into(),
            initial: initial(&photo.artist).into(),
            artwork: ArtworkState::Loaded,
            cover: preview,
            ..AlbumTile::default()
        });
        self.photos.borrow_mut().push(photo.url);
    }

    /// Stores the downloaded image as the artist's photo and shows it everywhere.
    fn apply(&self, image: &[u8]) {
        let (Some(artist), Some(url)) = (self.artist.get(), self.chosen.borrow().clone()) else {
            return;
        };
        match self.library.set_artist_photo(artist, image, Some(&url)) {
            Ok(_) => {
                self.close();
                self.view.refresh_artist_pictures();
            }
            Err(error) => {
                log::warn!("could not store the downloaded photo: {error}");
                if let Some(window) = self.window.upgrade() {
                    window
                        .global::<PhotoSearch>()
                        .set_status(PhotoSearchStatus::DownloadFailed);
                }
            }
        }
    }

    fn close(&self) {
        // Answers still on their way belong to no search now.
        self.next_search();
        self.artist.set(None);
        self.photos.borrow_mut().clear();
        self.tiles.set_vec(Vec::new());
        *self.chosen.borrow_mut() = None;
        if let Some(window) = self.window.upgrade() {
            window.global::<PhotoSearch>().set_open(false);
        }
    }

    fn next_search(&self) -> u64 {
        let next = self.search.get() + 1;
        self.search.set(next);
        next
    }
}

/// A preview as RGBA at the card's size.
fn decode(data: &[u8]) -> Option<Image> {
    let image = image::load_from_memory(data).ok()?;
    let rgba = image.thumbnail(TILE_PIXELS, TILE_PIXELS).into_rgba8();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
    );
    Some(Image::from_rgba8(buffer))
}
