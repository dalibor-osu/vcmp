//! The Artists page: the grid of album artists or everyone, an artist's page with
//! their albums and appearances, albums opened from there, and playing an artist.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::{AppWindow, Page};
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
    _dir: tempfile::TempDir,
    music: PathBuf,
    window: AppWindow,
    library: Rc<Library>,
    view: Rc<LibraryView>,
    _service: Rc<PlayerService>,
    sent: Recorder,
}

/// Singer has an album and a track on a compilation; Rapper only that track; Zed an
/// album of their own.
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    let mix = |title, artist, track| Song {
        title,
        artist,
        album: "Summer Mix",
        album_artist: Some("Various Artists"),
        track,
        disc: 1,
    };
    add(&music, "m1.flac", mix("Feature", "Singer", 1));
    add(&music, "m2.flac", mix("Verse", "Rapper", 2));
    add(
        &music,
        "s1.flac",
        Song {
            track: 1,
            ..Song::new("First", "Singer", "Debut")
        },
    );
    add(
        &music,
        "s2.flac",
        Song {
            track: 2,
            ..Song::new("Second", "Singer", "Debut")
        },
    );
    add(&music, "z.flac", Song::new("Solo", "Zed", "Zed Alone"));

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
    // Tall enough that an artist's whole page is in view: rows scrolled out of view
    // are only created when they come into it.
    window.window().set_size(LogicalSize::new(1100.0, 1600.0));
    find(&window, AccessibleRole::Tab, "Artists").invoke_accessible_default_action();
    assert_eq!(window.get_current_page(), Page::Artists);
    App {
        _dir: dir,
        music,
        window,
        library,
        view,
        _service: service,
        sent,
    }
}

fn find(window: &AppWindow, role: AccessibleRole, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
}

fn press(window: &AppWindow, key: Key) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

impl App {
    fn names(&self) -> Vec<String> {
        self.window
            .get_artists()
            .iter()
            .map(|tile| tile.title.to_string())
            .collect()
    }

    fn path(&self, name: &str) -> String {
        self.music.join(name).to_str().unwrap().to_string()
    }

    fn played(&self) -> (Vec<String>, Option<String>, Option<usize>) {
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
            .expect("a PlayContext was sent")
    }
}

#[test]
fn album_artists_by_default_everyone_on_request() {
    let app = start();
    assert_eq!(app.names(), ["Singer", "Various Artists", "Zed"]);
    // The card says how much there is: albums, or tracks for credit-only artists.
    find(&app.window, AccessibleRole::Button, "Singer, 1 album");

    find(&app.window, AccessibleRole::RadioButton, "All artists")
        .invoke_accessible_default_action();
    assert!(app.window.get_artists_show_all());
    assert_eq!(app.names(), ["Rapper", "Singer", "Various Artists", "Zed"]);
    find(&app.window, AccessibleRole::Button, "Rapper, 1 track");
}

#[test]
fn an_artist_page_shows_albums_and_appearances() {
    let app = start();
    find(&app.window, AccessibleRole::Button, "Singer, 1 album").invoke_accessible_default_action();
    assert!(app.window.get_artist_open());
    let detail = app.window.get_artist_detail();
    assert_eq!(
        (detail.name.as_str(), detail.album_count, detail.track_count),
        ("Singer", 1, 3)
    );
    let albums: Vec<String> = app
        .window
        .get_artist_albums()
        .iter()
        .map(|tile| tile.title.to_string())
        .collect();
    assert_eq!(albums, ["Debut"]);
    let appearance = app.window.get_artist_appearances().row_data(0).unwrap();
    assert_eq!(
        (appearance.title.as_str(), appearance.artist.as_str()),
        ("Feature", "Summer Mix"),
        "an appearance names its album"
    );

    // An album opens on the artist's page, and back returns to the artist.
    find(&app.window, AccessibleRole::Button, "Debut").invoke_accessible_default_action();
    assert!(app.window.get_artist_album_open());
    assert!(
        !app.window.get_album_open(),
        "the Albums page stays on its grid"
    );
    assert_eq!(app.window.get_album_detail().title, "Debut");
    find(&app.window, AccessibleRole::Button, "Singer").invoke_accessible_default_action();
    assert!(!app.window.get_artist_album_open());
    assert!(app.window.get_artist_open());

    press(&app.window, Key::Escape);
    assert!(
        !app.window.get_artist_open(),
        "Escape goes back to the grid"
    );
}

#[test]
fn the_artists_entry_returns_to_the_grid() {
    let app = start();
    find(&app.window, AccessibleRole::Button, "Zed, 1 album").invoke_accessible_default_action();
    find(&app.window, AccessibleRole::Button, "Zed Alone").invoke_accessible_default_action();
    assert!(app.window.get_artist_album_open());
    find(&app.window, AccessibleRole::Tab, "Artists").invoke_accessible_default_action();
    assert!(!app.window.get_artist_album_open() && !app.window.get_artist_open());
}

#[test]
fn playing_an_artist_plays_albums_then_appearances() {
    let app = start();
    find(&app.window, AccessibleRole::Button, "Singer, 1 album").invoke_accessible_default_action();
    find(&app.window, AccessibleRole::Button, "Play Singer").invoke_accessible_default_action();
    let (paths, origin, start) = app.played();
    assert_eq!(
        paths,
        [
            app.path("s1.flac"),
            app.path("s2.flac"),
            app.path("m1.flac")
        ]
    );
    let singer = app
        .library
        .artists()
        .unwrap()
        .into_iter()
        .find(|artist| artist.name == "Singer")
        .unwrap();
    assert_eq!(origin, Some(format!("artist:{}", singer.id.0)));
    assert_eq!(start, None);

    app.sent.0.borrow_mut().clear();
    find(&app.window, AccessibleRole::ListItem, "Feature, Summer Mix")
        .invoke_accessible_default_action();
    assert_eq!(
        app.played().2,
        Some(2),
        "from the appearance, after the album"
    );

    app.sent.0.borrow_mut().clear();
    app.window.invoke_queue_artist_appearance(0, true);
    assert!(matches!(
        app.sent.0.borrow().as_slice(),
        [Command::PlayNext(entries)] if *entries[0].track.id == *app.path("m1.flac")
    ));
}

#[test]
fn an_open_artist_follows_library_changes() {
    let app = start();
    find(&app.window, AccessibleRole::Button, "Zed, 1 album").invoke_accessible_default_action();
    std::fs::remove_file(app.music.join("z.flac")).unwrap();
    // A scan runs on its own connection, as the app's background scans do.
    let mut scanner = Library::open(
        &app._dir.path().join("library.sqlite3"),
        &app._dir.path().join("cache"),
    )
    .unwrap();
    scanner.scan(&AtomicBool::new(false), |_| {}).unwrap();
    app.view.reload();
    assert!(app.window.get_artist_open(), "the page stays open");
    assert!(app.window.get_artist_detail().missing);
    assert_eq!(app.names(), ["Singer", "Various Artists"]);
}
