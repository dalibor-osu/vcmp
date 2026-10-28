//! Search: the field and the pause before searching, what is found, opening a
//! result and coming back, and playing what was found.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use vcmp_app::library_view::{LibraryView, SEARCH_DELAY};
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::{AppWindow, Page, PlayerState, QueueSource};
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
    _dir: tempfile::TempDir,
    music: PathBuf,
    window: AppWindow,
    service: Rc<PlayerService>,
    sent: Recorder,
}

fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(
        &music,
        "1.flac",
        Song {
            album_artist: Some("μ's"),
            ..Song::new("Snow halation", "μ's", "ラブライブ! Best")
        },
    );
    add(
        &music,
        "2.flac",
        Song {
            track: 2,
            album_artist: Some("μ's"),
            ..Song::new("僕らのLIVE", "高坂穂乃果", "ラブライブ! Best")
        },
    );
    add(&music, "3.flac", Song::new("Levels", "Avicii", "Levels"));
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
    let service = PlayerService::connect(&window, library, view, Box::new(sent.clone()), None);
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 1600.0));
    App {
        _dir: dir,
        music,
        window,
        service,
        sent,
    }
}

fn find(window: &AppWindow, role: AccessibleRole, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
}

fn key(window: &AppWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

impl App {
    /// Types into the search field as the user would, then lets the pause pass.
    fn search(&self, text: &str) {
        self.window.set_search_text(text.into());
        self.window.invoke_search_edited(text.into());
        mock_elapsed_time(SEARCH_DELAY);
    }

    fn titles(&self) -> Vec<String> {
        self.window
            .get_search_tracks()
            .iter()
            .map(|row| row.title.to_string())
            .collect()
    }

    fn path(&self, name: &str) -> String {
        self.music.join(name).to_str().unwrap().to_string()
    }
}

#[test]
fn ctrl_f_filters_the_page_or_opens_search_and_typing_searches_after_a_pause() {
    let app = start();
    assert_eq!(
        app.window.get_current_page(),
        Page::Albums,
        "the app opens on Albums"
    );
    let ctrl_f = || {
        app.window.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        key(&app.window, "f");
        app.window
            .window()
            .dispatch_event(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
    };

    // A page with a filter gets the focus in it.
    ctrl_f();
    assert_eq!(app.window.get_current_page(), Page::Albums);
    key(&app.window, "z");
    assert_eq!(app.window.get_albums_filter(), "z");
    key(&app.window, Key::Escape);
    app.window.invoke_release_focus();

    // Elsewhere, Search opens.
    app.window.set_selected_index(7);
    ctrl_f();
    assert_eq!(app.window.get_current_page(), Page::Search);

    for c in "ライブ".chars() {
        key(&app.window, c.to_string());
    }
    assert_eq!(
        app.window.get_search_text(),
        "ライブ",
        "the field has focus"
    );
    assert!(app.titles().is_empty(), "not before typing pauses");
    mock_elapsed_time(SEARCH_DELAY);
    let mut titles = app.titles();
    titles.sort();
    assert_eq!(
        titles,
        ["Snow halation", "僕らのLIVE"],
        "inside album titles"
    );
    let albums: Vec<String> = app
        .window
        .get_search_albums()
        .iter()
        .map(|tile| tile.title.to_string())
        .collect();
    assert_eq!(albums, ["ラブライブ! Best"]);
    let row = app
        .window
        .get_search_tracks()
        .iter()
        .find(|row| row.title == "僕らのLIVE")
        .unwrap();
    assert_eq!(row.artist, "高坂穂乃果 · ラブライブ! Best");

    key(&app.window, Key::Escape);
    assert_eq!(app.window.get_search_text(), "", "Escape clears the field");
}

#[test]
fn nothing_found_says_so() {
    let app = start();
    find(&app.window, AccessibleRole::Tab, "Search").invoke_accessible_default_action();
    app.search("zzz");
    assert!(app.titles().is_empty());
    let message = "Nothing in your library matches “zzz”.";
    assert!(
        ElementHandle::find_by_accessible_label(&app.window, message)
            .next()
            .is_some()
    );
}

#[test]
fn a_result_opens_on_its_page_and_back_returns_to_search() {
    let app = start();
    find(&app.window, AccessibleRole::Tab, "Search").invoke_accessible_default_action();
    app.search("avicii");

    find(&app.window, AccessibleRole::Button, "Levels, Avicii").invoke_accessible_default_action();
    assert_eq!(app.window.get_current_page(), Page::Albums);
    assert!(app.window.get_album_open());
    find(&app.window, AccessibleRole::Button, "Search").invoke_accessible_default_action();
    assert_eq!(app.window.get_current_page(), Page::Search);
    assert!(
        !app.window.get_album_open(),
        "Albums shows its grid next time"
    );
    assert_eq!(app.titles(), ["Levels"], "the results are still there");

    find(&app.window, AccessibleRole::Button, "Avicii, 1 album").invoke_accessible_default_action();
    assert_eq!(app.window.get_current_page(), Page::Artists);
    assert!(app.window.get_artist_open());
    key(&app.window, Key::Escape);
    assert_eq!(
        app.window.get_current_page(),
        Page::Search,
        "Escape goes back too"
    );
}

#[test]
fn a_found_track_plays_the_results_from_there() {
    let app = start();
    find(&app.window, AccessibleRole::Tab, "Search").invoke_accessible_default_action();
    app.search("ライブ");
    find(
        &app.window,
        AccessibleRole::ListItem,
        "僕らのLIVE, 高坂穂乃果 · ラブライブ! Best",
    )
    .invoke_accessible_default_action();
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
    // The results in the order shown, from the one chosen.
    let titles = app.titles();
    let shown: Vec<String> = titles
        .iter()
        .map(|title| {
            app.path(if title == "Snow halation" {
                "1.flac"
            } else {
                "2.flac"
            })
        })
        .collect();
    let chosen = titles.iter().position(|title| title == "僕らのLIVE");
    assert_eq!(played, Some((shown, Some("search".into()), chosen)));

    app.service.handle_event(Event::QueueChanged(QueueView {
        origin: Some("search".into()),
        ..QueueView::default()
    }));
    assert_eq!(
        app.window.global::<PlayerState>().get_queue_source(),
        QueueSource::Search
    );
}
