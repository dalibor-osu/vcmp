//! Lyrics: local lyrics shown as they arrive, the line playing following the
//! engine's position and the clock, the lyrics view and the player bar's buttons,
//! and LRCLIB asked only when allowed and needed, with its answers kept. Recorders
//! stand in for the engine, the file reader and LRCLIB.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model};
use tempfile::TempDir;
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{
    LyricsReader, LyricsSearch, PlayerControl, PlayerService, SETTING_LYRICS_ONLINE,
};
use vcmp_app::{AppWindow, Overlay, PlayerState};
use vcmp_integrations::lrclib::{self, Answer};
use vcmp_library::{
    LYRICS_RETRY_NOT_FOUND, Library, LocalLyrics, LyricLine, LyricsKey, LyricsQuery, OnlineLyrics,
    TrackId,
};
use vcmp_playback::{Command, Event, NowPlaying, PlaybackState, QueueEntry, TrackRef};

#[derive(Clone, Default)]
struct Sent(Rc<RefCell<Vec<Command>>>);

impl PlayerControl for Sent {
    fn send(&self, command: Command) {
        self.0.borrow_mut().push(command);
    }
}

impl Sent {
    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

/// The files whose lyrics were asked for.
#[derive(Clone, Default)]
struct Reads(Rc<RefCell<Vec<PathBuf>>>);

impl LyricsReader for Reads {
    fn read(&self, path: PathBuf) {
        self.0.borrow_mut().push(path);
    }
}

impl Reads {
    fn take(&self) -> Vec<PathBuf> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

/// The tracks LRCLIB was asked about.
#[derive(Clone, Default)]
struct Asked(Rc<RefCell<Vec<(TrackId, lrclib::LyricsQuery)>>>);

impl LyricsSearch for Asked {
    fn find(&self, track: TrackId, query: lrclib::LyricsQuery) {
        self.0.borrow_mut().push((track, query));
    }
}

impl Asked {
    fn take(&self) -> Vec<(TrackId, lrclib::LyricsQuery)> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

struct App {
    _dir: TempDir,
    window: AppWindow,
    library: Rc<Library>,
    service: Rc<PlayerService>,
    sent: Sent,
    reads: Reads,
    asked: Asked,
    music: PathBuf,
}

/// A window with one album of two tracks, settings applied by `prepare`.
fn start(prepare: impl FnOnce(&Library)) -> App {
    // Once per test thread: a test that starts VCMP again would set the platform twice.
    thread_local!(static BACKEND: () = i_slint_backend_testing::init_no_event_loop());
    BACKEND.with(|_| {});
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(&music, "1.flac", Song::new("Opening", "Band", "Record"));
    add(
        &music,
        "2.flac",
        Song {
            track: 2,
            ..Song::new("Closing", "Band", "Record")
        },
    );
    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    prepare(&library);

    let library = Rc::new(library);
    let window = vcmp_app::create_window().unwrap();
    window.show().unwrap();
    let view = LibraryView::connect(&window, library.clone()).unwrap();
    let sent = Sent::default();
    let service =
        PlayerService::connect(&window, library.clone(), view, Box::new(sent.clone()), None);
    let reads = Reads::default();
    let asked = Asked::default();
    service.set_lyrics_reader(Box::new(reads.clone()));
    service.set_lyrics_search(Box::new(asked.clone()));
    App {
        _dir: dir,
        window,
        library,
        service,
        sent,
        reads,
        asked,
        music,
    }
}

fn line(ms: u64, text: &str) -> LyricLine {
    LyricLine {
        start: Duration::from_millis(ms),
        text: text.into(),
    }
}

fn synced() -> LocalLyrics {
    LocalLyrics {
        embedded_synced: Some(vec![
            line(0, "One"),
            line(400, "Two"),
            line(800, "Three"),
            line(900, "Four"),
        ]),
        ..LocalLyrics::default()
    }
}

fn plain() -> LocalLyrics {
    LocalLyrics {
        embedded_plain: Some("Plain words".into()),
        ..LocalLyrics::default()
    }
}

impl App {
    fn state(&self) -> PlayerState<'_> {
        self.window.global::<PlayerState>()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.music.join(name)
    }

    fn track(&self, name: &str) -> TrackId {
        self.library
            .track_id_for_path(&self.path(name))
            .unwrap()
            .unwrap()
    }

    /// Starts `name`, and hands over `local` as its lyrics read from disk.
    fn play(&self, name: &str, local: LocalLyrics) {
        let path = self.path(name);
        self.service
            .handle_event(Event::TrackChanged(Some(NowPlaying {
                entry: QueueEntry::new(TrackRef::local(path.to_str().unwrap())),
                duration: Some(Duration::from_secs(1)),
                origin: None,
            })));
        assert_eq!(
            self.reads.take(),
            std::slice::from_ref(&path),
            "read when it starts"
        );
        self.service.local_lyrics_read(&path, local);
    }

    fn lines(&self) -> Vec<String> {
        self.state()
            .get_lyrics_lines()
            .iter()
            .map(|line| line.to_string())
            .collect()
    }

    fn button(&self, label: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .unwrap_or_else(|| panic!("no button labelled {label:?}"))
    }
}

fn query(title: &str) -> lrclib::LyricsQuery {
    lrclib::LyricsQuery {
        title: title.into(),
        artist: "Band".into(),
        album: Some("Record".into()),
        duration: Duration::from_secs(1),
    }
}

fn stored_query(title: &str) -> LyricsQuery {
    LyricsQuery {
        title: title.into(),
        artist: "Band".into(),
        album: Some("Record".into()),
        duration_secs: 1,
    }
}

#[test]
fn synced_lyrics_follow_the_position_and_the_clock() {
    let app = start(|_| {});
    app.play("1.flac", synced());
    assert!(app.state().get_has_lyrics() && app.state().get_lyrics_synced());
    assert_eq!(app.lines(), ["One", "Two", "Three", "Four"]);
    assert_eq!(app.state().get_lyrics_current(), 0);

    app.service
        .handle_event(Event::Position(Duration::from_millis(500)));
    assert_eq!(app.state().get_lyrics_current(), 1);

    app.service
        .handle_event(Event::StateChanged(PlaybackState::Playing));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(350));
    assert_eq!(
        app.state().get_lyrics_current(),
        2,
        "the next line comes on time, between positions"
    );
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(150));
    assert_eq!(
        app.state().get_lyrics_current(),
        3,
        "the timer sets itself for the line after"
    );

    app.sent.take();
    app.state().invoke_seek_to_lyric(1);
    assert_eq!(app.sent.take(), [Command::Seek(Duration::from_millis(400))]);
    assert_eq!(app.state().get_lyrics_current(), 1);

    app.play("2.flac", LocalLyrics::default());
    assert!(!app.state().get_has_lyrics(), "the next track has none");
    assert_eq!(app.lines(), Vec::<String>::new());
}

#[test]
fn lyrics_of_a_track_no_longer_playing_are_dropped() {
    let app = start(|_| {});
    app.play("1.flac", LocalLyrics::default());
    app.service.local_lyrics_read(&app.path("2.flac"), synced());
    assert!(!app.state().get_has_lyrics());
}

#[test]
fn the_lyrics_and_now_playing_buttons_each_open_their_own_view() {
    let app = start(|library| {
        // The lyrics mode the view once remembered is ignored.
        library.set_setting("ui.lyrics_shown", "true").unwrap();
    });
    assert_eq!(app.state().get_overlay(), Overlay::None);
    assert_eq!(app.button("Lyrics").accessible_enabled(), Some(false));
    app.play("1.flac", plain());
    assert!(app.state().get_has_lyrics() && !app.state().get_lyrics_synced());
    assert_eq!(app.state().get_lyrics_text(), "Plain words");
    assert_eq!(app.button("Lyrics").accessible_enabled(), Some(true));

    app.button("Now playing").invoke_accessible_default_action();
    assert_eq!(
        app.state().get_overlay(),
        Overlay::NowPlaying,
        "now playing opens on the cover"
    );
    assert!(
        ElementHandle::find_by_accessible_label(&app.window, "Show lyrics")
            .next()
            .is_none(),
        "the view has no lyrics toggle of its own"
    );

    app.button("Lyrics").invoke_accessible_default_action();
    assert_eq!(app.state().get_overlay(), Overlay::Lyrics, "switches over");
    assert_eq!(app.button("Lyrics").accessible_checked(), Some(true));
    assert_eq!(app.button("Now playing").accessible_checked(), Some(false));

    app.button("Now playing").invoke_accessible_default_action();
    assert_eq!(app.state().get_overlay(), Overlay::NowPlaying, "and back");
    app.button("Now playing").invoke_accessible_default_action();
    assert_eq!(
        app.state().get_overlay(),
        Overlay::None,
        "pressed again, it closes"
    );

    app.button("Lyrics").invoke_accessible_default_action();
    assert_eq!(app.state().get_overlay(), Overlay::Lyrics);
    app.button("Lyrics").invoke_accessible_default_action();
    assert_eq!(
        app.state().get_overlay(),
        Overlay::None,
        "pressed again, it closes"
    );
    assert_eq!(
        app.library.setting("ui.lyrics_shown").unwrap().as_deref(),
        Some("true"),
        "the old setting is left as it was"
    );
}

#[test]
fn escape_closes_the_lyrics_view() {
    let app = start(|_| {});
    app.play("1.flac", synced());
    app.button("Lyrics").invoke_accessible_default_action();
    assert_eq!(app.state().get_overlay(), Overlay::Lyrics);
    app.window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    app.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased {
            text: Key::Escape.into(),
        });
    assert_eq!(app.state().get_overlay(), Overlay::None);
}

#[test]
fn the_lyrics_view_stays_open_on_a_track_without_lyrics() {
    let app = start(|_| {});
    app.play("1.flac", synced());
    app.button("Lyrics").invoke_accessible_default_action();
    let says_none = || {
        ElementHandle::find_by_accessible_label(&app.window, "No lyrics for this track")
            .next()
            .is_some()
    };
    assert!(!says_none());

    app.play("2.flac", LocalLyrics::default());
    assert!(!app.state().get_has_lyrics());
    assert_eq!(app.state().get_overlay(), Overlay::Lyrics, "the view stays");
    assert!(says_none(), "and says there are none");
    assert_eq!(
        app.button("Lyrics").accessible_enabled(),
        Some(true),
        "the button can still close it"
    );

    app.button("Lyrics").invoke_accessible_default_action();
    assert_eq!(app.state().get_overlay(), Overlay::None);
    assert_eq!(app.button("Lyrics").accessible_enabled(), Some(false));
}

#[test]
fn the_lyrics_take_the_reading_width_of_a_wide_window() {
    let app = start(|_| {});
    app.window
        .window()
        .set_size(slint::LogicalSize::new(1600.0, 900.0));
    app.play("1.flac", synced());
    app.button("Lyrics").invoke_accessible_default_action();
    let list = ElementHandle::find_by_accessible_label(&app.window, "Lyrics")
        .find(|element| element.accessible_role() == Some(AccessibleRole::List))
        .expect("the lyrics list");
    let width = list.size().width;
    assert!(
        (700.0..=720.0).contains(&width),
        "the list is {width} px wide, not the reading width"
    );
}

#[test]
fn lrclib_is_asked_only_with_the_switch_on_and_nothing_synced_on_disk() {
    let app = start(|_| {});
    app.play("1.flac", plain());
    assert_eq!(app.asked.take(), [], "the switch is off");

    app.state().invoke_set_lyrics_online(true);
    assert_eq!(
        app.library
            .setting(SETTING_LYRICS_ONLINE)
            .unwrap()
            .as_deref(),
        Some("true")
    );
    let track = app.track("1.flac");
    assert_eq!(app.asked.take(), [(track, query("Opening"))]);
    assert_eq!(
        app.state().get_lyrics_text(),
        "Plain words",
        "shown meanwhile"
    );

    app.play("2.flac", synced());
    assert_eq!(app.asked.take(), [], "synced lyrics are on disk");
}

#[test]
fn an_answer_is_kept_and_shown_while_the_track_plays() {
    let app = start(|library| {
        library.set_setting(SETTING_LYRICS_ONLINE, "true").unwrap();
    });
    app.play("1.flac", plain());
    let track = app.track("1.flac");
    assert_eq!(app.asked.take().len(), 1);

    app.service.lyrics_found(
        track,
        query("Opening"),
        Answer::Found {
            synced: Some("[00:00.10]Found".into()),
            plain: Some("Found".into()),
        },
    );
    assert!(
        app.state().get_lyrics_synced(),
        "synced online beats plain on disk"
    );
    assert_eq!(app.lines(), ["Found"]);
    let stored = app
        .library
        .lyrics_lookup(&LyricsKey::local(track))
        .unwrap()
        .unwrap();
    assert_eq!(stored.query, stored_query("Opening"));

    app.play("2.flac", LocalLyrics::default());
    app.asked.take();
    app.play("1.flac", plain());
    assert_eq!(app.asked.take(), [], "known from now on");
    assert_eq!(app.lines(), ["Found"]);
}

#[test]
fn a_miss_waits_a_changed_question_asks_again_and_a_failure_is_not_kept() {
    let app = start(|library| {
        library.set_setting(SETTING_LYRICS_ONLINE, "true").unwrap();
    });
    let track = app.track("1.flac");
    let key = LyricsKey::local(track);
    app.play("1.flac", LocalLyrics::default());
    assert_eq!(app.asked.take().len(), 1);
    app.service
        .lyrics_found(track, query("Opening"), Answer::NotFound);
    assert!(!app.state().get_has_lyrics());

    app.play("1.flac", LocalLyrics::default());
    assert_eq!(app.asked.take(), [], "the miss is kept");

    let long_ago = SystemTime::now() - LYRICS_RETRY_NOT_FOUND - Duration::from_secs(60);
    app.library
        .set_lyrics_lookup(&key, &stored_query("Opening"), None, long_ago)
        .unwrap();
    app.play("1.flac", LocalLyrics::default());
    assert_eq!(app.asked.take().len(), 1, "asked again after a while");

    let found = OnlineLyrics {
        synced: None,
        plain: Some("Old title's words".into()),
    };
    app.library
        .set_lyrics_lookup(
            &key,
            &stored_query("Old title"),
            Some(&found),
            SystemTime::now(),
        )
        .unwrap();
    app.play("1.flac", LocalLyrics::default());
    assert_eq!(
        app.asked.take(),
        [(track, query("Opening"))],
        "an answer to other tags counts for nothing"
    );
    assert!(!app.state().get_has_lyrics());

    app.service.lyrics_found(
        track,
        query("Opening"),
        Answer::Failed {
            error: "offline".into(),
            retry_after: None,
        },
    );
    assert_eq!(
        app.library.lyrics_lookup(&key).unwrap().unwrap().query,
        stored_query("Old title"),
        "a failure is not stored"
    );
    app.play("1.flac", LocalLyrics::default());
    assert_eq!(app.asked.take(), [], "not again right away");
}
