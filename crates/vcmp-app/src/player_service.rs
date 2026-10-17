//! Playback behind the `PlayerState` global: the player bar's controls, playing an
//! album from its page, and the engine's events turned into what the bar shows.
//! Volume, shuffle and repeat are kept in the library's settings.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

use slint::{ComponentHandle, Image};
use vcmp_library::{Library, Track};
use vcmp_playback::{
    Command, Event, NowPlaying, PlaybackState, Player, QueueEntry, RepeatMode, ShuffleMode,
    TrackRef,
};

use crate::albums::initial;
use crate::library_view::{LibraryView, format_duration};
use crate::{AppWindow, PlayState, PlayerProblem, PlayerState, Repeat, Shuffle, ShuffleKind};

pub const SETTING_VOLUME: &str = "playback.volume";
pub const SETTING_SHUFFLE: &str = "playback.shuffle";
pub const SETTING_SHUFFLE_KIND: &str = "playback.shuffle_kind";
pub const SETTING_REPEAT: &str = "playback.repeat";

/// Where commands go: the engine, or a recorder in tests.
pub trait PlayerControl {
    fn send(&self, command: Command);
}

impl PlayerControl for Player {
    fn send(&self, command: Command) {
        Player::send(self, command);
    }
}

pub struct PlayerService {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    view: Rc<LibraryView>,
    player: Box<dyn PlayerControl>,
    duration: Cell<Duration>,
}

thread_local! {
    /// The service engine events are delivered to, on the UI thread.
    static SERVICE: std::cell::RefCell<std::rc::Weak<PlayerService>> = std::cell::RefCell::default();
}

impl PlayerService {
    /// Connects `PlayerState` to `player` and applies the saved volume, shuffle and
    /// repeat.
    pub fn connect(
        window: &AppWindow,
        library: Rc<Library>,
        view: Rc<LibraryView>,
        player: Box<dyn PlayerControl>,
    ) -> Rc<Self> {
        let service = Rc::new(Self {
            window: window.as_weak(),
            library,
            view,
            player,
            duration: Cell::default(),
        });
        SERVICE.with(|current| *current.borrow_mut() = Rc::downgrade(&service));

        let state = window.global::<PlayerState>();
        let volume = service
            .setting(SETTING_VOLUME)
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let shuffle = if service.setting(SETTING_SHUFFLE).as_deref() == Some("on") {
            Shuffle::On
        } else {
            Shuffle::Off
        };
        let kind = match service.setting(SETTING_SHUFFLE_KIND).as_deref() {
            Some("true-random") => ShuffleKind::TrueRandom,
            _ => ShuffleKind::FewerRepeats,
        };
        let repeat = match service.setting(SETTING_REPEAT).as_deref() {
            Some("all") => Repeat::All,
            Some("one") => Repeat::One,
            _ => Repeat::Off,
        };
        state.set_volume(volume);
        state.set_shuffle(shuffle);
        state.set_shuffle_kind(kind);
        state.set_repeat(repeat);
        service.player.send(Command::SetVolume(volume));
        service
            .player
            .send(Command::SetShuffle(shuffle_mode(shuffle, kind)));
        service.player.send(Command::SetRepeat(repeat_mode(repeat)));

        let weak = Rc::downgrade(&service);
        let with = move |action: fn(&PlayerService)| {
            let weak = weak.clone();
            move || {
                if let Some(service) = weak.upgrade() {
                    action(&service);
                }
            }
        };
        state.on_toggle_play(with(|service| {
            service.player.send(Command::TogglePlayPause)
        }));
        state.on_next(with(|service| service.player.send(Command::Next)));
        state.on_previous(with(|service| service.player.send(Command::Previous)));

        let weak = Rc::downgrade(&service);
        state.on_seek({
            let weak = weak.clone();
            move |ms| {
                if let Some(service) = weak.upgrade() {
                    service
                        .player
                        .send(Command::Seek(Duration::from_millis(ms.max(0) as u64)));
                }
            }
        });
        state.on_volume_changed({
            let weak = weak.clone();
            move |volume| {
                if let Some(service) = weak.upgrade() {
                    service.set_volume(volume, false);
                }
            }
        });
        state.on_volume_committed({
            let weak = weak.clone();
            move |volume| {
                if let Some(service) = weak.upgrade() {
                    service.set_volume(volume, true);
                }
            }
        });
        state.on_set_shuffle({
            let weak = weak.clone();
            move |shuffle| {
                if let Some(service) = weak.upgrade() {
                    service.set_shuffle(shuffle);
                }
            }
        });
        state.on_set_shuffle_kind({
            let weak = weak.clone();
            move |kind| {
                if let Some(service) = weak.upgrade() {
                    service.set_shuffle_kind(kind);
                }
            }
        });
        state.on_set_repeat({
            let weak = weak.clone();
            move |repeat| {
                if let Some(service) = weak.upgrade() {
                    service.set_repeat(repeat);
                }
            }
        });
        window.on_play_album(move |start, shuffle| {
            if let Some(service) = weak.upgrade() {
                service.play_album(start, shuffle);
            }
        });
        service
    }

    /// Forwards the engine's events to the UI thread for as long as the engine runs.
    pub fn listen(events: Receiver<Event>) {
        thread::Builder::new()
            .name("vcmp-player-events".into())
            .spawn(move || {
                for event in events {
                    let delivered = slint::invoke_from_event_loop(move || {
                        if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                            service.handle_event(event);
                        }
                    });
                    if delivered.is_err() {
                        break;
                    }
                }
            })
            .expect("spawning the player event thread");
    }

    /// Plays the album open on its page from track `start` (-1: from its start, or a
    /// random track when shuffling). `shuffle` turns shuffle on first.
    pub fn play_album(&self, start: i32, shuffle: bool) {
        let Some(album) = self.view.shown_album() else {
            return;
        };
        let tracks = match self.library.album_tracks(album.id) {
            Ok(tracks) => tracks,
            Err(error) => {
                log::error!("could not read the tracks of {:?}: {error}", album.title);
                return;
            }
        };
        let entries: Vec<QueueEntry> = tracks.iter().filter_map(entry).collect();
        if shuffle {
            self.set_shuffle(Shuffle::On);
        }
        let start = if shuffle {
            None
        } else {
            usize::try_from(start).ok()
        };
        self.player.send(Command::PlayContext {
            entries,
            origin: Some(format!("album:{}", album.id.0)),
            start,
        });
    }

    /// Applies one engine event to `PlayerState`.
    pub fn handle_event(&self, event: Event) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        match event {
            Event::StateChanged(playback) => state.set_state(match playback {
                PlaybackState::Playing => PlayState::Playing,
                PlaybackState::Paused => PlayState::Paused,
                PlaybackState::Stopped => PlayState::Stopped,
            }),
            Event::TrackChanged(now) => self.show_track(&state, now),
            Event::Position(position) => {
                state.set_position_ms(position.as_millis() as i32);
                state.set_position_text(format_duration(position).into());
            }
            Event::QueueChanged(queue) => {
                state.set_shuffle(if queue.shuffle == ShuffleMode::Off {
                    Shuffle::Off
                } else {
                    Shuffle::On
                });
                state.set_repeat(match queue.repeat {
                    RepeatMode::Off => Repeat::Off,
                    RepeatMode::All => Repeat::All,
                    RepeatMode::One => Repeat::One,
                });
            }
            Event::TrackFailed { track, error } => {
                let name = Path::new(track.id.as_ref())
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned());
                state.set_problem(PlayerProblem::TrackFailed);
                state.set_problem_detail(format!("{} ({error})", name.unwrap_or_default()).into());
            }
            Event::OutputChanged(info) => {
                if state.get_problem() == PlayerProblem::NoOutput {
                    state.set_problem(PlayerProblem::None);
                }
                if info.fell_back {
                    log::warn!(
                        "the chosen output device is missing; playing through {}",
                        info.device
                    );
                }
            }
            Event::OutputFailed(error) => {
                state.set_problem(PlayerProblem::NoOutput);
                state.set_problem_detail(error.to_string().into());
            }
            Event::RestoreFailed(error) => log::warn!("could not restore the queue: {error}"),
        }
    }

    fn show_track(&self, state: &PlayerState, now: Option<NowPlaying>) {
        if state.get_problem() == PlayerProblem::TrackFailed {
            state.set_problem(PlayerProblem::None);
        }
        let Some(now) = now else {
            state.set_has_track(false);
            state.set_track_path("".into());
            state.set_has_cover(false);
            state.set_cover(Image::default());
            return;
        };
        let path = now.entry.track.id.to_string();
        let track = self
            .library
            .track_id_for_path(Path::new(&path))
            .ok()
            .flatten()
            .and_then(|id| self.library.track(id).ok().flatten());
        let title = track.as_ref().map_or_else(
            || {
                Path::new(&path)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default()
            },
            |track| track.title.clone(),
        );
        let cover = track
            .as_ref()
            .and_then(|track| track.album_id)
            .and_then(|album| self.library.album(album).ok().flatten())
            .and_then(|album| album.artwork)
            .and_then(|artwork| Image::load_from_path(&artwork.thumbnail).ok());
        let duration = now
            .duration
            .or(track.as_ref().map(|track| track.duration))
            .unwrap_or_default();
        self.duration.set(duration);

        state.set_has_track(true);
        state.set_initial(initial(&title).into());
        state.set_title(title.into());
        state.set_artist(
            track
                .and_then(|track| track.artist)
                .unwrap_or_default()
                .into(),
        );
        state.set_has_cover(cover.is_some());
        state.set_cover(cover.unwrap_or_default());
        state.set_track_path(path.into());
        state.set_duration_ms(duration.as_millis() as i32);
        state.set_duration_text(format_duration(duration).into());
        state.set_position_ms(0);
        state.set_position_text(format_duration(Duration::ZERO).into());
    }

    fn set_volume(&self, volume: f32, save: bool) {
        let volume = volume.clamp(0.0, 1.0);
        self.player.send(Command::SetVolume(volume));
        if let Some(window) = self.window.upgrade() {
            window.global::<PlayerState>().set_volume(volume);
        }
        if save {
            self.save(SETTING_VOLUME, &format!("{volume:.2}"));
        }
    }

    fn set_shuffle(&self, shuffle: Shuffle) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        state.set_shuffle(shuffle);
        self.player.send(Command::SetShuffle(shuffle_mode(
            shuffle,
            state.get_shuffle_kind(),
        )));
        self.save(
            SETTING_SHUFFLE,
            if shuffle == Shuffle::On { "on" } else { "off" },
        );
    }

    fn set_shuffle_kind(&self, kind: ShuffleKind) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        state.set_shuffle_kind(kind);
        if state.get_shuffle() == Shuffle::On {
            self.player
                .send(Command::SetShuffle(shuffle_mode(Shuffle::On, kind)));
        }
        self.save(
            SETTING_SHUFFLE_KIND,
            if kind == ShuffleKind::TrueRandom {
                "true-random"
            } else {
                "fewer-repeats"
            },
        );
    }

    fn set_repeat(&self, repeat: Repeat) {
        if let Some(window) = self.window.upgrade() {
            window.global::<PlayerState>().set_repeat(repeat);
        }
        self.player.send(Command::SetRepeat(repeat_mode(repeat)));
        self.save(
            SETTING_REPEAT,
            match repeat {
                Repeat::Off => "off",
                Repeat::All => "all",
                Repeat::One => "one",
            },
        );
    }

    fn setting(&self, key: &str) -> Option<String> {
        self.library.setting(key).unwrap_or_else(|error| {
            log::warn!("could not read setting {key}: {error}");
            None
        })
    }

    fn save(&self, key: &str, value: &str) {
        if let Err(error) = self.library.set_setting(key, value) {
            log::warn!("could not save setting {key}: {error}");
        }
    }
}

/// The queue entry for a track; artist and album ids keep shuffle from bunching them.
/// Tracks whose path is not valid Unicode can't be named to the engine and are left out.
fn entry(track: &Track) -> Option<QueueEntry> {
    let Some(path) = track.path.to_str() else {
        log::warn!(
            "left out of the queue, its path is not valid Unicode: {}",
            track.path.display()
        );
        return None;
    };
    let mut entry = QueueEntry::new(TrackRef::local(path));
    if let Some(artist) = track.artist_id {
        entry = entry.with_artist(artist.0.to_string());
    }
    if let Some(album) = track.album_id {
        entry = entry.with_album(album.0.to_string());
    }
    Some(entry)
}

fn shuffle_mode(shuffle: Shuffle, kind: ShuffleKind) -> ShuffleMode {
    match (shuffle, kind) {
        (Shuffle::Off, _) => ShuffleMode::Off,
        (Shuffle::On, ShuffleKind::TrueRandom) => ShuffleMode::True,
        (Shuffle::On, ShuffleKind::FewerRepeats) => ShuffleMode::LessRepeats,
    }
}

fn repeat_mode(repeat: Repeat) -> RepeatMode {
    match repeat {
        Repeat::Off => RepeatMode::Off,
        Repeat::All => RepeatMode::All,
        Repeat::One => RepeatMode::One,
    }
}
