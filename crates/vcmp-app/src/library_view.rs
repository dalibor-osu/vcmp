//! Connects the library to the window: the album grid, and the album page that
//! opens from it. Both are reloaded after every library change.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Image, ModelRc, VecModel};
use vcmp_library::{Album, Library, Track};

use crate::albums::{AlbumBrowser, initial};
use crate::{AlbumDetail, AppWindow, TrackRow};

pub struct LibraryView {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    browser: Rc<AlbumBrowser>,
    // The album the album page shows (or showed last).
    open: RefCell<Option<Album>>,
}

impl LibraryView {
    /// Shows the library's albums. Keep the view alive as long as the window.
    pub fn connect(window: &AppWindow, library: Rc<Library>) -> vcmp_library::Result<Rc<Self>> {
        let browser = AlbumBrowser::connect(window, &library.albums()?);
        let view = Rc::new(Self {
            window: window.as_weak(),
            library,
            browser,
            open: RefCell::default(),
        });
        let weak = Rc::downgrade(&view);
        window.on_album_activated(move |index| {
            let Some(view) = weak.upgrade() else { return };
            if let Some(album) = usize::try_from(index)
                .ok()
                .and_then(|index| view.browser.album(index))
            {
                view.open_album(album);
            }
        });
        Ok(view)
    }

    pub fn browser(&self) -> &Rc<AlbumBrowser> {
        &self.browser
    }

    /// The album the album page shows, while it is open.
    pub fn shown_album(&self) -> Option<Album> {
        let open = self
            .window
            .upgrade()
            .is_some_and(|window| window.get_album_open());
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
        self.browser.replace(&albums);
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let open = self.open.borrow().clone();
        if let (true, Some(open)) = (window.get_album_open(), open) {
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

    fn open_album(&self, album: Album) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        show_album(&window, &self.library, &album);
        *self.open.borrow_mut() = Some(album);
        window.set_album_open(true);
    }
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
