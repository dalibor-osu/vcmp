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
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, SharedString, VecModel,
};
use vcmp_app::library_view::{LibraryView, SETTING_SONGS_SORT};
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::{AppWindow, ColumnWidths, Page, SongColumn, SongRow, preferences};
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

#[test]
fn the_filter_keeps_songs_with_every_word_after_a_pause() {
    let app = start();
    app.show();
    app.window.set_songs_filter("bravo".into());
    app.window.invoke_songs_filter_edited("bravo".into());
    assert_eq!(app.titles().len(), 3, "not while typing");
    mock_elapsed_time(vcmp_app::library_view::SEARCH_DELAY);
    assert_eq!(app.titles(), ["Mango"], "by artist");

    app.window.set_songs_filter("second ap".into());
    app.window.invoke_songs_filter_edited("second ap".into());
    mock_elapsed_time(vcmp_app::library_view::SEARCH_DELAY);
    assert_eq!(app.titles(), ["Apple"], "album and title words together");

    // Playing plays what is shown.
    find(&app.window, AccessibleRole::ListItem, "Apple, Charlie")
        .invoke_accessible_default_action();
    assert_eq!(app.played().map(|played| played.0.len()), Some(1));

    app.window.set_songs_filter("zzz".into());
    app.window.invoke_songs_filter_edited("zzz".into());
    mock_elapsed_time(vcmp_app::library_view::SEARCH_DELAY);
    assert!(app.titles().is_empty());
    find(&app.window, AccessibleRole::Text, "No songs match “zzz”.");
}

#[test]
fn the_edges_between_columns_resize_them_and_the_widths_are_kept() {
    let app = start();
    preferences::connect(&app.window, app.library.clone()).unwrap();
    app.show();
    // A heading's label says whether its column sorts.
    let width = |name: &str| {
        let unsorted = format!("Sort by {name}");
        let sorted = format!("{name}, sorted");
        ElementHandle::find_by_accessible_label(&app.window, &unsorted)
            .chain(ElementHandle::find_by_accessible_label(
                &app.window,
                &sorted,
            ))
            .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .unwrap_or_else(|| panic!("no {name} heading"))
            .size()
            .width
    };
    let (title, artist, album) = (width("Title"), width("Artist"), width("Album"));

    // The edge after Title, dragged 60 px to the right, then on past where Artist
    // would be narrower than its least width.
    let edge = find(
        &app.window,
        AccessibleRole::Slider,
        "Width of the Title column",
    );
    let drag = |by: f32| {
        let window = app.window.window();
        let button = slint::platform::PointerEventButton::Left;
        let (position, size) = (edge.absolute_position(), edge.size());
        let from = LogicalPosition::new(
            position.x + size.width / 2.0,
            position.y + size.height / 2.0,
        );
        window.dispatch_event(WindowEvent::PointerMoved { position: from });
        window.dispatch_event(WindowEvent::PointerPressed {
            position: from,
            button,
        });
        for step in 1..=4 {
            let position = LogicalPosition::new(from.x + by * step as f32 / 4.0, from.y);
            window.dispatch_event(WindowEvent::PointerMoved { position });
        }
        let to = LogicalPosition::new(from.x + by, from.y);
        window.dispatch_event(WindowEvent::PointerReleased {
            position: to,
            button,
        });
    };
    drag(60.0);
    assert!((width("Title") - (title + 60.0)).abs() < 1.0, "Title grew");
    assert!(
        (width("Artist") - (artist - 60.0)).abs() < 1.0,
        "Artist gave the room"
    );
    assert!((width("Album") - album).abs() < 1.0, "Album kept its width");
    drag(200.0);
    assert!(
        (width("Artist") - 64.0).abs() < 1.0,
        "Artist keeps its least width"
    );
    assert!((width("Title") + width("Artist") - (title + artist)).abs() < 1.0);

    // From the keyboard, or assistive technology: a step of the two columns' room.
    let columns = app.window.global::<ColumnWidths>();
    let before = columns.get_song_artist();
    find(
        &app.window,
        AccessibleRole::Slider,
        "Width of the Artist column",
    )
    .invoke_accessible_increment_action();
    assert!(columns.get_song_artist() > before);

    // The widths are saved, and come back after a restart.
    let restarted = vcmp_app::create_window().unwrap();
    preferences::connect(&restarted, app.library.clone()).unwrap();
    let kept = restarted.global::<ColumnWidths>();
    for (now, then) in [
        (kept.get_song_title(), columns.get_song_title()),
        (kept.get_song_artist(), columns.get_song_artist()),
        (kept.get_song_album(), columns.get_song_album()),
    ] {
        assert!((now - then).abs() < 1e-4, "{now} came back as {then}");
    }
}
