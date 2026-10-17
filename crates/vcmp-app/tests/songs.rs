//! The Songs page: loading when first shown, sorting by a column and keeping the
//! order, playing in that order, and creating only the rows in view.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot, mock_elapsed_time};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, ModelRc, SharedString, VecModel};
use vcmp_app::library_view::{LibraryView, SETTING_SONGS_SORT};
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::{AppWindow, Page, SongColumn, SongRow};
use vcmp_library::Library;
use vcmp_playback::Command;

#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<Command>>>);

impl PlayerControl for Recorder {
    fn send(&self, command: Command) {
        self.0.borrow_mut().push(command);
    }
}

struct App {
    dir: tempfile::TempDir,
    music: PathBuf,
    window: AppWindow,
    library: Rc<Library>,
    _service: Rc<PlayerService>,
    sent: Recorder,
}

fn open(dir: tempfile::TempDir, music: PathBuf) -> App {
    let library = Rc::new(
        Library::open(
            &dir.path().join("library.sqlite3"),
            &dir.path().join("cache"),
        )
        .unwrap(),
    );
    let window = vcmp_app::create_window().unwrap();
    let view = LibraryView::connect(&window, library.clone()).unwrap();
    let sent = Recorder::default();
    let service =
        PlayerService::connect(&window, library.clone(), view, Box::new(sent.clone()), None);
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 900.0));
    App {
        dir,
        music,
        window,
        library,
        _service: service,
        sent,
    }
}

/// Three songs whose orders by artist, title and length all differ.
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(&music, "a.flac", Song::new("Zebra", "Alpha", "First"));
    add(&music, "b.flac", Song::new("Apple", "Charlie", "Second"));
    add(&music, "c.flac", Song::new("Mango", "Bravo", "Third"));
    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    drop(library);
    open(dir, music)
}

fn find(window: &AppWindow, role: AccessibleRole, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
}

fn key(window: &AppWindow, key: Key) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

impl App {
    fn show(&self) {
        find(&self.window, AccessibleRole::Tab, "Songs").invoke_accessible_default_action();
        assert_eq!(self.window.get_current_page(), Page::Songs);
        // The page asks for its songs from a change handler, which the event loop
        // runs; without one, advancing the clock does.
        mock_elapsed_time(Duration::ZERO);
    }

    fn titles(&self) -> Vec<String> {
        self.window
            .get_songs()
            .iter()
            .map(|row| row.title.to_string())
            .collect()
    }

    fn played(&self) -> Option<(Vec<String>, Option<String>, Option<usize>)> {
        self.sent
            .0
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::PlayContext {
                    entries,
                    origin,
                    start,
                } => Some((
                    entries.iter().map(|e| e.track.id.to_string()).collect(),
                    origin.clone(),
                    *start,
                )),
                _ => None,
            })
    }
}

#[test]
fn songs_load_when_the_page_is_first_shown() {
    let app = start();
    assert_eq!(app.window.get_songs().row_count(), 0, "not at startup");
    app.show();
    assert_eq!(
        app.titles(),
        ["Zebra", "Mango", "Apple"],
        "by artist at first"
    );
    find(&app.window, AccessibleRole::Text, "3 songs");
}

#[test]
fn headers_sort_and_reverse_and_the_order_is_kept() {
    let app = start();
    app.show();
    find(&app.window, AccessibleRole::Button, "Sort by Title").invoke_accessible_default_action();
    assert_eq!(app.titles(), ["Apple", "Mango", "Zebra"]);
    find(&app.window, AccessibleRole::Button, "Title, sorted").invoke_accessible_default_action();
    assert_eq!(
        app.titles(),
        ["Zebra", "Mango", "Apple"],
        "the same header reverses"
    );
    assert_eq!(app.window.get_songs_sort_column(), SongColumn::Title);
    assert!(app.window.get_songs_sort_descending());
    assert_eq!(
        app.library.setting(SETTING_SONGS_SORT).unwrap().as_deref(),
        Some("title:reverse")
    );

    // A restart shows the same order.
    let App { dir, music, .. } = app;
    let app = open(dir, music);
    app.show();
    assert_eq!(app.titles(), ["Zebra", "Mango", "Apple"]);
    find(
        &app.window,
        AccessibleRole::Button,
        "Title, sorted in reverse",
    );
}

#[test]
fn a_song_plays_every_song_in_the_table_order() {
    let app = start();
    app.show();
    find(&app.window, AccessibleRole::Button, "Sort by Title").invoke_accessible_default_action();
    find(&app.window, AccessibleRole::ListItem, "Mango, Bravo").invoke_accessible_default_action();
    let path = |name: &str| app.music.join(name).to_str().unwrap().to_string();
    assert_eq!(
        app.played(),
        Some((
            vec![path("b.flac"), path("c.flac"), path("a.flac")],
            Some("songs".into()),
            Some(1)
        ))
    );
}

#[test]
fn the_table_works_from_the_keyboard() {
    let app = start();
    app.show();
    let selected = |app: &App| -> Vec<String> {
        app.window
            .root_element()
            .query_descendants()
            .match_accessible_role(AccessibleRole::ListItem)
            .find_all()
            .into_iter()
            .filter(|row| row.accessible_item_selected() == Some(true))
            .filter_map(|row| row.accessible_label().map(|label| label.to_string()))
            .collect()
    };
    // Past the sidebar and the five headers to the rows.
    let reached = (0..20).any(|_| {
        key(&app.window, Key::Tab);
        !selected(&app).is_empty()
    });
    assert!(reached, "the rows are a tab stop");
    key(&app.window, Key::End);
    assert_eq!(selected(&app), ["Apple, Charlie"]);
    key(&app.window, Key::Return);
    assert_eq!(app.played().map(|played| played.2), Some(Some(2)));
}

#[test]
fn only_the_rows_in_view_are_created() {
    let app = start();
    app.show();
    let rows: Vec<SongRow> = (0..5000)
        .map(|i| SongRow {
            title: format!("Song {i}").into(),
            ..SongRow::default()
        })
        .collect();
    app.window.set_songs(ModelRc::new(VecModel::from(rows)));
    let created = app
        .window
        .root_element()
        .query_descendants()
        .match_accessible_role(AccessibleRole::ListItem)
        .find_all()
        .len();
    assert!(created < 60, "{created} rows created for 5000 songs");
}
