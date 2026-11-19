//! Play history in the window: which plays count and what is recorded with them,
//! the Songs page's Plays, Last played and Added columns and the menu choice that
//! shows them, and clearing the history from Settings. A recorder stands in for the
//! engine and for Last.fm.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use slint::{ComponentHandle, LogicalSize, Model};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{PlayerControl, PlayerService, ScrobbleSink};
use vcmp_app::preferences::{self, SONG_COLUMNS_SHOWN};
use vcmp_app::{AppWindow, Confirmation, Page, SongColumn, SongColumns, SongRow, days};
use vcmp_integrations::lastfm;
use vcmp_library::{Library, Scrobble, TrackId};
use vcmp_playback::{Command, Event, NowPlaying, PlaybackState, QueueEntry, TrackRef};

/// The engine, doing nothing.
struct Engine;

impl PlayerControl for Engine {
    fn send(&self, _: Command) {}
}

/// What reached Last.fm.
#[derive(Clone, Default)]
struct Scrobbles(Rc<RefCell<Vec<lastfm::Scrobble>>>);

impl ScrobbleSink for Scrobbles {
    fn now_playing(&self, _: lastfm::Track) {}

    fn scrobble(&self, play: lastfm::Scrobble) {
        self.0.borrow_mut().push(play);
    }
}

struct App {
    dir: tempfile::TempDir,
    music: PathBuf,
    window: AppWindow,
    library: Rc<Library>,
    player: Rc<PlayerService>,
    scrobbles: Scrobbles,
}

/// A window over two tracks: "Opening" by Band, and "Untagged" with no artist.
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(&music, "1.flac", Song::new("Opening", "Band", "Record"));
    add(&music, "2.flac", Song::new("Untagged", "", ""));
    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    let library = Rc::new(library);
    let window = vcmp_app::create_window().unwrap();
    preferences::connect(&window, library.clone()).unwrap();
    let view = LibraryView::connect(&window, library.clone()).unwrap();
    let player = PlayerService::connect(&window, library.clone(), view, Box::new(Engine), None);
    let scrobbles = Scrobbles::default();
    player.set_scrobbler(Box::new(scrobbles.clone()));
    window.show().unwrap();
    // Tall enough for Settings' Playback rows: the finder skips what is out of view.
    window.window().set_size(LogicalSize::new(1100.0, 1600.0));
    App {
        dir,
        music,
        window,
        library,
        player,
        scrobbles,
    }
}

impl App {
    fn id(&self, name: &str) -> TrackId {
        self.library
            .track_id_for_path(&self.music.join(name))
            .unwrap()
            .unwrap()
    }

    /// `name` starts playing, `length` long, from the context `origin`.
    fn play(&self, name: &str, length: Duration, origin: Option<&str>) {
        let path = self.music.join(name).to_str().unwrap().to_string();
        self.player
            .handle_event(Event::TrackChanged(Some(NowPlaying {
                entry: QueueEntry::new(TrackRef::local(path)),
                duration: Some(length),
                origin: origin.map(str::to_owned),
            })));
        self.player
            .handle_event(Event::StateChanged(PlaybackState::Playing));
    }

    /// Positions every half second, as the engine reports them while playing.
    fn listen(&self, from: f64, to: f64) {
        let mut at = from;
        while at < to {
            at = (at + 0.5).min(to);
            self.player
                .handle_event(Event::Position(Duration::from_secs_f64(at)));
        }
    }

    fn plays(&self, name: &str) -> u32 {
        self.library
            .track_play_stats(self.id(name))
            .unwrap()
            .map_or(0, |stats| stats.count)
    }

    /// Every recorded play's origin, oldest first.
    fn origins(&self) -> Vec<Option<String>> {
        let conn = rusqlite::Connection::open(self.dir.path().join("library.sqlite3")).unwrap();
        conn.prepare("SELECT origin FROM plays ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn open(&self, page: &str) {
        ElementHandle::find_by_accessible_label(&self.window, page)
            .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
            .unwrap_or_else(|| panic!("no {page} tab"))
            .invoke_accessible_default_action();
        // The Songs page asks for its songs from a change handler, which the event
        // loop runs; without one, advancing the clock does.
        mock_elapsed_time(Duration::ZERO);
    }

    /// Lets the window run its change handlers, as a frame would.
    fn frame(&self) {
        slint::platform::update_timers_and_animations();
        let _ = ElementHandle::find_by_accessible_label(&self.window, "Cancel").count();
        slint::platform::update_timers_and_animations();
    }

    fn heading(&self, name: &str) -> bool {
        ElementHandle::find_by_accessible_label(&self.window, &format!("Sort by {name}"))
            .chain(ElementHandle::find_by_accessible_label(
                &self.window,
                &format!("{name}, sorted"),
            ))
            .chain(ElementHandle::find_by_accessible_label(
                &self.window,
                &format!("{name}, sorted in reverse"),
            ))
            .any(|element| element.accessible_role() == Some(AccessibleRole::Button))
    }

    fn song(&self, title: &str) -> SongRow {
        self.window
            .get_songs()
            .iter()
            .find(|row| row.title == title)
            .unwrap_or_else(|| panic!("no song {title:?}"))
    }

    fn titles(&self) -> Vec<String> {
        self.window
            .get_songs()
            .iter()
            .map(|row| row.title.to_string())
            .collect()
    }
}

#[test]
fn a_play_counts_once_by_the_lastfm_rule_with_its_context() {
    let app = start();
    let before = SystemTime::now() - Duration::from_secs(1);
    app.play("1.flac", Duration::from_secs(200), Some("album:3"));
    app.listen(0.0, 99.5);
    assert_eq!(app.plays("1.flac"), 0, "not yet half heard");
    app.listen(99.5, 199.0);
    assert_eq!(app.plays("1.flac"), 1, "counted once");
    let stats = app.library.track_play_stats(app.id("1.flac")).unwrap();
    assert!(stats.unwrap().last >= before, "when the play started");
    assert_eq!(app.scrobbles.0.borrow().len(), 1, "and scrobbled");

    // From Up next the engine names no context.
    app.play("1.flac", Duration::from_secs(200), None);
    app.listen(0.0, 101.0);
    assert_eq!(app.plays("1.flac"), 2, "a repeat is a new play");
    assert_eq!(app.origins(), [Some("album:3".to_string()), None]);
}

#[test]
fn a_track_without_an_artist_counts_but_is_not_scrobbled() {
    let app = start();
    app.play("2.flac", Duration::from_secs(200), Some("songs"));
    app.listen(0.0, 120.0);
    assert_eq!(app.plays("2.flac"), 1);
    assert!(app.scrobbles.0.borrow().is_empty());
}

#[test]
fn short_tracks_skips_and_seeks_do_not_count() {
    let app = start();
    app.play("1.flac", Duration::from_secs(25), None);
    app.listen(0.0, 25.0);
    assert_eq!(app.plays("1.flac"), 0, "30 seconds or shorter");

    app.play("1.flac", Duration::from_secs(200), None);
    app.listen(0.0, 50.0);
    app.play("2.flac", Duration::from_secs(200), None);
    assert_eq!(app.plays("1.flac"), 0, "skipped after 50 s");

    app.play("1.flac", Duration::from_secs(200), None);
    app.listen(0.0, 50.0);
    // A seek ahead arrives as a jump in the positions.
    app.player
        .handle_event(Event::Position(Duration::from_secs(150)));
    app.listen(150.0, 199.0);
    assert_eq!(app.plays("1.flac"), 0, "99 s heard");
    assert!(app.library.play_stats(None).unwrap().is_empty());
}

#[test]
fn the_songs_columns_show_plays_and_dates_once_turned_on() {
    let app = start();
    app.open("Songs");
    assert_eq!(app.window.get_current_page(), Page::Songs);
    assert!(app.heading("Artist") && app.heading("Time"));
    for name in ["Plays", "Last played", "Added"] {
        assert!(!app.heading(name), "{name} starts hidden");
    }

    let columns = app.window.global::<SongColumns>();
    columns.set_plays(true);
    columns.set_last_played(true);
    columns.set_added(true);
    columns.set_artist(false);
    columns.invoke_toggled();
    for name in ["Plays", "Last played", "Added"] {
        assert!(app.heading(name), "{name} shows");
    }
    assert!(!app.heading("Artist"));
    assert_eq!(
        app.library.setting(SONG_COLUMNS_SHOWN).unwrap().as_deref(),
        Some("album labels year duration plays last_played added"),
        "the choice is kept"
    );

    let today = days::today();
    let opening = app.song("Opening");
    assert_eq!((opening.plays, opening.last_played.day), (0, -1));
    assert_eq!(opening.added.day, today, "first seen today");

    // A play that counts shows at once.
    app.play("1.flac", Duration::from_secs(200), Some("songs"));
    app.listen(0.0, 101.0);
    let opening = app.song("Opening");
    assert_eq!((opening.plays, opening.last_played.day), (1, today));

    // Sorted by plays, most first, the played track leads.
    app.window.invoke_sort_songs(SongColumn::Plays);
    app.window.invoke_sort_songs(SongColumn::Plays);
    assert!(app.window.get_songs_sort_descending());
    assert_eq!(app.titles(), ["Opening", "Untagged"]);

    // Hiding the column the table is sorted by goes back to the titles.
    columns.set_plays(false);
    columns.invoke_toggled();
    assert_eq!(app.window.get_songs_sort_column(), SongColumn::Title);
    assert!(!app.window.get_songs_sort_descending());

    // After a restart the columns come back as they were.
    let restarted = vcmp_app::create_window().unwrap();
    preferences::connect(&restarted, app.library.clone()).unwrap();
    let kept = restarted.global::<SongColumns>();
    assert!(!kept.get_artist() && !kept.get_plays());
    assert!(kept.get_last_played() && kept.get_added() && kept.get_album());
}

#[test]
fn clearing_the_history_from_settings_keeps_waiting_scrobbles() {
    let app = start();
    app.play("1.flac", Duration::from_secs(200), None);
    app.listen(0.0, 101.0);
    app.library
        .queue_scrobble(&Scrobble {
            artist: "Band".into(),
            title: "Opening".into(),
            album: None,
            album_artist: None,
            track_number: None,
            duration: None,
            played_at: SystemTime::now(),
        })
        .unwrap();
    assert_eq!(app.plays("1.flac"), 1);

    app.open("Settings");
    assert_eq!(app.window.get_current_page(), Page::Settings);
    app.open("Library");
    let press = |label: &str| {
        ElementHandle::find_by_accessible_label(&app.window, label)
            .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .unwrap_or_else(|| panic!("no button labelled {label:?}"))
            .invoke_accessible_default_action();
        app.frame();
    };
    press("Clear play history…");
    assert!(app.window.global::<Confirmation>().get_open(), "asks first");
    press("Keep the play history");
    assert_eq!(app.plays("1.flac"), 1, "kept when the answer is no");

    press("Clear play history…");
    press("Clear play history");
    assert_eq!(app.plays("1.flac"), 0);
    assert_eq!(app.library.queued_scrobble_count().unwrap(), 1);
}
