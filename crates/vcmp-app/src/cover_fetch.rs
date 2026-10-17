//! "Find cover online" behind the `CoverSearch` global. The album page of an album
//! without artwork asks for covers; MusicBrainz and the Cover Art Archive supply
//! candidates (`CoverPicker` in `vcmp-integrations`), which the picker shows as they
//! arrive. The one the user picks is downloaded, stored as the album's artwork
//! (`Library::set_fetched_cover`) and shown at once: on the album page and in the
//! grids here, in the player through `on_cover_changed`. Nothing goes to the network
//! until the user asks.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::SystemTime;

use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use vcmp_integrations::musicbrainz::{AlbumQuery, Candidate, Cover, CoverPicker, PickerEvent};
use vcmp_library::{AlbumId, CoverArtId, Library};

use crate::albums::{TILE_PIXELS, initial};
use crate::library_view::LibraryView;
use crate::{AlbumTile, AppWindow, ArtworkState, CoverSearch, CoverSearchStatus};

/// Where cover searches go: MusicBrainz and the Cover Art Archive, or a fake in tests.
/// Answers come back through `CoverFetch::handle`, tagged with the search's number.
pub trait CoverSource {
    fn search(&self, search: u64, album: AlbumQuery, tagged_release: Option<String>);
    fn download(&self, search: u64, cover: Cover);
    /// Stops the running search.
    fn cancel(&self);
}

impl CoverSource for CoverPicker {
    fn search(&self, search: u64, album: AlbumQuery, tagged_release: Option<String>) {
        CoverPicker::search(self, search, album, tagged_release);
    }

    fn download(&self, search: u64, cover: Cover) {
        CoverPicker::download(self, search, cover);
    }

    fn cancel(&self) {
        CoverPicker::cancel(self);
    }
}

/// Told about an album whose cover was just fetched.
type CoverChanged = Box<dyn Fn(AlbumId)>;

pub struct CoverFetch {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    view: Rc<LibraryView>,
    source: Box<dyn CoverSource>,
    // The search the picker shows; answers to earlier ones are dropped.
    search: Cell<u64>,
    // The album it is for, the covers found (in the picker's order) and the one
    // being downloaded.
    album: Cell<Option<AlbumId>>,
    covers: RefCell<Vec<Cover>>,
    tiles: Rc<VecModel<AlbumTile>>,
    chosen: RefCell<Option<Cover>>,
    on_cover_changed: RefCell<Option<CoverChanged>>,
}

thread_local! {
    /// The cover fetch answers are delivered to, on the UI thread.
    static FETCH: RefCell<Weak<CoverFetch>> = RefCell::default();
}

impl CoverFetch {
    /// Connects `CoverSearch` to the library and to `source`. Keep it alive as long
    /// as the window.
    pub fn connect(
        window: &AppWindow,
        library: Rc<Library>,
        view: Rc<LibraryView>,
        source: Box<dyn CoverSource>,
    ) -> Rc<Self> {
        let tiles = Rc::new(VecModel::default());
        let state = window.global::<CoverSearch>();
        state.set_covers(ModelRc::from(tiles.clone()));
        let fetch = Rc::new(Self {
            window: window.as_weak(),
            library,
            view,
            source,
            search: Cell::new(0),
            album: Cell::new(None),
            covers: RefCell::default(),
            tiles,
            chosen: RefCell::default(),
            on_cover_changed: RefCell::default(),
        });
        FETCH.with(|current| *current.borrow_mut() = Rc::downgrade(&fetch));
        let weak = Rc::downgrade(&fetch);
        state.on_find({
            let weak = weak.clone();
            move || {
                if let Some(fetch) = weak.upgrade() {
                    fetch.find();
                }
            }
        });
        state.on_choose({
            let weak = weak.clone();
            move |index| {
                if let (Some(fetch), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    fetch.choose(index);
                }
            }
        });
        state.on_cancel({
            let weak = weak.clone();
            move || {
                if let Some(fetch) = weak.upgrade() {
                    fetch.cancel();
                }
            }
        });
        state.on_remove(move || {
            if let Some(fetch) = weak.upgrade() {
                fetch.remove();
            }
        });
        fetch
    }

    /// Runs `action` with the cover fetch on the UI thread, from any thread. False
    /// once the event loop is gone.
    pub fn post(action: impl FnOnce(&CoverFetch) + Send + 'static) -> bool {
        slint::invoke_from_event_loop(move || {
            if let Some(fetch) = FETCH.with(|current| current.borrow().upgrade()) {
                action(&fetch);
            }
        })
        .is_ok()
    }

    /// Calls `action` with an album whose cover was just fetched, once the album page
    /// and the grids show it, so the player can follow.
    pub fn on_cover_changed(&self, action: impl Fn(AlbumId) + 'static) {
        *self.on_cover_changed.borrow_mut() = Some(Box::new(action));
    }

    /// Opens the picker for the album the album page shows and starts searching.
    pub fn find(&self) {
        let (Some(window), Some(album)) = (self.window.upgrade(), self.view.shown_album()) else {
            return;
        };
        let search = self.next_search();
        self.album.set(Some(album.id));
        self.covers.borrow_mut().clear();
        self.tiles.set_vec(Vec::new());
        *self.chosen.borrow_mut() = None;
        let state = window.global::<CoverSearch>();
        state.set_album_title(album.title.as_str().into());
        state.set_album_artist(album.artist.clone().unwrap_or_default().into());
        state.set_status(CoverSearchStatus::Searching);
        state.set_open(true);
        self.source.search(
            search,
            AlbumQuery {
                artist: album.artist.unwrap_or_default(),
                title: album.title,
                track_count: album.track_count,
            },
            album.musicbrainz_id,
        );
    }

    /// Downloads cover `index` of the picker to make it the album's.
    pub fn choose(&self, index: usize) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<CoverSearch>();
        if !state.get_open() || state.get_status() == CoverSearchStatus::Downloading {
            return;
        }
        let Some(cover) = self.covers.borrow().get(index).cloned() else {
            return;
        };
        *self.chosen.borrow_mut() = Some(cover.clone());
        state.set_status(CoverSearchStatus::Downloading);
        self.source.download(self.search.get(), cover);
    }

    /// Closes the picker and stops its search; a download under way is ignored.
    pub fn cancel(&self) {
        self.source.cancel();
        self.close();
    }

    /// Takes away the cover fetched for the album the album page shows, everywhere.
    pub fn remove(&self) {
        let Some(album) = self.view.shown_album() else {
            return;
        };
        if let Err(error) = self.library.clear_fetched_cover(album.id) {
            log::error!("could not remove the cover of {:?}: {error}", album.title);
            return;
        }
        self.view.reload();
        if let Some(action) = self.on_cover_changed.borrow().as_ref() {
            action(album.id);
        }
    }

    /// Takes an answer from the cover source.
    pub fn handle(&self, search: u64, event: PickerEvent) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<CoverSearch>();
        if search != self.search.get() || !state.get_open() {
            return;
        }
        match event {
            PickerEvent::Found(candidate) => self.add(candidate),
            PickerEvent::Searched(result) => {
                let found = !self.covers.borrow().is_empty();
                if let Err(error) = &result {
                    log::info!("the cover search failed: {error}");
                }
                // A download started meanwhile keeps its status.
                if state.get_status() == CoverSearchStatus::Searching {
                    state.set_status(match (result.is_ok(), found) {
                        (true, true) => CoverSearchStatus::Found,
                        (true, false) => CoverSearchStatus::NothingFound,
                        (false, true) => CoverSearchStatus::CutShort,
                        (false, false) => CoverSearchStatus::Failed,
                    });
                }
            }
            PickerEvent::Downloaded(Ok(image)) => self.apply(&image),
            PickerEvent::Downloaded(Err(error)) => {
                log::info!("the cover download failed: {error}");
                state.set_status(CoverSearchStatus::DownloadFailed);
            }
        }
    }

    fn add(&self, candidate: Candidate) {
        let Some(cover) = decode(&candidate.preview) else {
            log::info!("skipped a cover preview that is not an image");
            return;
        };
        // The year and country are data, and the separator is not a word to translate.
        let details: Vec<String> = candidate
            .year
            .map(|year| year.to_string())
            .into_iter()
            .chain(candidate.country)
            .collect();
        self.tiles.push(AlbumTile {
            title: candidate.title.as_str().into(),
            artist: details.join(" · ").into(),
            initial: initial(&candidate.title).into(),
            artwork: ArtworkState::Loaded,
            cover,
            ..AlbumTile::default()
        });
        self.covers.borrow_mut().push(candidate.cover);
    }

    /// Stores the downloaded image as the album's artwork and shows it everywhere.
    fn apply(&self, image: &[u8]) {
        let (Some(album), Some(cover)) = (self.album.get(), self.chosen.borrow().clone()) else {
            return;
        };
        let source = match cover {
            Cover::Release(id) => CoverArtId::Release(id),
            Cover::ReleaseGroup(id) => CoverArtId::ReleaseGroup(id),
        };
        match self.library.set_fetched_cover(album, image, &source) {
            Ok(Some(_)) => {
                // The Discord presence shows the cover the user picked.
                if let Err(error) =
                    self.library
                        .set_cover_lookup(album, Some(&source), SystemTime::now())
                {
                    log::warn!("could not store the picked cover for Discord: {error}");
                }
                self.close();
                self.view.reload();
                if let Some(action) = self.on_cover_changed.borrow().as_ref() {
                    action(album);
                }
            }
            Ok(None) => {
                log::info!("the downloaded cover is not an image VCMP can read");
                self.show_status(CoverSearchStatus::DownloadFailed);
            }
            Err(error) => {
                log::warn!("could not store the downloaded cover: {error}");
                self.show_status(CoverSearchStatus::DownloadFailed);
            }
        }
    }

    fn close(&self) {
        // Answers still on their way belong to no search now.
        self.next_search();
        self.album.set(None);
        self.covers.borrow_mut().clear();
        self.tiles.set_vec(Vec::new());
        *self.chosen.borrow_mut() = None;
        if let Some(window) = self.window.upgrade() {
            window.global::<CoverSearch>().set_open(false);
        }
    }

    fn show_status(&self, status: CoverSearchStatus) {
        if let Some(window) = self.window.upgrade() {
            window.global::<CoverSearch>().set_status(status);
        }
    }

    fn next_search(&self) -> u64 {
        let next = self.search.get() + 1;
        self.search.set(next);
        next
    }
}

/// A preview as RGBA at the card's size; RGB images cost the GPU renderer a
/// conversion every frame.
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
