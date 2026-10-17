//! Connects the library to the window: the album grid and the album page that
//! opens from it, the artist grid with the artist page and its albums, search, and
//! the songs table.
//! All of them are reloaded after every library change.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Image, ModelRc, Timer, TimerMode, VecModel};
use vcmp_library::{Album, AlbumId, Artist, ArtistId, Library, Track};

use crate::albums::{AlbumBrowser, Card, CoverGrid, Grid, cards, initial};
use crate::folders::FolderBrowser;
use crate::{AlbumDetail, AlbumTile, AppWindow, ArtistDetail, SongColumn, SongRow, TrackRow};

pub struct LibraryView {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    browser: Rc<AlbumBrowser>,
    // The album the album page shows (or showed last), from either page.
    open: RefCell<Option<Album>>,
    // The artists in the grid, in its order, and the grids' pictures.
    artists: RefCell<Vec<Artist>>,
    artist_grid: Rc<CoverGrid>,
    artist_albums_grid: Rc<CoverGrid>,
    // The artist the artist page shows (or showed last), with its albums and the
    // tracks it appears on elsewhere.
    artist: RefCell<Option<OpenArtist>>,
    // What the last search found, and the pause after typing before it runs.
    found: RefCell<Found>,
    search_artists_grid: Rc<CoverGrid>,
    search_albums_grid: Rc<CoverGrid>,
    search_timer: Timer,
    // Every album, before the Albums page's filter.
    all_albums: RefCell<Vec<Album>>,
    // Every track in the Songs page's order, once the page was shown, and those
    // the page's filter leaves, as shown.
    all_songs: RefCell<Option<Vec<Track>>>,
    songs: RefCell<Vec<Track>>,
    songs_filter_timer: Timer,
    folders: Rc<FolderBrowser>,
}

/// Setting key: the Songs page's sort, as "<column>" or "<column>:reverse".
pub const SETTING_SONGS_SORT: &str = "ui.songs_sort";

#[derive(Default)]
struct Found {
    artists: Vec<Artist>,
    albums: Vec<Album>,
    tracks: Vec<Track>,
}

/// How long typing must pause before a search runs; a one-letter search can take
/// 50 ms on a few thousand tracks.
pub const SEARCH_DELAY: Duration = Duration::from_millis(150);
/// The most of each kind a search shows: a couple of rows of cards, and tracks.
const SEARCH_CARDS: usize = 12;
const SEARCH_TRACKS: usize = 50;

#[derive(Clone)]
struct OpenArtist {
    artist: Artist,
    albums: Vec<Album>,
    appearances: Vec<Track>,
}

/// Everything by an artist, in play order: the tracks of their albums, then the
/// tracks they appear on elsewhere.
pub struct ArtistTracks {
    pub artist: Artist,
    pub tracks: Vec<Track>,
    /// Where the appearances start in `tracks`.
    pub appearances_from: usize,
}

impl LibraryView {
    /// Shows the library's albums. Keep the view alive as long as the window.
    pub fn connect(window: &AppWindow, library: Rc<Library>) -> vcmp_library::Result<Rc<Self>> {
        let albums = library.albums()?;
        let browser = AlbumBrowser::connect(window, &albums);
        let folders = FolderBrowser::connect(window, library.clone());
        let artist_grid = CoverGrid::connect(window, Grid::Artists, Vec::new(), 2);
        let artist_albums_grid = CoverGrid::connect(window, Grid::ArtistAlbums, Vec::new(), 2);
        let weak = Rc::downgrade(&artist_grid);
        window.on_artist_range_changed(move |first, count| {
            if let Some(grid) = weak.upgrade() {
                grid.show_range(first.max(0) as usize, count.max(0) as usize);
            }
        });
        let view = Rc::new(Self {
            window: window.as_weak(),
            library,
            browser,
            open: RefCell::default(),
            artists: RefCell::default(),
            artist_grid,
            artist_albums_grid,
            artist: RefCell::default(),
            found: RefCell::default(),
            search_artists_grid: CoverGrid::connect(window, Grid::SearchArtists, Vec::new(), 2),
            search_albums_grid: CoverGrid::connect(window, Grid::SearchAlbums, Vec::new(), 2),
            search_timer: Timer::default(),
            all_albums: RefCell::new(albums),
            all_songs: RefCell::default(),
            songs: RefCell::default(),
            songs_filter_timer: Timer::default(),
            folders,
        });
        if let Some((column, descending)) = view
            .library
            .setting(SETTING_SONGS_SORT)
            .ok()
            .flatten()
            .and_then(|value| parse_sort(&value))
        {
            window.set_songs_sort_column(column);
            window.set_songs_sort_descending(descending);
        }
        view.show_artists();
        let weak = Rc::downgrade(&view);
        window.on_album_activated(move |index| {
            let Some(view) = weak.upgrade() else { return };
            if let Some(album) = usize::try_from(index)
                .ok()
                .and_then(|index| view.browser.album(index))
            {
                view.open_album(album, false);
            }
        });
        let weak = Rc::downgrade(&view);
        window.on_artist_activated(move |index| {
            let Some(view) = weak.upgrade() else { return };
            let artist = usize::try_from(index)
                .ok()
                .and_then(|index| view.artists.borrow().get(index).cloned());
            if let Some(artist) = artist {
                view.open_artist(artist);
            }
        });
        let weak = Rc::downgrade(&view);
        window.on_artists_filter_changed(move || {
            if let Some(view) = weak.upgrade() {
                view.show_artists();
            }
        });
        let weak = Rc::downgrade(&view);
        window.on_search_edited(move |_| {
            let Some(view) = weak.upgrade() else { return };
            let weak = Rc::downgrade(&view);
            view.search_timer
                .start(TimerMode::SingleShot, SEARCH_DELAY, move || {
                    if let Some(view) = weak.upgrade() {
                        view.search();
                    }
                });
        });
        let weak = Rc::downgrade(&view);
        window.on_search_artist_activated(move |index| {
            let Some(view) = weak.upgrade() else { return };
            let artist = usize::try_from(index)
                .ok()
                .and_then(|index| view.found.borrow().artists.get(index).cloned());
            if let Some(artist) = artist {
                view.open_artist(artist);
            }
        });
        let weak = Rc::downgrade(&view);
        window.on_search_album_activated(move |index| {
            let Some(view) = weak.upgrade() else { return };
            let album = usize::try_from(index)
                .ok()
                .and_then(|index| view.found.borrow().albums.get(index).cloned());
            if let Some(album) = album {
                view.open_album(album, false);
            }
        });
        let weak = Rc::downgrade(&view);
        window.on_albums_filter_edited(move |_| {
            if let Some(view) = weak.upgrade() {
                view.show_albums();
            }
        });
        let weak = Rc::downgrade(&view);
        window.on_artists_filter_edited(move |_| {
            if let Some(view) = weak.upgrade() {
                view.show_artists();
            }
        });
        let weak = Rc::downgrade(&view);
        window.on_songs_filter_edited(move |_| {
            let Some(view) = weak.upgrade() else { return };
            // Tens of thousands of rows: filter once typing pauses.
            let weak = Rc::downgrade(&view);
            view.songs_filter_timer
                .start(TimerMode::SingleShot, SEARCH_DELAY, move || {
                    if let Some(view) = weak.upgrade() {
                        view.show_songs();
                    }
                });
        });
        let weak = Rc::downgrade(&view);
        window.on_songs_needed(move || {
            let Some(view) = weak.upgrade() else { return };
            if view.all_songs.borrow().is_none() {
                view.load_songs();
            }
        });
        let weak = Rc::downgrade(&view);
        window.on_sort_songs(move |column| {
            let Some(view) = weak.upgrade() else { return };
            let Some(window) = view.window.upgrade() else {
                return;
            };
            // The same column again reverses the order; another starts A to Z.
            let descending =
                window.get_songs_sort_column() == column && !window.get_songs_sort_descending();
            window.set_songs_sort_column(column);
            window.set_songs_sort_descending(descending);
            let value = format!(
                "{}{}",
                column_name(column),
                if descending { ":reverse" } else { "" }
            );
            if let Err(error) = view.library.set_setting(SETTING_SONGS_SORT, &value) {
                log::warn!("could not save the songs order: {error}");
            }
            view.load_songs();
        });
        let weak = Rc::downgrade(&view);
        window.on_artist_album_activated(move |index| {
            let Some(view) = weak.upgrade() else { return };
            let album = view.artist.borrow().as_ref().and_then(|open| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| open.albums.get(index).cloned())
            });
            if let Some(album) = album {
                view.open_album(album, true);
            }
        });
        Ok(view)
    }

    pub fn browser(&self) -> &Rc<AlbumBrowser> {
        &self.browser
    }

    /// The album an album page shows, while it is open.
    pub fn shown_album(&self) -> Option<Album> {
        let open = self
            .window
            .upgrade()
            .is_some_and(|window| window.get_album_open() || window.get_artist_album_open());
        if open {
            self.open.borrow().clone()
        } else {
            None
        }
    }

    /// Reads the albums again after the library changed: the grid, and the album
    /// page when one is open. An album that disappeared stays on screen, marked as
    /// gone, until the user leaves it.
    pub fn reload(&self) {
        let albums = match self.library.albums() {
            Ok(albums) => albums,
            Err(error) => {
                log::error!("could not read albums: {error}");
                return;
            }
        };
        *self.all_albums.borrow_mut() = albums.clone();
        self.show_albums();
        self.show_artists();
        self.search();
        if self.all_songs.borrow().is_some() {
            self.load_songs();
        }
        self.folders.reload();
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if window.get_artist_open() {
            self.reload_artist(&window);
        }
        let open = self.open.borrow().clone();
        let album_open = window.get_album_open() || window.get_artist_album_open();
        if let (true, Some(open)) = (album_open, open) {
            match albums.into_iter().find(|album| album.id == open.id) {
                Some(album) => show_album(&window, &self.library, &album),
                None => {
                    let mut detail = window.get_album_detail();
                    detail.missing = true;
                    window.set_album_detail(detail);
                    window.set_album_tracks(ModelRc::default());
                }
            }
        }
    }

    /// The artist the artist page shows, while it is open, with everything by them.
    pub fn shown_artist(&self) -> Option<ArtistTracks> {
        let window = self.window.upgrade()?;
        if !window.get_artist_open() {
            return None;
        }
        let open = self.artist.borrow().clone()?;
        let mut tracks = Vec::new();
        for album in &open.albums {
            match self.library.album_tracks(album.id) {
                Ok(album_tracks) => tracks.extend(album_tracks),
                Err(error) => {
                    log::error!("could not read the tracks of {:?}: {error}", album.title)
                }
            }
        }
        let appearances_from = tracks.len();
        tracks.extend(open.appearances);
        Some(ArtistTracks {
            artist: open.artist,
            tracks,
            appearances_from,
        })
    }

    /// Opens an album on the Albums page by its id, as the player bar does.
    pub fn open_album_by_id(&self, id: AlbumId) {
        match self.library.album(id) {
            Ok(Some(album)) => self.open_album(album, false),
            Ok(None) => {}
            Err(error) => log::error!("could not read album {}: {error}", id.0),
        }
    }

    /// Opens an artist on the Artists page by their id, as the player bar does.
    pub fn open_artist_by_id(&self, id: ArtistId) {
        match self.library.artist(id) {
            Ok(Some(artist)) => self.open_artist(artist),
            Ok(None) => {}
            Err(error) => log::error!("could not read artist {}: {error}", id.0),
        }
    }

    /// Opens an album on the Albums page, or (`from_artist`) on the artist's page.
    /// One album page is open at a time, so the other page's closes.
    fn open_album(&self, album: Album, from_artist: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        show_album(&window, &self.library, &album);
        *self.open.borrow_mut() = Some(album);
        window.set_album_open(!from_artist);
        window.set_artist_album_open(from_artist);
    }

    /// Fills the artist grid: album artists, or everyone credited on a track.
    fn show_artists(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let all = window.get_artists_show_all();
        let filter = Filter::new(&window.get_artists_filter());
        let artists: Vec<Artist> = match self.library.artists() {
            Ok(artists) => artists
                .into_iter()
                .filter(|artist| all || artist.album_count > 0)
                .filter(|artist| filter.matches(&[&artist.name]))
                .collect(),
            Err(error) => {
                log::error!("could not read artists: {error}");
                Vec::new()
            }
        };
        self.artist_grid.replace(self.artist_cards(&artists));
        *self.artists.borrow_mut() = artists;
    }

    /// Artist cards. Until artists have photos, an artist shows the cover of their
    /// newest album.
    fn artist_cards(&self, artists: &[Artist]) -> Vec<Card> {
        let albums = self.library.albums().unwrap_or_default();
        let picture = |artist: &Artist| -> Option<PathBuf> {
            albums
                .iter()
                .filter(|album| album.artist_id == Some(artist.id))
                .filter_map(|album| Some((album.year, album.artwork.as_ref()?)))
                .max_by_key(|(year, _)| *year)
                .map(|(_, artwork)| artwork.thumbnail.clone())
        };
        artists
            .iter()
            .map(|artist| (artist_tile(artist), picture(artist)))
            .collect()
    }

    /// Searches for the text in the search field and shows what it finds.
    pub fn search(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let text = window.get_search_text();
        let found = match self.library.search(&text, SEARCH_TRACKS) {
            Ok(results) => Found {
                artists: results.artists.into_iter().take(SEARCH_CARDS).collect(),
                albums: results.albums.into_iter().take(SEARCH_CARDS).collect(),
                tracks: results.tracks,
            },
            Err(error) => {
                log::error!("could not search for {text:?}: {error}");
                Found::default()
            }
        };
        self.search_artists_grid
            .replace(self.artist_cards(&found.artists));
        self.search_artists_grid.show_range(0, found.artists.len());
        self.search_albums_grid.replace(cards(&found.albums));
        self.search_albums_grid.show_range(0, found.albums.len());
        let rows: Vec<TrackRow> = found
            .tracks
            .iter()
            .map(|track| {
                // Artist and album in one column; the separator is not a word to
                // translate.
                let artist = track.artist.as_deref().unwrap_or_default();
                let album = track.album.as_deref().unwrap_or_default();
                let source = match (artist.is_empty(), album.is_empty()) {
                    (false, false) => format!("{artist} · {album}"),
                    (false, true) => artist.to_string(),
                    (true, _) => album.to_string(),
                };
                TrackRow {
                    title: track.title.as_str().into(),
                    artist: source.into(),
                    duration: format_duration(track.duration).into(),
                    path: track.path.to_string_lossy().as_ref().into(),
                    ..TrackRow::default()
                }
            })
            .collect();
        window.set_search_tracks(ModelRc::new(VecModel::from(rows)));
        *self.found.borrow_mut() = found;
    }

    /// Shows the albums the Albums page's filter leaves.
    fn show_albums(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let filter = Filter::new(&window.get_albums_filter());
        let albums: Vec<Album> = self
            .all_albums
            .borrow()
            .iter()
            .filter(|album| {
                filter.matches(&[&album.title, album.artist.as_deref().unwrap_or_default()])
            })
            .cloned()
            .collect();
        self.browser.replace(&albums);
    }

    /// Reads every track, in the Songs page's order, and shows those its filter
    /// leaves.
    fn load_songs(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let mut tracks = self.library.tracks().unwrap_or_else(|error| {
            log::error!("could not read the tracks: {error}");
            Vec::new()
        });
        sort_songs(
            &mut tracks,
            window.get_songs_sort_column(),
            window.get_songs_sort_descending(),
        );
        *self.all_songs.borrow_mut() = Some(tracks);
        self.show_songs();
    }

    /// Shows the songs the Songs page's filter leaves.
    fn show_songs(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let filter = Filter::new(&window.get_songs_filter());
        let tracks: Vec<Track> = self
            .all_songs
            .borrow()
            .iter()
            .flatten()
            .filter(|track| {
                filter.matches(&[
                    &track.title,
                    track.artist.as_deref().unwrap_or_default(),
                    track.album.as_deref().unwrap_or_default(),
                ])
            })
            .cloned()
            .collect();
        let rows: Vec<SongRow> = tracks
            .iter()
            .map(|track| SongRow {
                title: track.title.as_str().into(),
                artist: track.artist.clone().unwrap_or_default().into(),
                album: track.album.clone().unwrap_or_default().into(),
                year: track
                    .year
                    .map(|year| year.to_string())
                    .unwrap_or_default()
                    .into(),
                duration: format_duration(track.duration).into(),
                path: track.path.to_string_lossy().as_ref().into(),
            })
            .collect();
        window.set_songs(ModelRc::new(VecModel::from(rows)));
        *self.songs.borrow_mut() = tracks;
    }

    /// The tracks the Songs page shows, in its order (none until it was shown).
    pub fn songs(&self) -> Vec<Track> {
        self.songs.borrow().clone()
    }

    /// One track of the Songs page.
    pub fn song(&self, index: usize) -> Option<Track> {
        self.songs.borrow().get(index).cloned()
    }

    /// The folder browser.
    pub fn folders(&self) -> &Rc<FolderBrowser> {
        &self.folders
    }

    /// The tracks the last search found, in the order shown.
    pub fn found_tracks(&self) -> Vec<Track> {
        self.found.borrow().tracks.clone()
    }

    fn open_artist(&self, artist: Artist) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let albums = self
            .library
            .artist_albums(artist.id)
            .unwrap_or_else(|error| {
                log::error!("could not read the albums of {:?}: {error}", artist.name);
                Vec::new()
            });
        let appearances = self
            .library
            .artist_appearances(artist.id)
            .unwrap_or_else(|error| {
                log::error!("could not read the tracks of {:?}: {error}", artist.name);
                Vec::new()
            });
        let open = OpenArtist {
            artist,
            albums,
            appearances,
        };
        self.show_artist(&window, &open);
        *self.artist.borrow_mut() = Some(open);
        window.set_artist_album_open(false);
        window.set_artist_open(true);
    }

    /// Reads the open artist again; one who left the library stays on screen, marked
    /// as gone, until the user leaves the page.
    fn reload_artist(&self, window: &AppWindow) {
        let Some(open) = self.artist.borrow().clone() else {
            return;
        };
        match self.library.artist(open.artist.id) {
            Ok(Some(artist)) if artist.track_count > 0 => {
                let albums = self.library.artist_albums(artist.id).unwrap_or_default();
                let appearances = self
                    .library
                    .artist_appearances(artist.id)
                    .unwrap_or_default();
                let open = OpenArtist {
                    artist,
                    albums,
                    appearances,
                };
                self.show_artist(window, &open);
                *self.artist.borrow_mut() = Some(open);
            }
            _ => {
                let mut detail = window.get_artist_detail();
                detail.missing = true;
                window.set_artist_detail(detail);
                self.artist_albums_grid.replace(Vec::new());
                window.set_artist_appearances(ModelRc::default());
            }
        }
    }
}

/// A page filter's words, lowercased. A row passes when every word is in one of its
/// names, ignoring case; an empty filter passes everything.
struct Filter(Vec<String>);

impl Filter {
    fn new(text: &str) -> Self {
        Self(text.split_whitespace().map(str::to_lowercase).collect())
    }

    fn matches(&self, names: &[&str]) -> bool {
        if self.0.is_empty() {
            return true;
        }
        let names: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();
        self.0
            .iter()
            .all(|word| names.iter().any(|name| name.contains(word.as_str())))
    }
}

/// Sorts the Songs page. Missing values (untagged artist, album or year) go after
/// the rest, and ties fall back to the album order, so an artist's songs stay in
/// album and track order. Reversing turns the whole order around.
fn sort_songs(tracks: &mut [Track], column: SongColumn, descending: bool) {
    let text = |value: &Option<String>| {
        let value = value.as_deref().unwrap_or_default();
        (value.is_empty(), value.to_lowercase())
    };
    let year = |track: &Track| (track.year.is_none(), track.year);
    let album_order = |track: &Track| {
        (
            year(track),
            text(&track.album),
            track.disc_number,
            track.track_number,
        )
    };
    match column {
        SongColumn::Title => {
            tracks.sort_by_cached_key(|t| (t.title.to_lowercase(), text(&t.artist)))
        }
        SongColumn::Artist => tracks.sort_by_cached_key(|t| (text(&t.artist), album_order(t))),
        SongColumn::Album => tracks.sort_by_cached_key(|t| {
            (
                text(&t.album),
                t.disc_number,
                t.track_number,
                t.title.to_lowercase(),
            )
        }),
        SongColumn::Year => tracks.sort_by_cached_key(album_order),
        SongColumn::Duration => tracks.sort_by_key(|t| t.duration),
    }
    if descending {
        tracks.reverse();
    }
}

fn column_name(column: SongColumn) -> &'static str {
    match column {
        SongColumn::Title => "title",
        SongColumn::Artist => "artist",
        SongColumn::Album => "album",
        SongColumn::Year => "year",
        SongColumn::Duration => "duration",
    }
}

fn parse_sort(value: &str) -> Option<(SongColumn, bool)> {
    let (name, descending) = match value.strip_suffix(":reverse") {
        Some(name) => (name, true),
        None => (value, false),
    };
    let column = match name {
        "title" => SongColumn::Title,
        "artist" => SongColumn::Artist,
        "album" => SongColumn::Album,
        "year" => SongColumn::Year,
        "duration" => SongColumn::Duration,
        _ => return None,
    };
    Some((column, descending))
}

fn artist_tile(artist: &Artist) -> AlbumTile {
    AlbumTile {
        id: artist.id.0 as i32,
        title: artist.name.as_str().into(),
        initial: initial(&artist.name).into(),
        album_count: artist.album_count as i32,
        track_count: artist.track_count as i32,
        ..AlbumTile::default()
    }
}

impl LibraryView {
    fn show_artist(&self, window: &AppWindow, open: &OpenArtist) {
        // The artist's name is above the cards, so their year goes below each title.
        let mut albums = cards(&open.albums);
        for ((tile, _), album) in albums.iter_mut().zip(&open.albums) {
            tile.artist = album
                .year
                .map(|year| year.to_string())
                .unwrap_or_default()
                .into();
        }
        let count = albums.len();
        self.artist_albums_grid.replace(albums);
        // The page shows all of them at once.
        self.artist_albums_grid.show_range(0, count);
        show_artist_details(window, open);
    }
}

fn show_artist_details(window: &AppWindow, open: &OpenArtist) {
    // The same picture as on the artist's card: their newest album's cover.
    let picture = open
        .albums
        .iter()
        .filter_map(|album| Some((album.year, album.artwork.as_ref()?)))
        .max_by_key(|(year, _)| *year)
        .and_then(|(_, artwork)| Image::load_from_path(&artwork.thumbnail).ok());
    let appearances: Vec<TrackRow> = open
        .appearances
        .iter()
        .map(|track| TrackRow {
            title: track.title.as_str().into(),
            artist: track.album.clone().unwrap_or_default().into(),
            duration: format_duration(track.duration).into(),
            path: track.path.to_string_lossy().as_ref().into(),
            ..TrackRow::default()
        })
        .collect();
    window.set_artist_detail(ArtistDetail {
        name: open.artist.name.as_str().into(),
        initial: initial(&open.artist.name).into(),
        album_count: open.artist.album_count as i32,
        track_count: open.artist.track_count as i32,
        has_picture: picture.is_some(),
        picture: picture.unwrap_or_default(),
        missing: false,
    });
    window.set_artist_appearances(ModelRc::new(VecModel::from(appearances)));
}

fn show_album(window: &AppWindow, library: &Library, album: &Album) {
    let (tracks, failed) = match library.album_tracks(album.id) {
        Ok(tracks) => (tracks, false),
        Err(error) => {
            log::error!(
                "could not read the tracks of album {:?}: {error}",
                album.title
            );
            (Vec::new(), true)
        }
    };
    let total: Duration = tracks.iter().map(|track| track.duration).sum();
    let mut discs = tracks.iter().map(disc).collect::<Vec<_>>();
    discs.dedup();

    let mut previous_disc = None;
    let rows: Vec<TrackRow> = tracks
        .iter()
        .map(|track| {
            let disc_start = previous_disc != Some(disc(track));
            previous_disc = Some(disc(track));
            TrackRow {
                number: track
                    .track_number
                    .map(|n| n.to_string())
                    .unwrap_or_default()
                    .into(),
                title: track.title.as_str().into(),
                artist: shown_artist(track, album).into(),
                duration: format_duration(track.duration).into(),
                disc: disc(track) as i32,
                disc_start,
                path: track.path.to_string_lossy().as_ref().into(),
            }
        })
        .collect();

    let cover = album
        .artwork
        .as_ref()
        .and_then(|artwork| Image::load_from_path(&artwork.thumbnail).ok());
    window.set_album_detail(AlbumDetail {
        title: album.title.as_str().into(),
        artist: album.artist.clone().unwrap_or_default().into(),
        year: album
            .year
            .map(|year| year.to_string())
            .unwrap_or_default()
            .into(),
        track_count: rows.len() as i32,
        minutes: total.as_secs().div_ceil(60) as i32,
        has_cover: cover.is_some(),
        cover: cover.unwrap_or_default(),
        initial: initial(&album.title).into(),
        failed,
        missing: false,
    });
    window.set_album_tracks(ModelRc::new(VecModel::from(rows)));
    window.set_album_multi_disc(discs.len() > 1);
}

fn disc(track: &Track) -> u32 {
    track.disc_number.unwrap_or(1)
}

/// The track's artist, when it is not simply the album's artist.
fn shown_artist(track: &Track, album: &Album) -> String {
    match (&track.artist, &album.artist) {
        (Some(artist), Some(album_artist))
            if artist.to_lowercase() == album_artist.to_lowercase() =>
        {
            String::new()
        }
        (Some(artist), _) => artist.clone(),
        (None, _) => String::new(),
    }
}

/// `m:ss`, or `h:mm:ss` from an hour on.
pub(crate) fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, year: i32, seconds: u64) -> Track {
        Track {
            id: vcmp_library::TrackId(0),
            path: std::path::PathBuf::from(title),
            available: true,
            format: None,
            title: title.into(),
            artist: Some(artist.into()),
            artist_id: None,
            album: Some(format!("{artist} album")),
            album_id: None,
            album_artist: None,
            track_number: None,
            disc_number: None,
            year: Some(year),
            genre: None,
            duration: Duration::from_secs(seconds),
        }
    }

    #[test]
    fn songs_sort_by_each_column_and_reverse() {
        let mut tracks = vec![
            track("b", "Zed", 2001, 300),
            track("A", "alpha", 1999, 100),
            track("c", "Mid", 2020, 200),
        ];
        let titles =
            |tracks: &[Track]| -> Vec<String> { tracks.iter().map(|t| t.title.clone()).collect() };
        sort_songs(&mut tracks, SongColumn::Title, false);
        assert_eq!(titles(&tracks), ["A", "b", "c"], "ignoring case");
        sort_songs(&mut tracks, SongColumn::Artist, false);
        assert_eq!(titles(&tracks), ["A", "c", "b"]);
        sort_songs(&mut tracks, SongColumn::Year, true);
        assert_eq!(titles(&tracks), ["c", "b", "A"]);
        sort_songs(&mut tracks, SongColumn::Duration, false);
        assert_eq!(titles(&tracks), ["A", "c", "b"]);

        let mut untagged = track("0 untagged", "", 0, 10);
        untagged.artist = None;
        untagged.year = None;
        tracks.push(untagged);
        sort_songs(&mut tracks, SongColumn::Artist, false);
        assert_eq!(
            titles(&tracks).last().unwrap(),
            "0 untagged",
            "unknown artists last"
        );
        sort_songs(&mut tracks, SongColumn::Year, false);
        assert_eq!(
            titles(&tracks).last().unwrap(),
            "0 untagged",
            "unknown years last"
        );
    }

    #[test]
    fn filters_need_every_word_in_some_name() {
        let filter = Filter::new("  pink WALL ");
        assert!(filter.matches(&["The Wall", "Pink Floyd"]));
        assert!(!filter.matches(&["The Wall", "Roger Waters"]));
        assert!(Filter::new("").matches(&["anything"]));
        assert!(Filter::new("ライブ").matches(&["ラブライブ! Best"]));
    }

    #[test]
    fn the_songs_order_reads_back() {
        for column in [
            SongColumn::Title,
            SongColumn::Artist,
            SongColumn::Album,
            SongColumn::Year,
            SongColumn::Duration,
        ] {
            for descending in [false, true] {
                let value = format!(
                    "{}{}",
                    column_name(column),
                    if descending { ":reverse" } else { "" }
                );
                assert_eq!(parse_sort(&value), Some((column, descending)));
            }
        }
        assert_eq!(parse_sort("colour"), None);
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(Duration::from_secs(5)), "0:05");
        assert_eq!(format_duration(Duration::from_millis(225_900)), "3:45");
        assert_eq!(
            format_duration(Duration::from_secs(3 * 3600 + 7)),
            "3:00:07"
        );
    }
}
