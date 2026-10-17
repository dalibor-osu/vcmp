//! The Folders page: music folders at the top, folders and files inside them, the
//! path back up, playing a folder, and following library changes.

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
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::{AppWindow, Page, PlayerState};
use vcmp_library::Library;
use vcmp_playback::{Command, Event, QueueView};

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
    view: Rc<LibraryView>,
    service: Rc<PlayerService>,
    sent: Recorder,
}

/// music/ holds one loose file, Band/Live/ and Band/Studio/ (two files there).
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(&music, "loose.flac", Song::new("Loose", "Solo", "None"));
    add(
        &music.join("Band").join("Live"),
        "1.flac",
        Song::new("Live one", "Band", "Live"),
    );
    add(
        &music.join("Band").join("Studio"),
        "b.flac",
        Song::new("Second", "Band", "Studio"),
    );
    add(
        &music.join("Band").join("Studio"),
        "a.flac",
        Song::new("First", "Band", "Studio"),
    );
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
    let service =
        PlayerService::connect(&window, library, view.clone(), Box::new(sent.clone()), None);
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 1200.0));
    find(&window, AccessibleRole::Tab, "Folders").invoke_accessible_default_action();
    assert_eq!(window.get_current_page(), Page::Folders);
    // The page asks for its tracks from a change handler, which the event loop runs.
    mock_elapsed_time(Duration::ZERO);
    App {
        dir,
        music,
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

impl App {
    fn folders(&self) -> Vec<(String, i32)> {
        self.window
            .get_folder_rows()
            .iter()
            .map(|row| (row.name.to_string(), row.track_count))
            .collect()
    }

    fn files(&self) -> Vec<String> {
        self.window
            .get_folder_files()
            .iter()
            .map(|row| row.title.to_string())
            .collect()
    }

    fn crumbs(&self) -> Vec<String> {
        self.window
            .get_folder_crumbs()
            .iter()
            .map(|crumb| crumb.to_string())
            .collect()
    }

    fn open(&self, name: &str) {
        find(&self.window, AccessibleRole::ListItem, name).invoke_accessible_default_action();
    }
}

#[test]
fn folders_open_down_to_their_files_and_go_back_up() {
    let app = start();
    assert_eq!(
        app.folders(),
        [("music".to_string(), 4)],
        "the music folders first"
    );
    assert!(app.crumbs().is_empty());

    app.open("music");
    assert_eq!(app.folders(), [("Band".to_string(), 3)]);
    assert_eq!(app.files(), ["loose.flac"]);
    app.open("Band");
    app.open("Studio");
    assert_eq!(app.crumbs(), ["music", "Band", "Studio"]);
    assert!(app.folders().is_empty());
    assert_eq!(app.files(), ["a.flac", "b.flac"], "by file name");
    let row = app.window.get_folder_files().row_data(0).unwrap();
    assert_eq!(row.artist, "First · Band", "the tags come second");

    find(&app.window, AccessibleRole::Button, "Band").invoke_accessible_default_action();
    assert_eq!(app.crumbs(), ["music", "Band"]);
    // Backspace goes up from anywhere on the page: here, from the file list.
    app.open("Live");
    let in_files = |app: &App| {
        ElementHandle::find_by_accessible_label(&app.window, "1.flac, Live one · Band")
            .any(|row| row.accessible_item_selected() == Some(true))
    };
    assert!(
        (0..20).any(|_| {
            key(&app.window, Key::Tab);
            in_files(&app)
        }),
        "the files are a tab stop"
    );
    key(&app.window, Key::Backspace);
    assert_eq!(app.crumbs(), ["music", "Band"]);
    find(&app.window, AccessibleRole::Button, "Music folders").invoke_accessible_default_action();
    assert!(app.crumbs().is_empty());
}

#[test]
fn a_file_plays_its_folder_from_there() {
    let app = start();
    app.open("music");
    app.open("Band");
    app.open("Studio");
    find(
        &app.window,
        AccessibleRole::ListItem,
        "b.flac, Second · Band",
    )
    .invoke_accessible_default_action();
    let studio = app.music.join("Band").join("Studio");
    let played = app
        .sent
        .0
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PlayContext {
                entries,
                origin,
                start,
            } => Some((
                entries
                    .iter()
                    .map(|entry| entry.track.id.to_string())
                    .collect::<Vec<_>>(),
                origin.clone(),
                *start,
            )),
            _ => None,
        });
    let path = |name: &str| studio.join(name).to_str().unwrap().to_string();
    assert_eq!(
        played,
        Some((
            vec![path("a.flac"), path("b.flac")],
            Some(format!("folder:{}", studio.to_str().unwrap())),
            Some(1)
        ))
    );

    app.service.handle_event(Event::QueueChanged(QueueView {
        origin: played.and_then(|played| played.1),
        ..QueueView::default()
    }));
    assert_eq!(
        app.window.global::<PlayerState>().get_queue_origin(),
        "Studio",
        "the queue names the folder"
    );
}

#[test]
fn a_folder_that_lost_its_files_gives_way_to_the_one_above() {
    let app = start();
    app.open("music");
    app.open("Band");
    app.open("Live");
    std::fs::remove_file(app.music.join("Band").join("Live").join("1.flac")).unwrap();
    let mut scanner = Library::open(
        &app.dir.path().join("library.sqlite3"),
        &app.dir.path().join("cache"),
    )
    .unwrap();
    scanner.scan(&AtomicBool::new(false), |_| {}).unwrap();
    app.view.reload();
    assert_eq!(app.crumbs(), ["music", "Band"]);
    assert_eq!(app.folders(), [("Studio".to_string(), 2)]);
}
