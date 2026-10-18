//! Selecting several tracks in a track list and the songs table: Ctrl and Shift with
//! the mouse and the keyboard, and the menu's actions on every selected track.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, SharedString};
use vcmp_app::library_service::{Delivery, LibraryPaths};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::tag_editor::TagEditor;
use vcmp_app::{AppWindow, Page, SongSelection, TagEditorState, TrackSelection};
use vcmp_library::Library;
use vcmp_playback::Command;

#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<Command>>>);

impl PlayerControl for Recorder {
    fn send(&self, command: Command) {
        self.0.borrow_mut().push(command);
    }
}

impl Recorder {
    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

struct App {
    _dir: tempfile::TempDir,
    library: Rc<Library>,
    window: AppWindow,
    view: Rc<LibraryView>,
    _service: Rc<PlayerService>,
    _editor: Rc<TagEditor>,
    sent: Recorder,
}

const TITLES: [&str; 4] = ["One", "Two", "Three", "Four"];

/// One album of four songs, its page open.
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    for (track, title) in TITLES.into_iter().enumerate() {
        add(
            &music,
            &format!("{}.flac", track + 1),
            Song {
                track: track as u32 + 1,
                ..Song::new(title, "Band", "Album")
            },
        );
    }
    let paths = LibraryPaths {
        database: dir.path().join("library.sqlite3"),
        cache: dir.path().join("cache"),
    };
    let mut library = Library::open(&paths.database, &paths.cache).unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    let library = Rc::new(library);

    let window = vcmp_app::create_window().unwrap();
    let view = LibraryView::connect(&window, library.clone()).unwrap();
    let sent = Recorder::default();
    let service = PlayerService::connect(
        &window,
        library.clone(),
        view.clone(),
        Box::new(sent.clone()),
        None,
    );
    let editor = TagEditor::connect(
        &window,
        library.clone(),
        paths,
        view.clone(),
        Delivery::Manual,
    );
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 900.0));
    let album = library.tracks().unwrap()[0].album_id.unwrap();
    view.open_album_by_id(album);
    mock_elapsed_time(Duration::ZERO);
    // What the service sends as it starts.
    sent.take();
    App {
        _dir: dir,
        library,
        window,
        view,
        _service: service,
        _editor: editor,
        sent,
    }
}

fn key(window: &AppWindow, key: impl Into<SharedString>) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Runs `then` with `modifier` held down.
fn holding(window: &AppWindow, modifier: Key, then: impl FnOnce()) {
    let text: SharedString = modifier.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    then();
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

impl App {
    fn selected(&self) -> Vec<String> {
        self.window
            .get_album_tracks()
            .iter()
            .filter(|row| row.selected)
            .map(|row| row.title.to_string())
            .collect()
    }

    fn row(&self, title: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(&self.window, title)
            .find(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
            .unwrap_or_else(|| panic!("no row {title:?}"))
    }

    fn click(&self, title: &str) {
        let row = self.row(title);
        let (position, size) = (row.absolute_position(), row.size());
        let position = LogicalPosition::new(
            position.x + size.width / 3.0,
            position.y + size.height / 2.0,
        );
        let window = self.window.window();
        let button = PointerEventButton::Left;
        window.dispatch_event(WindowEvent::PointerMoved { position });
        window.dispatch_event(WindowEvent::PointerPressed { position, button });
        window.dispatch_event(WindowEvent::PointerReleased { position, button });
    }

    fn queued(&self) -> Vec<usize> {
        self.sent
            .take()
            .into_iter()
            .filter_map(|command| match command {
                Command::AddToQueue(entries) | Command::PlayNext(entries) => Some(entries.len()),
                _ => None,
            })
            .collect()
    }

    fn index(&self, title: &str) -> i32 {
        TITLES.iter().position(|t| *t == title).unwrap() as i32
    }
}

#[test]
fn ctrl_and_shift_clicks_select_and_a_plain_click_plays() {
    let app = start();
    holding(&app.window, Key::Control, || app.click("Two"));
    holding(&app.window, Key::Control, || app.click("Four"));
    assert_eq!(app.selected(), ["Two", "Four"]);
    assert!(app.sent.take().is_empty(), "selecting plays nothing");
    assert!(
        app.row("Two").accessible_item_selected().unwrap(),
        "selection is told to assistive technology"
    );

    // Shift selects from the last Ctrl-clicked row.
    holding(&app.window, Key::Shift, || app.click("Two"));
    assert_eq!(app.selected(), ["Two", "Three", "Four"]);
    holding(&app.window, Key::Control, || app.click("Three"));
    assert_eq!(app.selected(), ["Two", "Four"]);

    // The menu's actions take every selected track, in list order.
    app.window
        .invoke_queue_album_track(app.index("Four"), false);
    assert_eq!(app.queued(), [2]);
    // A track outside the selection acts alone.
    app.window.invoke_queue_album_track(app.index("One"), true);
    assert_eq!(app.queued(), [1]);

    app.click("One");
    assert!(app.selected().is_empty(), "a plain click clears");
    assert!(matches!(
        app.sent.take().as_slice(),
        [Command::PlayContext { .. }]
    ));
}

#[test]
fn the_keyboard_selects_too() {
    let app = start();
    let focused = |app: &App| {
        TITLES
            .iter()
            .any(|title| app.row(title).accessible_item_selected() == Some(true))
    };
    assert!(
        (0..30).any(|_| {
            key(&app.window, Key::Tab);
            focused(&app)
        }),
        "the tracks are a tab stop"
    );
    holding(&app.window, Key::Shift, || {
        key(&app.window, Key::DownArrow);
        key(&app.window, Key::DownArrow);
    });
    assert_eq!(app.selected(), ["One", "Two", "Three"]);
    key(&app.window, Key::DownArrow);
    holding(&app.window, Key::Control, || key(&app.window, Key::Space));
    assert_eq!(app.selected(), ["One", "Two", "Three", "Four"]);
    assert!(
        app.sent.take().is_empty(),
        "Ctrl+Space doesn't play or pause"
    );

    key(&app.window, Key::Escape);
    assert!(app.selected().is_empty());
    assert!(app.window.get_album_open(), "the first Escape only clears");
    holding(&app.window, Key::Control, || key(&app.window, "a"));
    assert_eq!(app.selected().len(), 4);
    key(&app.window, Key::Escape);
    key(&app.window, Key::Escape);
    assert!(!app.window.get_album_open(), "then Escape goes back");
}

#[test]
fn selected_tracks_go_into_a_playlist_and_the_tag_editor_together() {
    let app = start();
    holding(&app.window, Key::Control, || app.click("One"));
    holding(&app.window, Key::Control, || app.click("Three"));
    let rows = app.window.get_album_tracks();
    let selection = app.window.global::<TrackSelection>();

    selection.invoke_add_to_playlist(rows.clone(), app.index("Three"), -1);
    let playlist = app.library.playlists().unwrap()[0].id;
    let titles: Vec<String> = app
        .library
        .playlist_entries(playlist)
        .unwrap()
        .into_iter()
        .map(|entry| entry.track.title)
        .collect();
    assert_eq!(titles, ["One", "Three"]);

    selection.invoke_edit_tags(rows, app.index("One"));
    let editor = app.window.global::<TagEditorState>();
    assert!(editor.get_open());
    assert_eq!(editor.get_track_count(), 2);
    assert_eq!(editor.get_subject(), "Album");
    assert!(
        !editor.get_whole_album(),
        "a selection, not the whole album"
    );
}

#[test]
fn several_playlist_entries_are_removed_at_once() {
    let app = start();
    let playlist = app.library.create_playlist("Mix").unwrap();
    let ids: Vec<_> = app
        .library
        .tracks()
        .unwrap()
        .iter()
        .map(|track| track.id)
        .collect();
    app.library.add_to_playlist(playlist, &ids).unwrap();
    app.view.reload();
    app.window.set_selected_index(3);
    assert_eq!(app.window.get_current_page(), Page::Playlists);
    app.window.invoke_playlist_activated(0);
    mock_elapsed_time(Duration::ZERO);

    let rows = app.window.get_playlist_tracks();
    let selection = app.window.global::<TrackSelection>();
    assert_eq!(
        selection.invoke_change(rows.clone(), 1, 1, vcmp_app::SelectionChange::Toggle),
        1
    );
    assert_eq!(
        selection.invoke_change(rows.clone(), 3, 3, vcmp_app::SelectionChange::Toggle),
        2
    );
    app.window.invoke_remove_playlist_track(3);
    let titles: Vec<String> = app
        .library
        .playlist_entries(playlist)
        .unwrap()
        .into_iter()
        .map(|entry| entry.track.title)
        .collect();
    assert_eq!(titles, ["One", "Three"]);
}

#[test]
fn songs_table_rows_are_selected_and_used_together() {
    let app = start();
    app.window.invoke_songs_needed();
    let songs = app.window.get_songs();
    let selection = app.window.global::<SongSelection>();
    let position = |title: &str| songs.iter().position(|row| row.title == title).unwrap() as i32;
    let (one, four) = (position("One"), position("Four"));
    assert_eq!(
        selection.invoke_change(songs.clone(), one, one, vcmp_app::SelectionChange::Toggle),
        1
    );
    assert_eq!(
        selection.invoke_change(songs.clone(), four, one, vcmp_app::SelectionChange::Range),
        4
    );
    app.window.invoke_queue_song(four, true);
    assert_eq!(app.queued(), [4]);
    selection.invoke_add_to_playlist(songs.clone(), one, -1);
    let playlist = app.library.playlists().unwrap().remove(0);
    assert_eq!(playlist.track_count, 4);
    assert_eq!(
        selection.invoke_change(songs.clone(), 0, 0, vcmp_app::SelectionChange::None),
        0
    );
    assert!(songs.iter().all(|row| !row.selected));
}
