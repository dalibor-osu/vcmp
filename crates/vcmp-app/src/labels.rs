//! Labels in the window, behind the `Labels` global: the owner's own names for
//! tracks (`vcmp_library` labels), kept in the database only.
//!
//! - **Menus** put labels on tracks and take them off. A menu asks for the labels
//!   just before it opens (`prepare`), with whether every track it acts on holds
//!   each one, or some do. Choosing a label puts it on all of them, unless all hold
//!   it already, which takes it off all. The target is worked out again when the
//!   label is chosen, so a menu shown with old choices still acts on its own tracks.
//!   An album's menu acts on the tracks the album has now.
//! - **Rows** show a track's labels as chips. Every list builds its rows from tracks,
//!   in several modules, so [`chips`] reads them from what this module keeps for the
//!   window's thread.
//! - **Filters** on the Songs and Albums pages keep the tracks holding every label
//!   picked, and the albums with at least one such track.
//! - **The Labels section** lists the labels, and a label's page its tracks.
//! - **The dialog** makes a label (from Settings, the Labels page or a menu, which
//!   puts it on the menu's tracks) or renames and recolours one.
//!
//! After any change the library view shows it again in every list.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::{
    Color, ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel,
};
use vcmp_library::{
    AlbumId, Error, LABEL_NAME_MAX, LABEL_PALETTE, Label, LabelId, Library, Track, TrackId,
};

use crate::library_view::{LibraryView, format_duration};
use crate::playlists::source;
use crate::selection::{self, Selectable};
use crate::theme::{self, LABEL_COLOURS};
use crate::{
    AppWindow, LabelChoice, LabelDetail, LabelEntry, LabelFilterPage, LabelProblem, Labels,
    PaletteColour, SongSelection, TrackRow, TrackSelection,
};

thread_local! {
    /// What rows need to show labels, for the window on this thread.
    static CHIPS: RefCell<Chips> = RefCell::default();
}

#[derive(Default)]
struct Chips {
    // Each label's entry, and its place in name order.
    entries: HashMap<LabelId, (usize, LabelEntry)>,
    links: HashMap<TrackId, Vec<LabelId>>,
}

/// A track's labels as its row shows them, by name.
pub fn chips(track: TrackId) -> ModelRc<LabelEntry> {
    CHIPS.with(|chips| {
        let chips = chips.borrow();
        let Some(ids) = chips.links.get(&track) else {
            return ModelRc::default();
        };
        let mut entries: Vec<&(usize, LabelEntry)> =
            ids.iter().filter_map(|id| chips.entries.get(id)).collect();
        entries.sort_by_key(|(order, _)| *order);
        let entries: Vec<LabelEntry> = entries
            .into_iter()
            .map(|(_, entry)| entry.clone())
            .collect();
        ModelRc::new(VecModel::from(entries))
    })
}

/// Whether the track holds every one of `labels`.
fn holds_all(track: TrackId, labels: &[LabelId]) -> bool {
    CHIPS.with(|chips| {
        let chips = chips.borrow();
        let held = chips.links.get(&track);
        labels
            .iter()
            .all(|label| held.is_some_and(|held| held.contains(label)))
    })
}

/// The label's tracks, for playing them.
pub struct OpenLabel {
    pub id: LabelId,
    pub tracks: Vec<Track>,
}

pub struct LabelBrowser {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    view: RefCell<Weak<LibraryView>>,
    // Every label, by name, and for each album the labels its tracks hold.
    labels: RefCell<Vec<Label>>,
    album_labels: RefCell<HashMap<AlbumId, Vec<LabelId>>>,
    // Every available track's file, read when a menu first needs it after a library
    // change: looking a file up by its path asks the file system.
    paths: RefCell<Option<HashMap<String, TrackId>>>,
    songs_filter: RefCell<Vec<LabelId>>,
    albums_filter: RefCell<Vec<LabelId>>,
    // The label the dialog edits (None: a new one), and the tracks a new label from a
    // menu is put on.
    editing: Cell<Option<LabelId>>,
    new_for: RefCell<Vec<TrackId>>,
    // The label the label page shows (or showed last), and its tracks.
    open: Cell<Option<LabelId>>,
    open_tracks: RefCell<Vec<Track>>,
    // The last shades drawn for the colour picker, by hue.
    shades: RefCell<Option<(u16, Image)>>,
}

impl LabelBrowser {
    /// Shows the library's labels. Keep the browser alive as long as the window.
    pub fn connect(window: &AppWindow, library: Rc<Library>) -> Rc<Self> {
        let browser = Rc::new(Self {
            window: window.as_weak(),
            library,
            view: RefCell::default(),
            labels: RefCell::default(),
            album_labels: RefCell::default(),
            paths: RefCell::default(),
            songs_filter: RefCell::default(),
            albums_filter: RefCell::default(),
            editing: Cell::default(),
            new_for: RefCell::default(),
            open: Cell::default(),
            open_tracks: RefCell::default(),
            shades: RefCell::default(),
        });
        let state = window.global::<Labels>();
        let palette: Vec<PaletteColour> = LABEL_COLOURS
            .iter()
            .map(|(name, light, dark)| PaletteColour {
                name: (*name).into(),
                light: *light,
                dark: *dark,
            })
            .collect();
        state.set_palette(ModelRc::new(VecModel::from(palette)));
        state.set_hue_strip(hue_strip());
        browser.reload();

        let weak = Rc::downgrade(&browser);
        state.on_shades(move |hue| {
            weak.upgrade()
                .map(|browser| browser.shades(hue))
                .unwrap_or_default()
        });
        state.on_hex_of(|hue, saturation, value| hex(hsv(hue, saturation, value)).into());
        state.on_colour_of(|text| theme::parse_hex(&text).unwrap_or_default());

        // Menus by rows: the track lists and the songs table.
        let weak = Rc::downgrade(&browser);
        window
            .global::<TrackSelection>()
            .on_prepare_labels(move |rows, row| {
                if let Some(browser) = weak.upgrade() {
                    let tracks = browser.tracks_of_rows(&rows, row);
                    browser.prepare(&tracks);
                }
            });
        let weak = Rc::downgrade(&browser);
        window
            .global::<TrackSelection>()
            .on_toggle_label(move |rows, row, label| {
                if let Some(browser) = weak.upgrade() {
                    let tracks = browser.tracks_of_rows(&rows, row);
                    browser.toggle(&tracks, LabelId(i64::from(label)));
                }
            });
        let weak = Rc::downgrade(&browser);
        window
            .global::<TrackSelection>()
            .on_new_label(move |rows, row| {
                if let Some(browser) = weak.upgrade() {
                    let tracks = browser.tracks_of_rows(&rows, row);
                    browser.edit_new(tracks);
                }
            });
        let weak = Rc::downgrade(&browser);
        window
            .global::<SongSelection>()
            .on_prepare_labels(move |rows, row| {
                if let Some(browser) = weak.upgrade() {
                    let tracks = browser.tracks_of_rows(&rows, row);
                    browser.prepare(&tracks);
                }
            });
        let weak = Rc::downgrade(&browser);
        window
            .global::<SongSelection>()
            .on_toggle_label(move |rows, row, label| {
                if let Some(browser) = weak.upgrade() {
                    let tracks = browser.tracks_of_rows(&rows, row);
                    browser.toggle(&tracks, LabelId(i64::from(label)));
                }
            });
        let weak = Rc::downgrade(&browser);
        window
            .global::<SongSelection>()
            .on_new_label(move |rows, row| {
                if let Some(browser) = weak.upgrade() {
                    let tracks = browser.tracks_of_rows(&rows, row);
                    browser.edit_new(tracks);
                }
            });

        // Menus for one file, and for an album.
        let weak = Rc::downgrade(&browser);
        state.on_prepare_path(move |path| {
            if let Some(browser) = weak.upgrade() {
                let tracks = browser.track_of_path(&path);
                browser.prepare(&tracks);
            }
        });
        let weak = Rc::downgrade(&browser);
        state.on_toggle_path(move |path, label| {
            if let Some(browser) = weak.upgrade() {
                let tracks = browser.track_of_path(&path);
                browser.toggle(&tracks, LabelId(i64::from(label)));
            }
        });
        let weak = Rc::downgrade(&browser);
        state.on_new_for_path(move |path| {
            if let Some(browser) = weak.upgrade() {
                let tracks = browser.track_of_path(&path);
                browser.edit_new(tracks);
            }
        });
        let weak = Rc::downgrade(&browser);
        state.on_prepare_album(move |album| {
            if let Some(browser) = weak.upgrade() {
                let tracks = browser.tracks_of_album(album);
                browser.prepare(&tracks);
            }
        });
        let weak = Rc::downgrade(&browser);
        state.on_toggle_album(move |album, label| {
            if let Some(browser) = weak.upgrade() {
                let tracks = browser.tracks_of_album(album);
                browser.toggle(&tracks, LabelId(i64::from(label)));
            }
        });
        let weak = Rc::downgrade(&browser);
        state.on_new_for_album(move |album| {
            if let Some(browser) = weak.upgrade() {
                let tracks = browser.tracks_of_album(album);
                browser.edit_new(tracks);
            }
        });

        // Filters.
        let weak = Rc::downgrade(&browser);
        state.on_toggle_filter(move |page, label| {
            if let Some(browser) = weak.upgrade() {
                browser.toggle_filter(page, LabelId(i64::from(label)));
            }
        });
        let weak = Rc::downgrade(&browser);
        state.on_clear_filter(move |page| {
            if let Some(browser) = weak.upgrade() {
                browser.filter(page).borrow_mut().clear();
                browser.filter_changed(page);
            }
        });

        // The dialog, and deleting.
        let weak = Rc::downgrade(&browser);
        state.on_edit(move |id| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if id < 0 {
                browser.edit_new(Vec::new());
            } else {
                browser.edit(LabelId(i64::from(id)));
            }
        });
        let weak = Rc::downgrade(&browser);
        state.on_save(move |name, index, hex| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            // Past the palette's end: the custom colour.
            let colour = match usize::try_from(index)
                .ok()
                .and_then(|at| LABEL_PALETTE.get(at))
            {
                Some(name) => format!("palette:{name}"),
                None => hex.to_string(),
            };
            browser.save(&name, &colour);
        });
        let weak = Rc::downgrade(&browser);
        state.on_close_editor(move || {
            if let Some(browser) = weak.upgrade() {
                browser.close_editor();
            }
        });
        let weak = Rc::downgrade(&browser);
        state.on_delete(move |id| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            if let Err(error) = browser.library.delete_label(LabelId(i64::from(id))) {
                log::error!("could not delete the label: {error}");
            }
            browser.changed();
        });

        // The Labels section.
        let weak = Rc::downgrade(&browser);
        window.on_label_activated(move |index| {
            let Some(browser) = weak.upgrade() else {
                return;
            };
            let label = usize::try_from(index)
                .ok()
                .and_then(|index| browser.labels.borrow().get(index).map(|label| label.id));
            if let Some(label) = label {
                browser.open(label);
            }
        });
        browser
    }

    /// The view told when labels change, so every list shows them.
    pub(crate) fn set_view(&self, view: Weak<LibraryView>) {
        *self.view.borrow_mut() = view;
    }

    /// Reads the labels and their links again: the sidebar, Settings, the filters,
    /// the chips rows show, and the label page while one is open.
    pub fn reload(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let labels = self.library.labels().unwrap_or_else(|error| {
            log::error!("could not read the labels: {error}");
            Vec::new()
        });
        let links = self.library.label_links().unwrap_or_else(|error| {
            log::error!("could not read the labels' tracks: {error}");
            HashMap::new()
        });
        let album_labels = self.library.album_labels().unwrap_or_else(|error| {
            log::error!("could not read the albums' labels: {error}");
            HashMap::new()
        });
        let entries: Vec<LabelEntry> = labels.iter().map(entry).collect();
        CHIPS.with(|chips| {
            *chips.borrow_mut() = Chips {
                entries: labels
                    .iter()
                    .zip(&entries)
                    .enumerate()
                    .map(|(order, (label, entry))| (label.id, (order, entry.clone())))
                    .collect(),
                links,
            };
        });
        let known: HashSet<LabelId> = labels.iter().map(|label| label.id).collect();
        self.songs_filter
            .borrow_mut()
            .retain(|label| known.contains(label));
        self.albums_filter
            .borrow_mut()
            .retain(|label| known.contains(label));
        let state = window.global::<Labels>();
        state.set_all(ModelRc::new(VecModel::from(entries)));
        *self.labels.borrow_mut() = labels;
        *self.album_labels.borrow_mut() = album_labels;
        self.show_filters();
        if let Some(id) = self.open.get() {
            self.show(&window, id);
        }
    }

    /// The library changed: tracks came, went or moved, so their files are read again
    /// when a menu next needs them, and counts change.
    pub fn library_changed(&self) {
        self.paths.borrow_mut().take();
        self.reload();
    }

    /// Every label, by name.
    pub fn labels(&self) -> Vec<Label> {
        self.labels.borrow().clone()
    }

    /// Whether the Songs page's label filter leaves the track.
    pub fn song_passes(&self, track: TrackId) -> bool {
        holds_all(track, &self.songs_filter.borrow())
    }

    /// Whether the Albums page's label filter leaves the album: some track of it holds
    /// every label picked.
    pub fn album_passes(&self, album: AlbumId) -> bool {
        let picked = self.albums_filter.borrow();
        if picked.is_empty() {
            return true;
        }
        // Quick when no track of the album holds even one of them.
        let album_labels = self.album_labels.borrow();
        let Some(held) = album_labels.get(&album) else {
            return false;
        };
        if !picked.iter().all(|label| held.contains(label)) {
            return false;
        }
        if picked.len() == 1 {
            return true;
        }
        self.library
            .album_tracks(album)
            .unwrap_or_default()
            .iter()
            .any(|track| holds_all(track.id, &picked))
    }

    /// The label the label page shows, while it is open, with its tracks.
    pub fn shown(&self) -> Option<OpenLabel> {
        let window = self.window.upgrade()?;
        if !window.get_label_open() {
            return None;
        }
        Some(OpenLabel {
            id: self.open.get()?,
            tracks: self.open_tracks.borrow().clone(),
        })
    }

    /// Opens a label's page in the Labels section.
    pub fn open(&self, id: LabelId) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        self.open.set(Some(id));
        self.show(&window, id);
        window.set_label_open(true);
    }

    /// The labels a menu offers, for these tracks.
    fn prepare(&self, tracks: &[TrackId]) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let choices: Vec<LabelChoice> = self
            .labels
            .borrow()
            .iter()
            .map(|label| {
                let holding = tracks
                    .iter()
                    .filter(|track| holds_all(**track, &[label.id]))
                    .count();
                LabelChoice {
                    id: label.id.0 as i32,
                    name: label.name.as_str().into(),
                    checked: holding > 0 && holding == tracks.len(),
                    partial: holding > 0 && holding < tracks.len(),
                }
            })
            .collect();
        window
            .global::<Labels>()
            .set_choices(ModelRc::new(VecModel::from(choices)));
    }

    /// Puts the label on every track, or takes it off them all when all hold it.
    fn toggle(&self, tracks: &[TrackId], label: LabelId) {
        if tracks.is_empty() {
            return;
        }
        let result = if tracks.iter().all(|track| holds_all(*track, &[label])) {
            self.library.remove_label(label, tracks)
        } else {
            self.library.add_label(label, tracks)
        };
        if let Err(error) = result {
            log::error!(
                "could not change the label of {} tracks: {error}",
                tracks.len()
            );
        }
        self.changed();
    }

    /// The tracks a list's menu acts on: the selected rows when `row` is one of them.
    fn tracks_of_rows<T: Selectable + 'static>(&self, rows: &ModelRc<T>, row: i32) -> Vec<TrackId> {
        let paths = selection::chosen_paths(rows, row);
        let mut known = self.paths.borrow_mut();
        let known = known.get_or_insert_with(|| {
            self.library
                .tracks()
                .unwrap_or_default()
                .into_iter()
                .map(|track| (track.path.to_string_lossy().into_owned(), track.id))
                .collect()
        });
        paths
            .iter()
            .filter_map(|path| {
                known.get(path).copied().or_else(|| {
                    // A playlist entry whose file is gone isn't among the available.
                    self.library
                        .track_id_for_path(Path::new(path))
                        .ok()
                        .flatten()
                })
            })
            .collect()
    }

    fn track_of_path(&self, path: &str) -> Vec<TrackId> {
        match self.library.track_id_for_path(Path::new(path)) {
            Ok(track) => track.into_iter().collect(),
            Err(error) => {
                log::error!("could not look up {path}: {error}");
                Vec::new()
            }
        }
    }

    /// The tracks an album has now.
    fn tracks_of_album(&self, album: i32) -> Vec<TrackId> {
        self.library
            .album_tracks(AlbumId(i64::from(album)))
            .unwrap_or_else(|error| {
                log::error!("could not read the tracks of album {album}: {error}");
                Vec::new()
            })
            .into_iter()
            .map(|track| track.id)
            .collect()
    }

    /// Opens the dialog for a new label, put on `tracks` once made.
    fn edit_new(&self, tracks: Vec<TrackId>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        // The first colour of the palette no label has yet.
        let used: HashSet<String> = self
            .labels
            .borrow()
            .iter()
            .map(|label| label.colour.clone())
            .collect();
        let colour = LABEL_PALETTE
            .iter()
            .position(|name| !used.contains(&format!("palette:{name}")))
            .unwrap_or(0);
        self.editing.set(None);
        let state = window.global::<Labels>();
        state.set_editor_id(-1);
        state.set_editor_name(SharedString::default());
        state.set_editor_colour(colour as i32);
        state.set_editor_hex(SharedString::default());
        state.set_editor_tracks(tracks.len() as i32);
        state.set_editor_problem(LabelProblem::None);
        *self.new_for.borrow_mut() = tracks;
        state.set_editor_open(true);
    }

    fn edit(&self, id: LabelId) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(label) = self
            .labels
            .borrow()
            .iter()
            .find(|label| label.id == id)
            .cloned()
        else {
            return;
        };
        self.editing.set(Some(id));
        self.new_for.borrow_mut().clear();
        let state = window.global::<Labels>();
        let palette = label
            .colour
            .strip_prefix("palette:")
            .and_then(|name| LABEL_PALETTE.iter().position(|known| *known == name));
        state.set_editor_id(id.0 as i32);
        state.set_editor_name(label.name.as_str().into());
        state.set_editor_colour(palette.unwrap_or(LABEL_PALETTE.len()) as i32);
        state.set_editor_hex(if palette.is_some() {
            SharedString::default()
        } else {
            label.colour.as_str().into()
        });
        state.set_editor_tracks(0);
        state.set_editor_problem(LabelProblem::None);
        state.set_editor_open(true);
    }

    /// Makes the label, or renames and recolours the one edited; says what is wrong
    /// instead when it can't.
    fn save(&self, name: &str, colour: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<Labels>();
        let colour = normal_colour(colour);
        let length = name.trim().chars().count();
        let problem = if length == 0 {
            Some(LabelProblem::Empty)
        } else if length > LABEL_NAME_MAX {
            Some(LabelProblem::TooLong)
        } else if !vcmp_library::valid_label_colour(&colour) {
            Some(LabelProblem::Colour)
        } else {
            None
        };
        if let Some(problem) = problem {
            state.set_editor_problem(problem);
            return;
        }
        let result = match self.editing.get() {
            Some(id) => self
                .library
                .rename_label(id, name)
                .and_then(|()| self.library.set_label_colour(id, &colour)),
            None => self.library.create_label(name, &colour).and_then(|id| {
                let tracks = self.new_for.take();
                if tracks.is_empty() {
                    Ok(())
                } else {
                    self.library.add_label(id, &tracks)
                }
            }),
        };
        match result {
            Ok(()) => {
                self.close_editor();
                self.changed();
            }
            Err(Error::LabelNameTaken(_)) => state.set_editor_problem(LabelProblem::Taken),
            Err(error) => log::error!("could not save the label: {error}"),
        }
    }

    fn close_editor(&self) {
        self.editing.set(None);
        self.new_for.borrow_mut().clear();
        if let Some(window) = self.window.upgrade() {
            window.global::<Labels>().set_editor_open(false);
        }
    }

    fn filter(&self, page: LabelFilterPage) -> &RefCell<Vec<LabelId>> {
        match page {
            LabelFilterPage::Songs => &self.songs_filter,
            LabelFilterPage::Albums => &self.albums_filter,
        }
    }

    fn toggle_filter(&self, page: LabelFilterPage, label: LabelId) {
        {
            let mut picked = self.filter(page).borrow_mut();
            match picked.iter().position(|known| *known == label) {
                Some(at) => {
                    picked.remove(at);
                }
                None => picked.push(label),
            }
        }
        self.filter_changed(page);
    }

    fn filter_changed(&self, page: LabelFilterPage) {
        self.show_filters();
        if let Some(view) = self.view.borrow().upgrade() {
            view.label_filter_changed(page);
        }
    }

    /// Shows what each filter picked, in name order.
    fn show_filters(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<Labels>();
        let labels = self.labels.borrow();
        for page in [LabelFilterPage::Songs, LabelFilterPage::Albums] {
            let picked = self.filter(page).borrow();
            let entries: Vec<LabelEntry> = labels
                .iter()
                .filter(|label| picked.contains(&label.id))
                .map(entry)
                .collect();
            let choices: Vec<LabelChoice> = labels
                .iter()
                .map(|label| LabelChoice {
                    id: label.id.0 as i32,
                    name: label.name.as_str().into(),
                    checked: picked.contains(&label.id),
                    partial: false,
                })
                .collect();
            let (entries, choices) = (
                ModelRc::new(VecModel::from(entries)),
                ModelRc::new(VecModel::from(choices)),
            );
            match page {
                LabelFilterPage::Songs => {
                    state.set_songs_filter(entries);
                    state.set_songs_filter_choices(choices);
                }
                LabelFilterPage::Albums => {
                    state.set_albums_filter(entries);
                    state.set_albums_filter_choices(choices);
                }
            }
        }
    }

    /// Labels or links changed: reads them again, and every list shows them.
    pub fn changed(&self) {
        self.reload();
        if let Some(view) = self.view.borrow().upgrade() {
            view.labels_changed();
        }
    }

    /// Fills the label page; a label deleted meanwhile stays on screen, marked as
    /// gone, until the user leaves it.
    fn show(&self, window: &AppWindow, id: LabelId) {
        let labels = self.labels.borrow();
        let position = labels.iter().position(|label| label.id == id);
        window.set_open_label_index(position.map_or(-1, |at| at as i32));
        let Some(label) = position.map(|at| &labels[at]) else {
            let mut detail = window.get_label_detail();
            detail.missing = true;
            window.set_label_detail(detail);
            window.set_label_tracks(ModelRc::default());
            self.open_tracks.borrow_mut().clear();
            return;
        };
        let tracks = self.library.label_tracks(id).unwrap_or_else(|error| {
            log::error!("could not read the tracks of {:?}: {error}", label.name);
            Vec::new()
        });
        let total: Duration = tracks.iter().map(|track| track.duration).sum();
        let (light, dark) = theme::label_colours(&label.colour);
        window.set_label_detail(LabelDetail {
            name: label.name.as_str().into(),
            light,
            dark,
            track_count: tracks.len() as i32,
            minutes: total.as_secs().div_ceil(60) as i32,
            missing: false,
        });
        let rows: Vec<TrackRow> = tracks
            .iter()
            .enumerate()
            .map(|(index, track)| TrackRow {
                number: (index + 1).to_string().into(),
                title: track.title.as_str().into(),
                artist: source(track).into(),
                duration: format_duration(track.duration).into(),
                path: track.path.to_string_lossy().as_ref().into(),
                labels: chips(track.id),
                ..TrackRow::default()
            })
            .collect();
        window.set_label_tracks(ModelRc::new(VecModel::from(rows)));
        *self.open_tracks.borrow_mut() = tracks;
    }

    /// The shades of a hue, for the colour picker; drawn again only for another hue.
    fn shades(&self, hue: f32) -> Image {
        let key = (hue.clamp(0.0, 1.0) * 1000.0) as u16;
        if let Some((drawn, image)) = &*self.shades.borrow()
            && *drawn == key
        {
            return image.clone();
        }
        const SIZE: u32 = 64;
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(SIZE, SIZE);
        let width = pixels.width() as usize;
        for (index, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
            let (x, y) = (index % width, index / width);
            let saturation = x as f32 / (SIZE - 1) as f32;
            let value = 1.0 - y as f32 / (SIZE - 1) as f32;
            *pixel = rgba(hsv(hue, saturation, value));
        }
        let image = Image::from_rgba8(pixels);
        *self.shades.borrow_mut() = Some((key, image.clone()));
        image
    }
}

/// A label as the UI shows it.
fn entry(label: &Label) -> LabelEntry {
    let (light, dark) = theme::label_colours(&label.colour);
    LabelEntry {
        id: label.id.0 as i32,
        name: label.name.as_str().into(),
        light,
        dark,
        count: label.track_count as i32,
    }
}

/// A colour from the dialog as the library stores it: a hex code in lower case with
/// its `#`, however it was typed.
fn normal_colour(colour: &str) -> String {
    let colour = colour.trim();
    if colour.starts_with("palette:") {
        return colour.to_owned();
    }
    let digits = colour.strip_prefix('#').unwrap_or(colour);
    format!("#{}", digits.to_lowercase())
}

/// Every hue from left to right, at full saturation and brightness.
fn hue_strip() -> Image {
    const WIDTH: u32 = 256;
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(WIDTH, 1);
    for (x, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
        *pixel = rgba(hsv(x as f32 / (WIDTH - 1) as f32, 1.0, 1.0));
    }
    Image::from_rgba8(pixels)
}

/// A colour from hue, saturation and brightness, each 0 to 1.
fn hsv(hue: f32, saturation: f32, value: f32) -> Color {
    let hue = hue.clamp(0.0, 1.0) * 6.0;
    let (saturation, value) = (saturation.clamp(0.0, 1.0), value.clamp(0.0, 1.0));
    let sector = (hue.floor() as i32).min(5);
    let fraction = hue - sector as f32;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    let (r, g, b) = match sector {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    let byte = |c: f32| (c * 255.0).round() as u8;
    Color::from_rgb_u8(byte(r), byte(g), byte(b))
}

fn rgba(colour: Color) -> Rgba8Pixel {
    Rgba8Pixel::new(colour.red(), colour.green(), colour.blue(), 255)
}

/// A colour as #rrggbb.
fn hex(colour: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        colour.red(),
        colour.green(),
        colour.blue()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hues_go_round_the_colour_wheel() {
        assert_eq!(hex(hsv(0.0, 1.0, 1.0)), "#ff0000");
        assert_eq!(hex(hsv(1.0 / 3.0, 1.0, 1.0)), "#00ff00");
        assert_eq!(hex(hsv(2.0 / 3.0, 1.0, 1.0)), "#0000ff");
        assert_eq!(hex(hsv(1.0, 1.0, 1.0)), "#ff0000");
        assert_eq!(hex(hsv(0.5, 0.0, 0.5)), "#808080");
    }

    #[test]
    fn typed_colours_are_stored_one_way() {
        assert_eq!(normal_colour(" A1B2C3 "), "#a1b2c3");
        assert_eq!(normal_colour("#A1B2C3"), "#a1b2c3");
        assert_eq!(normal_colour("palette:teal"), "palette:teal");
        assert!(!vcmp_library::valid_label_colour(&normal_colour("#abc")));
    }
}
