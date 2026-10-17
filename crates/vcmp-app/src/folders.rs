//! The folder browser: the music folders, and inside them the folders and files the
//! library holds. It is built from the library's tracks rather than read from the
//! disk, so it is instant and shows only music VCMP knows.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use vcmp_library::{Library, Track};

use crate::library_view::format_duration;
use crate::{AppWindow, FolderRow, TrackRow};

pub struct FolderBrowser {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    // Every available track, once the page was shown.
    tracks: RefCell<Option<Vec<Track>>>,
    // The folder shown and the path down to it from its music folder (each step's
    // full path); empty for the list of music folders.
    path: RefCell<Vec<PathBuf>>,
    // What the page lists now.
    subfolders: RefCell<Vec<PathBuf>>,
    files: RefCell<Vec<Track>>,
}

impl FolderBrowser {
    pub fn connect(window: &AppWindow, library: Rc<Library>) -> Rc<Self> {
        let browser = Rc::new(Self {
            window: window.as_weak(),
            library,
            tracks: RefCell::default(),
            path: RefCell::default(),
            subfolders: RefCell::default(),
            files: RefCell::default(),
        });
        let weak = Rc::downgrade(&browser);
        window.on_folders_needed(move || {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if browser.tracks.borrow().is_none() {
                browser.reload_tracks();
                browser.show();
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_open_folder(move |index| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            let folder = usize::try_from(index)
                .ok()
                .and_then(|index| browser.subfolders.borrow().get(index).cloned());
            if let Some(folder) = folder {
                browser.path.borrow_mut().push(folder);
                browser.show();
            }
        });
        let weak = Rc::downgrade(&browser);
        window.on_folder_go_to(move |index| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            browser
                .path
                .borrow_mut()
                .truncate((index + 1).max(0) as usize);
            browser.show();
        });
        browser
    }

    /// Reads the tracks again after the library changed, once the page was shown.
    /// The folder shown stays, or its nearest folder that still holds tracks.
    pub fn reload(&self) {
        if self.tracks.borrow().is_none() {
            return;
        }
        self.reload_tracks();
        let tracks = self.tracks.borrow();
        let holds_tracks = |folder: &Path| {
            tracks
                .iter()
                .flatten()
                .any(|track| track.path.starts_with(folder))
        };
        let mut path = self.path.borrow_mut();
        // The music folder itself stays even when empty, as on the top level.
        while path.len() > 1 && !holds_tracks(path.last().expect("not empty")) {
            path.pop();
        }
        drop((path, tracks));
        self.show();
    }

    /// The files of the folder shown, in the order listed.
    pub fn files(&self) -> Vec<Track> {
        self.files.borrow().clone()
    }

    /// The folder shown, if it is not the list of music folders.
    pub fn folder(&self) -> Option<PathBuf> {
        self.path.borrow().last().cloned()
    }

    fn reload_tracks(&self) {
        let tracks = self.library.tracks().unwrap_or_else(|error| {
            log::error!("could not read the tracks: {error}");
            Vec::new()
        });
        *self.tracks.borrow_mut() = Some(tracks);
    }

    fn show(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let tracks = self.tracks.borrow();
        let tracks = tracks.as_deref().unwrap_or_default();
        let path = self.path.borrow();
        let (rows, subfolders, files) = match path.last() {
            None => self.music_folders(tracks),
            Some(folder) => contents(folder, tracks),
        };
        let crumbs: Vec<SharedString> = path.iter().map(|step| name(step).into()).collect();
        let file_rows: Vec<TrackRow> = files.iter().map(file_row).collect();
        window.set_folder_crumbs(ModelRc::new(VecModel::from(crumbs)));
        window.set_folder_rows(ModelRc::new(VecModel::from(rows)));
        window.set_folder_files(ModelRc::new(VecModel::from(file_rows)));
        *self.subfolders.borrow_mut() = subfolders;
        *self.files.borrow_mut() = files;
    }

    /// The top level: each music folder with the tracks under it.
    fn music_folders(&self, tracks: &[Track]) -> (Vec<FolderRow>, Vec<PathBuf>, Vec<Track>) {
        let sources = self.library.sources().unwrap_or_else(|error| {
            log::error!("could not read the music folders: {error}");
            Vec::new()
        });
        let rows = sources
            .iter()
            .map(|source| FolderRow {
                name: name(&source.path).into(),
                track_count: tracks
                    .iter()
                    .filter(|track| track.path.starts_with(&source.path))
                    .count() as i32,
                missing: !source.path.is_dir(),
            })
            .collect();
        let paths = sources.into_iter().map(|source| source.path).collect();
        (rows, paths, Vec::new())
    }
}

/// A folder's subfolders that hold tracks (by name, with their track counts), and
/// the tracks right inside it (by file name).
fn contents(folder: &Path, tracks: &[Track]) -> (Vec<FolderRow>, Vec<PathBuf>, Vec<Track>) {
    let mut subfolders: BTreeMap<(String, PathBuf), i32> = BTreeMap::new();
    let mut files = Vec::new();
    for track in tracks {
        let Ok(inside) = track.path.strip_prefix(folder) else {
            continue;
        };
        let mut components = inside.components();
        let first = components.next();
        match (first, components.next()) {
            (Some(_), None) => files.push(track.clone()),
            (Some(first), Some(_)) => {
                let path = folder.join(first);
                let key = (name(&path).to_lowercase(), path);
                *subfolders.entry(key).or_default() += 1;
            }
            (None, _) => {}
        }
    }
    files.sort_by_cached_key(|track| name(&track.path).to_lowercase());
    let rows = subfolders
        .iter()
        .map(|((_, path), count)| FolderRow {
            name: name(path).into(),
            track_count: *count,
            missing: false,
        })
        .collect();
    let paths = subfolders.into_keys().map(|(_, path)| path).collect();
    (rows, paths, files)
}

/// A file shows its file name first, since the browser is where badly tagged files
/// are found; the tags come second.
fn file_row(track: &Track) -> TrackRow {
    let artist = track.artist.as_deref().unwrap_or_default();
    let tags = if artist.is_empty() {
        track.title.clone()
    } else {
        format!("{} · {artist}", track.title)
    };
    TrackRow {
        title: name(&track.path).into(),
        artist: tags.into(),
        duration: format_duration(track.duration).into(),
        path: track.path.to_string_lossy().as_ref().into(),
        ..TrackRow::default()
    }
}

fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn track(path: &str) -> Track {
        Track {
            id: vcmp_library::TrackId(0),
            path: PathBuf::from(path),
            available: true,
            format: None,
            title: "t".into(),
            artist: None,
            artist_id: None,
            album: None,
            album_id: None,
            album_artist: None,
            track_number: None,
            disc_number: None,
            year: None,
            genre: None,
            duration: Duration::ZERO,
        }
    }

    #[test]
    fn a_folder_lists_subfolders_with_counts_then_its_files() {
        let tracks = [
            track("/m/b/2.flac"),
            track("/m/A/x/1.flac"),
            track("/m/A/2.flac"),
            track("/m/z.mp3"),
            track("/m/a.mp3"),
            track("/other/3.flac"),
        ];
        let (rows, paths, files) = contents(Path::new("/m"), &tracks);
        let rows: Vec<(String, i32)> = rows
            .into_iter()
            .map(|row| (row.name.to_string(), row.track_count))
            .collect();
        assert_eq!(rows, [("A".to_string(), 2), ("b".to_string(), 1)]);
        assert_eq!(paths, [PathBuf::from("/m/A"), PathBuf::from("/m/b")]);
        let files: Vec<String> = files.iter().map(|t| name(&t.path)).collect();
        assert_eq!(files, ["a.mp3", "z.mp3"]);
    }
}
