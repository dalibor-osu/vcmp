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
use vcmp_app::{
    AppWindow, LastfmState, LastfmStatus, PlayState, PlayerProblem, PlayerState, Repeat,
    ScrobbleMark, Shuffle, ShuffleKind,
};
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

    fn key_text(&self, text: &str) {
        let text: SharedString = text.into();
        let window = self.window.window();
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
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
fn the_bar_marks_a_lossless_file_and_the_last_fm_play() {
    let app = start(|_| {});
    let shown =
        |label: &str| ElementHandle::find_by_accessible_label(&app.window, label).count() > 0;
    app.service
        .handle_event(Event::TrackChanged(Some(NowPlaying {
            entry: app.entry("1.flac"),
            duration: Some(Duration::from_secs(100)),
        })));
    assert_eq!(app.state().get_lossless_format(), "FLAC");
    assert!(shown("Lossless (FLAC)"));
    assert_eq!(app.state().get_scrobble(), ScrobbleMark::Counting);
    assert!(!shown("Scrobbling to Last.fm"), "no account connected");

    app.window
        .global::<LastfmState>()
        .set_status(LastfmStatus::Connected);
    assert!(shown("Scrobbling to Last.fm"));
    app.service
        .handle_event(Event::StateChanged(PlaybackState::Playing));
    for quarter in 1..=200 {
        app.service
            .handle_event(Event::Position(Duration::from_millis(quarter * 250)));
    }
    assert_eq!(
        app.state().get_scrobble(),
        ScrobbleMark::Scrobbled,
        "half of it played"
    );
    assert!(shown("Scrobbled to Last.fm"));

    // Too short for Last.fm.
    app.service.handle_event(app.now_playing("2.flac"));
    assert_eq!(app.state().get_scrobble(), ScrobbleMark::None);
    assert!(!shown("Scrobbling to Last.fm") && !shown("Scrobbled to Last.fm"));

    app.service.handle_event(Event::TrackChanged(None));
    assert_eq!(app.state().get_lossless_format(), "");
    assert_eq!(app.state().get_scrobble(), ScrobbleMark::None);
}

#[test]
fn the_wheel_over_the_volume_slider_changes_the_volume() {
    let app = start(|_| {});
    app.state().invoke_volume_committed(0.5);
    app.sent.take();
    let slider = ElementHandle::find_by_accessible_label(&app.window, "Volume")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Slider))
        .expect("the volume slider");
    let (at, size) = (slider.absolute_position(), slider.size());
    let position = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
    let scroll = |delta_y: f32| {
        app.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        app.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y,
            });
    };

    // One notch up, in the steps a glide takes.
    scroll(20.0);
    scroll(40.0);
    assert!((app.state().get_volume() - 0.55).abs() < 0.001);
    assert!(matches!(
        app.sent.take()[..],
        [Command::SetVolume(_), Command::SetVolume(_)]
    ));
    assert_eq!(
        app.library.setting(SETTING_VOLUME).unwrap().as_deref(),
        Some("0.50"),
        "saved once the wheel rests"
    );
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(500));
    assert_eq!(
        app.library.setting(SETTING_VOLUME).unwrap().as_deref(),
        Some("0.55")
    );

    // Two notches down, and never below silence.
    scroll(-120.0);
    assert!((app.state().get_volume() - 0.45).abs() < 0.001);
    scroll(-6000.0);
    assert_eq!(app.state().get_volume(), 0.0);

    // The position slider leaves the wheel to the page.
    let position_slider = ElementHandle::find_by_accessible_label(&app.window, "Position")
        .next()
        .expect("the position slider");
    let at = position_slider.absolute_position();
    let size = position_slider.size();
    let position = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
    app.sent.take();
    app.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: 60.0,
        });
    assert_eq!(app.sent.take(), []);
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
fn the_file_the_engine_has_open_is_passed_on() {
    let app = start(|_| {});
    let seen = Rc::new(RefCell::new(Vec::new()));
    let record = seen.clone();
    app.service.on_open_file(move |path| {
        record
            .borrow_mut()
            .push(path.map(|path| path.to_string_lossy().into_owned()));
    });
    app.service
        .handle_event(Event::Decoding(Some(TrackRef::local(app.path("1.flac")))));
    app.service.handle_event(Event::Decoding(None));
    assert_eq!(*seen.borrow(), [None, Some(app.path("1.flac")), None]);
}

#[test]
fn unwritten_tags_are_mentioned_until_a_later_track_starts() {
    let app = start(|_| {});
    // Near the end of the first track the engine opens the second, which lets go of
    // the first; the tags waiting for it fail.
    app.service.handle_event(app.now_playing("1.flac"));
    app.service
        .handle_event(Event::Decoding(Some(TrackRef::local(app.path("2.flac")))));
    app.service
        .tags_not_written(std::path::Path::new(&app.path("1.flac")));
    assert_eq!(app.state().get_problem(), PlayerProblem::TagsNotWritten);
    assert_eq!(app.state().get_problem_detail(), "1.flac");
    ElementHandle::find_by_accessible_label(
        &app.window,
        "The new tags could not be written to 1.flac. Edit its tags to try again.",
    )
    .next()
    .expect("the player bar says so");

    app.service.handle_event(app.now_playing("2.flac"));
    assert_eq!(
        app.state().get_problem(),
        PlayerProblem::TagsNotWritten,
        "it stays while the next track plays"
    );
    app.service.handle_event(app.now_playing("1.flac"));
    assert_eq!(app.state().get_problem(), PlayerProblem::None);
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
fn a_fetched_cover_reaches_the_bar_and_discord_at_once() {
    let app = start(|_| {});
    app.play_with_discord("1.flac");
    let album = app.album();
    assert!(!app.state().get_has_cover());

    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(16, 16, image::Rgb([30, 102, 245]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let source = CoverArtId::ReleaseGroup("rg-1".into());
    app.library
        .set_fetched_cover(album, png.get_ref(), &source)
        .unwrap()
        .unwrap();
    app.library
        .set_cover_lookup(album, Some(&source), SystemTime::now())
        .unwrap();
    app.service.artwork_changed(album);
    assert!(app.state().get_has_cover(), "the bar shows it");
    assert_eq!(
        app.shown.cover(),
        Some(Cover::ReleaseGroup("rg-1".into()).url()),
        "so does Discord"
    );
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
fn up_next_rows_can_be_dragged_to_a_new_place() {
    let app = start(|_| {});
    app.window
        .window()
        .set_size(slint::LogicalSize::new(1100.0, 800.0));
    app.window.set_queue_open(true);
    app.service.handle_event(Event::QueueChanged(QueueView {
        up_next: vec![app.entry("1.flac"), app.entry("2.flac")],
        ..QueueView::default()
    }));
    // The panel slides in once made; the pointer finds it in its place.
    let _ = ElementHandle::find_by_element_type_name(&app.window, "QueuePanel").count();
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
    let centre = |label: &str| {
        let row = ElementHandle::find_by_accessible_label(&app.window, label)
            .find(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
            .unwrap();
        let (position, size) = (row.absolute_position(), row.size());
        slint::LogicalPosition::new(position.x + 60.0, position.y + size.height / 2.0)
    };
    let (from, to) = (centre("Opening, Band"), centre("Closing, Band"));
    let window = app.window.window();
    let button = slint::platform::PointerEventButton::Left;
    app.sent.take();
    window.dispatch_event(WindowEvent::PointerMoved { position: from });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    for step in 1..=4 {
        let y = from.y + (to.y - from.y) * step as f32 / 4.0;
        window.dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(from.x, y),
        });
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    assert_eq!(
        app.sent.take(),
        [Command::MoveInUpNext { from: 0, to: 1 }],
        "moved, and not played"
    );
}

#[test]
fn up_next_scrolls_while_a_drag_is_held_at_its_edge() {
    let app = start(|_| {});
    app.window
        .window()
        .set_size(slint::LogicalSize::new(1100.0, 600.0));
    app.window.set_queue_open(true);
    // Opening, Closing, Opening, ...: longer than the panel.
    let up_next = (0..30)
        .map(|i| app.entry(if i % 2 == 0 { "1.flac" } else { "2.flac" }))
        .collect();
    app.service.handle_event(Event::QueueChanged(QueueView {
        up_next,
        ..QueueView::default()
    }));
    let _ = ElementHandle::find_by_element_type_name(&app.window, "QueuePanel").count();
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
    // Only the rows in view can be found.
    let rows = |label: &str| -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&app.window, label)
            .filter(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
            .collect()
    };
    let centre = |row: &ElementHandle| {
        let (position, size) = (row.absolute_position(), row.size());
        slint::LogicalPosition::new(position.x + 60.0, position.y + size.height / 2.0)
    };
    assert!(
        rows("Opening, Band").len() < 15,
        "the list is longer than the panel"
    );
    // Drags from `from` to `to`, and holds the pointer still there for a while.
    let hold = |from: slint::LogicalPosition, to: slint::LogicalPosition| {
        let window = app.window.window();
        let button = slint::platform::PointerEventButton::Left;
        window.dispatch_event(WindowEvent::PointerMoved { position: from });
        window.dispatch_event(WindowEvent::PointerPressed {
            position: from,
            button,
        });
        for step in 1..=4 {
            let y = from.y + (to.y - from.y) * step as f32 / 4.0;
            window.dispatch_event(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(from.x, y),
            });
        }
        for _ in 0..300 {
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(20));
        }
        window.dispatch_event(WindowEvent::PointerReleased {
            position: to,
            button,
        });
    };
    app.sent.take();

    let first = centre(&rows("Opening, Band")[0]);
    hold(first, slint::LogicalPosition::new(first.x, 599.0));
    assert_eq!(
        app.sent.take(),
        [Command::MoveInUpNext { from: 0, to: 29 }],
        "held past the bottom, it goes to the end"
    );

    // The recorder leaves the queue as it was, scrolled to its end.
    let last = centre(rows("Closing, Band").last().unwrap());
    hold(last, slint::LogicalPosition::new(last.x, 1.0));
    assert_eq!(
        app.sent.take(),
        [Command::MoveInUpNext { from: 29, to: 0 }],
        "held past the top, it goes to the start"
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

#[test]
fn the_now_playing_button_opens_the_view_and_escape_or_the_background_closes_it() {
    let app = start(|_| {});
    let toggle = || {
        ElementHandle::find_by_accessible_label(&app.window, "Now playing")
            .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .expect("the now playing button")
    };
    assert_eq!(
        toggle().accessible_enabled(),
        Some(false),
        "nothing to show yet"
    );

    app.service.handle_event(app.now_playing("1.flac"));
    app.service.handle_event(Event::QueueChanged(QueueView {
        upcoming: vec![app.entry("2.flac")],
        ..QueueView::default()
    }));
    assert_eq!(app.state().get_album(), "Record");
    assert_eq!(
        (
            app.state().get_next_title().as_str(),
            app.state().get_next_artist().as_str()
        ),
        ("Closing", "Band")
    );

    toggle().invoke_accessible_default_action();
    assert!(app.state().get_now_playing_open());
    assert_eq!(toggle().accessible_checked(), Some(true));
    app.key(Key::Escape);
    assert!(!app.state().get_now_playing_open(), "Escape closes it");

    toggle().invoke_accessible_default_action();
    // A click on the background, away from the cover and the text.
    let size = app.window.window().size().to_logical(1.0);
    let position = slint::LogicalPosition::new(size.width - 40.0, 80.0);
    app.window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
    app.window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
    assert!(
        !app.state().get_now_playing_open(),
        "the background closes it"
    );
}

#[test]
fn the_playing_track_leads_to_its_album_and_artist() {
    let app = start(|_| {});
    app.service.handle_event(app.now_playing("1.flac"));
    assert!(app.state().get_has_album_link() && app.state().get_has_artist_link());

    app.press(AccessibleRole::Button, "Opening, go to the album Record");
    assert_eq!(app.window.get_current_page(), vcmp_app::Page::Albums);
    assert!(app.window.get_album_open());
    assert_eq!(app.window.get_album_detail().title, "Record");

    app.press(AccessibleRole::Button, "Band, go to the artist");
    assert_eq!(app.window.get_current_page(), vcmp_app::Page::Artists);
    assert!(app.window.get_artist_open());
    assert_eq!(app.window.get_artist_detail().name, "Band");

    // The same from the now playing view, which closes on the way.
    app.state().set_now_playing_open(true);
    let links: Vec<_> =
        ElementHandle::find_by_accessible_label(&app.window, "Record, go to the album")
            .filter(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .collect();
    assert_eq!(links.len(), 1, "the album name in the view");
    links[0].invoke_accessible_default_action();
    assert!(!app.state().get_now_playing_open());
    assert_eq!(app.window.get_current_page(), vcmp_app::Page::Albums);
}

#[test]
fn the_next_track_in_the_view_plays_when_chosen() {
    let app = start(|_| {});
    app.service.handle_event(app.now_playing("1.flac"));
    app.service.handle_event(Event::QueueChanged(QueueView {
        upcoming: vec![app.entry("2.flac")],
        ..QueueView::default()
    }));
    app.state().set_now_playing_open(true);
    app.sent.take();
    app.press(AccessibleRole::Button, "Play the next track, Closing");
    assert_eq!(app.sent.take(), [Command::Next]);
}

#[test]
fn space_plays_and_pauses_everywhere_but_in_text_fields() {
    let app = start(|_| {});
    app.sent.take();
    app.key(Key::Space);
    assert_eq!(app.sent.take(), [], "nothing loaded");
    app.service.handle_event(app.now_playing("1.flac"));
    app.key(Key::Space);
    assert_eq!(app.sent.take(), [Command::TogglePlayPause]);

    // An album page focuses its back button; Space still plays and pauses.
    app.open_album();
    app.key(Key::Space);
    assert_eq!(app.sent.take(), [Command::TogglePlayPause]);
    assert!(app.window.get_album_open(), "the back button kept still");

    // In a text field, Space types.
    app.press(AccessibleRole::Tab, "Search");
    app.window.invoke_release_focus();
    let ctrl = |pressed: bool| {
        let text: SharedString = Key::Control.into();
        app.window.window().dispatch_event(if pressed {
            WindowEvent::KeyPressed { text }
        } else {
            WindowEvent::KeyReleased { text }
        });
    };
    ctrl(true);
    app.key_text("f");
    ctrl(false);
    app.key_text("a");
    app.key(Key::Space);
    app.key_text("b");
    assert_eq!(app.window.get_search_text(), "a b");
    assert_eq!(app.sent.take(), []);

    // A click elsewhere takes the focus out of the field, and Space plays again.
    app.window.invoke_release_focus();
    app.key(Key::Space);
    assert_eq!(app.sent.take(), [Command::TogglePlayPause]);
}

#[test]
fn the_queue_panel_scrolls_and_queues_album_tracks() {
    let app = start(|_| {});
    app.window
        .window()
        .set_size(slint::LogicalSize::new(1100.0, 600.0));
    app.window.set_queue_open(true);
    // The panel slides in once made; the pointer below finds it in its place.
    let _ = ElementHandle::find_by_element_type_name(&app.window, "QueuePanel").count();
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
    let upcoming: Vec<QueueEntry> = (0..60)
        .map(|i| app.entry(if i % 2 == 0 { "1.flac" } else { "2.flac" }))
        .collect();
    app.service.handle_event(Event::QueueChanged(QueueView {
        upcoming,
        ..QueueView::default()
    }));
    // The heading above the album's tracks scrolls up with them, out of view (where
    // it is no longer kept).
    let heading_y = || {
        ElementHandle::find_by_accessible_label(&app.window, "Next up")
            .next()
            .map(|heading| heading.absolute_position().y)
    };
    let before = heading_y().expect("the heading");
    let size = app.window.window().size().to_logical(1.0);
    let position = slint::LogicalPosition::new(size.width - 100.0, size.height / 2.0);
    app.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    app.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: -300.0,
        });
    let after = heading_y();
    assert!(
        after.is_none_or(|after| after < before - 200.0),
        "the panel scrolled: {before} to {after:?}"
    );

    app.sent.take();
    app.state().invoke_queue_upcoming(1, true);
    app.state().invoke_queue_upcoming(0, false);
    let sent = app.sent.take();
    assert!(
        matches!(&sent[..], [Command::PlayNext(next), Command::AddToQueue(added)]
            if *next[0].track.id == *app.path("2.flac") && *added[0].track.id == *app.path("1.flac")),
        "{sent:?}"
    );
}

#[test]
fn queue_rows_get_their_album_cover() {
    let app = start(|_| {});
    app.service.handle_event(Event::QueueChanged(QueueView {
        up_next: vec![app.entry("1.flac")],
        upcoming: vec![app.entry("2.flac")],
        ..QueueView::default()
    }));
    let album = app.album();
    let row = app.state().get_up_next().row_data(0).unwrap();
    assert_eq!(row.album_id, album.0 as i32);
    assert!(!row.has_cover, "this album has no artwork");

    let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(4, 4);
    app.service.queue_cover_decoded(album, Some(pixels));
    assert!(app.state().get_up_next().row_data(0).unwrap().has_cover);
    assert!(app.state().get_upcoming().row_data(0).unwrap().has_cover);
}

#[test]
fn media_keys_control_the_player() {
    use slint::winit_030::winit::keyboard::{Key as WinitKey, NamedKey};
    use vcmp_app::shortcuts::play_media_key;

    let app = start(|_| {});
    let press = |key: NamedKey| play_media_key(&app.window, &WinitKey::Named(key));
    app.sent.take();
    press(NamedKey::MediaPlayPause);
    assert_eq!(app.sent.take(), [], "nothing to play yet");

    app.service.handle_event(app.now_playing("1.flac"));
    app.sent.take();
    press(NamedKey::MediaPlayPause);
    press(NamedKey::MediaTrackNext);
    press(NamedKey::MediaTrackPrevious);
    assert_eq!(
        app.sent.take(),
        [Command::TogglePlayPause, Command::Next, Command::Previous]
    );
    // Play while it plays, as Pause while it doesn't, changes nothing.
    app.window
        .global::<PlayerState>()
        .set_state(PlayState::Playing);
    press(NamedKey::MediaPlay);
    assert_eq!(app.sent.take(), []);
    press(NamedKey::MediaPause);
    assert_eq!(app.sent.take(), [Command::TogglePlayPause]);
}
