//! The Library check page: opened from Settings, its tabs' findings, "Not duplicates"
//! and showing hidden groups again, the actions handed to the player, the tag editor
//! and the cover picker, Back and Escape, and that none of it changes a music file.

mod common;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use tempfile::TempDir;
use vcmp_app::library_view::LibraryView;
use vcmp_app::navigation::History;
use vcmp_app::{
    AppWindow, CheckList, CheckTab, CoverSearch, LibraryCheck, LibraryState, Page, SourceFolder,
    TagEditorState,
};
use vcmp_library::{Library, Track};

/// What the page asked of the player: the tracks' titles, the context and the start.
type Played = Rc<RefCell<Vec<(Vec<String>, Option<String>, usize)>>>;

struct App {
    window: AppWindow,
    view: Rc<LibraryView>,
    music: PathBuf,
    played: Played,
    _dir: TempDir,
}

/// Two copies of "Song" by Band on two albums, and "Other" alone on a third; no
/// track has a year and no album a cover.
fn open() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    add(&music, "a.flac", Song::new("Song", "Band", "First"));
    add(&music, "b.flac", Song::new("song", "Band", "Second"));
    add(&music, "c.flac", Song::new("Other", "Band", "Third"));
    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();

    let library = Rc::new(library);
    let window = vcmp_app::create_window().unwrap();
    let view = LibraryView::connect(&window, library).unwrap();
    let played: Played = Rc::default();
    let record = played.clone();
    view.check()
        .on_play(move |tracks: &[Track], origin, start| {
            let titles = tracks.iter().map(|track| track.title.clone()).collect();
            record
                .borrow_mut()
                .push((titles, origin.map(|origin| origin.key()), start));
        });
    window.show().unwrap();
    App {
        window,
        view,
        music,
        played,
        _dir: dir,
    }
}

impl App {
    fn state(&self) -> LibraryCheck<'_> {
        self.window.global::<LibraryCheck>()
    }

    /// Runs the change handlers the event loop would.
    fn settle(&self) {
        i_slint_backend_testing::mock_elapsed_time(Duration::ZERO);
    }

    fn button(&self, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .unwrap_or_else(|| panic!("no button labelled {label:?}"))
    }

    fn open_settings(&self) {
        ElementHandle::find_by_accessible_label(&self.window, "Settings")
            .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
            .unwrap()
            .invoke_accessible_default_action();
        self.settle();
    }

    fn page_shown(&self) -> bool {
        ElementHandle::find_by_element_type_name(&self.window, "LibraryCheckPage")
            .next()
            .is_some()
    }
}

/// Every file's bytes and modification time under `dir`.
fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>, SystemTime)> {
    let mut found: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            (path.clone(), fs::read(&path).unwrap(), modified)
        })
        .collect();
    found.sort();
    found
}

#[test]
fn settings_opens_the_page_and_back_returns() {
    let app = open();
    // The row is there once the library has music folders.
    app.window
        .global::<LibraryState>()
        .set_folders(ModelRc::new(VecModel::from(vec![SourceFolder {
            id: 1,
            name: "music".into(),
            path: app.music.to_string_lossy().as_ref().into(),
            recursive: true,
            reachable: true,
        }])));
    let history = History::connect(&app.window, app.view.clone());
    app.open_settings();
    assert!(!app.page_shown());

    app.button("Check library…")
        .invoke_accessible_default_action();
    app.settle();
    assert!(app.state().get_open());
    assert_eq!(app.window.get_current_page(), Page::Settings);
    assert!(app.page_shown());

    history.back();
    app.settle();
    assert!(!app.state().get_open(), "Back goes to Settings");
    assert!(!app.page_shown());
    history.forward();
    app.settle();
    assert!(app.state().get_open(), "and Forward to the check again");

    // Escape leaves it too.
    let escape: SharedString = Key::Escape.into();
    app.window.window().dispatch_event(WindowEvent::KeyPressed {
        text: escape.clone(),
    });
    app.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: escape });
    app.settle();
    assert!(!app.state().get_open());

    // The sidebar's Settings entry shows Settings, not the check.
    app.state().invoke_show();
    app.settle();
    app.open_settings();
    assert!(!app.state().get_open());
}

#[test]
fn the_tabs_list_what_the_check_found() {
    let app = open();
    // The page is drawn only on Settings.
    app.open_settings();
    app.state().invoke_show();
    app.settle();
    let state = app.state();
    assert_eq!(state.get_duplicate_count(), 1);
    assert_eq!(state.get_hidden_count(), 0);
    let groups = state.get_duplicates();
    assert_eq!(groups.row_count(), 1);
    let group = groups.row_data(0).unwrap();
    assert_eq!(group.tracks.row_count(), 2);
    assert!(
        group.tracks.row_data(0).unwrap().details.contains("FLAC"),
        "format and length tell the copies apart"
    );

    // No track has a year: all three, under their three albums.
    assert_eq!(state.get_gap_track_count(), 3);
    assert_eq!(state.get_gap_album_count(), 3);
    let first = state
        .get_gaps()
        .row_data(0)
        .unwrap()
        .tracks
        .row_data(0)
        .unwrap();
    assert!(first.missing_year);
    assert!(!first.missing_title);

    assert_eq!(state.get_coverless_count(), 3);

    state.set_tab(CheckTab::MissingCovers);
    app.settle();
    assert!(
        ElementHandle::find_by_accessible_label(&app.window, "Find a cover for First online")
            .next()
            .is_some()
    );
}

#[test]
fn not_duplicates_hides_a_group_and_show_again_undoes_it() {
    let app = open();
    app.state().invoke_show();
    app.state().invoke_not_duplicates(0);
    let state = app.state();
    assert_eq!(state.get_duplicate_count(), 0);
    assert_eq!(state.get_hidden_count(), 1);
    assert_eq!(state.get_duplicates().row_count(), 0);

    state.invoke_set_show_hidden(true);
    let groups = state.get_duplicates();
    assert_eq!(groups.row_count(), 1);
    assert!(groups.row_data(0).unwrap().hidden);

    state.invoke_show_again(0);
    assert_eq!(state.get_duplicate_count(), 1);
    assert_eq!(state.get_hidden_count(), 0);
}

#[test]
fn actions_go_to_the_player_the_tag_editor_and_the_cover_picker() {
    let app = open();
    let before = files(&app.music);
    let edited: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
    let editor = app.window.global::<TagEditorState>();
    let record = edited.clone();
    editor.on_edit_track(move |path| record.borrow_mut().push(vec![path.to_string()]));
    let record = edited.clone();
    editor.on_edit_files(move |paths| {
        record
            .borrow_mut()
            .push(paths.iter().map(|path| path.to_string()).collect());
    });
    let covers: Rc<RefCell<Vec<i32>>> = Rc::default();
    let record = covers.clone();
    app.window
        .global::<CoverSearch>()
        .on_find_album(move |album| record.borrow_mut().push(album));

    app.state().invoke_show();
    let state = app.state();

    // A duplicate plays its group, from that copy, with no album as the context.
    state.invoke_play(CheckList::Duplicates, 0, 1);
    let (titles, origin, start) = app.played.borrow()[0].clone();
    assert_eq!(titles.len(), 2);
    assert_eq!(origin, None);
    assert_eq!(start, 1);

    // A track missing tags plays its album.
    state.invoke_play(CheckList::MissingTags, 0, 0);
    let (titles, origin, start) = app.played.borrow()[1].clone();
    assert_eq!(titles.len(), 1);
    assert!(origin.unwrap().starts_with("album:"));
    assert_eq!(start, 0);

    // One track, then a whole album block.
    state.invoke_edit_tags(CheckList::Duplicates, 0, 0);
    state.invoke_edit_tags(CheckList::MissingTags, 0, -1);
    let edited = edited.borrow();
    assert_eq!(edited.len(), 2);
    assert_eq!(edited[0].len(), 1);
    assert_eq!(edited[1].len(), 1);

    state.invoke_find_cover(0);
    assert_eq!(covers.borrow().len(), 1);

    assert_eq!(files(&app.music), before, "no music file changed");
}
