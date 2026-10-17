//! Playback in the window: playing an album from its page, the player bar's
//! controls, saved playback choices, and engine events shown in the bar. A recorder
//! stands in for the engine, so no sound device is needed.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};
use tempfile::TempDir;
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{
    CoverSearch, MediaSink, PlayerControl, PlayerService, Presence, PresenceSink, RETRY_NOT_FOUND,
    SETTING_DISCORD, SETTING_OUTPUT_DEVICE, SETTING_REPEAT, SETTING_SHUFFLE, SETTING_VOLUME,
};
use vcmp_app::saved_queue;
use vcmp_app::{AppWindow, PlayState, PlayerProblem, PlayerState, Repeat, Shuffle, ShuffleKind};
use vcmp_integrations::discord::Listening;
use vcmp_integrations::media_controls::{self, Control, MediaState};
use vcmp_integrations::musicbrainz::{AlbumQuery, Cover, Lookup};
use vcmp_library::{AlbumId, CoverArtId, Library};
use vcmp_playback::{
    AudioFormat, Command, Event, NowPlaying, OutputInfo, PlaybackError, PlaybackSnapshot,
    PlaybackState, QueueEntry, QueueSlot, QueueSnapshot, QueueView, RepeatMode, ShuffleMode,
    TrackRef,
};

/// The commands sent, and the snapshot the engine would give when asked.
#[derive(Clone, Default)]
struct Recorder(
    Rc<RefCell<Vec<Command>>>,
    Rc<RefCell<Option<PlaybackSnapshot>>>,
);

impl PlayerControl for Recorder {
    fn send(&self, command: Command) {
        self.0.borrow_mut().push(command);
    }

    fn snapshot(&self) -> Option<PlaybackSnapshot> {
        self.1.borrow().clone()
    }
}

impl Recorder {
    fn take(&self) -> Vec<Command> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

/// What reached Discord: `Some` for a shown track, `None` for a cleared presence.
#[derive(Clone, Default)]
struct Shown(Rc<RefCell<Vec<Option<Listening>>>>);

impl PresenceSink for Shown {
    fn show(&self, listening: Listening) {
        self.0.borrow_mut().push(Some(listening));
    }

    fn clear(&self) {
        self.0.borrow_mut().push(None);
    }
}

impl Shown {
    fn last(&self) -> Option<Option<Listening>> {
        self.0.borrow().last().cloned()
    }

    fn cover(&self) -> Option<String> {
        self.last()
            .flatten()
            .and_then(|listening| listening.cover_url)
    }
}

/// The albums sent to MusicBrainz.
#[derive(Clone, Default)]
struct Lookups(Rc<RefCell<Vec<(AlbumId, AlbumQuery)>>>);

impl CoverSearch for Lookups {
    fn find(&self, album: AlbumId, query: AlbumQuery) {
        self.0.borrow_mut().push((album, query));
    }
}

impl Lookups {
    fn take(&self) -> Vec<(AlbumId, AlbumQuery)> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

struct App {
    _dir: TempDir,
    window: AppWindow,
    library: Rc<Library>,
    service: Rc<PlayerService>,
    sent: Recorder,
    shown: Shown,
    lookups: Lookups,
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
    let shown = Shown::default();
    let lookups = Lookups::default();
    let service = PlayerService::connect(
        &window,
        library.clone(),
        view,
        Box::new(sent.clone()),
        Some(Presence {
            sink: Box::new(shown.clone()),
            covers: Box::new(lookups.clone()),
        }),
    );
    App {
        _dir: dir,
        window,
        library,
        service,
        sent,
        shown,
        lookups,
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

    fn album(&self) -> AlbumId {
        self.library.albums().unwrap()[0].id
    }

    /// Plays `name` with Discord running.
    fn play_with_discord(&self, name: &str) {
        self.service.set_discord_connected(true);
        self.service.handle_event(self.now_playing(name));
        self.service
            .handle_event(Event::StateChanged(PlaybackState::Playing));
    }

    fn entry(&self, name: &str) -> QueueEntry {
        QueueEntry::new(TrackRef::local(self.path(name)))
    }

    fn key(&self, key: Key) {
        let text: SharedString = key.into();
        let window = self.window.window();
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
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
    assert_eq!(listening.cover_url, None, "untagged album: no cover yet");
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

#[test]
fn an_untagged_album_is_looked_up_once_discord_is_there() {
    let app = start(|_| {});
    app.service.handle_event(app.now_playing("1.flac"));
    app.service
        .handle_event(Event::StateChanged(PlaybackState::Playing));
    assert_eq!(app.lookups.take(), [], "Discord is not running");

    app.service.set_discord_connected(true);
    let album = app.album();
    assert_eq!(
        app.lookups.take(),
        [(
            album,
            AlbumQuery {
                artist: "Band".into(),
                title: "Record".into(),
                track_count: 2,
            }
        )]
    );
    app.state().invoke_seek(500);
    app.service
        .handle_event(Event::Position(Duration::from_millis(500)));
    app.service.handle_event(app.now_playing("2.flac"));
    assert_eq!(app.lookups.take(), [], "one lookup at a time per album");

    let cover = Cover::Release("r-1".into());
    app.service.cover_found(album, Lookup::Found(cover.clone()));
    assert_eq!(
        app.shown.cover(),
        Some(cover.url()),
        "the playing track gets it"
    );
    assert_eq!(
        app.library.cover_lookup(album).unwrap().unwrap().cover,
        Some(CoverArtId::Release("r-1".into()))
    );

    app.service.handle_event(app.now_playing("1.flac"));
    assert_eq!(app.shown.cover(), Some(cover.url()), "known from now on");
    assert_eq!(app.lookups.take(), []);
}

#[test]
fn nothing_is_looked_up_while_the_presence_is_off() {
    let app = start(|library| {
        library.set_setting(SETTING_DISCORD, "false").unwrap();
    });
    app.play_with_discord("1.flac");
    assert_eq!(app.lookups.take(), []);
    assert_eq!(app.shown.last(), Some(None));
}

#[test]
fn an_album_without_a_match_waits_before_the_next_lookup() {
    let app = start(|_| {});
    app.play_with_discord("1.flac");
    let album = app.album();
    assert_eq!(app.lookups.take().len(), 1);
    app.service.cover_found(album, Lookup::NotFound);
    assert_eq!(app.shown.cover(), None);

    app.service.handle_event(app.now_playing("2.flac"));
    assert_eq!(app.lookups.take(), [], "the answer is kept");

    let long_ago = SystemTime::now() - RETRY_NOT_FOUND - Duration::from_secs(60);
    app.library.set_cover_lookup(album, None, long_ago).unwrap();
    app.service.handle_event(app.now_playing("1.flac"));
    assert_eq!(app.lookups.take().len(), 1, "asked again after a while");
}

#[test]
fn a_failed_lookup_is_not_kept() {
    let app = start(|_| {});
    app.play_with_discord("1.flac");
    let album = app.album();
    assert_eq!(app.lookups.take().len(), 1);
    app.service
        .cover_found(album, Lookup::Failed("offline".into()));
    assert_eq!(app.library.cover_lookup(album).unwrap(), None);

    app.service.handle_event(app.now_playing("2.flac"));
    assert_eq!(app.lookups.take(), [], "not right away");
}

#[test]
fn the_queue_panel_lists_up_next_and_the_rest_of_the_album() {
    let app = start(|_| {});
    app.window.set_queue_open(true);
    let album = app.album();
    app.service.handle_event(Event::QueueChanged(QueueView {
        current: Some(app.entry("1.flac")),
        up_next: vec![app.entry("2.flac")],
        upcoming: vec![app.entry("2.flac")],
        origin: Some(format!("album:{}", album.0)),
        ..QueueView::default()
    }));
    let row = app.state().get_up_next().row_data(0).unwrap();
    assert_eq!(
        (
            row.title.as_str(),
            row.artist.as_str(),
            row.duration.as_str()
        ),
        ("Closing", "Band", "0:01")
    );
    assert_eq!(app.state().get_upcoming().row_count(), 1);
    assert_eq!(app.state().get_queue_origin(), "Record");

    app.sent.take();
    let rows: Vec<_> = ElementHandle::find_by_accessible_label(&app.window, "Closing, Band")
        .filter(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
        .collect();
    assert_eq!(rows.len(), 2, "one in Up next, one from the album");
    rows[1].invoke_accessible_default_action();
    assert_eq!(
        app.sent.take(),
        [Command::SkipTo(1)],
        "counted after Up next"
    );

    app.press(AccessibleRole::Button, "Remove Closing from the queue");
    app.press(AccessibleRole::Button, "Clear the queue");
    assert_eq!(
        app.sent.take(),
        [Command::RemoveFromUpNext(0), Command::ClearUpNext]
    );
}

#[test]
fn the_queue_list_works_from_the_keyboard() {
    let app = start(|_| {});
    app.window.set_queue_open(true);
    app.service.handle_event(Event::QueueChanged(QueueView {
        up_next: vec![app.entry("1.flac"), app.entry("2.flac")],
        upcoming: vec![app.entry("1.flac")],
        ..QueueView::default()
    }));
    let selected = |app: &App| -> Vec<String> {
        ["Opening, Band", "Closing, Band"]
            .iter()
            .flat_map(|label| ElementHandle::find_by_accessible_label(&app.window, label))
            .filter(|row| row.accessible_item_selected() == Some(true))
            .filter_map(|row| row.accessible_label().map(|label| label.to_string()))
            .collect()
    };
    // Tab until the list has focus: its first track becomes the current one.
    let reached = (0..30).any(|_| {
        app.key(Key::Tab);
        !selected(&app).is_empty()
    });
    assert!(reached, "the queue list is a tab stop");
    assert_eq!(selected(&app), ["Opening, Band"]);

    app.sent.take();
    app.key(Key::DownArrow);
    assert_eq!(selected(&app), ["Closing, Band"]);
    app.key(Key::Return);
    app.key(Key::Delete);
    app.window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Alt.into(),
    });
    app.key(Key::UpArrow);
    app.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased {
            text: Key::Alt.into(),
        });
    assert_eq!(
        app.sent.take(),
        [
            Command::SkipTo(1),
            Command::RemoveFromUpNext(1),
            Command::MoveInUpNext { from: 1, to: 0 },
        ]
    );
}

#[test]
fn album_tracks_go_into_up_next() {
    let app = start(|_| {});
    app.open_album();
    app.sent.take();
    app.window.invoke_queue_album_track(1, true);
    app.window.invoke_queue_album_track(0, false);
    let sent = app.sent.take();
    assert!(
        matches!(&sent[0], Command::PlayNext(entries) if entries.len() == 1 && *entries[0].track.id == *app.path("2.flac")),
        "{sent:?}"
    );
    assert!(
        matches!(&sent[1], Command::AddToQueue(entries) if *entries[0].track.id == *app.path("1.flac")),
        "{sent:?}"
    );
}

#[test]
fn the_queue_is_saved_and_comes_back_paused() {
    let snapshot = PlaybackSnapshot {
        queue: QueueSnapshot {
            context: vec![QueueEntry::new(TrackRef::local("/music/a.flac"))],
            order: vec![0],
            current: Some(QueueSlot::Context(0)),
            ..QueueSnapshot::default()
        },
        position: Duration::from_secs(42),
    };
    let saved = saved_queue::encode(&snapshot);
    let app = start(|library| {
        library.set_setting(saved_queue::SETTING, &saved).unwrap();
    });
    assert!(
        app.sent
            .take()
            .contains(&Command::Restore(snapshot.clone())),
        "restored at start"
    );

    let later = PlaybackSnapshot {
        position: Duration::from_secs(7),
        ..snapshot
    };
    *app.sent.1.borrow_mut() = Some(later.clone());
    app.service.handle_event(app.now_playing("1.flac"));
    let stored = app.library.setting(saved_queue::SETTING).unwrap().unwrap();
    assert_eq!(
        saved_queue::decode(&stored),
        Ok(later),
        "saved as tracks change"
    );
}

#[test]
fn the_output_device_is_chosen_and_remembered() {
    let app = start(|library| {
        library
            .set_setting(SETTING_OUTPUT_DEVICE, "USB DAC")
            .unwrap();
    });
    let devices = |app: &App| -> Vec<String> {
        app.state()
            .get_output_devices()
            .iter()
            .map(|name| name.to_string())
            .collect()
    };
    assert_eq!(devices(&app), ["USB DAC"], "known before the list arrives");
    assert_eq!(app.state().get_output_device(), 0);

    app.service
        .set_output_devices(Ok(vec!["Speakers".into(), "Headphones".into()]));
    assert_eq!(
        devices(&app),
        ["Speakers", "Headphones", "USB DAC"],
        "a missing choice stays in the list"
    );
    assert_eq!(app.state().get_output_device(), 2);
    app.service.handle_event(Event::OutputChanged(OutputInfo {
        device: "Speakers".into(),
        format: AudioFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        fell_back: true,
    }));
    assert_eq!(app.state().get_output_fallback(), "Speakers");

    // The dropdown in Settings, from the keyboard: Up picks the device above.
    app.service
        .set_device_lister(|| Ok(vec!["Speakers".into(), "Headphones".into()]));
    app.press(AccessibleRole::Tab, "Settings");
    let dropdown = ElementHandle::find_by_accessible_label(&app.window, "Output device")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Combobox))
        .expect("the output device choice");
    assert_eq!(dropdown.accessible_value().as_deref(), Some("USB DAC"));
    app.sent.take();
    let chosen = (0..30).find_map(|_| {
        app.key(Key::Tab);
        app.key(Key::UpArrow);
        app.sent
            .take()
            .into_iter()
            .find(|command| matches!(command, Command::SetOutputDevice(_)))
    });
    assert_eq!(
        chosen,
        Some(Command::SetOutputDevice(Some("Headphones".into())))
    );
    assert_eq!(
        app.library
            .setting(SETTING_OUTPUT_DEVICE)
            .unwrap()
            .as_deref(),
        Some("Headphones")
    );
    assert_eq!(
        app.state().get_output_fallback(),
        "",
        "a new choice clears the note"
    );

    app.state().invoke_set_output_device(-1);
    assert_eq!(app.sent.take(), [Command::SetOutputDevice(None)]);
    assert_eq!(app.state().get_output_device(), -1, "the system default");
}

/// What the system's media widget was told.
#[derive(Clone, Default)]
struct Widget(Rc<RefCell<Vec<MediaState>>>);

impl MediaSink for Widget {
    fn update(&self, state: MediaState) {
        self.0.borrow_mut().push(state);
    }
}

impl Widget {
    fn last(&self) -> MediaState {
        self.0
            .borrow()
            .last()
            .cloned()
            .expect("the widget was updated")
    }
}

fn with_widget(app: &App) -> Widget {
    let widget = Widget::default();
    app.service.set_media_controls(Box::new(widget.clone()));
    widget
}

#[test]
fn media_keys_send_commands() {
    let app = start(|_| {});
    app.service.handle_event(app.now_playing("1.flac"));
    app.sent.take();

    for control in [
        Control::TogglePlayPause,
        Control::Play,
        Control::Pause,
        Control::Stop,
        Control::Next,
        Control::Previous,
        Control::SeekTo(Duration::from_millis(400)),
    ] {
        app.service.media_control(control);
    }
    assert_eq!(
        app.sent.take(),
        [
            Command::TogglePlayPause,
            Command::Play,
            Command::Pause,
            Command::Stop,
            Command::Next,
            Command::Previous,
            Command::Seek(Duration::from_millis(400)),
        ]
    );
    assert_eq!(
        app.state().get_position_ms(),
        400,
        "the bar follows the seek"
    );
}

#[test]
fn relative_seeks_stay_inside_the_track() {
    let app = start(|_| {});
    app.service.handle_event(app.now_playing("1.flac"));
    app.service
        .handle_event(Event::Position(Duration::from_millis(300)));
    app.sent.take();

    let by = Duration::from_secs(10);
    app.service
        .media_control(Control::SeekBy { forward: false, by });
    app.service
        .media_control(Control::SeekBy { forward: true, by });
    assert_eq!(
        app.sent.take(),
        [
            Command::Seek(Duration::ZERO),
            // The track is a second long.
            Command::Seek(Duration::from_secs(1)),
        ]
    );
}

#[test]
fn the_widget_changes_shuffle_repeat_and_volume() {
    let app = start(|_| {});
    app.sent.take();
    app.service.media_control(Control::SetShuffle(true));
    app.service
        .media_control(Control::SetRepeat(media_controls::Repeat::One));
    app.service.media_control(Control::SetVolume(0.25));
    assert_eq!(
        app.sent.take(),
        [
            Command::SetShuffle(ShuffleMode::LessRepeats),
            Command::SetRepeat(RepeatMode::One),
            Command::SetVolume(0.25),
        ]
    );
    assert_eq!(app.state().get_shuffle(), Shuffle::On);
    assert_eq!(app.state().get_repeat(), Repeat::One);
    assert_eq!(
        app.library.setting(SETTING_VOLUME).unwrap().as_deref(),
        Some("0.25")
    );
}

#[test]
fn the_playing_track_reaches_the_media_widget() {
    let app = start(|_| {});
    let widget = with_widget(&app);
    assert_eq!(widget.last().track, None, "nothing plays yet");

    app.service.handle_event(app.now_playing("1.flac"));
    app.service
        .handle_event(Event::StateChanged(PlaybackState::Playing));
    let shown = widget.last();
    let track = shown.track.expect("a track is shown");
    assert_eq!(track.id, app.path("1.flac"));
    assert_eq!(track.title, "Opening");
    assert_eq!(track.artist.as_deref(), Some("Band"));
    assert_eq!(track.album.as_deref(), Some("Record"));
    assert_eq!(track.duration, Some(Duration::from_secs(1)));
    assert!(shown.playing);

    app.service
        .handle_event(Event::Position(Duration::from_millis(250)));
    assert_eq!(widget.last().position, Duration::from_millis(250));

    app.service
        .handle_event(Event::StateChanged(PlaybackState::Paused));
    assert!(!widget.last().playing);

    app.state().invoke_toggle_mute();
    assert_eq!(widget.last().volume, 0.0, "muted");

    app.service.handle_event(Event::TrackChanged(None));
    assert_eq!(widget.last().track, None, "the queue ran out");
}
