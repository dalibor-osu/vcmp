//! Playlists in the window: making one from a track menu or the Playlists page,
//! naming it, its page with missing files, playing it, editing its order, the
//! sidebar list and search.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use vcmp_app::library_view::{LibraryView, SEARCH_DELAY};
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::{AppWindow, Page, PlayerState, Playlists};
use vcmp_library::{Library, PlaylistId};
use vcmp_playback::{Command, Event, NowPlaying, QueueEntry, QueueView, TrackRef};

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
    library: Rc<Library>,
    window: AppWindow,
    view: Rc<LibraryView>,
    service: Rc<PlayerService>,
    sent: Recorder,
}

/// Three songs on one album.
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    for (track, title) in ["One", "Two", "Three"].into_iter().enumerate() {
        add(
            &music,
            &format!("{}.flac", track + 1),
            Song {
                track: track as u32 + 1,
                ..Song::new(title, "Band", "Album")
            },
        );
    }
    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
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
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 1200.0));
    mock_elapsed_time(Duration::ZERO);
    App {
        dir,
        music,
        library,
        window,
        view,
        service,
        sent,
    }
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

fn alt(window: &AppWindow, pressed: Key) {
    let alt: SharedString = Key::Alt.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: alt.clone() });
    key(window, pressed);
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: alt });
}

impl App {
    fn path(&self, file: &str) -> SharedString {
        self.music.join(file).to_string_lossy().as_ref().into()
    }

    fn names(&self) -> Vec<String> {
        let names = self.window.global::<Playlists>().get_names();
        names.iter().map(|name| name.to_string()).collect()
    }

    fn rows(&self) -> Vec<(String, bool)> {
        self.window
            .get_playlist_tracks()
            .iter()
            .map(|row| (row.title.to_string(), row.unavailable))
            .collect()
    }

    fn entry_titles(&self, playlist: PlaylistId) -> Vec<String> {
        self.library
            .playlist_entries(playlist)
            .unwrap()
            .into_iter()
            .map(|entry| entry.track.title)
            .collect()
    }

    fn played(&self) -> (usize, Option<String>, Option<usize>) {
        self.sent
            .0
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PlayContext {
                    entries,
                    origin,
                    start,
                } => Some((entries.len(), origin.clone(), *start)),
                _ => None,
            })
            .expect("something played")
    }

    fn open_page(&self, label: &str) {
        find(&self.window, AccessibleRole::Tab, label).invoke_accessible_default_action();
        mock_elapsed_time(Duration::ZERO);
    }
}

#[test]
fn a_track_menu_adds_to_a_new_playlist_and_then_to_that_one() {
    let app = start();
    let playlists = app.window.global::<Playlists>();
    assert_eq!(playlists.get_new_name(), "Playlist 1");
    playlists.invoke_add_track(-1, app.path("2.flac"));
    assert_eq!(app.names(), ["Playlist 1"]);
    assert_eq!(playlists.get_new_name(), "Playlist 2");
    playlists.invoke_add_track(0, app.path("1.flac"));

    let tile = app.window.get_playlists().row_data(0).unwrap();
    assert_eq!((tile.title.as_str(), tile.track_count), ("Playlist 1", 2));
    let id = app.library.playlists().unwrap()[0].id;
    assert_eq!(app.entry_titles(id), ["Two", "One"], "added to the end");
    assert!(
        app.window.get_playlists().row_count() == 1 && !app.window.get_playlist_open(),
        "the page stays where it was"
    );

    // The sidebar lists it, and opens it.
    app.open_page("Albums");
    find(&app.window, AccessibleRole::Button, "Playlist 1").invoke_accessible_default_action();
    assert_eq!(app.window.get_current_page(), Page::Playlists);
    assert!(app.window.get_playlist_open());
    assert_eq!(app.window.get_playlist_detail().name, "Playlist 1");
    assert_eq!(app.window.get_open_playlist_index(), 0);
}

#[test]
fn a_new_playlist_opens_with_its_name_ready_to_type() {
    let app = start();
    app.open_page("Playlists");
    ElementHandle::find_by_accessible_label(
        &app.window,
        "No playlists yet. Make one with New playlist, or from any track's menu: right-click a track, or press the Menu key, and choose Add to playlist.",
    )
    .next()
    .expect("the empty state says how to make one");

    find(&app.window, AccessibleRole::Button, "New playlist").invoke_accessible_default_action();
    mock_elapsed_time(Duration::ZERO);
    assert!(app.window.get_playlist_open());
    assert_eq!(app.window.get_playlist_detail().name, "Playlist 1");
    let field = ElementHandle::find_by_accessible_label(&app.window, "Playlist name")
        .next()
        .expect("the name is a field");
    field.set_accessible_value("Road trip");
    key(&app.window, Key::Return);
    assert_eq!(app.window.get_playlist_detail().name, "Road trip");
    assert_eq!(app.names(), ["Road trip"]);
    assert!(
        ElementHandle::find_by_accessible_label(&app.window, "Playlist name")
            .next()
            .is_none(),
        "the field goes once the name is kept"
    );
    ElementHandle::find_by_accessible_label(
        &app.window,
        "This playlist is empty. To add a track, open its menu with a right click or the Menu key, and choose Add to playlist.",
    )
    .next()
    .expect("an empty playlist says how to fill it");
}

#[test]
fn missing_files_stay_dimmed_and_playback_skips_them() {
    let app = start();
    let id = app.library.create_playlist("Mix").unwrap();
    let tracks = app.library.tracks().unwrap();
    let ids: Vec<_> = tracks.iter().map(|track| track.id).collect();
    app.library.add_to_playlist(id, &ids).unwrap();
    std::fs::remove_file(app.music.join("2.flac")).unwrap();
    // Scanning takes its own connection, as the library service does.
    let mut scanner = Library::open(
        &app.dir.path().join("library.sqlite3"),
        &app.dir.path().join("cache"),
    )
    .unwrap();
    scanner.scan(&AtomicBool::new(false), |_| {}).unwrap();
    app.view.reload();

    app.open_page("Playlists");
    app.window.invoke_playlist_activated(0);
    assert_eq!(
        app.rows(),
        [
            ("One".to_string(), false),
            ("Two".to_string(), true),
            ("Three".to_string(), false)
        ]
    );
    let detail = app.window.get_playlist_detail();
    assert_eq!((detail.track_count, detail.unavailable_count), (3, 1));
    ElementHandle::find_by_accessible_label(&app.window, "Two, file not found")
        .next()
        .expect("the missing file says so in words");

    let origin = Some(format!("playlist:{}", id.0));
    app.window.invoke_play_playlist(-1, false);
    assert_eq!(app.played(), (2, origin.clone(), None));
    find(&app.window, AccessibleRole::ListItem, "Three, Band · Album")
        .invoke_accessible_default_action();
    assert_eq!(app.played(), (2, origin.clone(), Some(1)));
    // Starting at the missing one plays from the next that is there.
    app.window.invoke_play_playlist(1, false);
    assert_eq!(app.played(), (2, origin.clone(), Some(1)));

    // The queue panel names the playlist it plays.
    app.service.handle_event(Event::QueueChanged(QueueView {
        origin,
        ..QueueView::default()
    }));
    assert_eq!(app.window.global::<PlayerState>().get_queue_origin(), "Mix");
}

#[test]
fn entries_move_and_go_from_the_keyboard() {
    let app = start();
    let id = app.library.create_playlist("Mix").unwrap();
    let ids: Vec<_> = app
        .library
        .tracks()
        .unwrap()
        .iter()
        .map(|track| track.id)
        .collect();
    app.library.add_to_playlist(id, &ids).unwrap();
    app.view.reload();
    app.open_page("Playlists");
    app.window.invoke_playlist_activated(0);
    mock_elapsed_time(Duration::ZERO);

    let in_list = |app: &App| {
        ElementHandle::find_by_accessible_label(&app.window, "One, Band · Album")
            .any(|row| row.accessible_item_selected() == Some(true))
    };
    assert!(
        (0..20).any(|_| {
            key(&app.window, Key::Tab);
            in_list(&app)
        }),
        "the tracks are a tab stop"
    );
    alt(&app.window, Key::DownArrow);
    assert_eq!(app.entry_titles(id), ["Two", "One", "Three"]);
    alt(&app.window, Key::DownArrow);
    assert_eq!(app.entry_titles(id), ["Two", "Three", "One"]);
    alt(&app.window, Key::DownArrow);
    assert_eq!(
        app.entry_titles(id),
        ["Two", "Three", "One"],
        "the last stays last"
    );
    alt(&app.window, Key::UpArrow);
    assert_eq!(app.entry_titles(id), ["Two", "One", "Three"]);
    key(&app.window, Key::Delete);
    assert_eq!(app.entry_titles(id), ["Two", "Three"]);
    assert_eq!(
        app.rows(),
        [("Two".to_string(), false), ("Three".to_string(), false)]
    );
    assert_eq!(
        app.window.get_playlist_tracks().row_data(1).unwrap().number,
        "2"
    );
}

#[test]
fn deleting_a_playlist_keeps_its_tracks() {
    let app = start();
    let id = app.library.create_playlist("Mix").unwrap();
    let one = app.library.tracks().unwrap()[0].id;
    app.library.add_to_playlist(id, &[one]).unwrap();
    app.view.reload();
    app.window.invoke_playlist_activated(0);
    assert!(app.window.get_playlist_open());

    app.window.invoke_delete_playlist();
    assert!(!app.window.get_playlist_open());
    assert!(app.names().is_empty());
    assert_eq!(app.window.get_playlists().row_count(), 0);
    assert_eq!(app.library.tracks().unwrap().len(), 3);
}

#[test]
fn search_finds_playlists_by_name() {
    let app = start();
    app.library.create_playlist("Summer evenings").unwrap();
    app.library.create_playlist("Winter").unwrap();
    app.view.reload();
    app.window.set_search_text("even".into());
    app.window.invoke_search_edited("even".into());
    mock_elapsed_time(SEARCH_DELAY + Duration::from_millis(10));
    let found: Vec<String> = app
        .window
        .get_search_playlists()
        .iter()
        .map(|tile| tile.title.to_string())
        .collect();
    assert_eq!(found, ["Summer evenings"]);
    app.window.invoke_search_playlist_activated(0);
    assert!(app.window.get_playlist_open());
    assert_eq!(app.window.get_playlist_detail().name, "Summer evenings");
}

#[test]
fn a_whole_album_goes_into_a_playlist_named_after_it() {
    let app = start();
    let album = app.library.tracks().unwrap()[0].album_id.unwrap();
    app.view.open_album_by_id(album);
    mock_elapsed_time(Duration::ZERO);
    find(
        &app.window,
        AccessibleRole::Button,
        "Add Album to a playlist",
    );

    let playlists = app.window.global::<Playlists>();
    playlists.invoke_add_album(-1);
    assert_eq!(app.names(), ["Album"]);
    let id = app.library.playlists().unwrap()[0].id;
    assert_eq!(app.entry_titles(id), ["One", "Two", "Three"]);
    playlists.invoke_add_album(0);
    assert_eq!(app.entry_titles(id).len(), 6, "added again to the same one");
}

#[test]
fn queued_and_playing_tracks_can_be_added_to_a_playlist() {
    let app = start();
    let entry = |file: &str| QueueEntry::new(TrackRef::local(app.path(file).as_str()));
    app.service.handle_event(Event::QueueChanged(QueueView {
        up_next: vec![entry("2.flac")],
        upcoming: vec![entry("3.flac")],
        ..QueueView::default()
    }));
    let state = app.window.global::<PlayerState>();
    let up_next = state.get_up_next().row_data(0).unwrap();
    assert_eq!(up_next.path, app.path("2.flac"), "the menu knows the file");
    let upcoming = state.get_upcoming().row_data(0).unwrap();
    assert_eq!(upcoming.path, app.path("3.flac"));

    // The now playing view has the same menu for the track playing.
    app.service
        .handle_event(Event::TrackChanged(Some(NowPlaying {
            entry: entry("1.flac"),
            duration: None,
        })));
    state.set_now_playing_open(true);
    mock_elapsed_time(Duration::ZERO);
    find(&app.window, AccessibleRole::Button, "More for One");
    app.window
        .global::<Playlists>()
        .invoke_add_track(-1, state.get_track_path());
    let id = app.library.playlists().unwrap()[0].id;
    assert_eq!(app.entry_titles(id), ["One"]);
}

#[test]
fn entries_can_be_dragged_to_a_new_place() {
    let app = start();
    let id = app.library.create_playlist("Mix").unwrap();
    let ids: Vec<_> = app
        .library
        .tracks()
        .unwrap()
        .iter()
        .map(|track| track.id)
        .collect();
    app.library.add_to_playlist(id, &ids).unwrap();
    app.view.reload();
    app.open_page("Playlists");
    app.window.invoke_playlist_activated(0);
    mock_elapsed_time(Duration::ZERO);

    let centre = |title: &str| {
        let row = find(
            &app.window,
            AccessibleRole::ListItem,
            &format!("{title}, Band · Album"),
        );
        let (position, size) = (row.absolute_position(), row.size());
        slint::LogicalPosition::new(
            position.x + size.width / 3.0,
            position.y + size.height / 2.0,
        )
    };
    let drag = |from: slint::LogicalPosition, to: slint::LogicalPosition| {
        let window = app.window.window();
        let button = slint::platform::PointerEventButton::Left;
        window.dispatch_event(WindowEvent::PointerMoved { position: from });
        window.dispatch_event(WindowEvent::PointerPressed {
            position: from,
            button,
        });
        for step in 1..=4 {
            let y = from.y + (to.y - from.y) * step as f32 / 4.0;
            let position = slint::LogicalPosition::new(from.x, y);
            window.dispatch_event(WindowEvent::PointerMoved { position });
        }
        window.dispatch_event(WindowEvent::PointerReleased {
            position: to,
            button,
        });
    };

    // One down past Three: it lands last, and nothing plays.
    drag(centre("One"), centre("Three"));
    assert_eq!(app.entry_titles(id), ["Two", "Three", "One"]);
    // And back up to the top.
    drag(centre("One"), centre("Two"));
    assert_eq!(app.entry_titles(id), ["One", "Two", "Three"]);
    assert!(
        !app.sent
            .0
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::PlayContext { .. })),
        "a drag is not a click"
    );
    // A plain click still plays.
    let position = centre("Two");
    let window = app.window.window();
    let button = slint::platform::PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    assert_eq!(app.played().2, Some(1));
}

#[test]
fn a_drag_held_at_the_edge_scrolls_the_list() {
    let app = start();
    app.window
        .window()
        .set_size(LogicalSize::new(1100.0, 600.0));
    let id = app.library.create_playlist("Long").unwrap();
    let ids: Vec<_> = app
        .library
        .tracks()
        .unwrap()
        .iter()
        .map(|track| track.id)
        .collect();
    for _ in 0..15 {
        app.library.add_to_playlist(id, &ids).unwrap();
    }
    app.view.reload();
    app.open_page("Playlists");
    app.window.invoke_playlist_activated(0);
    mock_elapsed_time(Duration::ZERO);

    let rows = |title: &str| -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&app.window, &format!("{title}, Band · Album"))
            .filter(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
            .collect()
    };
    let centre = |row: &ElementHandle| {
        let (position, size) = (row.absolute_position(), row.size());
        slint::LogicalPosition::new(
            position.x + size.width / 3.0,
            position.y + size.height / 2.0,
        )
    };
    // Drags from `from` to `to`, and holds the pointer still there for a while.
    let hold = |from: slint::LogicalPosition, to: slint::LogicalPosition| {
        let window = app.window.window();
        let button = slint::platform::PointerEventButton::Left;
        window.dispatch_event(WindowEvent::PointerMoved { position: from });
        window.dispatch_event(WindowEvent::PointerPressed {
            position: from,
            button,
        });
        for step in 1..=4 {
            let y = from.y + (to.y - from.y) * step as f32 / 4.0;
            let position = slint::LogicalPosition::new(from.x, y);
            window.dispatch_event(WindowEvent::PointerMoved { position });
        }
        for _ in 0..300 {
            mock_elapsed_time(Duration::from_millis(20));
        }
        window.dispatch_event(WindowEvent::PointerReleased {
            position: to,
            button,
        });
    };
    let mut expected: Vec<String> = ["One", "Two", "Three"]
        .repeat(15)
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(app.entry_titles(id), expected);
    // Rows outside the view can't be found.
    assert_eq!(rows("One").len(), 2, "the list is longer than the window");

    // The first track, held past the bottom edge, goes to the end.
    let first = centre(&rows("One")[0]);
    hold(first, slint::LogicalPosition::new(first.x, 599.0));
    let moved = expected.remove(0);
    expected.push(moved);
    assert_eq!(app.entry_titles(id), expected);

    // The last one, held at the top edge, goes back to the start.
    let last = centre(rows("One").last().unwrap());
    assert!(
        rows("Three").last().unwrap().absolute_position().y < last.y,
        "the view stays at the end of the list, where the moved track is"
    );
    hold(last, slint::LogicalPosition::new(last.x, 1.0));
    let moved = expected.pop().unwrap();
    expected.insert(0, moved);
    assert_eq!(app.entry_titles(id), expected);
}
