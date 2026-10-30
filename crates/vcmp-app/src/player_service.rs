//! Playback behind the `PlayerState` global: the player bar's controls, playing an
//! album from its page, and the engine's events turned into what the bar shows.
//! Volume, shuffle, repeat, the output device and the queue itself are kept in the
//! library's settings, so they come back after a restart. The queue panel shows Up
//! next and what follows. The playing track also goes to the Discord presence, with
//! a cover looked up on MusicBrainz for albums whose files carry no release ID,
//! and to the OS media controls, whose media keys and widget drive the player.
//! Plays go to Last.fm once they count by its rules, and so does the track playing.
//! The file the engine has open is passed on, so tag edits to it can wait for it.
//! The sleep timer counts down here and then has the engine stop after the track
//! playing at that moment. AI agents control playback through `agent_playback`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use slint::{
    ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer,
    TimerMode, VecModel,
};
use vcmp_library::{
    Album, AlbumId, ArtistId, Artwork, CoverArtId, CoverLookup, LabelId, Library, PlaylistId, Track,
};
use vcmp_playback::{
    Command, Event, NowPlaying, PlaybackError, PlaybackSnapshot, PlaybackState, Player, QueueEntry,
    QueueView, RepeatMode, ShuffleMode, TrackRef,
};

use vcmp_integrations::discord::{DiscordPresence, Listening};
use vcmp_integrations::lastfm::{self, Play};
use vcmp_integrations::media_controls::{self, Control, MediaControls, MediaState, MediaTrack};
use vcmp_integrations::musicbrainz::{AlbumQuery, Cover, CoverFinder, Lookup};

mod agent_playback;

pub use agent_playback::AppPlayback;

use crate::albums::initial;
use crate::library_view::{LibraryView, format_duration};
use crate::moving_pictures::{self, Picture};
use crate::saved_queue;
use crate::selection::{self, Selectable};
use crate::{
    AppWindow, PlayState, PlayerProblem, PlayerState, QueueRow, QueueSource, Repeat, ScrobbleMark,
    Shuffle, ShuffleKind, SleepTimer,
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

/// Where the system's media widget learns what plays: the OS media controls, or a
/// recorder in tests.
pub trait MediaSink {
    fn update(&self, state: MediaState);
}

impl MediaSink for MediaControls {
    fn update(&self, state: MediaState) {
        MediaControls::update(self, state);
    }
}

/// Where plays go for Last.fm: the Last.fm service, or a recorder in tests.
pub trait ScrobbleSink {
    /// A track started playing, or resumed.
    fn now_playing(&self, track: lastfm::Track);
    /// A play counts by Last.fm's rules.
    fn scrobble(&self, play: lastfm::Scrobble);
}

impl<T: ScrobbleSink + ?Sized> ScrobbleSink for Rc<T> {
    fn now_playing(&self, track: lastfm::Track) {
        (**self).now_playing(track);
    }

    fn scrobble(&self, play: lastfm::Scrobble) {
        (**self).scrobble(play);
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

/// `PlayerState.set-sleep-timer`'s value for stopping after the current track.
pub const SLEEP_END_OF_TRACK: i32 = -1;
/// The sleep timer counts down in whole minutes.
const SLEEP_TICK: Duration = Duration::from_secs(60);

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
    // The playing track's album and artist, and the albums being looked up or that
    // just failed.
    album: Cell<Option<AlbumId>>,
    artist: Cell<Option<ArtistId>>,
    pending_covers: RefCell<HashSet<AlbumId>>,
    failed_covers: RefCell<HashMap<AlbumId, Instant>>,
    // Output devices: the listed names, and how to list them.
    devices: RefCell<Vec<String>>,
    list_devices: Cell<DeviceLister>,
    // The OS media controls, once registered, and the track they show.
    media: RefCell<Option<Box<dyn MediaSink>>>,
    media_track: RefCell<Option<MediaTrack>>,
    // The playing track's full-size cover, for the now playing view, and whether it
    // is decoded or on its way.
    large_cover: RefCell<Option<PathBuf>>,
    large_cover_requested: Cell<bool>,
    // The queue as the engine last reported it, and the queue panel's small album
    // covers: decoded ones (None: the album has none), those on their way, and the
    // thread that decodes them.
    queue_view: RefCell<QueueView>,
    queue_covers: RefCell<HashMap<AlbumId, Option<Image>>>,
    queue_covers_pending: RefCell<HashSet<AlbumId>>,
    queue_cover_jobs: RefCell<Option<mpsc::Sender<(AlbumId, PathBuf)>>>,
    // Last.fm: where plays go, the playing track's play as Last.fm counts it, and
    // whether "now playing" went out since the track last started or resumed.
    scrobbles: RefCell<Option<Box<dyn ScrobbleSink>>>,
    play: RefCell<Option<(lastfm::Track, Play)>>,
    announced: Cell<bool>,
    // The file the engine has open (`Event::Decoding`), and who is told when it changes.
    open_file: RefCell<Option<PathBuf>>,
    on_open_file: RefCell<Option<OpenFileListener>>,
    // The file open when tags could not be written: the player bar says so until a
    // track after it starts.
    tags_notice_file: RefCell<Option<PathBuf>>,
    // The sleep timer's minute ticks, and the minutes left (0 while none counts down).
    sleep_timer: Timer,
    sleep_minutes: Cell<i32>,
}

/// Told the file the engine has open, or `None`.
type OpenFileListener = Box<dyn Fn(Option<&Path>)>;

/// Pixels the queue panel's covers are decoded at: 40 px rows at up to 2.4 times.
const QUEUE_COVER_PIXELS: u32 = 96;
/// Covers kept before the cache starts over.
const QUEUE_COVER_CACHE: usize = 500;

/// The longest side the now playing view's cover is decoded at.
pub(crate) const LARGE_COVER_PIXELS: u32 = 1200;

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
            artist: Cell::default(),
            pending_covers: RefCell::default(),
            failed_covers: RefCell::default(),
            devices: RefCell::default(),
            list_devices: Cell::new(vcmp_playback::output_devices),
            media: RefCell::default(),
            media_track: RefCell::default(),
            large_cover: RefCell::default(),
            large_cover_requested: Cell::new(false),
            queue_view: RefCell::default(),
            queue_covers: RefCell::default(),
            queue_covers_pending: RefCell::default(),
            queue_cover_jobs: RefCell::default(),
            scrobbles: RefCell::default(),
            play: RefCell::default(),
            announced: Cell::new(false),
            open_file: RefCell::default(),
            on_open_file: RefCell::default(),
            tags_notice_file: RefCell::default(),
            sleep_timer: Timer::default(),
            sleep_minutes: Cell::new(0),
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
        state.on_set_sleep_timer({
            let weak = weak.clone();
            move |minutes| {
                if let Some(service) = weak.upgrade() {
                    service.set_sleep_timer(minutes);
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
        state.on_queue_upcoming({
            let weak = weak.clone();
            move |index, next| {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                let entry = usize::try_from(index)
                    .ok()
                    .and_then(|index| service.queue_view.borrow().upcoming.get(index).cloned());
                if let Some(entry) = entry {
                    service.player.send(if next {
                        Command::PlayNext(vec![entry])
                    } else {
                        Command::AddToQueue(vec![entry])
                    });
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
        state.on_load_large_cover(with(|service| service.load_large_cover()));
        state.on_go_to_album(with(|service| {
            if let Some(album) = service.album.get() {
                service.view.open_album_by_id(album);
            }
        }));
        state.on_go_to_artist(with(|service| {
            if let Some(artist) = service.artist.get() {
                service.view.open_artist_by_id(artist);
            }
        }));
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
                if let Some(service) = weak.upgrade()
                    && let Some(window) = service.window.upgrade()
                {
                    let tracks = service.view.found_tracks();
                    service.queue_chosen(&window.get_search_tracks(), index, &tracks, next);
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
                if let Some(service) = weak.upgrade()
                    && let Some(window) = service.window.upgrade()
                {
                    let tracks = service.view.songs();
                    service.queue_chosen(&window.get_songs(), index, &tracks, next);
                }
            }
        });
        window.on_play_folder_file({
            let weak = weak.clone();
            move |index| {
                if let (Some(service), Ok(index)) = (weak.upgrade(), usize::try_from(index)) {
                    service.play_folder(index);
                }
            }
        });
        window.on_queue_folder_file({
            let weak = weak.clone();
            move |index, next| {
                if let Some(service) = weak.upgrade()
                    && let Some(window) = service.window.upgrade()
                {
                    let files = service.view.folders().files();
                    service.queue_chosen(&window.get_folder_files(), index, &files, next);
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
        window.on_play_playlist({
            let weak = weak.clone();
            move |start, shuffle| {
                if let Some(service) = weak.upgrade() {
                    service.play_playlist(start, shuffle);
                }
            }
        });
        window.on_queue_playlist_track({
            let weak = weak.clone();
            move |index, next| {
                if let Some(service) = weak.upgrade() {
                    service.queue_playlist_track(index, next);
                }
            }
        });
        window.on_play_label({
            let weak = weak.clone();
            move |start, shuffle| {
                if let Some(service) = weak.upgrade() {
                    service.play_label(start, shuffle);
                }
            }
        });
        window.on_queue_label_track({
            let weak = weak.clone();
            move |index, next| {
                if let Some(service) = weak.upgrade() {
                    service.queue_label_track(index, next);
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

    /// Shows the playing track in the system's media widget from now on.
    pub fn set_media_controls(&self, sink: Box<dyn MediaSink>) {
        *self.media.borrow_mut() = Some(sink);
        self.update_media();
    }

    /// Tells `listener` the file the engine has open, now and whenever that changes.
    /// On Windows that file can't be replaced, so the tag editor waits for it.
    pub fn on_open_file(&self, listener: impl Fn(Option<&Path>) + 'static) {
        listener(self.open_file.borrow().as_deref());
        *self.on_open_file.borrow_mut() = Some(Box::new(listener));
    }

    /// Says in the player bar that new tags for `path`, which waited for playback,
    /// could not be written.
    pub fn tags_not_written(&self, path: &Path) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        state.set_problem(PlayerProblem::TagsNotWritten);
        state.set_problem_detail(name.into());
        *self.tags_notice_file.borrow_mut() = self.open_file.borrow().clone();
    }

    /// Hands plays to Last.fm from now on.
    pub fn set_scrobbler(&self, sink: Box<dyn ScrobbleSink>) {
        *self.scrobbles.borrow_mut() = Some(sink);
        self.announce();
    }

    /// Does what a media key or the system's media widget asks.
    pub fn media_control(&self, control: Control) {
        match control {
            Control::Play => self.player.send(Command::Play),
            Control::Pause => self.player.send(Command::Pause),
            Control::TogglePlayPause => self.player.send(Command::TogglePlayPause),
            Control::Stop => self.player.send(Command::Stop),
            Control::Next => self.player.send(Command::Next),
            Control::Previous => self.player.send(Command::Previous),
            Control::SeekTo(position) => self.seek(position),
            Control::SeekBy { forward, by } => {
                let position = self.position.get();
                let mut target = if forward {
                    position + by
                } else {
                    position.saturating_sub(by)
                };
                if let Some(duration) = self.media_track.borrow().as_ref().and_then(|t| t.duration)
                {
                    target = target.min(duration);
                }
                self.seek(target);
            }
            Control::SetShuffle(on) => {
                self.set_shuffle(if on { Shuffle::On } else { Shuffle::Off })
            }
            Control::SetRepeat(repeat) => self.set_repeat(match repeat {
                media_controls::Repeat::Off => Repeat::Off,
                media_controls::Repeat::One => Repeat::One,
                media_controls::Repeat::All => Repeat::All,
            }),
            Control::SetVolume(volume) => self.set_volume(volume, true),
            Control::Raise => {
                if let Some(window) = self.window.upgrade() {
                    window.window().set_minimized(false);
                    let _ = window.show();
                }
            }
            Control::Quit => {
                let _ = slint::quit_event_loop();
            }
        }
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

    /// Plays the files of the folder shown in the folder browser, from file `start`.
    pub fn play_folder(&self, start: usize) {
        let folders = self.view.folders();
        let Some(folder) = folders.folder() else {
            return;
        };
        let entries: Vec<QueueEntry> = folders.files().iter().filter_map(entry).collect();
        self.player.send(Command::PlayContext {
            entries,
            origin: Some(format!("folder:{}", folder.to_string_lossy())),
            start: Some(start),
        });
    }

    /// Plays the open playlist from entry `start` (-1: from its beginning), or
    /// shuffled. Entries whose file is gone are left out; starting at one plays from
    /// the next that is there.
    pub fn play_playlist(&self, start: i32, shuffle: bool) {
        let Some(playlist) = self.view.playlists().shown() else {
            return;
        };
        let start = usize::try_from(start).ok();
        let first = start.map(|start| {
            playlist.entries[..start.min(playlist.entries.len())]
                .iter()
                .filter(|item| item.track.available)
                .count()
        });
        let entries: Vec<QueueEntry> = playlist
            .entries
            .iter()
            .filter(|item| item.track.available)
            .filter_map(|item| entry(&item.track))
            .collect();
        if first.is_some_and(|first| first >= entries.len()) {
            return;
        }
        if shuffle {
            self.set_shuffle(Shuffle::On);
        }
        self.player.send(Command::PlayContext {
            entries,
            origin: Some(format!("playlist:{}", playlist.id.0)),
            start: if shuffle { None } else { first },
        });
    }

    /// Puts entry `index` of the open playlist (or the selected entries, when it is one
    /// of them) into Up next; entries whose file is gone are left out.
    pub fn queue_playlist_track(&self, index: i32, next: bool) {
        let (Some(window), Some(playlist)) = (self.window.upgrade(), self.view.playlists().shown())
        else {
            return;
        };
        let tracks: Vec<Track> = playlist
            .entries
            .into_iter()
            .map(|entry| entry.track)
            .collect();
        self.queue_chosen(&window.get_playlist_tracks(), index, &tracks, next);
    }

    /// Plays the open label's tracks from track `start` (-1: from the first), or
    /// shuffled, as a context of its own named after the label.
    pub fn play_label(&self, start: i32, shuffle: bool) {
        let Some(label) = self.view.labels().shown() else {
            return;
        };
        let entries: Vec<QueueEntry> = label.tracks.iter().filter_map(entry).collect();
        let start = usize::try_from(start).ok();
        if entries.is_empty() || start.is_some_and(|start| start >= entries.len()) {
            return;
        }
        if shuffle {
            self.set_shuffle(Shuffle::On);
        }
        self.player.send(Command::PlayContext {
            entries,
            origin: Some(format!("label:{}", label.id.0)),
            start: if shuffle { None } else { start },
        });
    }

    /// Puts track `index` of the open label (or the selected tracks, when it is one
    /// of them) into Up next.
    pub fn queue_label_track(&self, index: i32, next: bool) {
        let (Some(window), Some(label)) = (self.window.upgrade(), self.view.labels().shown())
        else {
            return;
        };
        self.queue_chosen(&window.get_label_tracks(), index, &label.tracks, next);
    }

    /// Puts the tracks a list's menu acts on into Up next, at its front (`next`) or
    /// its end: the list's selected tracks when row `index` is one of them, or else
    /// that track. `tracks` are the list's tracks in its rows' order.
    fn queue_chosen<T: Selectable>(
        &self,
        rows: &ModelRc<T>,
        index: i32,
        tracks: &[Track],
        next: bool,
    ) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let entries: Vec<QueueEntry> = selection::chosen(rows, index)
            .into_iter()
            .filter_map(|row| tracks.get(row))
            .filter(|track| track.available)
            .filter_map(entry)
            .collect();
        if entries.is_empty() {
            return;
        }
        self.player.send(if next {
            Command::PlayNext(entries)
        } else {
            Command::AddToQueue(entries)
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

    /// Puts appearance `index` of the open artist (or the selected appearances) into
    /// Up next.
    pub fn queue_artist_appearance(&self, index: i32, next: bool) {
        let (Some(window), Some(artist)) = (self.window.upgrade(), self.view.shown_artist()) else {
            return;
        };
        let appearances = &artist.tracks[artist.appearances_from..];
        self.queue_chosen(&window.get_artist_appearances(), index, appearances, next);
    }

    /// Puts track `index` of the open album (or its selected tracks) into Up next: at
    /// its front, or its end.
    pub fn queue_album_track(&self, index: i32, next: bool) {
        let (Some(window), Some(album)) = (self.window.upgrade(), self.view.shown_album()) else {
            return;
        };
        match self.library.album_tracks(album.id) {
            Ok(tracks) => self.queue_chosen(&window.get_album_tracks(), index, &tracks, next),
            Err(error) => log::error!("could not read the tracks of {:?}: {error}", album.title),
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

    /// Decodes the playing track's full-size cover off the UI thread, for the now
    /// playing view; the bar's small cover stands in until it arrives.
    fn load_large_cover(&self) {
        if self.large_cover_requested.replace(true) {
            return;
        }
        let Some(path) = self.large_cover.borrow().clone() else {
            return;
        };
        let wanted = path.clone();
        thread::Builder::new()
            .name("vcmp-large-cover".into())
            .spawn(move || {
                let Ok(image) = image::open(&path) else {
                    log::info!("could not decode the cover {}", path.display());
                    return;
                };
                let rgba = image
                    .thumbnail(LARGE_COVER_PIXELS, LARGE_COVER_PIXELS)
                    .into_rgba8();
                let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                );
                Self::post(move |service| service.show_large_cover(&wanted, buffer));
            })
            .expect("spawning the cover thread");
    }

    fn show_large_cover(&self, path: &Path, buffer: SharedPixelBuffer<Rgba8Pixel>) {
        // The track changed while the cover was decoded.
        if self.large_cover.borrow().as_deref() != Some(path) {
            return;
        }
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<PlayerState>();
            state.set_large_cover(Image::from_rgba8(buffer));
            state.set_has_large_cover(true);
        }
    }

    /// Makes `artwork` the cover the now playing view shows, decoded once the view
    /// opens, and its animation, if it has one, what moves there.
    fn use_large_cover(&self, state: &PlayerState, artwork: Option<&Artwork>) {
        self.forget_large_cover(state);
        *self.large_cover.borrow_mut() = artwork.map(|artwork| artwork.original.clone());
        if let Some(window) = self.window.upgrade() {
            let animation = artwork.and_then(|artwork| self.library.cover_animation(artwork));
            moving_pictures::show(&window, Picture::NowPlaying, animation);
        }
        if state.get_now_playing_open() {
            self.load_large_cover();
        }
    }

    fn forget_large_cover(&self, state: &PlayerState) {
        *self.large_cover.borrow_mut() = None;
        self.large_cover_requested.set(false);
        state.set_has_large_cover(false);
        state.set_large_cover(Image::default());
    }

    /// Fills the queue panel.
    fn show_queue(&self, state: &PlayerState, view: &QueueView) {
        *self.queue_view.borrow_mut() = view.clone();
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
        let next = view
            .up_next
            .first()
            .or(view.upcoming.first())
            .map(|entry| self.queue_row(entry));
        state.set_next_title(
            next.as_ref()
                .map(|row| row.title.clone())
                .unwrap_or_default(),
        );
        state.set_next_artist(next.map(|row| row.artist).unwrap_or_default());
        let origin = view
            .origin
            .as_deref()
            .and_then(|origin| origin.split_once(':'))
            .and_then(|(kind, key)| match kind {
                "album" => self
                    .library
                    .album(AlbumId(key.parse().ok()?))
                    .ok()
                    .flatten()
                    .map(|album| album.title),
                "artist" => self
                    .library
                    .artist(ArtistId(key.parse().ok()?))
                    .ok()
                    .flatten()
                    .map(|artist| artist.name),
                "folder" => Path::new(key)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                // Tracks an agent chose, named after the agent.
                "agent" => Some(key.to_owned()),
                "playlist" => self
                    .library
                    .playlist(PlaylistId(key.parse().ok()?))
                    .ok()
                    .flatten()
                    .map(|playlist| playlist.name),
                "label" => self
                    .library
                    .label(LabelId(key.parse().ok()?))
                    .ok()
                    .flatten()
                    .map(|label| label.name),
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
            Some(track) => {
                let cover = track.album_id.and_then(|album| self.queue_cover(album));
                QueueRow {
                    title: track.title.into(),
                    artist: track.artist.unwrap_or_default().into(),
                    duration: format_duration(track.duration).into(),
                    album_id: track.album_id.map_or(-1, |album| album.0 as i32),
                    has_cover: cover.is_some(),
                    cover: cover.unwrap_or_default(),
                    path: entry.track.id.as_ref().into(),
                }
            }
            None => QueueRow {
                title: path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default()
                    .into(),
                album_id: -1,
                path: entry.track.id.as_ref().into(),
                ..QueueRow::default()
            },
        }
    }

    /// An album's small cover for the queue panel, once decoded; asks for it
    /// otherwise, and the rows get it when it arrives.
    fn queue_cover(&self, album: AlbumId) -> Option<Image> {
        if let Some(cover) = self.queue_covers.borrow().get(&album) {
            return cover.clone();
        }
        if !self.queue_covers_pending.borrow_mut().insert(album) {
            return None;
        }
        let thumbnail = self
            .library
            .album(album)
            .ok()
            .flatten()
            .and_then(|album| album.artwork)
            .map(|artwork| artwork.thumbnail);
        let Some(thumbnail) = thumbnail else {
            self.queue_covers_pending.borrow_mut().remove(&album);
            self.queue_covers.borrow_mut().insert(album, None);
            return None;
        };
        let mut jobs = self.queue_cover_jobs.borrow_mut();
        let sender = jobs.get_or_insert_with(|| {
            let (sender, inbox) = mpsc::channel::<(AlbumId, PathBuf)>();
            thread::Builder::new()
                .name("vcmp-queue-covers".into())
                .spawn(move || {
                    for (album, path) in inbox {
                        let buffer = image::open(&path).ok().map(|image| {
                            let rgba = image
                                .thumbnail(QUEUE_COVER_PIXELS, QUEUE_COVER_PIXELS)
                                .into_rgba8();
                            SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                                rgba.as_raw(),
                                rgba.width(),
                                rgba.height(),
                            )
                        });
                        if !Self::post(move |service| service.queue_cover_decoded(album, buffer)) {
                            break;
                        }
                    }
                })
                .expect("spawning the queue cover thread");
            sender
        });
        let _ = sender.send((album, thumbnail));
        None
    }

    /// Takes a decoded queue cover: keeps it, and shows it on the rows of its album.
    pub fn queue_cover_decoded(
        &self,
        album: AlbumId,
        buffer: Option<SharedPixelBuffer<Rgba8Pixel>>,
    ) {
        self.queue_covers_pending.borrow_mut().remove(&album);
        let cover = buffer.map(Image::from_rgba8);
        {
            let mut covers = self.queue_covers.borrow_mut();
            // A long session meets many albums; the queue shows few at a time.
            if covers.len() >= QUEUE_COVER_CACHE {
                covers.clear();
            }
            covers.insert(album, cover.clone());
        }
        let (Some(cover), Some(window)) = (cover, self.window.upgrade()) else {
            return;
        };
        let state = window.global::<PlayerState>();
        for model in [state.get_up_next(), state.get_upcoming()] {
            for index in 0..model.row_count() {
                if let Some(mut row) = model.row_data(index)
                    && row.album_id == album.0 as i32
                {
                    row.cover = cover.clone();
                    row.has_cover = true;
                    model.set_row_data(index, row);
                }
            }
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
            if let Some(track) = self.media_track.borrow_mut().as_mut() {
                track.cover_url = Some(cover.url());
            }
            self.update_presence();
            self.update_media();
        }
    }

    /// Shows an album's new artwork wherever the player shows it: on its rows in the
    /// queue panel and, while it plays, in the bar, the now playing view, the
    /// system's media widget and the Discord presence.
    pub fn artwork_changed(&self, album: AlbumId) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        self.queue_covers.borrow_mut().remove(&album);
        let queued = [state.get_up_next(), state.get_upcoming()]
            .iter()
            .any(|rows| rows.iter().any(|row| row.album_id == album.0 as i32));
        if queued {
            let _ = self.queue_cover(album);
        }
        if self.album.get() != Some(album) {
            return;
        }
        let Some(album) = self.library.album(album).ok().flatten() else {
            return;
        };
        let cover = album
            .artwork
            .as_ref()
            .and_then(|artwork| Image::load_from_path(&artwork.thumbnail).ok());
        state.set_has_cover(cover.is_some());
        state.set_cover(cover.unwrap_or_default());
        self.use_large_cover(&state, album.artwork.as_ref());
        let cover_url = self.known_cover(&album);
        if let Some(listening) = self.listening.borrow_mut().as_mut() {
            listening.cover_url = cover_url.clone();
        }
        if let Some(track) = self.media_track.borrow_mut().as_mut() {
            track.cover_file = album.artwork.map(|artwork| artwork.thumbnail);
            track.cover_url = cover_url;
        }
        self.update_presence();
        self.update_media();
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
                if playback != PlaybackState::Playing {
                    self.announced.set(false);
                }
                if playback == PlaybackState::Stopped {
                    self.sleep_timer_done();
                }
                self.announce();
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
                self.count_play(position);
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
            Event::StopAfterCurrent(on) => {
                if self.sleep_minutes.get() == 0 {
                    self.show_sleep_timer(on);
                }
            }
            Event::Decoding(track) => {
                let path = track.map(|track| PathBuf::from(track.id.as_ref()));
                *self.open_file.borrow_mut() = path.clone();
                if let Some(listener) = &*self.on_open_file.borrow() {
                    listener(path.as_deref());
                }
                return;
            }
        }
        self.update_media();
    }

    fn show_track(&self, state: &PlayerState, now: Option<NowPlaying>) {
        let path = now
            .as_ref()
            .map(|now| PathBuf::from(now.entry.track.id.as_ref()));
        // A tags notice comes as the engine moves on to the next track, and stays while
        // that one plays.
        let tags_notice_over = state.get_problem() == PlayerProblem::TagsNotWritten
            && *self.tags_notice_file.borrow() != path;
        if state.get_problem() == PlayerProblem::TrackFailed || tags_notice_over {
            state.set_problem(PlayerProblem::None);
        }
        let Some(now) = now else {
            state.set_has_track(false);
            state.set_track_path("".into());
            state.set_lossless_format(SharedString::default());
            state.set_has_cover(false);
            state.set_cover(Image::default());
            state.set_album(SharedString::default());
            self.use_large_cover(state, None);
            *self.listening.borrow_mut() = None;
            *self.media_track.borrow_mut() = None;
            self.album.set(None);
            self.artist.set(None);
            state.set_has_album_link(false);
            state.set_has_artist_link(false);
            self.start_play(None, Duration::ZERO);
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
        *self.media_track.borrow_mut() = Some(MediaTrack {
            id: path.clone(),
            title: title.clone(),
            artist: track.as_ref().and_then(|track| track.artist.clone()),
            album: album.as_ref().map(|album| album.title.clone()),
            cover_file: album
                .as_ref()
                .and_then(|album| album.artwork.as_ref())
                .map(|artwork| artwork.thumbnail.clone()),
            cover_url: self
                .listening
                .borrow()
                .as_ref()
                .and_then(|listening| listening.cover_url.clone()),
            duration: (!duration.is_zero()).then_some(duration),
        });
        self.album.set(album.as_ref().map(|album| album.id));
        self.artist
            .set(track.as_ref().and_then(|track| track.artist_id));
        state.set_has_album_link(album.is_some());
        state.set_has_artist_link(self.artist.get().is_some());
        self.position.set(Duration::ZERO);
        self.start_play(track.as_ref(), duration);
        self.update_presence();

        state.set_lossless_format(lossless_format(track.as_ref()).into());
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
        state.set_album(
            album
                .as_ref()
                .map(|album| album.title.as_str())
                .unwrap_or_default()
                .into(),
        );
        self.use_large_cover(
            state,
            album.as_ref().and_then(|album| album.artwork.as_ref()),
        );
        state.set_track_path(path.into());
        state.set_duration_ms(duration.as_millis() as i32);
        state.set_duration_text(format_duration(duration).into());
        self.seek_target.set(None);
        show_position(state, Duration::ZERO);
    }

    /// Shows tags the editor just wrote: the playing track's names, links and cover,
    /// and the queue. The play goes on as it was, and Last.fm gets the tags it started
    /// with.
    pub fn tags_changed(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        // Albums may have been regrouped under new ids.
        self.queue_covers.borrow_mut().clear();
        let view = self.queue_view.borrow().clone();
        self.show_queue(&state, &view);
        if !state.get_has_track() {
            return;
        }
        let path = state.get_track_path();
        let Some(track) = self
            .library
            .track_id_for_path(Path::new(path.as_str()))
            .ok()
            .flatten()
            .and_then(|id| self.library.track(id).ok().flatten())
        else {
            return;
        };
        let album = track
            .album_id
            .and_then(|album| self.library.album(album).ok().flatten());
        let cover = album
            .as_ref()
            .and_then(|album| album.artwork.as_ref())
            .and_then(|artwork| Image::load_from_path(&artwork.thumbnail).ok());
        self.album.set(album.as_ref().map(|album| album.id));
        self.artist.set(track.artist_id);
        state.set_has_album_link(album.is_some());
        state.set_has_artist_link(track.artist_id.is_some());
        state.set_initial(initial(&track.title).into());
        state.set_title(track.title.as_str().into());
        state.set_artist(track.artist.clone().unwrap_or_default().into());
        state.set_album(
            album
                .as_ref()
                .map(|album| album.title.as_str())
                .unwrap_or_default()
                .into(),
        );
        state.set_has_cover(cover.is_some());
        state.set_cover(cover.unwrap_or_default());
        self.use_large_cover(
            &state,
            album.as_ref().and_then(|album| album.artwork.as_ref()),
        );
    }

    /// A new play for Last.fm: of `track`, which is `duration` long, or of nothing.
    /// Tracks without an artist can't be scrobbled.
    fn start_play(&self, track: Option<&Track>, duration: Duration) {
        let duration = (!duration.is_zero()).then_some(duration);
        *self.play.borrow_mut() = track.and_then(|track| {
            let track = lastfm::Track {
                artist: track.artist.clone()?,
                title: track.title.clone(),
                album: track.album.clone(),
                album_artist: track.album_artist.clone(),
                track_number: track.track_number,
                duration,
            };
            Some((track, Play::new(duration)))
        });
        let eligible = self.play.borrow().is_some() && lastfm::scrobble_point(duration).is_some();
        self.show_scrobble(if eligible {
            ScrobbleMark::Counting
        } else {
            ScrobbleMark::None
        });
        self.announced.set(false);
        self.announce();
    }

    /// Tells Last.fm what plays, once each time a track starts or resumes.
    fn announce(&self) {
        if !self.playing.get() || self.announced.get() {
            return;
        }
        let scrobbles = self.scrobbles.borrow();
        let play = self.play.borrow();
        if let (Some(sink), Some((track, _))) = (scrobbles.as_ref(), play.as_ref()) {
            self.announced.set(true);
            sink.now_playing(track.clone());
        }
    }

    /// Follows the play through the engine's positions, and hands it to Last.fm
    /// once it counts.
    fn count_play(&self, position: Duration) {
        let play = {
            let mut current = self.play.borrow_mut();
            let Some((track, play)) = current.as_mut() else {
                return;
            };
            if !play.position(position, self.playing.get(), SystemTime::now()) {
                return;
            }
            lastfm::Scrobble {
                track: track.clone(),
                started: play.started().unwrap_or_else(SystemTime::now),
            }
        };
        self.show_scrobble(ScrobbleMark::Scrobbled);
        if let Some(sink) = self.scrobbles.borrow().as_ref() {
            sink.scrobble(play);
        }
    }

    fn show_scrobble(&self, mark: ScrobbleMark) {
        if let Some(window) = self.window.upgrade() {
            window.global::<PlayerState>().set_scrobble(mark);
        }
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

    /// Tells the system's media widget what plays, and how.
    fn update_media(&self) {
        let media = self.media.borrow();
        let (Some(media), Some(window)) = (media.as_ref(), self.window.upgrade()) else {
            return;
        };
        let state = window.global::<PlayerState>();
        media.update(MediaState {
            track: self.media_track.borrow().clone(),
            playing: self.playing.get(),
            position: self.position.get(),
            volume: if self.muted.get() {
                0.0
            } else {
                state.get_volume()
            },
            shuffle: state.get_shuffle() == Shuffle::On,
            repeat: match state.get_repeat() {
                Repeat::Off => media_controls::Repeat::Off,
                Repeat::One => media_controls::Repeat::One,
                Repeat::All => media_controls::Repeat::All,
            },
        });
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
        // An Unknown album is no release MusicBrainz could know.
        let Some(Album {
            title,
            artist: Some(artist),
            track_count,
            musicbrainz_id: None,
            unknown: false,
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
        if let Some((_, play)) = self.play.borrow_mut().as_mut() {
            play.jumped();
        }
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
        self.update_media();
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
        self.update_media();
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

    /// Starts the sleep timer for `minutes`, has playback stop after the current track
    /// for [`SLEEP_END_OF_TRACK`], or turns the timer off for 0.
    fn set_sleep_timer(&self, minutes: i32) {
        self.sleep_timer.stop();
        self.sleep_minutes.set(minutes.max(0));
        self.player
            .send(Command::StopAfterCurrent(minutes == SLEEP_END_OF_TRACK));
        if minutes > 0 {
            self.sleep_timer.start(TimerMode::Repeated, SLEEP_TICK, || {
                if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                    service.sleep_tick();
                }
            });
        }
        self.show_sleep_timer(minutes == SLEEP_END_OF_TRACK);
    }

    /// A minute of the sleep timer has passed. When it runs out, playback stops after
    /// the track playing then; when nothing plays, there is nothing left to stop.
    fn sleep_tick(&self) {
        let left = self.sleep_minutes.get() - 1;
        self.sleep_minutes.set(left.max(0));
        if left > 0 {
            self.show_sleep_timer(false);
            return;
        }
        self.sleep_timer.stop();
        let playing = self.playing.get();
        if playing {
            self.player.send(Command::StopAfterCurrent(true));
        }
        self.show_sleep_timer(playing);
    }

    /// Ends the sleep timer once playback has stopped, by itself or by Stop.
    fn sleep_timer_done(&self) {
        self.sleep_timer.stop();
        self.sleep_minutes.set(0);
        self.show_sleep_timer(false);
    }

    fn show_sleep_timer(&self, end_of_track: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<PlayerState>();
        let minutes = self.sleep_minutes.get();
        state.set_sleep_minutes(minutes);
        state.set_sleep_timer(if minutes > 0 {
            SleepTimer::Timed
        } else if end_of_track {
            SleepTimer::EndOfTrack
        } else {
            SleepTimer::Off
        });
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

/// The format's name when the track is stored without loss, for the bar's mark.
fn lossless_format(track: Option<&Track>) -> &'static str {
    track
        .and_then(|track| track.format)
        .filter(|format| format.is_lossless())
        .map_or("", |format| format.name())
}

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
