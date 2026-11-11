//! Playlists in the window: the Playlists page's grid, the playlist page, the list in
//! the sidebar, the Add to playlist menus, and exporting a playlist as an M3U file or
//! importing one.
//! Reloaded after every library change, since an entry dims while its file is gone
//! and comes back with it.
//!
//! The Favourites playlist (`crate::favourites`) is drawn as a playlist, first in the
//! grid and the sidebar, but it is no playlist of the library: it can't be renamed,
//! deleted, reordered or given a picture, and the Add to playlist menus don't list
//! it. Places in the grid and the sidebar count from it; places in
//! `Playlists.names`, which the menus use, count the owner's playlists alone.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};
use vcmp_library::{
    FavouritesSummary, Library, M3uExport, M3uImport, Playlist, PlaylistEntry, PlaylistId, Track,
    TrackId, animation, m3u_file_name,
};

use crate::albums::{Card, CoverGrid, Grid, initial};
use crate::favourites::{liked, row_id};
use crate::library_view::format_duration;
use crate::moving_pictures::{self, Picture};
use crate::selection;
use crate::{
    AlbumTile, AppWindow, Favourites, PlaylistDetail, PlaylistImport, Playlists, SongSelection,
    TrackRow, TrackSelection, UnmatchedEntry,
};

/// A playlist the playlist page can show: the favourites, or one of the library's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaylistKey {
    Favourites,
    Playlist(PlaylistId),
}

/// Image files the picture chooser offers.
const PICTURE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];

/// Playlist file extensions the export's save dialog and the import's open dialog
/// offer; the first is added to an export's name chosen without either.
const M3U_EXTENSIONS: &[&str] = &["m3u8", "m3u"];

pub struct PlaylistBrowser {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    grid: Rc<CoverGrid>,
    // Every playlist of the library, in the grid's and the sidebar's order, after
    // Favourites.
    playlists: RefCell<Vec<Playlist>>,
    // The playlist the playlist page shows (or showed last), and its entries; the
    // favourites' are numbered from 0, newest first.
    open: Cell<Option<PlaylistKey>>,
    entries: RefCell<Vec<PlaylistEntry>>,
    // The last picture chosen for the open playlist could not be used.
    picture_failed: Cell<bool>,
    // The open playlist's last export, and the folder of the last one of any
    // playlist, where the save dialog starts.
    exported: RefCell<Option<Exported>>,
    export_folder: RefCell<Option<PathBuf>>,
    // The folder of the last file imported, where the open dialog starts.
    import_folder: RefCell<Option<PathBuf>>,
}

/// What an export of the open playlist did, for its page.
struct Exported {
    file: PathBuf,
    // `None`: the file could not be written.
    report: Option<M3uExport>,
}

/// The open playlist's entries, for playing it.
pub struct OpenPlaylist {
    pub key: PlaylistKey,
    pub entries: Vec<PlaylistEntry>,
}

impl PlaylistBrowser {
    /// Shows the library's playlists. Keep the browser alive as long as the window.
    pub fn connect(window: &AppWindow, library: Rc<Library>) -> Rc<Self> {
        let grid = CoverGrid::connect(window, Grid::Playlists, Vec::new(), 1);
        let browser = Rc::new(Self {
            window: window.as_weak(),
            library,
            grid,
            playlists: RefCell::default(),
            open: Cell::default(),
            entries: RefCell::default(),
            picture_failed: Cell::default(),
            exported: RefCell::default(),
            export_folder: RefCell::default(),
            import_folder: RefCell::default(),
        });
        browser.reload();

        let weak = Rc::downgrade(&browser);
        window.on_playlist_range_changed(move |first, count| {
            if let Some(browser) = weak.upgrade() {
                browser
                    .grid
                    .show_range(first.max(0) as usize, count.max(0) as usize);
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_playlist_activated(move |index| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if let Some(key) = browser.key_at(index) {
                browser.open(key);
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_create_playlist(move |name| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            match browser.library.create_playlist(&name) {
                Ok(id) => {
                    browser.reload();
                    browser.open(PlaylistKey::Playlist(id));
                }
                Err(error) => log::error!("could not make a playlist: {error}"),
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_import_playlist(move || {
            if let Some(browser) = weak.upgrade() {
                browser.choose_import_file();
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_rename_playlist(move |name| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            // An empty name keeps the old one.
            if name.trim().is_empty() {
                return;
            }
            browser.edit("rename the playlist", |library, id| {
                library.rename_playlist(id, &name)
            });
        });
        let weak = Rc::downgrade(&browser);
        window.on_delete_playlist(move || {
            if let Some(browser) = weak.upgrade() {
                browser.edit("delete the playlist", Library::delete_playlist);
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_choose_playlist_picture(move || {
            if let Some(browser) = weak.upgrade() {
                browser.choose_picture();
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_clear_playlist_picture(move || {
            if let Some(browser) = weak.upgrade() {
                browser.picture_failed.set(false);
                browser.edit(
                    "remove the playlist's picture",
                    Library::clear_playlist_image,
                );
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_export_playlist(move || {
            if let Some(browser) = weak.upgrade() {
                browser.choose_export_file();
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_remove_playlist_track(move |index| {
            let (Some(browser), Ok(index)) = (weak.upgrade(), usize::try_from(index)) else {
                return;
            };
            let Some(window) = browser.window.upgrade() else {
                return;
            };
            // The selected entries, when the one asked for is among them.
            let positions: Vec<u32> = selection::chosen(&window.get_playlist_tracks(), index)
                .into_iter()
                .map(|position| position as u32)
                .collect();
            browser.edit("remove the tracks", |library, id| {
                library.remove_from_playlist(id, &positions)
            });
        });
        let weak = Rc::downgrade(&browser);
        window.on_move_playlist_track(move |from, to| {
            let (Some(browser), Ok(from), Ok(to)) =
                (weak.upgrade(), u32::try_from(from), u32::try_from(to))
            else {
                return;
            };
            browser.edit("move the track", |library, id| {
                library.move_in_playlist(id, from, to)
            });
        });
        let weak = Rc::downgrade(&browser);
        window
            .global::<Playlists>()
            .on_add_track(move |index, path| {
                if let Some(browser) = weak.upgrade() {
                    browser.add_track(index, Path::new(path.as_str()));
                }
            });
        let weak = Rc::downgrade(&browser);
        window
            .global::<TrackSelection>()
            .on_add_to_playlist(move |rows, row, index| {
                if let Some(browser) = weak.upgrade() {
                    browser.add_paths(index, &selection::chosen_paths(&rows, row));
                }
            });
        let weak = Rc::downgrade(&browser);
        window
            .global::<SongSelection>()
            .on_add_to_playlist(move |rows, row, index| {
                if let Some(browser) = weak.upgrade() {
                    browser.add_paths(index, &selection::chosen_paths(&rows, row));
                }
            });
        browser
    }

    /// Reads the playlists again: the grid, the sidebar and menus, and the playlist
    /// page while one is open.
    pub fn reload(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let playlists = self.library.playlists().unwrap_or_else(|error| {
            log::error!("could not read the playlists: {error}");
            Vec::new()
        });
        let favourites = self.library.favourites_summary().unwrap_or_else(|error| {
            log::error!("could not read the favourites: {error}");
            FavouritesSummary::default()
        });
        let mut grid = vec![favourites_card(&window, &favourites)];
        grid.extend(cards(&playlists));
        self.grid.replace(grid);
        let names: Vec<SharedString> = playlists
            .iter()
            .map(|playlist| playlist.name.as_str().into())
            .collect();
        window
            .global::<Playlists>()
            .set_names(ModelRc::new(VecModel::from(names)));
        *self.playlists.borrow_mut() = playlists;
        if let Some(key) = self.open.get() {
            self.show(&window, key);
        }
    }

    /// The favourites changed: the Favourites card and, while it is open, its page.
    pub fn favourites_changed(&self) {
        self.reload();
    }

    /// Opens a playlist on the Playlists page.
    pub fn open(&self, key: PlaylistKey) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if self.open.get() != Some(key) {
            self.picture_failed.set(false);
            self.exported.take();
        }
        self.open.set(Some(key));
        self.show(&window, key);
        window.set_playlist_open(true);
    }

    /// The playlist the playlist page shows, while it is open.
    pub fn shown(&self) -> Option<OpenPlaylist> {
        let window = self.window.upgrade()?;
        if !window.get_playlist_open() {
            return None;
        }
        Some(OpenPlaylist {
            key: self.open.get()?,
            entries: self.entries.borrow().clone(),
        })
    }

    /// The owner's playlist at `index` in `Playlists.names`.
    fn at(&self, index: i32) -> Option<Playlist> {
        let index = usize::try_from(index).ok()?;
        self.playlists.borrow().get(index).cloned()
    }

    /// The playlist at `index` in the grid and the sidebar, Favourites first.
    fn key_at(&self, index: i32) -> Option<PlaylistKey> {
        match index {
            0 => Some(PlaylistKey::Favourites),
            index => self
                .at(index - 1)
                .map(|playlist| PlaylistKey::Playlist(playlist.id)),
        }
    }

    /// Fills the playlist page. A playlist that is gone closes it.
    fn show(&self, window: &AppWindow, key: PlaylistKey) {
        match key {
            PlaylistKey::Favourites => self.show_favourites(window),
            PlaylistKey::Playlist(id) => self.show_playlist(window, id),
        }
    }

    /// Fills the playlist page with the favourites, newest first.
    fn show_favourites(&self, window: &AppWindow) {
        let (favourites, failed) = match self.library.favourites() {
            Ok(favourites) => (favourites, false),
            Err(error) => {
                log::error!("could not read the favourites: {error}");
                (Vec::new(), true)
            }
        };
        let entries: Vec<PlaylistEntry> = favourites
            .into_iter()
            .enumerate()
            .map(|(position, favourite)| PlaylistEntry {
                position: position as u32,
                track: favourite.track,
            })
            .collect();
        let available: Vec<&Track> = entries
            .iter()
            .map(|entry| &entry.track)
            .filter(|track| track.available)
            .collect();
        let minutes = available
            .iter()
            .map(|track| track.duration)
            .sum::<std::time::Duration>()
            .as_secs()
            .div_ceil(60);
        let rows: Vec<TrackRow> = entries.iter().map(entry_row).collect();
        let (exported_to, export_failed, export_missing, export_left_out, export_from_servers) =
            self.export_report();
        window.set_playlist_detail(PlaylistDetail {
            favourites: true,
            track_count: entries.len() as i32,
            unavailable_count: (entries.len() - available.len()) as i32,
            minutes: minutes as i32,
            failed,
            exported_to,
            export_failed,
            export_missing,
            export_left_out,
            export_from_servers,
            ..PlaylistDetail::default()
        });
        moving_pictures::show(window, Picture::Playlist, None);
        window.set_playlist_tracks(ModelRc::new(VecModel::from(rows)));
        window.set_open_playlist_index(0);
        *self.entries.borrow_mut() = entries;
    }

    /// What the open playlist's last export did, as its page says it: the file, and
    /// whether writing it failed, how many entries' files were missing and how many
    /// were left out.
    fn export_report(&self) -> (SharedString, bool, i32, i32, i32) {
        let exported = self.exported.borrow();
        let report = exported
            .as_ref()
            .and_then(|exported| exported.report.as_ref());
        (
            exported
                .as_ref()
                .map(|exported| exported.file.to_string_lossy().as_ref().into())
                .unwrap_or_default(),
            exported.is_some() && report.is_none(),
            report.map_or(0, |report| report.missing as i32),
            report.map_or(0, |report| report.left_out.len() as i32),
            report.map_or(0, |report| report.from_servers.len() as i32),
        )
    }

    /// Fills the playlist page with one of the library's playlists. One that is gone
    /// closes it.
    fn show_playlist(&self, window: &AppWindow, id: PlaylistId) {
        let position = self
            .playlists
            .borrow()
            .iter()
            .position(|playlist| playlist.id == id);
        let playlist = position.and_then(|position| self.playlists.borrow().get(position).cloned());
        let Some(playlist) = playlist else {
            self.open.set(None);
            self.entries.borrow_mut().clear();
            window.set_open_playlist_index(-1);
            window.set_playlist_open(false);
            return;
        };
        let (entries, failed) = match self.library.playlist_entries(id) {
            Ok(entries) => (entries, false),
            Err(error) => {
                log::error!("could not read playlist {:?}: {error}", playlist.name);
                (Vec::new(), true)
            }
        };
        let rows: Vec<TrackRow> = entries.iter().map(entry_row).collect();
        let picture = playlist
            .picture
            .as_ref()
            .and_then(|path| Image::load_from_path(path).ok());
        let (exported_to, export_failed, export_missing, export_left_out, export_from_servers) =
            self.export_report();
        window.set_playlist_detail(PlaylistDetail {
            favourites: false,
            name: playlist.name.as_str().into(),
            track_count: playlist.track_count as i32,
            unavailable_count: (playlist.track_count - playlist.available_count) as i32,
            minutes: playlist.duration.as_secs().div_ceil(60) as i32,
            has_picture: picture.is_some(),
            picture: picture.unwrap_or_default(),
            frames: ModelRc::default(),
            own_picture: playlist.own_picture,
            initial: initial(&playlist.name).into(),
            failed,
            picture_failed: self.picture_failed.get(),
            exported_to,
            export_failed,
            export_missing,
            export_left_out,
            export_from_servers,
        });
        // Only the owner's own picture may move, not one made from covers.
        let animation = playlist
            .picture
            .as_deref()
            .filter(|_| playlist.own_picture)
            .and_then(animation::beside);
        moving_pictures::show(window, Picture::Playlist, animation);
        window.set_playlist_tracks(ModelRc::new(VecModel::from(rows)));
        // After Favourites.
        window.set_open_playlist_index(position.map_or(-1, |position| position as i32 + 1));
        *self.entries.borrow_mut() = entries;
    }

    /// Changes the open playlist, then shows the change everywhere. Favourites
    /// can't be changed this way.
    fn edit(
        &self,
        what: &str,
        change: impl FnOnce(&Library, PlaylistId) -> vcmp_library::Result<()>,
    ) {
        let Some(PlaylistKey::Playlist(id)) = self.open.get() else {
            return;
        };
        if let Err(error) = change(&self.library, id) {
            log::error!("could not {what}: {error}");
        }
        self.reload();
    }

    fn choose_picture(self: &Rc<Self>) {
        let Some(PlaylistKey::Playlist(id)) = self.open.get() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let picked = slint::spawn_local(async move {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("Images", PICTURE_EXTENSIONS)
                .pick_file()
                .await
            else {
                return;
            };
            let Some(browser) = weak.upgrade() else {
                return;
            };
            let result = browser.library.set_playlist_image(id, file.path());
            if browser.open.get() == Some(PlaylistKey::Playlist(id)) {
                browser.picture_failed.set(result.is_err());
            }
            if let Err(error) = result {
                log::error!(
                    "could not use {} as the picture: {error}",
                    file.path().display()
                );
            }
            browser.reload();
        });
        if let Err(error) = picked {
            log::error!("could not open the picture chooser: {error}");
        }
    }

    /// Asks where to save the open playlist, then exports it there.
    fn choose_export_file(self: &Rc<Self>) {
        let (Some(window), Some(key)) = (self.window.upgrade(), self.open.get()) else {
            return;
        };
        let name = match key {
            PlaylistKey::Favourites => window.global::<Favourites>().get_name().to_string(),
            PlaylistKey::Playlist(id) => self
                .playlists
                .borrow()
                .iter()
                .find(|playlist| playlist.id == id)
                .map(|playlist| playlist.name.clone())
                .unwrap_or_default(),
        };
        let name = m3u_file_name(&name);
        let mut dialog = rfd::AsyncFileDialog::new()
            .add_filter("M3U", M3U_EXTENSIONS)
            .set_file_name(name);
        if let Some(folder) = self.export_folder.borrow().as_ref() {
            dialog = dialog.set_directory(folder);
        }
        let weak = Rc::downgrade(self);
        let chosen = slint::spawn_local(async move {
            let Some(file) = dialog.save_file().await else {
                return;
            };
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if browser.open.get() == Some(key) {
                browser.export_open(file.path());
            }
        });
        if let Err(error) = chosen {
            log::error!("could not open the save dialog: {error}");
        }
    }

    /// Exports the open playlist to `file` (with `.m3u8` added when it names
    /// neither M3U extension) and says on its page how that went.
    pub fn export_open(&self, file: &Path) {
        let Some(key) = self.open.get() else {
            return;
        };
        let known = file
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                M3U_EXTENSIONS
                    .iter()
                    .any(|known| extension.eq_ignore_ascii_case(known))
            });
        let file = if known {
            file.to_path_buf()
        } else {
            let mut name = file.as_os_str().to_owned();
            name.push(".");
            name.push(M3U_EXTENSIONS[0]);
            PathBuf::from(name)
        };
        let written = match key {
            PlaylistKey::Favourites => self.library.export_favourites(&file),
            PlaylistKey::Playlist(id) => self.library.export_playlist(id, &file),
        };
        let report = match written {
            Ok(report) => {
                for path in &report.left_out {
                    log::warn!("left out of {}: {}", file.display(), path.display());
                }
                for name in &report.from_servers {
                    log::info!("left out of {}, from a server: {name}", file.display());
                }
                Some(report)
            }
            Err(error) => {
                log::error!("could not export the playlist: {error}");
                None
            }
        };
        *self.export_folder.borrow_mut() = file.parent().map(Path::to_path_buf);
        *self.exported.borrow_mut() = Some(Exported { file, report });
        self.reload();
    }

    /// Asks for an M3U file, then imports it.
    fn choose_import_file(self: &Rc<Self>) {
        let mut dialog = rfd::AsyncFileDialog::new().add_filter("M3U", M3U_EXTENSIONS);
        if let Some(folder) = self.import_folder.borrow().as_ref() {
            dialog = dialog.set_directory(folder);
        }
        let weak = Rc::downgrade(self);
        let chosen = slint::spawn_local(async move {
            let Some(file) = dialog.pick_file().await else {
                return;
            };
            if let Some(browser) = weak.upgrade() {
                browser.import(file.path());
            }
        });
        if let Err(error) = chosen {
            log::error!("could not open the file dialog: {error}");
        }
    }

    /// Makes a playlist of the library's tracks an M3U file lists and opens it. When
    /// an entry matched nothing, nothing matched, or the file could not be read, the
    /// report says so over the page.
    pub fn import(&self, file: &Path) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        *self.import_folder.borrow_mut() = file.parent().map(Path::to_path_buf);
        let fallback = window.global::<Playlists>().get_new_name();
        let (failed, imported) = match self.library.import_playlist(file, &fallback) {
            Ok(imported) => (false, imported),
            Err(error) => {
                log::error!("could not import {}: {error}", file.display());
                (true, M3uImport::default())
            }
        };
        if let Some(id) = imported.playlist {
            self.reload();
            self.open(PlaylistKey::Playlist(id));
            if imported.unmatched.is_empty() {
                return;
            }
        }
        let unmatched: Vec<UnmatchedEntry> = imported
            .unmatched
            .iter()
            .map(|entry| UnmatchedEntry {
                title: entry.title.as_deref().unwrap_or_default().into(),
                location: entry.location.as_str().into(),
            })
            .collect();
        let report = window.global::<PlaylistImport>();
        report.set_file(
            file.file_name()
                .map(|name| name.to_string_lossy().as_ref().into())
                .unwrap_or_default(),
        );
        report.set_failed(failed);
        report.set_name(imported.name.as_str().into());
        report.set_entries(imported.entries as i32);
        report.set_unmatched(ModelRc::new(VecModel::from(unmatched)));
        report.set_open(true);
    }

    /// Adds the library's track at `path` to playlist `index`, or to a new playlist
    /// for -1.
    fn add_track(&self, index: i32, path: &Path) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let track = match self.library.track_id_for_path(path) {
            Ok(Some(track)) => track,
            Ok(None) => {
                log::warn!("not in the library, so not added: {}", path.display());
                return;
            }
            Err(error) => {
                log::error!("could not look up {}: {error}", path.display());
                return;
            }
        };
        let name = window.global::<Playlists>().get_new_name();
        self.add_tracks(index, &[track], &name);
    }

    /// Adds the library's tracks with these files to playlist `index`, or for -1 to a
    /// new playlist, in this order.
    fn add_paths(&self, index: i32, paths: &[String]) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let tracks: Vec<TrackId> = paths
            .iter()
            .filter_map(
                |path| match self.library.track_id_for_path(Path::new(path)) {
                    Ok(track) => track,
                    Err(error) => {
                        log::error!("could not look up {path}: {error}");
                        None
                    }
                },
            )
            .collect();
        if !tracks.is_empty() {
            let name = window.global::<Playlists>().get_new_name();
            self.add_tracks(index, &tracks, &name);
        }
    }

    /// Adds tracks to playlist `index`, or for -1 to a new playlist called
    /// `new_name`.
    pub fn add_tracks(&self, index: i32, tracks: &[TrackId], new_name: &str) {
        let playlist = match index {
            -1 => self.library.create_playlist(new_name),
            index => match self.at(index) {
                Some(playlist) => Ok(playlist.id),
                None => return,
            },
        };
        if let Err(error) = playlist.and_then(|id| self.library.add_to_playlist(id, tracks)) {
            log::error!(
                "could not add {} tracks to the playlist: {error}",
                tracks.len()
            );
        }
        self.reload();
    }
}

/// Playlist cards: the name, the number of tracks, and the picture.
pub(crate) fn cards(playlists: &[Playlist]) -> Vec<Card> {
    playlists
        .iter()
        .map(|playlist| {
            let tile = AlbumTile {
                id: playlist.id.0 as i32,
                title: playlist.name.as_str().into(),
                track_count: playlist.track_count as i32,
                initial: initial(&playlist.name).into(),
                ..AlbumTile::default()
            };
            (tile, playlist.picture.clone())
        })
        .collect()
}

/// The Favourites card: its fixed name, its number of tracks, and a heart for a
/// picture.
fn favourites_card(window: &AppWindow, favourites: &FavouritesSummary) -> Card {
    let tile = AlbumTile {
        title: window.global::<Favourites>().get_name(),
        track_count: favourites.track_count as i32,
        favourites: true,
        ..AlbumTile::default()
    };
    (tile, None)
}

/// An entry as a row: its place in the playlist, then where the track comes from.
fn entry_row(entry: &PlaylistEntry) -> TrackRow {
    let track = &entry.track;
    TrackRow {
        number: (entry.position + 1).to_string().into(),
        title: track.title.as_str().into(),
        artist: source(track).into(),
        duration: format_duration(track.duration).into(),
        path: track.key().into(),
        unavailable: !track.available,
        labels: crate::labels::chips(track.id),
        track_id: row_id(track.id),
        liked: liked(track.id),
        ..TrackRow::default()
    }
}

/// Artist and album in one column; the separator is not a word to translate.
pub(crate) fn source(track: &Track) -> String {
    let artist = track.artist.as_deref().unwrap_or_default();
    let album = track.album.as_deref().unwrap_or_default();
    match (artist.is_empty(), album.is_empty()) {
        (false, false) => format!("{artist} · {album}"),
        (false, true) => artist.to_string(),
        (true, _) => album.to_string(),
    }
}
