//! Playback in the window: playing an album from its page, the player bar's
//! controls, saved playback choices, and engine events shown in the bar. A recorder
//! stands in for the engine, so no sound device is needed.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, Model};
use tempfile::TempDir;
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{
    PlayerControl, PlayerService, PresenceSink, SETTING_DISCORD, SETTING_REPEAT, SETTING_SHUFFLE,
    SETTING_VOLUME,
};
use vcmp_app::{AppWindow, PlayState, PlayerProblem, PlayerState, Repeat, Shuffle, ShuffleKind};
use vcmp_integrations::discord::Listening;
use vcmp_library::Library;
use vcmp_playback::{
    Command, Event, NowPlaying, PlaybackError, PlaybackState, QueueEntry, RepeatMode, ShuffleMode,
    TrackRef,
};

#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<Command>>>);

impl PlayerControl for Recorder {
    fn send(&self, command: Command) {
        self.0.borrow_mut().push(command);
    }
}

impl Recorder {
    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

/// What reached Discord: `Some` for a shown track, `None` for a cleared presence.
#[derive(Clone, Default)]
struct Presence(Rc<RefCell<Vec<Option<Listening>>>>);

impl PresenceSink for Presence {
    fn show(&self, listening: Listening) {
        self.0.borrow_mut().push(Some(listening));
    }

    fn clear(&self) {
        self.0.borrow_mut().push(None);
    }
}

impl Presence {
    fn last(&self) -> Option<Option<Listening>> {
        self.0.borrow().last().cloned()
    }
}

struct App {
    _dir: TempDir,
    window: AppWindow,
    library: Rc<Library>,
    service: Rc<PlayerService>,
    sent: Recorder,
    shown: Presence,
    music: std::path::PathBuf,
}

/// A window with one two-track album, and saved settings applied by `prepare`.
fn start(prepare: impl FnOnce(&Library)) -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(
        &music,
        "1.flac",
        Song {
            track: 1,
            ..Song::new("Opening", "Band", "Record")
        },
    );
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
    let sent = Recorder::default();
    let shown = Presence::default();
    let service = PlayerService::connect(
        &window,
        library.clone(),
        view,
        Box::new(sent.clone()),
        Some(Box::new(shown.clone())),
    );
    App {
        _dir: dir,
        window,
        library,
        service,
        sent,
        shown,
        music,
    }
}

impl App {
    fn state(&self) -> PlayerState<'_> {
        self.window.global::<PlayerState>()
    }

    fn press(&self, role: AccessibleRole, label: &str) {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(role))
            .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
            .invoke_accessible_default_action();
    }

    fn open_album(&self) {
        self.press(AccessibleRole::Button, "Record, Band");
        assert!(self.window.get_album_open());
    }

    fn path(&self, name: &str) -> String {
        self.music.join(name).to_str().unwrap().to_string()
    }

    fn now_playing(&self, name: &str) -> Event {
        Event::TrackChanged(Some(NowPlaying {
            entry: QueueEntry::new(TrackRef::local(self.path(name))),
            duration: Some(Duration::from_secs(1)),
        }))
    }
}

fn played_context(commands: &[Command]) -> (Vec<String>, Option<String>, Option<usize>) {
    commands
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
                    .collect(),
                origin.clone(),
                *start,
            )),
            _ => None,
        })
        .expect("a PlayContext was sent")
}

#[test]
fn the_play_button_plays_the_album_in_order() {
    let app = start(|_| {});
    app.sent.take();
    app.open_album();
    app.press(AccessibleRole::Button, "Play Record");

    let (paths, origin, start) = played_context(&app.sent.take());
    assert_eq!(paths, [app.path("1.flac"), app.path("2.flac")]);
    assert!(origin.is_some_and(|origin| origin.starts_with("album:")));
    assert_eq!(start, None);
}

#[test]
fn choosing_a_track_plays_the_album_from_it() {
    let app = start(|_| {});
    app.open_album();
    app.sent.take();
    app.press(AccessibleRole::ListItem, "Closing");
    assert_eq!(played_context(&app.sent.take()).2, Some(1));
}

#[test]
fn the_shuffle_button_turns_shuffle_on_and_plays() {
    let app = start(|_| {});
    app.open_album();
    app.sent.take();
    app.press(AccessibleRole::Button, "Shuffle Record");

    let sent = app.sent.take();
    assert!(
        sent.contains(&Command::SetShuffle(ShuffleMode::LessRepeats)),
        "{sent:?}"
    );
    assert_eq!(played_context(&sent).2, None);
    assert_eq!(app.state().get_shuffle(), Shuffle::On);
    assert_eq!(
        app.library.setting(SETTING_SHUFFLE).unwrap().as_deref(),
        Some("on")
    );
}

#[test]
fn the_player_bar_controls_send_commands() {
    let app = start(|_| {});
    app.service.handle_event(app.now_playing("1.flac"));
    app.sent.take();

    app.press(AccessibleRole::Button, "Play");
    app.press(AccessibleRole::Button, "Next track");
    app.press(AccessibleRole::Button, "Previous track");
    app.state().invoke_seek(500);
    app.press(AccessibleRole::Button, "Repeat");
    app.state().invoke_volume_changed(0.5);
    assert_eq!(
        app.sent.take(),
        [
            Command::TogglePlayPause,
            Command::Next,
            Command::Previous,
            Command::Seek(Duration::from_millis(500)),
            Command::SetRepeat(RepeatMode::All),
            Command::SetVolume(0.5),
        ]
    );
    assert_eq!(
        app.library.setting(SETTING_REPEAT).unwrap().as_deref(),
        Some("all")
    );
    assert_eq!(
        app.library.setting(SETTING_VOLUME).unwrap(),
        None,
        "saved only when let go"
    );
    app.state().invoke_volume_committed(0.5);
    assert_eq!(
        app.library.setting(SETTING_VOLUME).unwrap().as_deref(),
        Some("0.50")
    );
}

#[test]
fn the_shuffle_kind_from_settings_applies_while_shuffling() {
    let app = start(|_| {});
    app.state().invoke_set_shuffle(Shuffle::On);
    app.sent.take();
    app.state().invoke_set_shuffle_kind(ShuffleKind::TrueRandom);
    assert_eq!(app.sent.take(), [Command::SetShuffle(ShuffleMode::True)]);
}

#[test]
fn saved_choices_are_applied_at_start() {
    let app = start(|library| {
        library.set_setting(SETTING_VOLUME, "0.30").unwrap();
        library.set_setting(SETTING_SHUFFLE, "on").unwrap();
        library
            .set_setting("playback.shuffle_kind", "true-random")
            .unwrap();
        library.set_setting(SETTING_REPEAT, "one").unwrap();
    });
    assert_eq!(
        app.sent.take(),
        [
            Command::SetVolume(0.3),
            Command::SetShuffle(ShuffleMode::True),
            Command::SetRepeat(RepeatMode::One)
        ]
    );
    assert_eq!(app.state().get_shuffle_kind(), ShuffleKind::TrueRandom);
    assert_eq!(app.state().get_repeat(), Repeat::One);
}

#[test]
fn engine_events_fill_the_player_bar() {
    let app = start(|_| {});
    assert!(!app.state().get_has_track());

    app.service.handle_event(app.now_playing("2.flac"));
    app.service
        .handle_event(Event::StateChanged(PlaybackState::Playing));
    app.service
        .handle_event(Event::Position(Duration::from_millis(400)));
    let state = app.state();
    assert!(state.get_has_track());
    assert_eq!(
        (state.get_title().as_str(), state.get_artist().as_str()),
        ("Closing", "Band")
    );
    assert_eq!(state.get_state(), PlayState::Playing);
    assert_eq!(
        (state.get_position_ms(), state.get_duration_text().as_str()),
        (400, "0:01")
    );
    assert_eq!(state.get_track_path(), app.path("2.flac").as_str());

    // The album page marks the playing track by its path.
    app.open_album();
    let rows: Vec<_> = app.window.get_album_tracks().iter().collect();
    assert_eq!(
        rows.iter()
            .filter(|row| row.path == state.get_track_path())
            .count(),
        1
    );

    app.service.handle_event(Event::TrackChanged(None));
    assert!(!app.state().get_has_track());
}

#[test]
fn playback_problems_are_shown_and_cleared() {
    let app = start(|_| {});
    app.service
        .handle_event(Event::OutputFailed(PlaybackError::Output(
            "no device".into(),
        )));
    assert_eq!(app.state().get_problem(), PlayerProblem::NoOutput);
    assert!(app.state().get_problem_detail().contains("no device"));

    app.service.handle_event(Event::TrackFailed {
        track: TrackRef::local(app.path("1.flac")),
        error: PlaybackError::Decode("broken".into()),
    });
    assert_eq!(app.state().get_problem(), PlayerProblem::TrackFailed);
    assert!(app.state().get_problem_detail().contains("1.flac"));

    app.service.handle_event(app.now_playing("2.flac"));
    assert_eq!(
        app.state().get_problem(),
        PlayerProblem::None,
        "a new track clears a skipped one"
    );
}

#[test]
fn seeking_moves_the_bar_at_once_and_ignores_stale_positions() {
    let app = start(|_| {});
    app.service.handle_event(app.now_playing("1.flac"));
    app.service
        .handle_event(Event::Position(Duration::from_millis(100)));

    app.state().invoke_seek(800);
    assert_eq!(app.state().get_position_ms(), 800, "no wait for the engine");
    // A position the engine sent before it took the seek would jump the bar back.
    app.service
        .handle_event(Event::Position(Duration::from_millis(5000)));
    assert_eq!(app.state().get_position_ms(), 800);
    app.service
        .handle_event(Event::Position(Duration::from_millis(850)));
    assert_eq!(app.state().get_position_ms(), 850);
}

#[test]
fn the_speaker_button_mutes_and_unmutes() {
    let app = start(|_| {});
    app.state().invoke_volume_changed(0.6);
    app.sent.take();

    app.press(AccessibleRole::Button, "Mute");
    assert!(app.state().get_muted());
    assert_eq!(app.sent.take(), [Command::SetVolume(0.0)]);

    app.press(AccessibleRole::Button, "Unmute");
    assert!(!app.state().get_muted());
    assert_eq!(
        app.sent.take(),
        [Command::SetVolume(0.6)],
        "the volume from before"
    );

    app.press(AccessibleRole::Button, "Mute");
    app.state().invoke_volume_changed(0.3);
    assert!(!app.state().get_muted(), "moving the slider unmutes");
    assert_eq!(app.sent.take().last(), Some(&Command::SetVolume(0.3)));
}

#[test]
fn discord_shows_the_playing_track_and_follows_pause_and_seek() {
    let app = start(|_| {});
    assert!(app.state().get_discord_available());
    assert!(app.state().get_discord(), "on by default");

    app.service.handle_event(app.now_playing("2.flac"));
    assert_eq!(
        app.shown.last(),
        Some(None),
        "nothing shown before playback starts"
    );
    app.service
        .handle_event(Event::StateChanged(PlaybackState::Playing));
    let listening = app.shown.last().flatten().expect("shown while playing");
    assert_eq!(listening.title, "Closing");
    assert_eq!(listening.artist.as_deref(), Some("Band"));
    assert_eq!(listening.album.as_deref(), Some("Record"));
    assert_eq!(
        listening.album_musicbrainz_id, None,
        "untagged album: no cover"
    );
    assert_eq!(listening.duration, Some(Duration::from_secs(1)));

    app.state().invoke_seek(700);
    app.service
        .handle_event(Event::Position(Duration::from_millis(700)));
    assert_eq!(
        app.shown.last().flatten().map(|l| l.position),
        Some(Duration::from_millis(700))
    );

    app.service
        .handle_event(Event::StateChanged(PlaybackState::Paused));
    assert_eq!(app.shown.last(), Some(None), "cleared while paused");

    app.service
        .handle_event(Event::StateChanged(PlaybackState::Playing));
    app.state().invoke_set_discord(false);
    assert_eq!(app.shown.last(), Some(None), "switching it off clears it");
    assert_eq!(
        app.library.setting(SETTING_DISCORD).unwrap().as_deref(),
        Some("false")
    );
    app.service.handle_event(app.now_playing("1.flac"));
    assert_eq!(app.shown.last(), Some(None), "stays off");
}
