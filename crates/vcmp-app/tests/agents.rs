//! Changes AI agents ask for: the note in the sidebar, the history in Settings, the
//! review (accept, reject, decide later), undo, a playlist deletion's warning, a file
//! in use, the backups, and the switch. Files are copies in a temporary directory.

mod common;

use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, LogicalSize, Model};
use vcmp_app::agents::{
    AgentService, SETTING_ENABLED, SETTING_PORT, SETTING_TOKEN, relay_connection,
};
use vcmp_app::library_service::{Delivery, LibraryPaths};
use vcmp_app::library_view::LibraryView;
use vcmp_app::{
    AgentState, AppWindow, ChangeField, ChangeKind, ChangeStatus, ReviewStatus, ServerStatus,
};
use vcmp_library::{ChangeSetId, FieldEdit, Library, TagEdit, TrackId};

struct App {
    dir: tempfile::TempDir,
    paths: LibraryPaths,
    library: Rc<Library>,
    window: AppWindow,
    agents: Rc<AgentService>,
}

/// Two songs of one album, and a single.
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(&music, "1.flac", Song::new("One", "Band", "Album"));
    add(&music, "2.flac", Song::new("Two", "Band", "Album"));
    add(&music, "single.flac", Song::new("Single", "Solo", "Lone"));
    let paths = LibraryPaths {
        database: dir.path().join("library.sqlite3"),
        cache: dir.path().join("cache"),
    };
    let mut library = Library::open(&paths.database, &paths.cache).unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    // A free port for the server, not the one a running VCMP may hold.
    library.set_setting(SETTING_PORT, "0").unwrap();
    open(dir, paths)
}

/// Opens the window and the service on the library in `dir`.
fn open(dir: tempfile::TempDir, paths: LibraryPaths) -> App {
    let library = Rc::new(Library::open(&paths.database, &paths.cache).unwrap());
    let window = vcmp_app::create_window().unwrap();
    let view = LibraryView::connect(&window, library.clone()).unwrap();
    let agents = AgentService::connect(
        &window,
        library.clone(),
        paths.clone(),
        view,
        Delivery::Manual,
    );
    window.show().unwrap();
    // Tall enough for the whole Settings page: the finder skips what is scrolled out.
    window.window().set_size(LogicalSize::new(1100.0, 2400.0));
    App {
        dir,
        paths,
        library,
        window,
        agents,
    }
}

impl App {
    fn state(&self) -> AgentState<'_> {
        self.window.global::<AgentState>()
    }

    fn id(&self, title: &str) -> TrackId {
        self.library
            .tracks()
            .unwrap()
            .into_iter()
            .find(|track| track.title == title)
            .unwrap_or_else(|| panic!("no track {title:?}"))
            .id
    }

    fn title_of(&self, id: TrackId) -> String {
        self.library.track(id).unwrap().unwrap().title
    }

    fn retitle(&self, agent: &str, titles: &[(&str, &str)]) -> ChangeSetId {
        let edits: Vec<(TrackId, TagEdit)> = titles
            .iter()
            .map(|(from, to)| {
                (
                    self.id(from),
                    TagEdit {
                        title: FieldEdit::Set((*to).into()),
                        ..TagEdit::default()
                    },
                )
            })
            .collect();
        self.agents.submit_tags(agent, &edits).unwrap()
    }

    fn find(&self, role: AccessibleRole, label: &str) -> ElementHandle {
        self.try_find(role, label)
            .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
    }

    fn try_find(&self, role: AccessibleRole, label: &str) -> Option<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(role))
    }

    fn press(&self, label: &str) {
        self.find(AccessibleRole::Button, label)
            .invoke_accessible_default_action();
    }

    fn open_settings(&self) {
        self.find(AccessibleRole::Tab, "Settings")
            .invoke_accessible_default_action();
    }

    fn accept(&self, button: &str) {
        self.press(button);
        assert!(self.agents.wait_until_written(Duration::from_secs(20)));
    }

    fn history(&self) -> Vec<(ChangeKind, ChangeStatus)> {
        self.state()
            .get_history()
            .iter()
            .map(|entry| (entry.kind, entry.status))
            .collect()
    }
}

#[test]
fn a_waiting_change_shows_in_the_sidebar_and_in_settings() {
    let app = start();
    assert!(
        app.try_find(
            AccessibleRole::Button,
            "1 change by an agent waits for your review, opens it"
        )
        .is_none()
    );

    app.retitle("Claude", &[("One", "Uno")]);
    assert_eq!(app.state().get_pending_count(), 1);
    app.find(
        AccessibleRole::Button,
        "1 change by an agent waits for your review, opens it",
    );
    app.open_settings();
    app.find(AccessibleRole::Button, "Review the change asked by Claude");
    assert_eq!(app.history(), [(ChangeKind::Tags, ChangeStatus::Pending)]);
    assert_eq!(app.title_of(app.id("One")), "One", "nothing is written yet");
}

#[test]
fn accepting_writes_the_files() {
    let app = start();
    let one = app.id("One");
    app.retitle("Claude", &[("One", "Uno")]);

    app.press("1 change by an agent waits for your review, opens it");
    let state = app.state();
    assert!(state.get_review_open());
    assert_eq!(state.get_review_kind(), ChangeKind::Tags);
    assert_eq!(state.get_review_agent(), "Claude");
    let rows: Vec<_> = state.get_review_rows().iter().collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].field, ChangeField::Title);
    assert_eq!(
        (rows[0].before.as_str(), rows[0].after.as_str()),
        ("One", "Uno")
    );

    app.accept("Write 1 file");
    assert_eq!(state.get_review_status(), ReviewStatus::Done);
    assert_eq!(state.get_review_written(), 1);
    assert_eq!(state.get_review_problems().row_count(), 0);
    assert_eq!(app.title_of(one), "Uno");
    assert_eq!(state.get_pending_count(), 0);
    assert_eq!(app.history(), [(ChangeKind::Tags, ChangeStatus::Accepted)]);

    app.press("Close");
    assert!(!state.get_review_open());
}

#[test]
fn the_same_change_in_several_files_is_one_row() {
    let app = start();
    let edit = TagEdit {
        album_artist: FieldEdit::Set("Band".into()),
        ..TagEdit::default()
    };
    app.agents
        .submit_tags(
            "Claude",
            &[(app.id("One"), edit.clone()), (app.id("Two"), edit)],
        )
        .unwrap();
    app.press("1 change by an agent waits for your review, opens it");
    let rows: Vec<_> = app.state().get_review_rows().iter().collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].field, ChangeField::AlbumArtist);
    assert_eq!(rows[0].files, 2);
    assert_eq!(
        (rows[0].before.as_str(), rows[0].after.as_str()),
        ("", "Band")
    );
}

#[test]
fn rejecting_or_deciding_later_writes_nothing() {
    let app = start();
    app.retitle("Claude", &[("One", "Uno")]);
    let state = app.state();

    app.press("1 change by an agent waits for your review, opens it");
    app.press("Close; the change keeps waiting for your review");
    assert!(!state.get_review_open());
    assert_eq!(state.get_pending_count(), 1);

    app.press("1 change by an agent waits for your review, opens it");
    app.press("Reject the change; nothing is written");
    assert!(!state.get_review_open());
    assert_eq!(state.get_pending_count(), 0);
    assert_eq!(app.history(), [(ChangeKind::Tags, ChangeStatus::Rejected)]);
    assert_eq!(app.title_of(app.id("One")), "One");
}

#[test]
fn an_undo_from_settings_is_reviewed_and_puts_the_tags_back() {
    let app = start();
    let one = app.id("One");
    app.retitle("Claude", &[("One", "Uno")]);
    app.press("1 change by an agent waits for your review, opens it");
    app.accept("Write 1 file");
    app.press("Close");

    app.open_settings();
    app.press("Undo the change asked by Claude");
    let state = app.state();
    assert!(state.get_review_open());
    assert_eq!(state.get_review_kind(), ChangeKind::Undo);
    assert_eq!(state.get_review_agent(), "", "the owner asked");
    let rows: Vec<_> = state.get_review_rows().iter().collect();
    assert_eq!(rows[0].field, ChangeField::Title);
    assert_eq!(
        (rows[0].before.as_str(), rows[0].after.as_str()),
        ("Uno", "One")
    );
    assert_eq!(
        app.title_of(one),
        "Uno",
        "nothing is written before accepting"
    );

    app.accept("Undo in 1 file");
    assert_eq!(app.title_of(one), "One");
    assert_eq!(
        app.history(),
        [
            (ChangeKind::Undo, ChangeStatus::Accepted),
            (ChangeKind::Tags, ChangeStatus::Undone),
        ]
    );
    // The undo is a change of its own, so undoing it writes the agent's tags again;
    // the change it undid can't be undone a second time.
    let can_undo: Vec<bool> = state
        .get_history()
        .iter()
        .map(|entry| entry.can_undo)
        .collect();
    assert_eq!(can_undo, [true, false]);
}

#[test]
fn deleting_a_playlist_warns_that_it_cannot_be_undone() {
    let app = start();
    let playlist = app.library.create_playlist("Road trip").unwrap();
    app.agents
        .submit_playlist_deletion("Claude", playlist)
        .unwrap();

    app.press("1 change by an agent waits for your review, opens it");
    let state = app.state();
    assert_eq!(state.get_review_kind(), ChangeKind::Playlist);
    assert_eq!(state.get_review_playlist(), "Road trip");
    assert!(
        ElementHandle::find_by_accessible_label(
            &app.window,
            "Deleting a playlist can't be undone."
        )
        .next()
        .is_some()
    );
    app.accept("Delete playlist");
    assert!(app.library.playlist(playlist).unwrap().is_none());
    assert_eq!(
        app.history(),
        [(ChangeKind::Playlist, ChangeStatus::Accepted)]
    );
    assert!(!state.get_history().row_data(0).unwrap().can_undo);
}

#[test]
fn a_file_playback_holds_open_keeps_the_change_waiting_where_it_cannot_be_replaced() {
    let app = start();
    let one = app.library.track(app.id("One")).unwrap().unwrap();
    app.retitle("Claude", &[("One", "Uno")]);
    app.agents.set_open_files_locked(true);
    app.agents.set_playback_file(Some(&one.path));

    app.press("1 change by an agent waits for your review, opens it");
    let state = app.state();
    assert_eq!(state.get_review_playing(), "1.flac");
    // Accept is disabled; pressing it anyway writes nothing.
    app.press("Write 1 file");
    assert_eq!(state.get_review_status(), ReviewStatus::Waiting);

    app.agents.set_playback_file(None);
    assert_eq!(state.get_review_playing(), "");
    app.accept("Write 1 file");
    assert_eq!(app.title_of(one.id), "Uno");
}

#[test]
fn deleting_the_backups_asks_first() {
    let app = start();
    let cover = TagEdit {
        cover: FieldEdit::Set(png()),
        ..TagEdit::default()
    };
    app.agents
        .submit_tags("Claude", &[(app.id("One"), cover)])
        .unwrap();
    app.press("1 change by an agent waits for your review, opens it");
    app.accept("Write 1 file");
    app.press("Close");
    let state = app.state();
    assert!(state.get_has_finished());
    assert_ne!(state.get_backups_size(), "");

    app.open_settings();
    app.press("Delete backups…");
    app.press("Keep the backups");
    assert_eq!(state.get_history().row_count(), 1);
    app.press("Delete backups…");
    app.press("Delete backups");
    assert_eq!(state.get_history().row_count(), 0);
    assert_eq!(state.get_backups_size(), "");
    assert!(!state.get_has_finished());
}

#[test]
fn the_switch_starts_the_server_and_is_kept() {
    let app = start();
    let state = app.state();
    assert!(!state.get_enabled(), "off by default");
    assert!(!app.agents.serving());
    app.open_settings();
    app.find(AccessibleRole::Switch, "Let AI agents use your library")
        .invoke_accessible_default_action();
    assert!(state.get_enabled());
    assert!(app.agents.serving());
    assert_eq!(state.get_status(), ServerStatus::Running);
    let token = state.get_token().to_string();
    assert_eq!(token.len(), 64);
    let port = state.get_port().to_string();
    assert_ne!(port, "0");
    assert_eq!(
        state.get_address(),
        format!("http://127.0.0.1:{port}/mcp").as_str()
    );
    let command = state.get_setup_command();
    assert!(command.contains(&token), "{command}");
    assert!(command.contains("--scope user"), "{command}");
    let desktop = state.get_desktop_config();
    assert!(desktop.starts_with("\"vcmp\": {"), "{desktop}");
    assert!(desktop.contains("\"args\":[\"mcp\"]"), "{desktop}");
    app.find(AccessibleRole::Button, "Copy the token");
    assert_eq!(
        app.library.setting(SETTING_ENABLED).unwrap().as_deref(),
        Some("true")
    );

    app.press("Make a new token; agents with the old one can't connect any more");
    assert_ne!(state.get_token().as_str(), token);
    assert!(app.agents.serving());

    app.find(AccessibleRole::Switch, "Let AI agents use your library")
        .invoke_accessible_default_action();
    assert!(!app.agents.serving());
    assert_eq!(state.get_status(), ServerStatus::Off);
}

#[test]
fn a_taken_port_says_so_until_another_is_chosen() {
    let app = start();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    app.library
        .set_setting(SETTING_PORT, &port.to_string())
        .unwrap();
    app.state().invoke_set_enabled(true);
    assert!(!app.agents.serving());
    assert_eq!(app.state().get_status(), ServerStatus::Failed);

    // Below 1024, or not a number: left as it was.
    app.state().invoke_set_port("80".into());
    assert_eq!(app.state().get_port(), port.to_string().as_str());
    drop(taken);
    app.state().invoke_set_port(port.to_string().into());
    assert!(app.agents.serving());
    assert_eq!(app.state().get_status(), ServerStatus::Running);
}

#[test]
fn the_switch_is_kept_and_waiting_changes_are_discarded_at_start() {
    let app = start();
    app.state().invoke_set_enabled(true);
    app.retitle("Claude", &[("One", "Uno")]);

    let App {
        dir,
        paths,
        library,
        window,
        agents,
    } = app;
    drop((agents, window, library));
    let app = open(dir, paths);
    assert!(app.state().get_enabled());
    assert_eq!(app.state().get_pending_count(), 0);
    assert_eq!(app.history(), [(ChangeKind::Tags, ChangeStatus::Discarded)]);
}

#[test]
fn the_relay_reads_where_to_connect_or_says_why_it_cannot() {
    let dir = tempfile::tempdir().unwrap();
    let paths = LibraryPaths {
        database: dir.path().join("library.sqlite3"),
        cache: dir.path().join("cache"),
    };
    let missing = relay_connection(&paths).unwrap_err();
    assert!(missing.contains("no library"), "{missing}");
    assert!(!paths.database.exists(), "the relay makes no library");

    let library = Library::open(&paths.database, &paths.cache).unwrap();
    let off = relay_connection(&paths).unwrap_err();
    assert!(off.contains("switch is off"), "{off}");

    library.set_setting(SETTING_ENABLED, "true").unwrap();
    library.set_setting(SETTING_TOKEN, "secret").unwrap();
    library.set_setting(SETTING_PORT, "9123").unwrap();
    let connection = relay_connection(&paths).unwrap();
    assert_eq!(
        (connection.port, connection.token.as_str()),
        (9123, "secret")
    );
}

fn png() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(16, 16, image::Rgb([200, 30, 90]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}
