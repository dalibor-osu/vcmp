//! OS media controls: media keys and the system's now-playing widget drive VCMP, and
//! the widget shows the playing track. MPRIS on Linux, System Media Transport
//! Controls on Windows, Now Playing on macOS, all through the `playwire` crate.
//!
//! On Linux and Windows the controls run on their own thread: D-Bus and WinRT calls
//! can stall, and the caller never waits for them. Updates queue up and only the
//! latest is published. Windows registers from the window's thread first. macOS
//! wants Now Playing touched from the main thread, so there the controls are updated
//! in place; those calls only set properties (`playwire` reads the cover on a
//! background thread).

use std::path::PathBuf;
use std::time::Duration;

use playwire::{Capabilities, Event, PlaybackState, PlayerConfig, Track};

/// The MPRIS bus name element (`org.mpris.MediaPlayer2.vcmp`) and the name shown
/// where the platform shows one.
const NAME: &str = "VCMP";
const BUS_NAME: &str = "vcmp";
/// The `.desktop` file GNOME and KDE look up for VCMP's icon (`vcmp.desktop`).
pub const DESKTOP_ENTRY: &str = "vcmp";
/// MPRIS track ids are object paths under this prefix.
const TRACK_ID_PREFIX: &str = "/io/github/dalibor_osu/vcmp/track";

/// What the system is asked to do by a media key or its media widget.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Play,
    Pause,
    TogglePlayPause,
    Stop,
    Next,
    Previous,
    /// To this position in the track.
    SeekTo(Duration),
    /// Forwards (`true`) or back by this much.
    SeekBy {
        forward: bool,
        by: Duration,
    },
    SetShuffle(bool),
    SetRepeat(Repeat),
    /// 0.0 to 1.0.
    SetVolume(f32),
    /// Bring VCMP's window up.
    Raise,
    /// The user quit VCMP from the widget.
    Quit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Repeat {
    #[default]
    Off,
    One,
    All,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaTrack {
    /// Stays the same for one track and differs between tracks (its path).
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The cover thumbnail on disk.
    pub cover_file: Option<PathBuf>,
    /// The cover on the web, when known (the Cover Art Archive).
    pub cover_url: Option<String>,
    pub duration: Option<Duration>,
}

/// Everything the system shows. `track: None` clears the widget.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaState {
    pub track: Option<MediaTrack>,
    pub playing: bool,
    pub position: Duration,
    /// 0.0 to 1.0.
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: Repeat,
}

/// Detaches from the system when dropped.
pub struct MediaControls {
    #[cfg(not(target_os = "macos"))]
    updates: std::sync::mpsc::Sender<Option<MediaState>>,
    #[cfg(not(target_os = "macos"))]
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(target_os = "macos")]
    controls: std::cell::RefCell<playwire::MediaControls>,
}

impl MediaControls {
    /// Registers VCMP with the system. `window` is the native window handle (an
    /// `HWND`), which Windows requires and the other platforms ignore. `on_control`
    /// hears the user's requests, on a system thread: hand them on, don't block.
    ///
    /// When the system has no media service to register with (no session bus,
    /// another VCMP already registered), VCMP runs without media controls: an error
    /// here on Windows and macOS, a line in the log on Linux, where registering
    /// happens on the controls' thread.
    pub fn start(
        window: Option<u64>,
        on_control: impl Fn(Control) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let mut config = PlayerConfig::new(BUS_NAME)
            .desktop_entry(DESKTOP_ENTRY)
            .track_id_prefix(TRACK_ID_PREFIX)
            .supported_uri_schemes(Vec::new());
        config.identity = NAME.to_string();
        if let Some(window) = window {
            config = config.hwnd(window);
        }
        let handler = move |event| {
            if let Some(control) = control(event) {
                on_control(control);
            }
        };
        Self::run(config, handler)
    }

    /// Linux registers on the controls' own thread, so a slow session bus never holds
    /// up the caller.
    #[cfg(not(any(windows, target_os = "macos")))]
    fn run(
        config: PlayerConfig,
        handler: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        Self::spawn(move || playwire::MediaControls::new(config, handler))
    }

    /// Windows registers on the thread that owns the window, then hands the controls
    /// to their own thread.
    #[cfg(windows)]
    fn run(
        config: PlayerConfig,
        handler: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let controls = playwire::MediaControls::new(config, handler).map_err(|e| e.to_string())?;
        Self::spawn(move || Ok(controls))
    }

    #[cfg(not(target_os = "macos"))]
    fn spawn(
        register: impl FnOnce() -> playwire::Result<playwire::MediaControls> + Send + 'static,
    ) -> Result<Self, String> {
        use std::sync::mpsc;

        let (updates, inbox) = mpsc::channel::<Option<MediaState>>();
        let thread = std::thread::Builder::new()
            .name("vcmp-media-controls".into())
            .spawn(move || {
                let mut controls = match register() {
                    Ok(controls) => controls,
                    Err(error) => {
                        log::info!("running without OS media controls: {error}");
                        return;
                    }
                };
                // `None` is the request to stop.
                while let Ok(Some(mut state)) = inbox.recv() {
                    let mut stop = false;
                    for newer in inbox.try_iter() {
                        match newer {
                            Some(newer) => state = newer,
                            None => stop = true,
                        }
                    }
                    if let Err(error) = controls.set_state(&playback_state(&state)) {
                        log::debug!("the media controls did not take an update: {error}");
                    }
                    if stop {
                        break;
                    }
                }
                controls.detach();
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            updates,
            thread: Some(thread),
        })
    }

    #[cfg(target_os = "macos")]
    fn run(
        config: PlayerConfig,
        handler: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let controls = playwire::MediaControls::new(config, handler).map_err(|e| e.to_string())?;
        Ok(Self {
            controls: std::cell::RefCell::new(controls),
        })
    }

    /// Shows `state` in the system's widget.
    pub fn update(&self, state: MediaState) {
        #[cfg(not(target_os = "macos"))]
        let _ = self.updates.send(Some(state));
        #[cfg(target_os = "macos")]
        if let Err(error) = self
            .controls
            .borrow_mut()
            .set_state(&playback_state(&state))
        {
            log::debug!("the media controls did not take an update: {error}");
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl Drop for MediaControls {
    fn drop(&mut self) {
        let _ = self.updates.send(None);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What a system event asks VCMP to do; `None` for what VCMP does not offer.
fn control(event: Event) -> Option<Control> {
    Some(match event {
        Event::Play => Control::Play,
        Event::Pause => Control::Pause,
        Event::PlayPause => Control::TogglePlayPause,
        Event::Stop => Control::Stop,
        Event::Next => Control::Next,
        Event::Previous => Control::Previous,
        Event::SeekTo(position) => Control::SeekTo(position),
        Event::SeekBy(seconds) if seconds.is_finite() => Control::SeekBy {
            forward: seconds >= 0.0,
            by: Duration::try_from_secs_f64(seconds.abs()).ok()?,
        },
        Event::SetShuffle(shuffle) => Control::SetShuffle(shuffle),
        Event::SetRepeat(repeat) => Control::SetRepeat(match repeat {
            playwire::Repeat::Off => Repeat::Off,
            playwire::Repeat::One => Repeat::One,
            playwire::Repeat::All => Repeat::All,
        }),
        Event::SetVolume(volume) if volume.is_finite() => {
            Control::SetVolume(volume.clamp(0.0, 1.0) as f32)
        }
        Event::Raise => Control::Raise,
        Event::Quit => Control::Quit,
        _ => return None,
    })
}

fn playback_state(state: &MediaState) -> PlaybackState {
    let track = state.track.as_ref();
    PlaybackState {
        track: track.map(|track| Track {
            id: track.id.clone(),
            title: track.title.clone(),
            artists: track.artist.iter().cloned().collect(),
            album: track.album.clone().unwrap_or_default(),
            artwork_url: artwork_url(track, cfg!(windows)).unwrap_or_default(),
            url: String::new(),
        }),
        playing: state.playing && track.is_some(),
        position: state.position,
        duration: track.and_then(|track| track.duration),
        volume: f64::from(state.volume.clamp(0.0, 1.0)),
        repeat: match state.repeat {
            Repeat::Off => playwire::Repeat::Off,
            Repeat::One => playwire::Repeat::One,
            Repeat::All => playwire::Repeat::All,
        },
        shuffle: state.shuffle,
        capabilities: Capabilities {
            can_go_next: track.is_some(),
            can_go_previous: track.is_some(),
            can_seek: track.is_some_and(|track| track.duration.is_some()),
        },
    }
}

/// Where the system finds the cover. The thumbnail on disk, as a `file://` address;
/// Windows only takes covers from the web, so there it is the web cover or none.
fn artwork_url(track: &MediaTrack, web_only: bool) -> Option<String> {
    let file = track
        .cover_file
        .as_ref()
        .filter(|_| !web_only)
        .and_then(|path| url::Url::from_file_path(path).ok())
        .map(String::from);
    file.or_else(|| track.cover_url.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> MediaTrack {
        MediaTrack {
            id: "/music/Record/01 Opening.flac".into(),
            title: "Opening".into(),
            artist: Some("Band".into()),
            album: Some("Record".into()),
            cover_file: None,
            cover_url: None,
            duration: Some(Duration::from_secs(200)),
        }
    }

    #[test]
    fn system_events_become_controls() {
        assert_eq!(control(Event::Play), Some(Control::Play));
        assert_eq!(control(Event::Pause), Some(Control::Pause));
        assert_eq!(control(Event::PlayPause), Some(Control::TogglePlayPause));
        assert_eq!(control(Event::Stop), Some(Control::Stop));
        assert_eq!(control(Event::Next), Some(Control::Next));
        assert_eq!(control(Event::Previous), Some(Control::Previous));
        assert_eq!(
            control(Event::SeekTo(Duration::from_secs(42))),
            Some(Control::SeekTo(Duration::from_secs(42)))
        );
        assert_eq!(
            control(Event::SetRepeat(playwire::Repeat::One)),
            Some(Control::SetRepeat(Repeat::One))
        );
        assert_eq!(
            control(Event::SetShuffle(true)),
            Some(Control::SetShuffle(true))
        );
        assert_eq!(control(Event::Quit), Some(Control::Quit));
        assert_eq!(control(Event::Raise), Some(Control::Raise));
        assert_eq!(control(Event::OpenUri("file:///a.flac".into())), None);
    }

    #[test]
    fn relative_seeks_keep_their_direction() {
        assert_eq!(
            control(Event::SeekBy(-10.0)),
            Some(Control::SeekBy {
                forward: false,
                by: Duration::from_secs(10)
            })
        );
        assert_eq!(
            control(Event::SeekBy(2.5)),
            Some(Control::SeekBy {
                forward: true,
                by: Duration::from_millis(2500)
            })
        );
        assert_eq!(control(Event::SeekBy(f64::NAN)), None);
    }

    #[test]
    fn volume_requests_stay_in_range() {
        assert_eq!(
            control(Event::SetVolume(1.7)),
            Some(Control::SetVolume(1.0))
        );
        assert_eq!(
            control(Event::SetVolume(-1.0)),
            Some(Control::SetVolume(0.0))
        );
        assert_eq!(control(Event::SetVolume(f64::INFINITY)), None);
    }

    #[test]
    fn the_playing_track_reaches_the_system() {
        let state = playback_state(&MediaState {
            track: Some(track()),
            playing: true,
            position: Duration::from_secs(12),
            volume: 0.5,
            shuffle: true,
            repeat: Repeat::All,
        });
        let shown = state.track.as_ref().unwrap();
        assert_eq!(shown.id, "/music/Record/01 Opening.flac");
        assert_eq!(shown.title, "Opening");
        assert_eq!(shown.artists, ["Band"]);
        assert_eq!(shown.album, "Record");
        assert!(state.playing);
        assert_eq!(state.position, Duration::from_secs(12));
        assert_eq!(state.duration, Some(Duration::from_secs(200)));
        assert_eq!(state.volume, 0.5);
        assert!(state.shuffle);
        assert_eq!(state.repeat, playwire::Repeat::All);
        assert!(state.capabilities.can_go_next && state.capabilities.can_seek);
    }

    #[test]
    fn nothing_playing_clears_the_widget() {
        let state = playback_state(&MediaState {
            playing: true,
            ..MediaState::default()
        });
        assert_eq!(state.track, None);
        assert!(!state.playing);
        assert!(!state.capabilities.can_go_next);
        assert!(!state.capabilities.can_seek);
    }

    #[test]
    fn a_track_of_unknown_length_cannot_be_seeked() {
        let state = playback_state(&MediaState {
            track: Some(MediaTrack {
                duration: None,
                ..track()
            }),
            ..MediaState::default()
        });
        assert!(!state.capabilities.can_seek);
        assert_eq!(state.duration, None);
    }

    #[test]
    fn covers_come_from_disk_or_the_web() {
        let dir = std::env::temp_dir().join("vcmp covers");
        let on_disk = MediaTrack {
            cover_file: Some(dir.join("ab#c.jpg")),
            cover_url: Some("https://coverartarchive.org/release/x/front-250".into()),
            ..track()
        };
        let file = artwork_url(&on_disk, false).unwrap();
        assert!(file.starts_with("file:///"), "{file}");
        // Spaces and '#' would end or break the address.
        assert!(file.contains("vcmp%20covers/ab%23c.jpg"), "{file}");
        assert_eq!(
            url::Url::parse(&file).unwrap().to_file_path().unwrap(),
            dir.join("ab#c.jpg")
        );
        assert_eq!(
            artwork_url(&on_disk, true).as_deref(),
            Some("https://coverartarchive.org/release/x/front-250")
        );
        let web_less = MediaTrack {
            cover_url: None,
            ..on_disk
        };
        assert_eq!(artwork_url(&web_less, true), None);
        assert_eq!(artwork_url(&track(), false), None);
    }
}
