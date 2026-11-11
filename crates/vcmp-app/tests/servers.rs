mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use slint::{ComponentHandle, LogicalSize, Model};
use vcmp_app::library_service::{Delivery, LibraryPaths};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::servers::ServerService;
use vcmp_app::{AppWindow, MusicServerStatus, MusicServers, Page};
use vcmp_client::{Connection, MemoryTokens, TokenStore};
use vcmp_library::Library;
use vcmp_playback::{Command, SourceKind};

#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<Command>>>);

impl PlayerControl for Recorder {
    fn send(&self, command: Command) {
        self.0.borrow_mut().push(command);
    }
}

struct App {
    server: Option<vcmp_server::Server>,
    user: String,
    window: AppWindow,
    library: Rc<Library>,
    servers: Rc<ServerService>,
    tokens: MemoryTokens,
    sent: Recorder,
    _player: Rc<PlayerService>,
    _dir: tempfile::TempDir,
}

fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(dir.path()).unwrap();
    let music = base.join("server-music");
    add(
        &music.join("Afar"),
        "1.flac",
        Song::new("First", "Remote", "Afar"),
    );
    add(
        &music.join("Afar"),
        "2.flac",
        Song::new("Second", "Remote", "Afar"),
    );

    let mut config = vcmp_server::Config::new(base.join("server-data"));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.music = vec![music];
    config.rescan_interval = None;
    let (_, tokens) = vcmp_server::tokens(&config).unwrap();
    let server = vcmp_server::Server::start(config).unwrap();
    let probe = Connection::new(&format!("http://{}", server.address()), &tokens[0].token).unwrap();
    let start = Instant::now();
    while probe
        .scan_status()
        .map(|status| status.running || status.last.is_none())
        .unwrap_or(true)
    {
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(50));
    }

    let paths = LibraryPaths {
        database: base.join("app/library.sqlite3"),
        cache: base.join("app/cache"),
    };
    let library = Rc::new(Library::open(&paths.database, &paths.cache).unwrap());
    let window = vcmp_app::create_window().unwrap();
    let view = LibraryView::connect(&window, library.clone()).unwrap();
    let sent = Recorder::default();
    let player = PlayerService::connect(
        &window,
        library.clone(),
        view.clone(),
        Box::new(sent.clone()),
        None,
    );
    let memory = MemoryTokens::default();
    let servers = ServerService::connect(
        &window,
        library.clone(),
        paths,
        view,
        Arc::new(memory.clone()),
        None,
        Delivery::Manual,
    );
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 2400.0));
    App {
        user: tokens[0].token.clone(),
        server: Some(server),
        window,
        library,
        servers,
        tokens: memory,
        sent,
        _player: player,
        _dir: dir,
    }
}

impl App {
    fn find(&self, role: AccessibleRole, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(role))
            .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
    }

    fn open(&self, tab: &str) {
        self.find(AccessibleRole::Tab, tab)
            .invoke_accessible_default_action();
        mock_elapsed_time(Duration::ZERO);
    }

    fn state(&self) -> MusicServers<'_> {
        self.window.global::<MusicServers>()
    }

    fn address(&self) -> String {
        format!("http://{}", self.server.as_ref().unwrap().address())
    }

    fn connect(&self, address: &str, token: &str) {
        self.open("Settings");
        self.find(AccessibleRole::TextInput, "Server address")
            .set_accessible_value(address);
        self.find(AccessibleRole::TextInput, "Token")
            .set_accessible_value(token);
        self.find(AccessibleRole::Button, "Connect")
            .invoke_accessible_default_action();
    }

    fn idle(&self) {
        assert!(self.servers.wait_until_idle(Duration::from_secs(30)));
    }
}

#[test]
fn a_connected_server_brings_its_albums_marked_with_its_name() {
    let app = start();
    app.connect(&app.address(), &app.user);
    app.idle();

    let rows: Vec<_> = app.state().get_servers().iter().collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "VCMP server");
    assert_eq!(rows[0].status, MusicServerStatus::Online);
    assert!(!rows[0].admin);
    assert_eq!(app.state().get_address(), "");
    assert_eq!(
        app.state().get_token(),
        "",
        "the field is emptied once kept"
    );
    let server = app.library.servers().unwrap().remove(0);
    assert_eq!(
        app.tokens.load(&server.server_id).unwrap().as_deref(),
        Some(app.user.as_str()),
        "the token is in the keyring, not the database"
    );

    app.open("Albums");
    assert_eq!(app.window.get_current_page(), Page::Albums);
    app.find(AccessibleRole::Button, "Afar, Remote · VCMP server")
        .invoke_accessible_default_action();
    app.sent.0.borrow_mut().clear();
    app.find(AccessibleRole::Button, "Play Afar")
        .invoke_accessible_default_action();
    let played = app
        .sent
        .0
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PlayContext { entries, .. } => Some(entries.clone()),
            _ => None,
        });
    let entries = played.expect("the album played");
    assert_eq!(entries.len(), 2);
    assert!(
        entries
            .iter()
            .all(|entry| entry.track.source == SourceKind::Server
                && entry.track.id.starts_with("vcmp-server:"))
    );
}

#[test]
fn a_server_that_goes_away_dims_its_albums_and_removing_it_keeps_nothing_on_the_pages() {
    let mut app = start();
    app.connect(&app.address(), &app.user);
    app.idle();
    let id = app.library.servers().unwrap()[0].id;

    drop(app.server.take());
    app.servers.start_sync(id);
    app.idle();
    assert_eq!(
        app.state().get_servers().row_data(0).unwrap().status,
        MusicServerStatus::Away
    );
    app.open("Albums");
    app.find(
        AccessibleRole::Button,
        "Afar, Remote · VCMP server, its server can't be reached right now",
    );

    app.servers.remove(id);
    assert_eq!(app.state().get_servers().row_count(), 0);
    assert!(app.library.albums().unwrap().is_empty());
    assert!(app.library.servers().unwrap().is_empty());
    assert!(
        app.tokens.0.lock().unwrap().is_empty(),
        "the token is deleted"
    );
}

#[test]
fn a_wrong_token_or_plain_http_to_the_internet_is_caught_before_anything_is_kept() {
    let app = start();
    app.connect(&app.address(), "not-the-token-at-all");
    assert!(app.state().get_adding());
    app.idle();
    assert_eq!(app.state().get_error(), vcmp_app::MusicServerError::Token);
    assert_eq!(app.state().get_servers().row_count(), 0);
    assert!(app.library.servers().unwrap().is_empty());

    app.state().set_error(vcmp_app::MusicServerError::None);
    app.connect("http://music.example.com", &app.user);
    assert!(
        app.state().get_insecure(),
        "plain http to a public host warns"
    );
    assert!(!app.state().get_adding(), "and waits for Connect anyway");
    app.find(AccessibleRole::Button, "Cancel")
        .invoke_accessible_default_action();
    assert!(!app.state().get_insecure());
}
