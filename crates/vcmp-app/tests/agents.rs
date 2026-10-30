//! Changes AI agents ask for: the note in the sidebar, the history in Settings, the
//! review (accept, reject, decide later), undo, a change of labels, a playlist
//! deletion's warning, a file in use, the backups, and the switch. Then playback agents
//! control, with a recorder standing in for the engine. Files are copies in a
//! temporary directory.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, LogicalSize, Model};
use vcmp_app::agents::{
    AgentService, SETTING_ENABLED, SETTING_PLAYBACK, SETTING_PORT, SETTING_TOKEN, relay_connection,
};
use vcmp_app::library_service::{Delivery, LibraryPaths};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{
    PlayerControl, PlayerService, SETTING_OUTPUT_DEVICE, SETTING_SHUFFLE, SETTING_VOLUME,
};
use vcmp_app::{
    AgentState, AppWindow, ChangeField, ChangeKind, ChangeRow, ChangeStatus, PlayerState,
    ReviewStatus, ServerStatus, Shuffle, SleepTimer,
};
use vcmp_library::{ChangeSetId, FieldEdit, LabelEdit, Library, NewLabel, TagEdit, Track, TrackId};
use vcmp_mcp::{Context, Control, Options, Play, PlayState, Repeat};
use vcmp_playback::{
    Command, Event, NowPlaying, PlaybackSnapshot, PlaybackState, QueueEntry, QueueView, RepeatMode,
    ShuffleMode, TrackRef,
};

struct App {
    dir: tempfile::TempDir,
    paths: LibraryPaths,
    library: Rc<Library>,
    window: AppWindow,
    view: Rc<LibraryView>,
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
        view.clone(),
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
        view,
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

    /// The review's rows: those every file shares, then each file's own.
    fn review_rows(&self) -> Vec<ChangeRow> {
        let state = self.state();
        let mut rows: Vec<ChangeRow> = state.get_review_shared().iter().collect();
        for group in state.get_review_groups().iter() {
            rows.extend(group.rows.iter());
        }
        rows
    }

    /// Settings, and the dialog listing the changes by agents.
    fn open_history(&self) {
        self.open_settings();
        self.press("Show the changes by agents");
        assert!(self.state().get_history_open());
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
    app.open_history();
    app.press("Review the change asked by Claude");
    assert!(app.state().get_review_open());
    assert!(
        !app.state().get_history_open(),
        "the review takes its place"
    );
    app.press("Close; the change keeps waiting for your review");
    assert!(app.state().get_history_open(), "and gives it back");
    app.press("Close the changes by agents");
    assert!(!app.state().get_history_open());
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
    let rows = app.review_rows();
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
fn the_review_lists_shared_changes_once_then_each_file() {
    let app = start();
    let edit = |title: &str| TagEdit {
        title: FieldEdit::Set(title.into()),
        album_artist: FieldEdit::Set("Band".into()),
        ..TagEdit::default()
    };
    app.agents
        .submit_tags(
            "Claude",
            &[(app.id("Two"), edit("Dos")), (app.id("One"), edit("Uno"))],
        )
        .unwrap();
    app.press("1 change by an agent waits for your review, opens it");
    let state = app.state();
    let shared: Vec<ChangeRow> = state.get_review_shared().iter().collect();
    assert_eq!(shared.len(), 1, "the album artist, once");
    assert_eq!(shared[0].field, ChangeField::AlbumArtist);
    assert_eq!(
        (shared[0].before.as_str(), shared[0].after.as_str()),
        ("", "Band")
    );
    let groups: Vec<_> = state.get_review_groups().iter().collect();
    let titles: Vec<&str> = groups.iter().map(|group| group.title.as_str()).collect();
    assert_eq!(titles, ["One", "Two"], "each file, by path");
    for (group, after) in groups.iter().zip(["Uno", "Dos"]) {
        let rows: Vec<ChangeRow> = group.rows.iter().collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].field, ChangeField::Title);
        assert_eq!(rows[0].after, after);
        assert!(group.file.ends_with(".flac"), "{}", group.file);
    }
    assert!(
        ElementHandle::find_by_accessible_label(&app.window, "After: Uno")
            .next()
            .is_some(),
        "values are read out with their column"
    );
}

#[test]
fn a_new_cover_is_shown_in_the_review_and_written() {
    let app = start();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(32, 32, image::Rgb([200, 40, 160]))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let edit = TagEdit {
        cover: FieldEdit::Set(bytes.into_inner()),
        ..TagEdit::default()
    };
    app.agents
        .submit_tags(
            "Claude",
            &[(app.id("One"), edit.clone()), (app.id("Two"), edit)],
        )
        .unwrap();
    app.press("1 change by an agent waits for your review, opens it");
    let state = app.state();
    let shared: Vec<ChangeRow> = state.get_review_shared().iter().collect();
    assert_eq!(shared.len(), 1, "the same cover for both files, once");
    assert_eq!(shared[0].field, ChangeField::Cover);
    assert!(!shared[0].has_before_cover);
    assert!(shared[0].has_after_cover);
    assert_eq!(shared[0].after_cover.size().width, 32, "the image itself");
    assert_eq!(state.get_review_groups().row_count(), 0);
    assert!(
        ElementHandle::find_by_accessible_label(&app.window, "After: the new cover")
            .next()
            .is_some()
    );

    app.accept("Write 2 files");
    assert_eq!(state.get_review_written(), 2);
    let album = app.library.track(app.id("One")).unwrap().unwrap().album_id;
    assert!(
        album
            .and_then(|album| app.library.album(album).unwrap())
            .is_some_and(|album| album.artwork.is_some()),
        "the album has the cover now"
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

    app.open_history();
    app.press("Undo the change asked by Claude");
    let state = app.state();
    assert!(state.get_review_open());
    assert_eq!(state.get_review_kind(), ChangeKind::Undo);
    assert_eq!(state.get_review_agent(), "", "the owner asked");
    let rows = app.review_rows();
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
fn a_change_of_labels_is_reviewed_and_can_be_undone() {
    let app = start();
    let (one, two) = (app.id("One"), app.id("Two"));
    let chill = || LabelEdit {
        add: vec!["chill".into()],
        remove: Vec::new(),
    };
    app.agents
        .submit_labels(
            "Claude",
            &[(one, chill()), (two, chill())],
            &[NewLabel {
                name: "chill".into(),
                colour: "palette:teal".into(),
            }],
        )
        .unwrap();
    assert!(app.library.label_named("chill").unwrap().is_none());

    app.press("1 change by an agent waits for your review, opens it");
    let state = app.state();
    assert_eq!(state.get_review_kind(), ChangeKind::Labels);
    assert_eq!(state.get_review_new_labels(), "chill");
    let shared: Vec<ChangeRow> = state.get_review_shared().iter().collect();
    assert_eq!(shared.len(), 1, "both tracks get the label: listed once");
    assert_eq!(shared[0].field, ChangeField::Label);
    assert_eq!(
        (
            shared[0].key.as_str(),
            shared[0].before.as_str(),
            shared[0].after.as_str()
        ),
        ("chill", "", "chill")
    );
    assert_eq!(state.get_review_groups().row_count(), 0);

    app.accept("Change 2 tracks");
    assert_eq!(state.get_review_written(), 2);
    let on = |id| -> Vec<String> {
        app.library
            .track_labels_of(id)
            .unwrap()
            .into_iter()
            .map(|label| label.name)
            .collect()
    };
    assert_eq!(on(one), ["chill"]);
    assert_eq!(on(two), ["chill"]);
    assert_eq!(
        app.history(),
        [(ChangeKind::Labels, ChangeStatus::Accepted)]
    );
    app.press("Close");

    app.open_history();
    app.press("Undo the change asked by Claude");
    assert_eq!(state.get_review_kind(), ChangeKind::LabelUndo);
    let shared: Vec<ChangeRow> = state.get_review_shared().iter().collect();
    assert_eq!(
        (shared[0].before.as_str(), shared[0].after.as_str()),
        ("chill", ""),
        "the undo takes the label off"
    );
    app.accept("Undo on 2 tracks");
    assert!(on(one).is_empty());
    assert!(
        app.library.label_named("chill").unwrap().is_none(),
        "the label it made goes too"
    );
    assert_eq!(
        app.history(),
        [
            (ChangeKind::LabelUndo, ChangeStatus::Accepted),
            (ChangeKind::Labels, ChangeStatus::Undone)
        ]
    );
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
        view,
        agents,
    } = app;
    drop((agents, view, window, library));
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

/// The commands the player was sent.
#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<Command>>>);

impl PlayerControl for Recorder {
    fn send(&self, command: Command) {
        self.0.borrow_mut().push(command);
    }

    fn snapshot(&self) -> Option<PlaybackSnapshot> {
        None
    }
}

impl Recorder {
    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

/// The player in `app`'s window, with the commands it sends from now on.
fn player(app: &App) -> (Rc<PlayerService>, Recorder) {
    let sent = Recorder::default();
    let service = PlayerService::connect(
        &app.window,
        app.library.clone(),
        app.view.clone(),
        Box::new(sent.clone()),
        None,
    );
    sent.take();
    (service, sent)
}

fn track(app: &App, title: &str) -> Track {
    app.library.track(app.id(title)).unwrap().unwrap()
}

fn queued(track: &Track) -> QueueEntry {
    QueueEntry::new(TrackRef::local(track.path.to_str().unwrap()))
}

#[test]
fn agents_play_through_the_player_as_its_buttons_do() {
    let app = start();
    let (player, sent) = player(&app);
    let one = track(&app, "One");
    let two = track(&app, "Two");
    let album = one.album_id.unwrap();

    player
        .agent_play(Play {
            tracks: vec![one.clone(), two.clone()],
            context: Context::Album(album),
            start: Some(1),
            shuffle: Some(true),
        })
        .unwrap();
    let commands = sent.take();
    assert_eq!(commands[0], Command::SetShuffle(ShuffleMode::LessRepeats));
    let Command::PlayContext {
        entries,
        origin,
        start,
    } = &commands[1]
    else {
        panic!("{commands:?}");
    };
    let paths: Vec<&str> = entries
        .iter()
        .map(|entry| entry.track.id.as_ref())
        .collect();
    assert_eq!(
        paths,
        [one.path.to_str().unwrap(), two.path.to_str().unwrap()]
    );
    assert_eq!(entries[0].album.as_deref(), Some(&*album.0.to_string()));
    assert_eq!(origin.as_deref(), Some(&*format!("album:{}", album.0)));
    assert_eq!(*start, Some(1));
    assert_eq!(
        app.window.global::<PlayerState>().get_shuffle(),
        Shuffle::On
    );
    assert_eq!(
        app.library.setting(SETTING_SHUFFLE).unwrap().as_deref(),
        Some("on"),
        "remembered as the button does"
    );

    player
        .agent_play(Play {
            tracks: vec![two.clone()],
            context: Context::Agent("Claude".into()),
            start: None,
            shuffle: None,
        })
        .unwrap();
    assert!(matches!(
        &sent.take()[..],
        [Command::PlayContext { origin: Some(origin), start: None, .. }] if origin == "agent:Claude"
    ));
    player.agent_queue(&[one.clone()], true).unwrap();
    player.agent_queue(&[two.clone()], false).unwrap();
    assert_eq!(
        sent.take(),
        [
            Command::PlayNext(vec![
                queued(&one)
                    .with_artist(one.artist_id.unwrap().0.to_string())
                    .with_album(album.0.to_string())
            ]),
            Command::AddToQueue(vec![
                queued(&two)
                    .with_artist(two.artist_id.unwrap().0.to_string())
                    .with_album(album.0.to_string())
            ]),
        ]
    );
}

#[test]
fn agents_read_what_the_player_shows() {
    let app = start();
    let (player, sent) = player(&app);
    let one = track(&app, "One");
    let two = track(&app, "Two");
    let single = track(&app, "Single");
    let album = one.album_id.unwrap();

    let idle = player.agent_status().unwrap();
    assert_eq!(idle.state, PlayState::Stopped);
    assert_eq!(idle.current, None);
    assert_eq!(idle.context, None);

    player.handle_event(Event::QueueChanged(QueueView {
        current: Some(queued(&one)),
        up_next: vec![queued(&single)],
        upcoming: vec![queued(&two)],
        origin: Some(format!("album:{}", album.0)),
        shuffle: ShuffleMode::Off,
        repeat: RepeatMode::All,
    }));
    player.handle_event(Event::TrackChanged(Some(NowPlaying {
        entry: queued(&one),
        duration: Some(Duration::from_secs(90)),
    })));
    player.handle_event(Event::StateChanged(PlaybackState::Playing));
    player.handle_event(Event::Position(Duration::from_secs(12)));
    player.agent_sleep_timer(vcmp_mcp::SleepTimer::Minutes(30));
    assert_eq!(sent.take(), [Command::StopAfterCurrent(false)]);

    let status = player.agent_status().unwrap();
    assert_eq!(status.state, PlayState::Playing);
    assert_eq!(status.current, Some(one.path.clone()));
    assert_eq!(status.position, Duration::from_secs(12));
    assert_eq!(status.duration, Some(Duration::from_secs(90)));
    assert_eq!(status.repeat, Repeat::All);
    assert!(!status.shuffle);
    assert_eq!(status.context, Some(Context::Album(album)));
    assert_eq!(status.up_next, [single.path.clone()]);
    assert_eq!(status.upcoming, [two.path.clone()]);
    assert_eq!(status.sleep_timer, vcmp_mcp::SleepTimer::Minutes(30));
    assert_eq!(
        app.window.global::<PlayerState>().get_sleep_timer(),
        SleepTimer::Timed,
        "the bar shows the timer"
    );

    player.agent_sleep_timer(vcmp_mcp::SleepTimer::EndOfTrack);
    assert_eq!(sent.take(), [Command::StopAfterCurrent(true)]);
    assert_eq!(
        player.agent_status().unwrap().sleep_timer,
        vcmp_mcp::SleepTimer::EndOfTrack
    );
}

#[test]
fn agents_press_buttons_within_the_queue_and_set_options() {
    let app = start();
    let (player, sent) = player(&app);
    let one = track(&app, "One");
    let two = track(&app, "Two");

    let seek = Control::Seek(Duration::from_secs(5));
    assert_eq!(
        player.agent_control(seek).unwrap_err(),
        "nothing is playing"
    );
    assert!(player.agent_control(Control::SkipTo(0)).is_err());
    player.handle_event(Event::QueueChanged(QueueView {
        current: Some(queued(&one)),
        up_next: vec![queued(&two)],
        upcoming: Vec::new(),
        origin: None,
        shuffle: ShuffleMode::Off,
        repeat: RepeatMode::Off,
    }));
    player.handle_event(Event::TrackChanged(Some(NowPlaying {
        entry: queued(&one),
        duration: Some(Duration::from_secs(60)),
    })));
    let long = player
        .agent_control(Control::Seek(Duration::from_secs(61)))
        .unwrap_err();
    assert!(long.contains("1:00"), "{long}");
    sent.take();

    for control in [
        Control::TogglePlayPause,
        Control::Next,
        seek,
        Control::SkipTo(0),
        Control::RemoveFromUpNext(0),
        Control::ClearUpNext,
        Control::Raise,
    ] {
        player.agent_control(control).unwrap();
    }
    assert!(
        player
            .agent_control(Control::MoveInUpNext { from: 0, to: 1 })
            .is_err()
    );
    assert_eq!(
        sent.take(),
        [
            Command::TogglePlayPause,
            Command::Next,
            Command::Seek(Duration::from_secs(5)),
            Command::SkipTo(0),
            Command::RemoveFromUpNext(0),
            Command::ClearUpNext,
        ]
    );

    player.agent_options(Options {
        shuffle: Some(true),
        shuffle_kind: Some(vcmp_mcp::ShuffleKind::TrueRandom),
        repeat: Some(Repeat::One),
        volume: Some(0.3),
    });
    assert_eq!(
        sent.take(),
        [
            Command::SetShuffle(ShuffleMode::True),
            Command::SetRepeat(RepeatMode::One),
            Command::SetVolume(0.3),
        ]
    );
    assert_eq!(
        app.library.setting(SETTING_VOLUME).unwrap().as_deref(),
        Some("0.30")
    );
    let status = player.agent_status().unwrap();
    assert!(status.shuffle);
    assert_eq!(status.shuffle_kind, vcmp_mcp::ShuffleKind::TrueRandom);
    assert_eq!(status.volume, 0.3);
}

#[test]
fn agents_choose_the_output_device_by_name() {
    let app = start();
    let (player, sent) = player(&app);
    let listed = player.agent_output_devices(vec!["Speakers".into(), "USB DAC".into()]);
    assert_eq!(listed.devices, ["Speakers", "USB DAC"]);
    assert_eq!(listed.chosen, None);

    player.agent_set_output_device(Some("usb dac")).unwrap();
    assert_eq!(
        sent.take(),
        [Command::SetOutputDevice(Some("USB DAC".into()))]
    );
    assert_eq!(
        app.library
            .setting(SETTING_OUTPUT_DEVICE)
            .unwrap()
            .as_deref(),
        Some("USB DAC")
    );
    assert_eq!(
        app.window.global::<PlayerState>().get_output_device(),
        1,
        "Settings shows the choice"
    );
    let unknown = player.agent_set_output_device(Some("Radio")).unwrap_err();
    assert!(unknown.contains("list_output_devices"), "{unknown}");
    assert!(sent.take().is_empty());

    player.agent_set_output_device(None).unwrap();
    assert_eq!(sent.take(), [Command::SetOutputDevice(None)]);
    assert_eq!(player.agent_status().unwrap().output.chosen, None);
}

#[test]
fn playback_waits_for_its_own_switch() {
    let app = start();
    let (player, _) = player(&app);
    app.open_settings();
    assert!(
        app.try_find(AccessibleRole::Switch, "Let AI agents control playback")
            .is_none(),
        "shown only while agents may connect"
    );
    let refused = player.agents_may_play().unwrap_err();
    assert!(
        refused.contains("Let AI agents control playback"),
        "{refused}"
    );

    app.find(AccessibleRole::Switch, "Let AI agents use your library")
        .invoke_accessible_default_action();
    let switch = app.find(AccessibleRole::Switch, "Let AI agents control playback");
    switch.invoke_accessible_default_action();
    assert!(app.state().get_playback_enabled());
    assert_eq!(
        app.library.setting(SETTING_PLAYBACK).unwrap().as_deref(),
        Some("true")
    );
    player.agents_may_play().unwrap();

    switch.invoke_accessible_default_action();
    assert!(player.agents_may_play().is_err(), "off again at once");
}
