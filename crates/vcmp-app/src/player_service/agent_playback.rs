//! Playback AI agents ask for through `vcmp_mcp`'s server: brought from its thread to
//! the player on the UI thread and done at once, as the window's own buttons do, so
//! the bar, the queue panel, the OS media controls, Discord and Last.fm follow. The
//! server has already left out the tracks that can't be played.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use slint::ComponentHandle;
use vcmp_integrations::media_controls::Control as MediaControl;
use vcmp_library::{AlbumId, ArtistId, LabelId, PlaylistId, Track};
use vcmp_mcp::{Context, Control, Options, OutputDevices, Play, Playback, Status};
use vcmp_playback::{Command, PlaybackError, QueueEntry};

use super::{PlayerService, SEARCH_ORIGIN, SLEEP_END_OF_TRACK, SONGS_ORIGIN, entry};
use crate::agents::{BRIDGE_TIMEOUT, SETTING_PLAYBACK};
use crate::library_view::format_duration;
use crate::{PlayState, PlayerState, Repeat, Shuffle, ShuffleKind, SleepTimer};

/// The context origin of tracks an agent chose to play, before the agent's name.
const AGENT_ORIGIN: &str = "agent";
/// How long a request that the engine acts on waits for the window to show the
/// result. A skip waits for the next file to open, which a disk that sleeps may take
/// seconds for.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

/// What agents ask of the player, brought from the server's thread to the service on
/// the UI thread. Each call waits for the window to take it.
pub struct AppPlayback;

impl AppPlayback {
    fn on_ui<T: Send + 'static>(
        work: impl FnOnce(&PlayerService) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (sender, receiver) = mpsc::channel();
        if !PlayerService::post(move |service| {
            let _ = sender.send(service.agents_may_play().and_then(|()| work(service)));
        }) {
            return Err("VCMP is closing".into());
        }
        match receiver.recv_timeout(BRIDGE_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err("VCMP didn't answer in time".into()),
            // The work was dropped unrun: there is no player service.
            Err(RecvTimeoutError::Disconnected) => {
                Err("VCMP's player isn't running: it could not start its sound output".into())
            }
        }
    }

    /// Like [`on_ui`](Self::on_ui) for work that sends the engine commands, then waits
    /// until the window has taken the events they caused, so that `status` reads the
    /// result: a skip changes the track only once the engine has opened the next file.
    /// Past [`SETTLE_TIMEOUT`] it answers anyway; the request still stands.
    fn on_ui_settled<T: Send + 'static>(
        work: impl FnOnce(&PlayerService) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (result, settled) = Self::on_ui(move |service| {
            let result = work(service)?;
            Ok((result, service.agent_sync()))
        })?;
        if settled.recv_timeout(SETTLE_TIMEOUT) == Err(RecvTimeoutError::Timeout) {
            log::warn!("the player took longer than {SETTLE_TIMEOUT:?} over an agent's request");
        }
        Ok(result)
    }

    /// Lists the output devices on this thread, as listing may take a while, and
    /// gives the list to the service.
    fn list_devices() -> Result<Vec<String>, String> {
        let lister = Self::on_ui(|service| Ok(service.list_devices.get()))?;
        lister().map_err(|error: PlaybackError| {
            format!("the output devices could not be listed: {error}")
        })
    }
}

impl Playback for AppPlayback {
    fn play(&self, play: Play) -> Result<(), String> {
        Self::on_ui_settled(move |service| service.agent_play(play))
    }

    fn queue(&self, tracks: Vec<Track>, next: bool) -> Result<(), String> {
        Self::on_ui_settled(move |service| service.agent_queue(&tracks, next))
    }

    fn control(&self, control: Control) -> Result<(), String> {
        Self::on_ui_settled(move |service| service.agent_control(control))
    }

    fn set_options(&self, options: Options) -> Result<(), String> {
        Self::on_ui_settled(move |service| {
            service.agent_options(options);
            Ok(())
        })
    }

    fn output_devices(&self) -> Result<OutputDevices, String> {
        let devices = Self::list_devices()?;
        Self::on_ui(move |service| Ok(service.agent_output_devices(devices)))
    }

    fn set_output_device(&self, device: Option<String>) -> Result<(), String> {
        let devices = Self::list_devices()?;
        Self::on_ui(move |service| {
            service.agent_output_devices(devices);
            service.agent_set_output_device(device.as_deref())
        })
    }

    fn set_sleep_timer(&self, timer: vcmp_mcp::SleepTimer) -> Result<(), String> {
        Self::on_ui(move |service| {
            service.agent_sleep_timer(timer);
            Ok(())
        })
    }

    fn status(&self) -> Result<Status, String> {
        Self::on_ui(|service| service.agent_status())
    }
}

impl PlayerService {
    /// Whether the owner lets agents control playback; the reason to tell the agent
    /// when not.
    pub fn agents_may_play(&self) -> Result<(), String> {
        if self.setting(SETTING_PLAYBACK).as_deref() == Some("true") {
            Ok(())
        } else {
            Err("The owner hasn't let AI agents control playback: they can turn on \"Let AI agents control playback\" in VCMP's Settings, under AI agents.".into())
        }
    }

    /// Asks the engine to answer once it has done the commands sent so far. The
    /// receiver hears once the window has taken that answer, and so every event
    /// before it.
    pub fn agent_sync(&self) -> Receiver<()> {
        let token = self.last_sync.get() + 1;
        self.last_sync.set(token);
        let (sender, receiver) = mpsc::channel();
        self.syncs.borrow_mut().push((token, sender));
        self.player.send(Command::Sync(token));
        receiver
    }

    /// The engine answered [`Command::Sync`] with `token`, and so every token before.
    pub(super) fn synced(&self, token: u64) {
        self.syncs.borrow_mut().retain(|(waiting, sender)| {
            if *waiting > token {
                return true;
            }
            let _ = sender.send(());
            false
        });
    }

    /// Plays what an agent chose, as the window's play buttons do: shuffle turned on
    /// or off first when asked.
    pub fn agent_play(&self, play: Play) -> Result<(), String> {
        let entries: Vec<QueueEntry> = play.tracks.iter().filter_map(entry).collect();
        if entries.is_empty() {
            return Err("there is nothing to play".into());
        }
        if let Some(on) = play.shuffle {
            self.set_shuffle(if on { Shuffle::On } else { Shuffle::Off });
        }
        self.player.send(Command::PlayContext {
            entries,
            origin: Some(origin(&play.context)),
            start: play.start,
        });
        Ok(())
    }

    /// Puts tracks an agent chose into Up next: at its front (`next`) or its end.
    pub fn agent_queue(&self, tracks: &[Track], next: bool) -> Result<(), String> {
        let entries: Vec<QueueEntry> = tracks.iter().filter_map(entry).collect();
        if entries.is_empty() {
            return Err("there is nothing to queue".into());
        }
        self.player.send(if next {
            Command::PlayNext(entries)
        } else {
            Command::AddToQueue(entries)
        });
        Ok(())
    }

    /// Presses a player button for an agent, or changes Up next. Positions are
    /// checked against the queue as the panel shows it.
    pub fn agent_control(&self, control: Control) -> Result<(), String> {
        let view = self.queue_view.borrow().clone();
        let up_next = view.up_next.len();
        let in_up_next = |position: usize| {
            if position < up_next {
                Ok(position)
            } else {
                Err(format!(
                    "Up next has {up_next} entries, from position 0; get_now_playing lists them"
                ))
            }
        };
        match control {
            Control::Play => self.media_control(MediaControl::Play),
            Control::Pause => self.media_control(MediaControl::Pause),
            Control::TogglePlayPause => self.media_control(MediaControl::TogglePlayPause),
            Control::Stop => self.media_control(MediaControl::Stop),
            Control::Next => self.media_control(MediaControl::Next),
            Control::Previous => self.media_control(MediaControl::Previous),
            Control::Seek(position) => {
                if view.current.is_none() {
                    return Err("nothing is playing".into());
                }
                let duration = self.media_track.borrow().as_ref().and_then(|t| t.duration);
                if let Some(duration) = duration
                    && position > duration
                {
                    return Err(format!(
                        "the track is only {} long",
                        format_duration(duration)
                    ));
                }
                self.media_control(MediaControl::SeekTo(position));
            }
            Control::Raise => self.media_control(MediaControl::Raise),
            Control::SkipTo(position) => {
                let entries = up_next + view.upcoming.len();
                if position >= entries {
                    return Err(format!(
                        "the queue lists {entries} entries after the current track, from position 0; get_now_playing lists them"
                    ));
                }
                self.player.send(Command::SkipTo(position));
            }
            Control::RemoveFromUpNext(position) => {
                self.player
                    .send(Command::RemoveFromUpNext(in_up_next(position)?));
            }
            Control::MoveInUpNext { from, to } => {
                self.player.send(Command::MoveInUpNext {
                    from: in_up_next(from)?,
                    to: in_up_next(to)?,
                });
            }
            Control::ClearUpNext => self.player.send(Command::ClearUpNext),
        }
        Ok(())
    }

    /// Sets the playback options an agent named, as the bar and Settings do, and
    /// remembers them the same way.
    pub fn agent_options(&self, options: Options) {
        // The kind first, so shuffle turned on picks that way at once.
        if let Some(kind) = options.shuffle_kind {
            self.set_shuffle_kind(match kind {
                vcmp_mcp::ShuffleKind::FewerRepeats => ShuffleKind::FewerRepeats,
                vcmp_mcp::ShuffleKind::TrueRandom => ShuffleKind::TrueRandom,
            });
        }
        if let Some(on) = options.shuffle {
            self.set_shuffle(if on { Shuffle::On } else { Shuffle::Off });
        }
        if let Some(repeat) = options.repeat {
            self.set_repeat(match repeat {
                vcmp_mcp::Repeat::Off => Repeat::Off,
                vcmp_mcp::Repeat::All => Repeat::All,
                vcmp_mcp::Repeat::One => Repeat::One,
            });
        }
        if let Some(volume) = options.volume {
            self.set_volume(volume, true);
        }
    }

    /// Takes a fresh list of output devices, as Settings does when it opens, and
    /// answers it with the chosen device.
    pub fn agent_output_devices(&self, devices: Vec<String>) -> OutputDevices {
        self.set_output_devices(Ok(devices));
        self.output_status()
    }

    /// Plays through the listed device an agent named (exactly, or else ignoring
    /// case), or the system default for `None`, and remembers it.
    pub fn agent_set_output_device(&self, device: Option<&str>) -> Result<(), String> {
        let index = match device {
            None => -1,
            Some(name) => {
                let devices = self.devices.borrow();
                let index = devices
                    .iter()
                    .position(|listed| listed == name)
                    .or_else(|| {
                        let name = name.to_lowercase();
                        devices
                            .iter()
                            .position(|listed| listed.to_lowercase() == name)
                    })
                    .ok_or_else(|| {
                        format!("there is no output device named {name:?}; see list_output_devices")
                    })?;
                index as i32
            }
        };
        self.set_output_device(index);
        Ok(())
    }

    pub fn agent_sleep_timer(&self, timer: vcmp_mcp::SleepTimer) {
        self.set_sleep_timer(match timer {
            vcmp_mcp::SleepTimer::Off => 0,
            vcmp_mcp::SleepTimer::EndOfTrack => SLEEP_END_OF_TRACK,
            vcmp_mcp::SleepTimer::Minutes(minutes) => minutes.min(i32::MAX as u32) as i32,
        });
    }

    /// What the player bar and the queue panel show.
    pub fn agent_status(&self) -> Result<Status, String> {
        let window = self.window.upgrade().ok_or("VCMP is closing")?;
        let state = window.global::<PlayerState>();
        let view = self.queue_view.borrow();
        let path = |entry: &QueueEntry| PathBuf::from(entry.track.id.as_ref());
        Ok(Status {
            state: match state.get_state() {
                PlayState::Playing => vcmp_mcp::PlayState::Playing,
                PlayState::Paused => vcmp_mcp::PlayState::Paused,
                PlayState::Stopped => vcmp_mcp::PlayState::Stopped,
            },
            current: view.current.as_ref().map(path),
            position: self.position.get(),
            duration: self.media_track.borrow().as_ref().and_then(|t| t.duration),
            volume: state.get_volume(),
            muted: self.muted.get(),
            shuffle: state.get_shuffle() == Shuffle::On,
            shuffle_kind: match state.get_shuffle_kind() {
                ShuffleKind::FewerRepeats => vcmp_mcp::ShuffleKind::FewerRepeats,
                ShuffleKind::TrueRandom => vcmp_mcp::ShuffleKind::TrueRandom,
            },
            repeat: match state.get_repeat() {
                Repeat::Off => vcmp_mcp::Repeat::Off,
                Repeat::All => vcmp_mcp::Repeat::All,
                Repeat::One => vcmp_mcp::Repeat::One,
            },
            output: self.output_status(),
            sleep_timer: match state.get_sleep_timer() {
                SleepTimer::Off => vcmp_mcp::SleepTimer::Off,
                SleepTimer::EndOfTrack => vcmp_mcp::SleepTimer::EndOfTrack,
                SleepTimer::Timed => {
                    vcmp_mcp::SleepTimer::Minutes(self.sleep_minutes.get().max(0) as u32)
                }
            },
            context: view.origin.as_deref().and_then(context),
            up_next: view.up_next.iter().map(path).collect(),
            upcoming: view.upcoming.iter().map(path).collect(),
            context_len: view.context_len,
        })
    }

    fn output_status(&self) -> OutputDevices {
        let fallback = self
            .window
            .upgrade()
            .map(|window| window.global::<PlayerState>().get_output_fallback())
            .filter(|device| !device.is_empty())
            .map(String::from);
        OutputDevices {
            devices: self.devices.borrow().clone(),
            chosen: self.saved_output_device(),
            fallback,
        }
    }
}

/// The queue's origin key for a context, as the window's own play buttons set it.
fn origin(context: &Context) -> String {
    match context {
        Context::Agent(name) => format!("{AGENT_ORIGIN}:{name}"),
        Context::Album(id) => format!("album:{}", id.0),
        Context::Artist(id) => format!("artist:{}", id.0),
        Context::Playlist(id) => format!("playlist:{}", id.0),
        Context::Label(id) => format!("label:{}", id.0),
        Context::Folder(path) => format!("folder:{}", path.to_string_lossy()),
        Context::Search => SEARCH_ORIGIN.to_owned(),
        Context::Songs => SONGS_ORIGIN.to_owned(),
    }
}

/// The context an origin key names.
fn context(origin: &str) -> Option<Context> {
    match origin {
        SEARCH_ORIGIN => return Some(Context::Search),
        SONGS_ORIGIN => return Some(Context::Songs),
        _ => {}
    }
    let (kind, key) = origin.split_once(':')?;
    let id = || key.parse::<i64>().ok();
    Some(match kind {
        "album" => Context::Album(AlbumId(id()?)),
        "artist" => Context::Artist(ArtistId(id()?)),
        "playlist" => Context::Playlist(PlaylistId(id()?)),
        "label" => Context::Label(LabelId(id()?)),
        "folder" => Context::Folder(PathBuf::from(key)),
        AGENT_ORIGIN => Context::Agent(key.to_owned()),
        _ => return None,
    })
}
