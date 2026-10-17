//! Playback behind the `PlayerState` global: the player bar's controls, playing an
//! album from its page, and the engine's events turned into what the bar shows.
//! Volume, shuffle, repeat, the output device and the queue itself are kept in the
//! library's settings, so they come back after a restart. The queue panel shows Up
//! next and what follows. The playing track also goes to the Discord presence, with
//! a cover looked up on MusicBrainz for albums whose files carry no release ID.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};
use vcmp_library::{Album, AlbumId, ArtistId, CoverArtId, CoverLookup, Library, Track};
use vcmp_playback::{
    Command, Event, NowPlaying, PlaybackError, PlaybackSnapshot, PlaybackState, Player, QueueEntry,
    QueueView, RepeatMode, ShuffleMode, TrackRef,
};

use vcmp_integrations::discord::{DiscordPresence, Listening};
use vcmp_integrations::musicbrainz::{AlbumQuery, Cover, CoverFinder, Lookup};

use crate::albums::initial;
use crate::library_view::{LibraryView, format_duration};
use crate::saved_queue;
use crate::{
    AppWindow, PlayState, PlayerProblem, PlayerState, QueueRow, QueueSource, Repeat, Shuffle,
    ShuffleKind,
};

pub const SETTING_VOLUME: &str = "playback.volume";
pub const SETTING_SHUFFLE: &str = "playback.shuffle";
pub const SETTING_SHUFFLE_KIND: &str = "playback.shuffle_kind";
pub const SETTING_REPEAT: &str = "playback.repeat";
/// The context origins of tracks played from search results and from the Songs page.
const SEARCH_ORIGIN: &str = "search";
const SONGS_ORIGIN: &str = "songs";

/// The output device by name; absent or empty for the system default.
pub const SETTING_OUTPUT_DEVICE: &str = "playback.output_device";

/// Where commands go: the engine, or a recorder in tests.
pub trait PlayerControl {
    fn send(&self, command: Command);
    /// The queue and position now, for saving.
    fn snapshot(&self) -> Option<PlaybackSnapshot> {
        None
    }
}

impl PlayerControl for Player {
    fn send(&self, command: Command) {
        Player::send(self, command);
    }

    fn snapshot(&self) -> Option<PlaybackSnapshot> {
        Player::snapshot(self)
    }
}

/// Lists the output devices; slow on some systems, so it runs off the UI thread.
pub type DeviceLister = fn() -> Result<Vec<String>, PlaybackError>;

/// Where "now listening" goes: Discord, or a recorder in tests.
pub trait PresenceSink {
    fn show(&self, listening: Listening);
    fn clear(&self);
}

impl PresenceSink for DiscordPresence {
    fn show(&self, listening: Listening) {
        DiscordPresence::show(self, listening);
    }

    fn clear(&self) {
        DiscordPresence::clear(self);
    }
}

/// Where cover lookups go: MusicBrainz, or a recorder in tests. The answer comes
/// back through `PlayerService::cover_found`.
pub trait CoverSearch {
    fn find(&self, album: AlbumId, query: AlbumQuery);
}

impl CoverSearch for CoverFinder<AlbumId> {
    fn find(&self, album: AlbumId, query: AlbumQuery) {
        CoverFinder::find(self, album, query);
    }
}

/// The Discord presence, and the lookup that finds covers for it.
pub struct Presence {
    pub sink: Box<dyn PresenceSink>,
    pub covers: Box<dyn CoverSearch>,
}

/// Setting key: show the playing track in Discord ("true" or "false"; absent: true).
pub const SETTING_DISCORD: &str = "integrations.discord";

/// An album nothing was found for is looked up again after this long, as MusicBrainz
/// keeps growing.
pub const RETRY_NOT_FOUND: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// A lookup that failed (offline, a server error) waits this long before the next.
const RETRY_FAILED: Duration = Duration::from_secs(10 * 60);

pub struct PlayerService {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    view: Rc<LibraryView>,
    player: Box<dyn PlayerControl>,
    // The position asked for by the last seek and when, until the engine reports it.
    seek_target: Cell<Option<(Duration, Instant)>>,
    // Muted keeps the volume to come back to; the engine plays at 0 meanwhile.
    muted: Cell<bool>,
    presence: Option<Presence>,
    // What the presence shows while playing, and where in the track playback is.
    listening: RefCell<Option<Listening>>,
    playing: Cell<bool>,
    position: Cell<Duration>,
    // Covers are only looked up while Discord is there to show them.
    discord_connected: Cell<bool>,
    // The playing track's album, and the albums being looked up or that just failed.
    album: Cell<Option<AlbumId>>,
    pending_covers: RefCell<HashSet<AlbumId>>,
    failed_covers: RefCell<HashMap<AlbumId, Instant>>,
    // Output devices: the listed names, and how to list them.
    devices: RefCell<Vec<String>>,
    list_devices: Cell<DeviceLister>,
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
        presence: Option<Presence>,
    ) -> Rc<Self> {
        let service = Rc::new(Self {
            window: window.as_weak(),
            library,
            view,
            player,
            seek_target: Cell::default(),
            muted: Cell::new(false),
            presence,
            listening: RefCell::default(),
            playing: Cell::new(false),
            position: Cell::default(),
            discord_connected: Cell::new(false),
            album: Cell::default(),
            pending_covers: RefCell::default(),
            failed_covers: RefCell::default(),
            devices: RefCell::default(),
            list_devices: Cell::new(vcmp_playback::output_devices),
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
        state.set_discord_available(service.presence.is_some());
        state.set_discord(service.setting(SETTING_DISCORD).as_deref() != Some("false"));
        // Until the devices are listed, the chosen one is the only name known.
        service.set_output_devices(Ok(Vec::new()));
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
                    service.seek(Duration::from_millis(ms.max(0) as u64));
                }
            }
        });
        state.on_toggle_mute({
            let weak = weak.clone();
            move || {
                if let Some(service) = weak.upgrade() {
                    service.toggle_mute();
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
        state.on_set_discord({
            let weak = weak.clone();
            move |enabled| {
                if let Some(service) = weak.upgrade() {
                    service.set_discord(enabled);
                }
            }
        });
        state.on_play_queue_entry({
            let weak = weak.clone();
            move |index| {
                if let (Some(service), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    service.player.send(Command::SkipTo(index));
                }
            }
        });
        state.on_remove_from_up_next({
            let weak = weak.clone();
            move |index| {
                if let (Some(service), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    service.player.send(Command::RemoveFromUpNext(index));
                }
            }
        });
        state.on_move_in_up_next({
            let weak = weak.clone();
            move |from, to| {
                if let (Some(service), Ok(from), Ok(to)) =
                    (weak.upgrade(), usize::try_from(from), usize::try_from(to))
                {
                    service.player.send(Command::MoveInUpNext { from, to });
                }
            }
        });
        state.on_clear_up_next(with(|service| service.player.send(Command::ClearUpNext)));
        state.on_set_output_device({
            let weak = weak.clone();
            move |index| {
                if let Some(service) = weak.upgrade() {
                    service.set_output_device(index);
                }
            }
        });
        state.on_refresh_output_devices(with(|service| service.refresh_output_devices()));
        window.on_play_artist({
            let weak = weak.clone();
            move |shuffle| {
                if let Some(service) = weak.upgrade() {
                    service.play_artist(None, shuffle);
                }
            }
        });
        window.on_play_artist_appearance({
            let weak = weak.clone();
            move |index| {
                if let (Some(service), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    service.play_artist(Some(index), false);
                }
            }
        });
        window.on_queue_artist_appearance({
            let weak = weak.clone();
            move |index, next| {
                if let Some(service) = weak.upgrade() {
                    service.queue_artist_appearance(index, next);
                }
            }
        });
        window.on_play_search_track({
            let weak = weak.clone();
            move |index| {
                if let (Some(service), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    service.play_found(index);
                }
            }
        });
        window.on_queue_search_track({
            let weak = weak.clone();
            move |index, next| {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                let tracks = service.view.found_tracks();
                let track = usize::try_from(index)
                    .ok()
                    .and_then(|index| tracks.get(index));
                if let Some(entry) = track.and_then(entry) {
                    service.player.send(if next {
                        Command::PlayNext(vec![entry])
                    } else {
                        Command::AddToQueue(vec![entry])
                    });
                }
            }
        });
        window.on_play_song({
            let weak = weak.clone();
            move |index| {
                if let (Some(service), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    service.play_songs(index);
                }
            }
        });
        window.on_queue_song({
            let weak = weak.clone();
            move |index, next| {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                let track = usize::try_from(index)
                    .ok()
                    .and_then(|index| service.view.song(index));
                if let Some(entry) = track.as_ref().and_then(entry) {
                    service.player.send(if next {
                        Command::PlayNext(vec![entry])
                    } else {
                        Command::AddToQueue(vec![entry])
                    });
                }
            }
        });
        window.on_queue_album_track({
            let weak = weak.clone();
            move |index, next| {
                if let Some(service) = weak.upgrade() {
                    service.queue_album_track(index, next);
                }
            }
        });
        window.on_play_album(move |start, shuffle| {
            if let Some(service) = weak.upgrade() {
                service.play_album(start, shuffle);
            }
        });
        service.restore_queue();
        service
    }

    /// Lists output devices with `lister` instead of asking the system (tests).
    pub fn set_device_lister(&self, lister: DeviceLister) {
        self.list_devices.set(lister);
    }

    /// Saves the queue and position, to come back to after a restart.
    pub fn save_queue(&self) {
        let Some(snapshot) = self.player.snapshot() else {
            return;
        };
        if let Err(error) = self
            .library
            .set_setting(saved_queue::SETTING, &saved_queue::encode(&snapshot))
        {
            log::warn!("could not save the queue: {error}");
        }
    }

    /// Loads the saved queue, paused at the saved position.
    fn restore_queue(&self) {
        let Some(text) = self.setting(saved_queue::SETTING) else {
            return;
        };
        match saved_queue::decode(&text) {
            Ok(snapshot) if snapshot.queue.current.is_some() => {
                self.player.send(Command::Restore(snapshot));
            }
            Ok(_) => {}
            Err(error) => log::warn!("the saved queue could not be read: {error}"),
        }
    }

    /// Plays everything by the open artist: their albums, then their appearances
    /// elsewhere; from appearance `from` when given.
    pub fn play_artist(&self, from: Option<usize>, shuffle: bool) {
        let Some(artist) = self.view.shown_artist() else {
            return;
        };
        let start = from.map(|index| artist.appearances_from + index);
        let entries: Vec<QueueEntry> = artist.tracks.iter().filter_map(entry).collect();
        if shuffle {
            self.set_shuffle(Shuffle::On);
        }
        self.player.send(Command::PlayContext {
            entries,
            origin: Some(format!("artist:{}", artist.artist.id.0)),
            start: if shuffle { None } else { start },
        });
    }

    /// Plays the tracks the search found, from track `start`.
    pub fn play_found(&self, start: usize) {
        let entries: Vec<QueueEntry> = self.view.found_tracks().iter().filter_map(entry).collect();
        self.player.send(Command::PlayContext {
            entries,
            origin: Some(SEARCH_ORIGIN.to_string()),
            start: Some(start),
        });
    }

    /// Plays every song in the Songs page's order, from song `start`.
    pub fn play_songs(&self, start: usize) {
        let entries: Vec<QueueEntry> = self.view.songs().iter().filter_map(entry).collect();
        self.player.send(Command::PlayContext {
            entries,
            origin: Some(SONGS_ORIGIN.to_string()),
            start: Some(start),
        });
    }

    /// Puts appearance `index` of the open artist into Up next.
    pub fn queue_artist_appearance(&self, index: i32, next: bool) {
        let Some(artist) = self.view.shown_artist() else {
            return;
        };
        let track = usize::try_from(index)
            .ok()
            .and_then(|index| artist.tracks.get(artist.appearances_from + index));
        if let Some(entry) = track.and_then(entry) {
            self.player.send(if next {
                Command::PlayNext(vec![entry])
            } else {
                Command::AddToQueue(vec![entry])
            });
        }
    }

    /// Puts track `index` of the open album into Up next: at its front, or its end.
    pub fn queue_album_track(&self, index: i32, next: bool) {
        let Some(album) = self.view.shown_album() else {
            return;
        };
        let track = match self.library.album_tracks(album.id) {
            Ok(tracks) => usize::try_from(index)
                .ok()
                .and_then(|index| tracks.into_iter().nth(index)),
            Err(error) => {
                log::error!("could not read the tracks of {:?}: {error}", album.title);
                return;
            }
        };
        if let Some(entry) = track.as_ref().and_then(entry) {
            self.player.send(if next {
                Command::PlayNext(vec![entry])
            } else {
                Command::AddToQueue(vec![entry])
            });
        }
    }

    /// Takes a fresh list of output devices.
    pub fn set_output_devices(&self, devices: Result<Vec<String>, PlaybackError>) {
        let mut devices = devices.unwrap_or_else(|error| {
            log::warn!("could not list the output devices: {error}");
            Vec::new()
        });
        // The chosen device stays choosable while it is unplugged.
        if let Some(chosen) = self.saved_output_device()
            && !devices.contains(&chosen)
        {
            devices.push(chosen);
        }
        *self.devices.borrow_mut() = devices;
        self.show_devices();
    }

    fn refresh_output_devices(&self) {
        let lister = self.list_devices.get();
        thread::Builder::new()
            .name("vcmp-output-devices".into())
            .spawn(move || {
                let devices = lister();
                Self::post(move |service| service.set_output_devices(devices));
            })
            .expect("spawning the device listing thread");
    }

    fn set_output_device(&self, index: i32) {
        let device = usize::try_from(index)
            .ok()
            .and_then(|index| self.devices.borrow().get(index).cloned());
        self.save(SETTING_OUTPUT_DEVICE, device.as_deref().unwrap_or(""));
        self.player.send(Command::SetOutputDevice(device));
        if let Some(window) = self.window.upgrade() {
            window
                .global::<PlayerState>()
                .set_output_fallback("".into());
        }
        self.show_devices();
    }

    fn saved_output_device(&self) -> Option<String> {
        self.setting(SETTING_OUTPUT_DEVICE)
            .filter(|device| !device.is_empty())
    }

    fn show_devices(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        let devices = self.devices.borrow();
        let chosen = self.saved_output_device();
        let index = chosen
            .and_then(|chosen| devices.iter().position(|device| *device == chosen))
            .map_or(-1, |index| index as i32);
        let names: Vec<SharedString> = devices.iter().map(SharedString::from).collect();
        state.set_output_devices(ModelRc::new(VecModel::from(names)));
        state.set_output_device(index);
    }

    /// Fills the queue panel.
    fn show_queue(&self, state: &PlayerState, view: &QueueView) {
        let rows = |entries: &[QueueEntry]| -> ModelRc<QueueRow> {
            ModelRc::new(VecModel::from(
                entries
                    .iter()
                    .map(|entry| self.queue_row(entry))
                    .collect::<Vec<_>>(),
            ))
        };
        state.set_up_next(rows(&view.up_next));
        state.set_upcoming(rows(&view.upcoming));
        let origin = view
            .origin
            .as_deref()
            .and_then(|origin| origin.split_once(':'))
            .and_then(|(kind, id)| Some((kind, id.parse::<i64>().ok()?)))
            .and_then(|(kind, id)| match kind {
                "album" => self
                    .library
                    .album(AlbumId(id))
                    .ok()
                    .flatten()
                    .map(|album| album.title),
                "artist" => self
                    .library
                    .artist(ArtistId(id))
                    .ok()
                    .flatten()
                    .map(|artist| artist.name),
                _ => None,
            })
            .unwrap_or_default();
        state.set_queue_origin(origin.into());
        state.set_queue_source(match view.origin.as_deref() {
            Some(SEARCH_ORIGIN) => QueueSource::Search,
            Some(SONGS_ORIGIN) => QueueSource::Songs,
            _ => QueueSource::Named,
        });
    }

    fn queue_row(&self, entry: &QueueEntry) -> QueueRow {
        let path = Path::new(entry.track.id.as_ref());
        let track = self
            .library
            .track_id_for_path(path)
            .ok()
            .flatten()
            .and_then(|id| self.library.track(id).ok().flatten());
        match track {
            Some(track) => QueueRow {
                title: track.title.into(),
                artist: track.artist.unwrap_or_default().into(),
                duration: format_duration(track.duration).into(),
            },
            None => QueueRow {
                title: path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default()
                    .into(),
                artist: SharedString::default(),
                duration: SharedString::default(),
            },
        }
    }

    /// Forwards the engine's events to the UI thread for as long as the engine runs.
    pub fn listen(events: Receiver<Event>) {
        thread::Builder::new()
            .name("vcmp-player-events".into())
            .spawn(move || {
                for event in events {
                    if !Self::post(move |service| service.handle_event(event)) {
                        break;
                    }
                }
            })
            .expect("spawning the player event thread");
    }

    /// Runs `action` with the service on the UI thread, from any thread. False once
    /// the event loop is gone.
    pub fn post(action: impl FnOnce(&PlayerService) + Send + 'static) -> bool {
        slint::invoke_from_event_loop(move || {
            if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                action(&service);
            }
        })
        .is_ok()
    }

    /// Discord was reached or lost.
    pub fn set_discord_connected(&self, connected: bool) {
        self.discord_connected.set(connected);
        self.look_up_cover();
    }

    /// Takes the answer to a cover lookup: stores it, and shows a found cover if its
    /// album is still playing.
    pub fn cover_found(&self, album: AlbumId, lookup: Lookup) {
        self.pending_covers.borrow_mut().remove(&album);
        let cover = match lookup {
            Lookup::Found(cover) => Some(cover),
            Lookup::NotFound => None,
            Lookup::Failed(error) => {
                log::info!("the cover lookup failed: {error}");
                self.failed_covers
                    .borrow_mut()
                    .insert(album, Instant::now());
                return;
            }
        };
        let stored = cover.as_ref().map(|cover| match cover {
            Cover::Release(id) => CoverArtId::Release(id.clone()),
            Cover::ReleaseGroup(id) => CoverArtId::ReleaseGroup(id.clone()),
        });
        if let Err(error) = self
            .library
            .set_cover_lookup(album, stored.as_ref(), SystemTime::now())
        {
            log::warn!("could not store the cover lookup: {error}");
        }
        let Some(cover) = cover else {
            return;
        };
        if self.album.get() == Some(album) {
            if let Some(listening) = self.listening.borrow_mut().as_mut() {
                listening.cover_url = Some(cover.url());
            }
            self.update_presence();
        }
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
            Event::StateChanged(playback) => {
                state.set_state(match playback {
                    PlaybackState::Playing => PlayState::Playing,
                    PlaybackState::Paused => PlayState::Paused,
                    PlaybackState::Stopped => PlayState::Stopped,
                });
                self.playing.set(playback == PlaybackState::Playing);
                self.update_presence();
            }
            Event::TrackChanged(now) => {
                let playing = now.is_some();
                self.show_track(&state, now);
                if playing {
                    self.save_queue();
                }
            }
            Event::Position(position) => {
                if let Some((target, asked)) = self.seek_target.get() {
                    // Positions sent before the engine took the seek would jump the
                    // bar back for a moment.
                    if asked.elapsed() < SEEK_SETTLE && position.abs_diff(target) > SEEK_TOLERANCE {
                        return;
                    }
                    self.seek_target.set(None);
                    // Discord's progress bar has to jump with the seek.
                    self.position.set(position);
                    self.update_presence();
                }
                self.position.set(position);
                show_position(&state, position);
            }
            Event::QueueChanged(queue) => {
                self.show_queue(&state, &queue);
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
                state.set_output_fallback(if info.fell_back {
                    info.device.into()
                } else {
                    SharedString::default()
                });
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
            *self.listening.borrow_mut() = None;
            self.album.set(None);
            self.update_presence();
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
        let album = track
            .as_ref()
            .and_then(|track| track.album_id)
            .and_then(|album| self.library.album(album).ok().flatten());
        let cover = album
            .as_ref()
            .and_then(|album| album.artwork.as_ref())
            .and_then(|artwork| Image::load_from_path(&artwork.thumbnail).ok());
        let duration = now
            .duration
            .or(track.as_ref().map(|track| track.duration))
            .unwrap_or_default();

        *self.listening.borrow_mut() = Some(Listening {
            title: title.clone(),
            artist: track.as_ref().and_then(|track| track.artist.clone()),
            album: album.as_ref().map(|album| album.title.clone()),
            cover_url: album.as_ref().and_then(|album| self.known_cover(album)),
            position: Duration::ZERO,
            duration: (!duration.is_zero()).then_some(duration),
        });
        self.album.set(album.map(|album| album.id));
        self.position.set(Duration::ZERO);
        self.update_presence();

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
        self.seek_target.set(None);
        show_position(state, Duration::ZERO);
    }

    /// Shows the track in Discord while it plays (and the presence is on); clears it
    /// otherwise.
    fn update_presence(&self) {
        let Some(presence) = &self.presence else {
            return;
        };
        match self.listening.borrow().as_ref() {
            Some(listening) if self.discord_enabled() && self.playing.get() => {
                presence.sink.show(Listening {
                    position: self.position.get(),
                    ..listening.clone()
                });
            }
            _ => presence.sink.clear(),
        }
        self.look_up_cover();
    }

    fn discord_enabled(&self) -> bool {
        self.window
            .upgrade()
            .is_some_and(|window| window.global::<PlayerState>().get_discord())
    }

    /// The cover of `album` on the web: from its tags, or from an earlier lookup.
    fn known_cover(&self, album: &Album) -> Option<String> {
        if let Some(release) = &album.musicbrainz_id {
            return Some(Cover::Release(release.clone()).url());
        }
        match self.library.cover_lookup(album.id) {
            Ok(lookup) => lookup?.cover.map(|cover| match cover {
                CoverArtId::Release(id) => Cover::Release(id).url(),
                CoverArtId::ReleaseGroup(id) => Cover::ReleaseGroup(id).url(),
            }),
            Err(error) => {
                log::warn!("could not read the cover lookup: {error}");
                None
            }
        }
    }

    /// Asks MusicBrainz for the playing album's cover when Discord shows the track
    /// without one and the album was not looked up yet (or long ago, or failed a
    /// while back).
    fn look_up_cover(&self) {
        let Some(presence) = &self.presence else {
            return;
        };
        let Some(album) = self.album.get() else {
            return;
        };
        let shown_without_cover = self
            .listening
            .borrow()
            .as_ref()
            .is_some_and(|listening| listening.cover_url.is_none());
        if !(shown_without_cover
            && self.playing.get()
            && self.discord_connected.get()
            && self.discord_enabled())
            || self.pending_covers.borrow().contains(&album)
            || self
                .failed_covers
                .borrow()
                .get(&album)
                .is_some_and(|failed| failed.elapsed() < RETRY_FAILED)
        {
            return;
        }
        match self.library.cover_lookup(album) {
            Ok(None) => {}
            Ok(Some(CoverLookup {
                cover: None,
                looked_up,
            })) if looked_up.elapsed().is_ok_and(|age| age >= RETRY_NOT_FOUND) => {}
            Ok(Some(_)) => return,
            Err(error) => {
                log::warn!("could not read the cover lookup: {error}");
                return;
            }
        }
        let Some(Album {
            title,
            artist: Some(artist),
            track_count,
            musicbrainz_id: None,
            ..
        }) = self.library.album(album).ok().flatten()
        else {
            return;
        };
        self.pending_covers.borrow_mut().insert(album);
        presence.covers.find(
            album,
            AlbumQuery {
                artist,
                title,
                track_count,
            },
        );
    }

    fn set_discord(&self, enabled: bool) {
        if let Some(window) = self.window.upgrade() {
            window.global::<PlayerState>().set_discord(enabled);
        }
        self.save(SETTING_DISCORD, if enabled { "true" } else { "false" });
        self.update_presence();
    }

    /// Seeks, and shows the new position at once instead of waiting for the engine.
    fn seek(&self, position: Duration) {
        self.player.send(Command::Seek(position));
        self.seek_target.set(Some((position, Instant::now())));
        if let Some(window) = self.window.upgrade() {
            show_position(&window.global::<PlayerState>(), position);
        }
    }

    /// Moving the slider while muted unmutes at the new volume.
    fn set_volume(&self, volume: f32, save: bool) {
        let volume = volume.clamp(0.0, 1.0);
        self.player.send(Command::SetVolume(volume));
        self.muted.set(false);
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<PlayerState>();
            state.set_volume(volume);
            state.set_muted(false);
        }
        if save {
            self.save(SETTING_VOLUME, &format!("{volume:.2}"));
        }
    }

    fn toggle_mute(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        let muted = !self.muted.get();
        self.muted.set(muted);
        state.set_muted(muted);
        self.player.send(Command::SetVolume(if muted {
            0.0
        } else {
            state.get_volume()
        }));
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

/// How long after a seek stale positions are ignored, and how far from the target a
/// position may be and still count as the seek's own.
const SEEK_SETTLE: Duration = Duration::from_secs(1);
const SEEK_TOLERANCE: Duration = Duration::from_millis(1500);

fn show_position(state: &PlayerState, position: Duration) {
    state.set_position_ms(position.as_millis() as i32);
    state.set_position_text(format_duration(position).into());
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
