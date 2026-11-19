//! Last.fm in the window: the Settings row only in builds with an API key,
//! connecting an account, and plays reaching the queue in the library database.
//! Recorders stand in for the scrobbler thread, the browser and the engine.

mod common;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::ComponentHandle;
use tempfile::TempDir;
use vcmp_app::lastfm_service::{
    self, LastfmService, LastfmWorker, QueueStore, SETTING_LOVE, SETTING_USER,
};
use vcmp_app::library_service::LibraryPaths;
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::{AppWindow, LastfmImport, LastfmProblem, LastfmState, LastfmStatus};
use vcmp_integrations::lastfm::{
    self, Client, ConnectError, Credentials, Error, ImportError, Imported, MemorySessions, Report,
    Scrobbler, Timing, Transport,
};
use vcmp_library::{Library, TrackId};
use vcmp_playback::{Command, Event, NowPlaying, PlaybackState, QueueEntry, TrackRef};

/// What was asked of the scrobbler thread.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    Resume(String),
    Connect,
    Cancel,
    Disconnect,
    NowPlaying(lastfm::Track),
    Flush,
    FlushLoves,
    Import,
}

#[derive(Clone, Default)]
struct Worker(Rc<RefCell<Vec<Asked>>>);

impl Worker {
    fn take(&self) -> Vec<Asked> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

impl LastfmWorker for Worker {
    fn resume(&self, user: &str) {
        self.0.borrow_mut().push(Asked::Resume(user.into()));
    }

    fn connect(&self) {
        self.0.borrow_mut().push(Asked::Connect);
    }

    fn cancel(&self) {
        self.0.borrow_mut().push(Asked::Cancel);
    }

    fn disconnect(&self) {
        self.0.borrow_mut().push(Asked::Disconnect);
    }

    fn now_playing(&self, track: lastfm::Track) {
        self.0.borrow_mut().push(Asked::NowPlaying(track));
    }

    fn flush(&self) {
        self.0.borrow_mut().push(Asked::Flush);
    }

    fn flush_loves(&self) {
        self.0.borrow_mut().push(Asked::FlushLoves);
    }

    fn import_loved(&self) {
        self.0.borrow_mut().push(Asked::Import);
    }
}

/// The engine, doing nothing.
struct Engine;

impl PlayerControl for Engine {
    fn send(&self, _: Command) {}
}

struct App {
    _dir: TempDir,
    window: AppWindow,
    library: Rc<Library>,
    paths: LibraryPaths,
    music: std::path::PathBuf,
    worker: Worker,
    opened: Rc<RefCell<Vec<String>>>,
    browser_works: Rc<RefCell<bool>>,
    service: Option<Rc<LastfmService>>,
}

/// A window with one tagged track, `prepare` run on the library first; the Last.fm
/// service connects only when `with_key`, as in a build with VCMP's API key.
fn start(with_key: bool, prepare: impl FnOnce(&Library)) -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(
        &music,
        "1.flac",
        Song {
            track: 4,
            ..Song::new("Opening", "Band", "Record")
        },
    );
    let paths = LibraryPaths {
        database: dir.path().join("library.sqlite3"),
        cache: dir.path().join("cache"),
    };
    let mut library = Library::open(&paths.database, &paths.cache).unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    prepare(&library);
    let library = Rc::new(library);

    let window = vcmp_app::create_window().unwrap();
    // Shown, so the Settings page is laid out and its controls exist. Tall enough for
    // the Last.fm row: the finder skips what is scrolled out of view.
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1100.0, 1600.0));
    let worker = Worker::default();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let browser_works = Rc::new(RefCell::new(true));
    let service = with_key.then(|| {
        let (opened, works) = (opened.clone(), browser_works.clone());
        LastfmService::connect(
            &window,
            library.clone(),
            Box::new(worker.clone()),
            Box::new(move |url| {
                opened.borrow_mut().push(url.to_string());
                if *works.borrow() {
                    Ok(())
                } else {
                    Err("no browser found".into())
                }
            }),
        )
    });
    App {
        _dir: dir,
        window,
        library,
        paths,
        music,
        worker,
        opened,
        browser_works,
        service,
    }
}

impl App {
    fn state(&self) -> LastfmState<'_> {
        self.window.global::<LastfmState>()
    }

    fn service(&self) -> &LastfmService {
        self.service.as_ref().expect("the Last.fm service")
    }

    fn buttons(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .filter(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .collect()
    }

    fn press(&self, label: &str) {
        self.buttons(label)
            .first()
            .unwrap_or_else(|| panic!("no button labelled {label:?}"))
            .invoke_accessible_default_action();
    }

    /// Settings, on its Online services section.
    fn open_settings(&self) {
        for tab in ["Settings", "Online services"] {
            ElementHandle::find_by_accessible_label(&self.window, tab)
                .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
                .unwrap_or_else(|| panic!("the {tab} tab"))
                .invoke_accessible_default_action();
        }
    }

    fn user_setting(&self) -> Option<String> {
        self.library.setting(SETTING_USER).unwrap()
    }

    /// Connects "listener", as allowing VCMP on last.fm would.
    fn connect_listener(&self) {
        self.open_settings();
        self.press("Connect to Last.fm");
        self.service().report(Report::Authorize(
            "https://www.last.fm/api/auth/?token=t".into(),
        ));
        self.service().report(Report::Connected {
            user: "listener".into(),
            saved: Ok(()),
        });
        self.worker.take();
    }
}

#[test]
fn the_row_is_hidden_without_an_api_key() {
    let app = start(false, |_| {});
    assert!(lastfm_service::start(&app.window, app.library.clone(), &app.paths, None).is_none());
    assert!(!app.state().get_available());
    app.open_settings();
    assert!(app.buttons("Connect to Last.fm").is_empty());
    assert!(Credentials::new(None, Some("secret")).is_none());
}

#[test]
fn the_row_shows_with_an_api_key() {
    let app = start(true, |_| {});
    assert!(app.state().get_available());
    app.open_settings();
    assert_eq!(app.buttons("Connect to Last.fm").len(), 1);
    assert_eq!(app.state().get_status(), LastfmStatus::Disconnected);
    assert!(app.worker.take().is_empty(), "nothing to resume");
}

#[test]
fn connecting_opens_last_fm_and_waits_for_the_user() {
    let app = start(true, |_| {});
    app.open_settings();
    app.press("Connect to Last.fm");
    assert_eq!(app.worker.take(), [Asked::Connect]);
    assert_eq!(app.state().get_status(), LastfmStatus::Waiting);
    assert_eq!(app.buttons("Stop connecting to Last.fm").len(), 1);

    let page = "https://www.last.fm/api/auth/?api_key=k&token=t";
    app.service().report(Report::Authorize(page.into()));
    assert_eq!(*app.opened.borrow(), [page]);
    app.press("Open the Last.fm page again");
    assert_eq!(app.opened.borrow().len(), 2);

    app.service().report(Report::Connected {
        user: "listener".into(),
        saved: Ok(()),
    });
    assert_eq!(app.state().get_status(), LastfmStatus::Connected);
    assert_eq!(app.state().get_user(), "listener");
    assert_eq!(app.user_setting().as_deref(), Some("listener"));
    assert_eq!(app.buttons("Disconnect from Last.fm").len(), 1);
}

#[test]
fn cancelling_stops_waiting_and_wins_over_a_late_answer() {
    let app = start(true, |_| {});
    app.open_settings();
    app.press("Connect to Last.fm");
    app.press("Stop connecting to Last.fm");
    assert_eq!(app.worker.take(), [Asked::Connect, Asked::Cancel]);
    assert_eq!(app.state().get_status(), LastfmStatus::Disconnected);

    // The page was already on its way, and the user allowed VCMP anyway.
    app.service()
        .report(Report::Authorize("https://www.last.fm/api/auth/".into()));
    assert!(app.opened.borrow().is_empty());
    app.service().report(Report::Connected {
        user: "listener".into(),
        saved: Ok(()),
    });
    assert_eq!(app.worker.take(), [Asked::Disconnect]);
    assert_eq!(app.state().get_status(), LastfmStatus::Disconnected);
    assert_eq!(app.user_setting(), None);
}

#[test]
fn connecting_can_fail_and_be_tried_again() {
    let app = start(true, |_| {});
    app.open_settings();
    app.press("Connect to Last.fm");
    app.service()
        .report(Report::ConnectFailed(ConnectError::Failed(Error::Network(
            "connection refused".into(),
        ))));
    assert_eq!(app.state().get_status(), LastfmStatus::Disconnected);
    assert_eq!(app.state().get_problem(), LastfmProblem::ConnectFailed);
    assert_eq!(app.state().get_problem_detail(), "connection refused");

    app.press("Connect to Last.fm");
    assert_eq!(app.state().get_problem(), LastfmProblem::None);
    app.service()
        .report(Report::ConnectFailed(ConnectError::TimedOut));
    assert_eq!(app.state().get_problem(), LastfmProblem::TimedOut);
    assert_eq!(app.worker.take(), [Asked::Connect, Asked::Connect]);
}

#[test]
fn a_browser_that_does_not_open_is_explained() {
    let app = start(true, |_| {});
    *app.browser_works.borrow_mut() = false;
    app.open_settings();
    app.press("Connect to Last.fm");
    app.service()
        .report(Report::Authorize("https://www.last.fm/api/auth/".into()));
    assert_eq!(app.state().get_status(), LastfmStatus::Waiting);
    assert_eq!(app.state().get_problem(), LastfmProblem::Browser);
    assert_eq!(app.state().get_problem_detail(), "no browser found");

    *app.browser_works.borrow_mut() = true;
    app.press("Open the Last.fm page again");
    assert_eq!(app.state().get_problem(), LastfmProblem::None);
}

#[test]
fn a_saved_account_resumes_at_start() {
    let app = start(true, |library| {
        library.set_setting(SETTING_USER, "listener").unwrap();
    });
    assert_eq!(app.worker.take(), [Asked::Resume("listener".into())]);
    assert_eq!(app.state().get_status(), LastfmStatus::Connected);
    assert_eq!(app.state().get_user(), "listener");

    // The keyring lost the key: connect again.
    app.service().report(Report::KeyMissing);
    assert_eq!(app.state().get_status(), LastfmStatus::Disconnected);
    assert_eq!(app.state().get_problem(), LastfmProblem::KeyMissing);
    assert_eq!(app.user_setting().as_deref(), Some(""));
}

#[test]
fn an_unreadable_keyring_keeps_the_account_for_the_next_start() {
    let app = start(true, |library| {
        library.set_setting(SETTING_USER, "listener").unwrap();
    });
    app.service()
        .report(Report::KeyUnreadable("the keyring is locked".into()));
    assert_eq!(app.state().get_status(), LastfmStatus::Disconnected);
    assert_eq!(app.state().get_problem(), LastfmProblem::Keyring);
    assert_eq!(app.user_setting().as_deref(), Some("listener"));
}

#[test]
fn disconnecting_forgets_the_account() {
    let app = start(true, |_| {});
    app.connect_listener();
    app.press("Disconnect from Last.fm");
    assert_eq!(app.worker.take(), [Asked::Disconnect]);
    assert_eq!(app.state().get_status(), LastfmStatus::Disconnected);
    assert_eq!(app.state().get_user(), "");
    assert_eq!(app.user_setting().as_deref(), Some(""));
}

#[test]
fn a_revoked_key_asks_to_connect_again() {
    let app = start(true, |_| {});
    app.connect_listener();
    app.service().report(Report::Rejected);
    assert_eq!(app.state().get_status(), LastfmStatus::Disconnected);
    assert_eq!(app.state().get_problem(), LastfmProblem::Rejected);
    assert_eq!(
        app.state().get_user(),
        "listener",
        "named in the explanation"
    );
    assert_eq!(app.user_setting().as_deref(), Some(""));
}

/// The player, connected to `app`'s Last.fm service.
fn player(app: &App) -> Rc<PlayerService> {
    let view = LibraryView::connect(&app.window, app.library.clone()).unwrap();
    let service = PlayerService::connect(
        &app.window,
        app.library.clone(),
        view,
        Box::new(Engine),
        None,
    );
    service.set_scrobbler(Box::new(app.service.clone().unwrap()));
    service
}

fn play(app: &App, player: &PlayerService, length: Duration) {
    let path = app.music.join("1.flac").to_str().unwrap().to_string();
    player.handle_event(Event::TrackChanged(Some(NowPlaying {
        entry: QueueEntry::new(TrackRef::local(path)),
        duration: Some(length),
        origin: None,
    })));
    player.handle_event(Event::StateChanged(PlaybackState::Playing));
}

/// Positions every half second, as the engine reports them while playing.
fn listen(player: &PlayerService, from: f64, to: f64) {
    let mut at = from;
    while at < to {
        at = (at + 0.5).min(to);
        player.handle_event(Event::Position(Duration::from_secs_f64(at)));
    }
}

fn opening(length: Duration) -> lastfm::Track {
    lastfm::Track {
        artist: "Band".into(),
        title: "Opening".into(),
        album: Some("Record".into()),
        album_artist: None,
        track_number: Some(4),
        duration: Some(length),
    }
}

#[test]
fn a_play_that_counts_waits_in_the_queue() {
    let app = start(true, |_| {});
    app.connect_listener();
    let player = player(&app);
    let length = Duration::from_secs(200);
    play(&app, &player, length);
    assert_eq!(app.worker.take(), [Asked::NowPlaying(opening(length))]);

    let before = SystemTime::now();
    listen(&player, 0.0, 99.5);
    assert_eq!(app.library.queued_scrobble_count().unwrap(), 0);
    listen(&player, 99.5, 120.0);
    assert_eq!(app.worker.take(), [Asked::Flush]);
    let queued = app.library.queued_scrobbles(50).unwrap();
    assert_eq!(queued.len(), 1, "counted once");
    let scrobble = &queued[0].scrobble;
    assert_eq!(
        (scrobble.artist.as_str(), scrobble.title.as_str()),
        ("Band", "Opening")
    );
    assert_eq!(scrobble.album.as_deref(), Some("Record"));
    assert_eq!(scrobble.duration, Some(length));
    // Stored to the millisecond; the play began just before the first position.
    assert!(scrobble.played_at <= SystemTime::now());
    assert!(scrobble.played_at + Duration::from_secs(1) >= before);
    assert_eq!(app.state().get_unsent(), 1);
}

#[test]
fn pauses_and_seeks_do_not_count_and_resuming_says_now_playing_again() {
    let app = start(true, |_| {});
    app.connect_listener();
    let player = player(&app);
    play(&app, &player, Duration::from_secs(200));
    listen(&player, 0.0, 50.0);
    player.handle_event(Event::StateChanged(PlaybackState::Paused));
    // A seek ahead arrives as a jump in the positions.
    player.handle_event(Event::Position(Duration::from_secs(150)));
    player.handle_event(Event::StateChanged(PlaybackState::Playing));
    listen(&player, 150.0, 199.0);
    assert_eq!(
        app.library.queued_scrobble_count().unwrap(),
        0,
        "99 s heard"
    );
    let asked = app.worker.take();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(
        asked
            .iter()
            .all(|asked| matches!(asked, Asked::NowPlaying(_)))
    );

    // The repeat of a track is a new play.
    play(&app, &player, Duration::from_secs(200));
    listen(&player, 0.0, 100.0);
    assert_eq!(app.library.queued_scrobble_count().unwrap(), 1);
}

#[test]
fn nothing_is_kept_without_an_account() {
    let app = start(true, |_| {});
    let player = player(&app);
    play(&app, &player, Duration::from_secs(60));
    listen(&player, 0.0, 60.0);
    assert!(app.worker.take().is_empty());
    assert_eq!(app.library.queued_scrobble_count().unwrap(), 0);
}

type Answer = Result<(u16, String), String>;

/// Answers Last.fm requests from a script, and counts the plays in each batch.
struct Script {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    batches: Arc<Mutex<Vec<usize>>>,
}

impl Transport for Script {
    fn post(&mut self, _: &str, form: &[(String, String)]) -> Result<(u16, String), String> {
        let plays = form
            .iter()
            .filter(|(name, _)| name.starts_with("timestamp["))
            .count();
        self.batches.lock().unwrap().push(plays);
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("an answer for each request")
    }

    fn get(&mut self, _: &str, _: &[(String, String)]) -> Result<(u16, String), String> {
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("an answer for each request")
    }
}

fn accepted(count: usize) -> Result<(u16, String), String> {
    Ok((
        200,
        format!(r#"{{"scrobbles":{{"@attr":{{"accepted":{count},"ignored":0}}}}}}"#),
    ))
}

#[test]
fn the_queue_survives_a_restart_and_drains_in_batches() {
    let dir = tempfile::tempdir().unwrap();
    let open = || {
        Library::open(
            &dir.path().join("library.sqlite3"),
            &dir.path().join("cache"),
        )
        .unwrap()
    };
    let library = open();
    for index in 0..120 {
        library
            .queue_scrobble(&vcmp_library::Scrobble {
                artist: "Band".into(),
                title: format!("Track {index}"),
                album: None,
                album_artist: None,
                track_number: None,
                duration: Some(Duration::from_secs(200)),
                played_at: UNIX_EPOCH + Duration::from_secs(1_700_000_000 + 200 * index),
            })
            .unwrap();
    }
    let sessions = MemorySessions::default();
    sessions
        .0
        .lock()
        .unwrap()
        .insert("listener".into(), "sk".into());
    let run = |answers: Vec<Result<(u16, String), String>>, expected: usize| {
        let batches = Arc::new(Mutex::new(Vec::new()));
        let (sender, reports) = mpsc::channel();
        let scrobbler = Scrobbler::start(
            Client::new(
                Script {
                    answers: Arc::new(Mutex::new(answers.into())),
                    batches: batches.clone(),
                },
                Credentials::new(Some("key"), Some("secret")).unwrap(),
            ),
            sessions.clone(),
            QueueStore(open()),
            Timing {
                backoff: Duration::from_secs(60),
                ..Timing::default()
            },
            move |report| {
                let _ = sender.send(report);
            },
        );
        scrobbler.resume("listener");
        let reports: Vec<Report> = (0..expected)
            .map(|_| reports.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        while batches.lock().unwrap().is_empty() {
            std::thread::sleep(Duration::from_millis(2));
        }
        let sent = batches.lock().unwrap().clone();
        (reports, sent)
    };

    // Offline: the first batch fails, and VCMP closes before the next try.
    let (reports, sent) = run(vec![Err("offline".into())], 0);
    assert!(reports.is_empty());
    assert_eq!(sent, [50]);
    assert_eq!(open().queued_scrobble_count().unwrap(), 120);

    // Started again, online.
    let (reports, sent) = run(vec![accepted(50), accepted(50), accepted(20)], 3);
    assert_eq!(
        reports,
        [Report::Sent(50), Report::Sent(50), Report::Sent(20)]
    );
    assert_eq!(sent, [50, 50, 20]);
    assert_eq!(open().queued_scrobble_count().unwrap(), 0);
}

/// The library's hearts, heard by `app`'s Last.fm service as in the app.
fn hearts(app: &App) -> Rc<LibraryView> {
    let view = LibraryView::connect(&app.window, app.library.clone()).unwrap();
    app.service.as_ref().unwrap().attach(view.hearts());
    view
}

impl App {
    fn opening_id(&self) -> TrackId {
        self.library
            .track_id_for_path(&self.music.join("1.flac"))
            .unwrap()
            .unwrap()
    }

    fn love_switch(&self) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.window, "Love on Last.fm")
            .filter(|element| element.accessible_role() == Some(AccessibleRole::Switch))
            .collect()
    }

    fn waiting_loves(&self) -> Vec<vcmp_library::Love> {
        self.library
            .queued_loves(50)
            .unwrap()
            .into_iter()
            .map(|queued| queued.love)
            .collect()
    }
}

fn love(love: bool) -> vcmp_library::Love {
    vcmp_library::Love {
        artist: "Band".into(),
        title: "Opening".into(),
        love,
    }
}

#[test]
fn the_love_and_import_rows_show_while_connected() {
    let app = start(true, |_| {});
    app.open_settings();
    assert!(app.love_switch().is_empty());
    assert!(app.buttons("Import loved tracks from Last.fm").is_empty());
    app.connect_listener();
    assert_eq!(app.love_switch().len(), 1);
    assert!(!app.state().get_love(), "off by default");
    assert_eq!(app.buttons("Import loved tracks from Last.fm").len(), 1);
}

#[test]
fn hearts_are_loved_only_while_the_switch_is_on() {
    let app = start(true, |_| {});
    let view = hearts(&app);
    let id = app.opening_id();
    view.hearts().set(&[id], true);
    assert!(app.waiting_loves().is_empty(), "no account");

    app.connect_listener();
    view.hearts().set(&[id], false);
    assert!(app.waiting_loves().is_empty(), "the switch is off");

    app.love_switch()[0].invoke_accessible_default_action();
    assert!(app.state().get_love());
    assert_eq!(
        app.library.setting(SETTING_LOVE).unwrap().as_deref(),
        Some("1")
    );
    view.hearts().set(&[id], true);
    assert_eq!(app.waiting_loves(), [love(true)]);
    assert_eq!(app.worker.take(), [Asked::FlushLoves]);
    assert_eq!(app.state().get_unsent_loves(), 1);

    // Liked already: nothing changes, so nothing more is sent.
    view.hearts().set(&[id], true);
    assert!(app.worker.take().is_empty());

    // Taken away before the love went out: only the unlove waits.
    view.hearts().set(&[id], false);
    assert_eq!(app.waiting_loves(), [love(false)]);

    // Switched off, the unlove waiting still goes out, but no new heart is queued.
    app.love_switch()[0].invoke_accessible_default_action();
    assert!(!app.state().get_love());
    view.hearts().set(&[id], true);
    assert_eq!(app.waiting_loves(), [love(false)]);
    assert_eq!(app.worker.take(), [Asked::FlushLoves]);
}

#[test]
fn the_switch_is_remembered() {
    let app = start(true, |library| {
        library.set_setting(SETTING_LOVE, "1").unwrap();
    });
    assert!(app.state().get_love());
}

#[test]
fn disconnecting_drops_the_loves_waiting() {
    let app = start(true, |_| {});
    let view = hearts(&app);
    app.connect_listener();
    app.state().invoke_set_love(true);
    view.hearts().set(&[app.opening_id()], true);
    assert_eq!(app.state().get_unsent_loves(), 1);
    app.press("Disconnect from Last.fm");
    assert_eq!(app.state().get_unsent_loves(), 0);
}

#[test]
fn an_import_shows_its_pages_then_what_matched() {
    let app = start(true, |_| {});
    let _view = hearts(&app);
    app.connect_listener();
    app.open_settings();
    app.press("Import loved tracks from Last.fm");
    assert_eq!(app.worker.take(), [Asked::Import]);
    assert!(app.state().get_importing());
    assert!(
        app.buttons("Import loved tracks from Last.fm")[0].accessible_enabled() == Some(false),
        "one import at a time"
    );
    app.service()
        .report(Report::ImportProgress { page: 2, pages: 5 });
    assert_eq!(app.state().get_import_page(), 2);
    assert_eq!(app.state().get_import_pages(), 5);

    // The scrobbler thread liked the track in the database.
    let id = app.opening_id();
    app.library
        .like_at(&[(id, UNIX_EPOCH + Duration::from_secs(1_700_000_000))])
        .unwrap();
    assert!(!vcmp_app::favourites::liked(id));
    app.service().report(Report::Imported(Imported {
        matched: 1,
        unmatched: 3,
    }));
    assert!(!app.state().get_importing());
    assert_eq!(app.state().get_import_result(), LastfmImport::Done);
    assert_eq!(app.state().get_import_matched(), 1);
    assert_eq!(app.state().get_import_unmatched(), 3);
    assert!(vcmp_app::favourites::liked(id), "the hearts show it");
    assert!(app.worker.take().is_empty(), "imported likes are not loved");
}

#[test]
fn a_failed_import_is_explained_and_can_run_again() {
    let app = start(true, |_| {});
    app.connect_listener();
    app.open_settings();
    app.press("Import loved tracks from Last.fm");
    app.service()
        .report(Report::ImportFailed(ImportError::Lastfm(Error::Network(
            "connection refused".into(),
        ))));
    assert!(!app.state().get_importing());
    assert_eq!(app.state().get_import_result(), LastfmImport::Failed);
    assert_eq!(app.state().get_import_detail(), "connection refused");
    app.press("Import loved tracks from Last.fm");
    assert_eq!(app.worker.take(), [Asked::Import, Asked::Import]);
    assert_eq!(app.state().get_import_result(), LastfmImport::None);
}

fn loved_page(page: u32, pages: u32, tracks: &[(&str, &str)]) -> Result<(u16, String), String> {
    let tracks: Vec<String> = tracks
        .iter()
        .map(|(artist, title)| {
            format!(
                r#"{{"name":"{title}","artist":{{"name":"{artist}"}},"date":{{"uts":"1600000000"}}}}"#
            )
        })
        .collect();
    Ok((
        200,
        format!(
            r#"{{"lovedtracks":{{"track":[{}],"@attr":{{"page":"{page}","totalPages":"{pages}"}}}}}}"#,
            tracks.join(",")
        ),
    ))
}

#[test]
fn an_import_likes_the_library_tracks_that_match() {
    let app = start(true, |_| {});
    // A second copy of the song, and one that is not loved.
    add(
        &app.music,
        "2.flac",
        Song {
            track: 1,
            ..Song::new("Opening", "Band", "Compilation")
        },
    );
    add(&app.music, "3.flac", Song::new("Closing", "Band", "Record"));
    let mut library = Library::open(&app.paths.database, &app.paths.cache).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    let sessions = MemorySessions::default();
    sessions
        .0
        .lock()
        .unwrap()
        .insert("listener".into(), "sk".into());
    let (sender, reports) = mpsc::channel();
    let scrobbler = Scrobbler::start(
        Client::new(
            Script {
                answers: Arc::new(Mutex::new(
                    vec![
                        loved_page(1, 2, &[("BAND", "opening"), ("Other", "Elsewhere")]),
                        loved_page(2, 2, &[("B\u{e1}nd", "\u{d3}pening")]),
                    ]
                    .into(),
                )),
                batches: Arc::default(),
            },
            Credentials::new(Some("key"), Some("secret")).unwrap(),
        ),
        sessions,
        QueueStore(library),
        Timing::default(),
        move |report| {
            let _ = sender.send(report);
        },
    );
    scrobbler.resume("listener");
    scrobbler.import_loved();
    let next = || reports.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(next(), Report::ImportProgress { page: 1, pages: 2 });
    assert_eq!(next(), Report::ImportProgress { page: 2, pages: 2 });
    assert_eq!(
        next(),
        Report::Imported(Imported {
            matched: 2,
            unmatched: 1
        })
    );
    let mut liked: Vec<(String, Option<String>, SystemTime)> = app
        .library
        .favourites()
        .unwrap()
        .into_iter()
        .map(|favourite| {
            (
                favourite.track.title,
                favourite.track.album,
                favourite.liked,
            )
        })
        .collect();
    liked.sort();
    let loved_at = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    assert_eq!(
        liked,
        [
            ("Opening".into(), Some("Compilation".into()), loved_at),
            ("Opening".into(), Some("Record".into()), loved_at),
        ]
    );
    assert_eq!(app.library.queued_love_count().unwrap(), 0);
}
