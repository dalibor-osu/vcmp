//! Back and forward through the places the window showed, as a browser does: the
//! mouse's back and forward buttons (caught in `src/shortcuts.rs`) and Alt+Left and
//! Alt+Right. A place is a section on its grid, or the album, artist or playlist
//! page open in it.
//!
//! The window calls `navigated` whenever the section or the page open in it changes,
//! however that came about (the sidebar, a card, a back button, Escape, a link in the
//! player bar), so every way of moving around is remembered without each knowing
//! about the history. Going back or forward shows the place again through the same
//! ways a click would; the `navigated` calls that follow find the window where the
//! history already is, and change nothing.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_library::{AlbumId, ArtistId, PlaylistId};

use crate::library_view::LibraryView;
use crate::{AppWindow, Page};

/// The most places kept behind.
const LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// A section on its grid (or its only view).
    Section(Page),
    Album(AlbumId),
    Artist(ArtistId),
    /// An album opened from an artist's page.
    ArtistAlbum(ArtistId, AlbumId),
    Playlist(PlaylistId),
}

pub struct History {
    window: slint::Weak<AppWindow>,
    view: Rc<LibraryView>,
    here: RefCell<Place>,
    back: RefCell<Vec<Place>>,
    forward: RefCell<Vec<Place>>,
}

impl History {
    /// Keeps the history of `window`. Keep it alive as long as the window.
    pub fn connect(window: &AppWindow, view: Rc<LibraryView>) -> Rc<Self> {
        let history = Rc::new(Self {
            window: window.as_weak(),
            view,
            here: RefCell::new(Place::Section(window.get_current_page())),
            back: RefCell::default(),
            forward: RefCell::default(),
        });
        let weak = Rc::downgrade(&history);
        window.on_navigated(move || {
            if let Some(history) = weak.upgrade() {
                history.navigated();
            }
        });
        let weak = Rc::downgrade(&history);
        window.on_go_back(move || {
            if let Some(history) = weak.upgrade() {
                history.back();
            }
        });
        let weak = Rc::downgrade(&history);
        window.on_go_forward(move || {
            if let Some(history) = weak.upgrade() {
                history.forward();
            }
        });
        history
    }

    /// Where the window is now.
    pub fn here(&self) -> Option<Place> {
        let window = self.window.upgrade()?;
        let page = window.get_current_page();
        let place = match page {
            Page::Albums if window.get_album_open() => Place::Album(self.view.shown_album()?.id),
            Page::Artists if window.get_artist_album_open() => Place::ArtistAlbum(
                self.view.shown_artist_entry()?.id,
                self.view.shown_album()?.id,
            ),
            Page::Artists if window.get_artist_open() => {
                Place::Artist(self.view.shown_artist_entry()?.id)
            }
            Page::Playlists if window.get_playlist_open() => {
                Place::Playlist(self.view.playlists().shown()?.id)
            }
            other => Place::Section(other),
        };
        Some(place)
    }

    /// The window moved: the place it left goes behind, and nothing is ahead.
    fn navigated(&self) {
        let Some(now) = self.here() else {
            return;
        };
        let left = self.here.replace(now);
        if left != now {
            let mut back = self.back.borrow_mut();
            back.push(left);
            if back.len() > LIMIT {
                back.remove(0);
            }
            self.forward.borrow_mut().clear();
        }
    }

    /// Shows the place before this one.
    pub fn back(&self) {
        let Some(place) = self.back.borrow_mut().pop() else {
            return;
        };
        let left = self.here.replace(place);
        self.forward.borrow_mut().push(left);
        self.show(place);
    }

    /// Shows the place gone back from.
    pub fn forward(&self) {
        let Some(place) = self.forward.borrow_mut().pop() else {
            return;
        };
        let left = self.here.replace(place);
        self.back.borrow_mut().push(left);
        self.show(place);
    }

    fn show(&self, place: Place) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        window.set_opened_from_search(false);
        match place {
            Place::Section(page) => window.invoke_show_section(section(page)),
            Place::Album(album) => {
                window.invoke_show_section(section(Page::Albums));
                self.view.open_album_by_id(album);
            }
            Place::Artist(artist) => {
                window.invoke_show_section(section(Page::Artists));
                self.view.open_artist_by_id(artist);
            }
            Place::ArtistAlbum(artist, album) => {
                window.invoke_show_section(section(Page::Artists));
                self.view.open_artist_by_id(artist);
                self.view.open_artist_album_by_id(album);
            }
            Place::Playlist(playlist) => {
                window.invoke_show_section(section(Page::Playlists));
                self.view.playlists().open(playlist);
            }
        }
        // A page that is gone (an album removed meanwhile) leaves the window where
        // it ended up; that is where the history is now.
        if let Some(now) = self.here() {
            *self.here.borrow_mut() = now;
        }
    }
}

/// The sidebar position of a section.
fn section(page: Page) -> i32 {
    match page {
        Page::Search => 0,
        Page::Albums => 1,
        Page::Artists => 2,
        Page::Playlists => 3,
        Page::Songs => 4,
        Page::Folders => 5,
        Page::Settings => 6,
        Page::About => 7,
    }
}
