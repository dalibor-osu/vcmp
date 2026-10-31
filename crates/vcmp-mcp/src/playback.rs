//! Playback agents control: playing tracks, albums, playlists, labels and artists,
//! Up next, the player's buttons and options, the output device, the sleep timer, and
//! what plays now. Unlike tag changes, playback acts at once, without review.
//!
//! What to play is read here, on the server's library connection, so the tracks that
//! can't be played are named to the agent; the app plays the rest through
//! [`Playback`], as the window's buttons do.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use vcmp_library::{AlbumId, ArtistId, LabelId, Library, PlaylistId, Track, TrackId};

use crate::tools::{MAX_FILES, VcmpServer, agent, failed, track_out};

/// What every agent is told about playback when it connects, after the library's part.
pub(crate) const INSTRUCTIONS: &str = "\
Playing: play_tracks, play_album, play_playlist, play_label and play_artist replace \
what plays with those tracks and start at once, as VCMP's own play buttons do; \
queue_tracks puts tracks into Up next, which plays before the rest. Nothing waits for \
review, but the owner may have switched playback control off: then these tools say \
so, and you can tell them where to switch it on. Tracks whose files are missing are left out and named in the answer under \
left_out with the reason: tell the owner about them. playback_control plays, pauses, \
stops, skips, seeks and brings VCMP's window to the front; edit_queue changes Up next; \
set_playback_options sets shuffle, repeat and volume; list_output_devices and \
set_output_device choose where the sound goes; set_sleep_timer stops playback after \
this track or after some minutes. get_now_playing tells what plays, where in it, and \
what comes next. When asked for music by mood, a label of that name is the owner's own \
choice: play_label plays it.";

/// Playback agents ask for, done by the app on its own thread at once, so the window,
/// the OS media controls, Discord and Last.fm follow. Each call returns once the app
/// has done it and [`Playback::status`] shows the result, such as the next track after
/// a skip; the error is a reason to tell the agent.
pub trait Playback: Send + Sync + 'static {
    /// Makes `play.tracks` what plays, and plays.
    fn play(&self, play: Play) -> Result<(), String>;
    /// Puts tracks into Up next: at its front (`next`) or its end.
    fn queue(&self, tracks: Vec<Track>, next: bool) -> Result<(), String>;
    fn control(&self, control: Control) -> Result<(), String>;
    fn set_options(&self, options: Options) -> Result<(), String>;
    /// Lists the output devices afresh; the window's list follows.
    fn output_devices(&self) -> Result<OutputDevices, String>;
    /// Plays through the device with this name, or the system default for `None`. A
    /// name the device list doesn't hold is refused.
    fn set_output_device(&self, device: Option<String>) -> Result<(), String>;
    fn set_sleep_timer(&self, timer: SleepTimer) -> Result<(), String>;
    /// What the player shows now.
    fn status(&self) -> Result<Status, String>;
}

/// Tracks to play, as the queue's context.
#[derive(Clone, Debug, PartialEq)]
pub struct Play {
    /// Playable tracks, in play order.
    pub tracks: Vec<Track>,
    pub context: Context,
    /// The index in `tracks` to start with; `None`: the first, or a random one when
    /// shuffling.
    pub start: Option<usize>,
    /// Turns shuffle on or off first; `None` leaves it as it is.
    pub shuffle: Option<bool>,
}

/// What the queue plays from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Context {
    /// Tracks the agent with this name chose; the queue panel names it.
    Agent(String),
    Album(AlbumId),
    /// Everything by an artist: their albums, then their appearances elsewhere.
    Artist(ArtistId),
    Playlist(PlaylistId),
    Label(LabelId),
    /// Played from the window only.
    Folder(PathBuf),
    Search,
    Songs,
}

/// The player's buttons, and Up next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Play,
    Pause,
    TogglePlayPause,
    Stop,
    Next,
    Previous,
    Seek(Duration),
    /// Plays an entry of the queue, counted over Up next and then what follows it
    /// ([`Status::up_next`], then [`Status::upcoming`]); the entries before it go.
    SkipTo(usize),
    RemoveFromUpNext(usize),
    MoveInUpNext {
        from: usize,
        to: usize,
    },
    ClearUpNext,
    /// Brings the window to the front.
    Raise,
}

/// Playback options to change; `None` leaves one as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Options {
    pub shuffle: Option<bool>,
    pub shuffle_kind: Option<ShuffleKind>,
    pub repeat: Option<Repeat>,
    /// 0.0 to 1.0, as the volume slider.
    pub volume: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShuffleKind {
    /// Every track once per cycle, keeping the same artist and album apart.
    FewerRepeats,
    /// Every next track picked at random from all of them.
    TrueRandom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    Off,
    All,
    One,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepTimer {
    Off,
    /// Stops once the track playing now ends.
    EndOfTrack,
    /// Stops after the track playing once this many minutes have passed.
    Minutes(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputDevices {
    /// By name, as Settings lists them.
    pub devices: Vec<String>,
    /// The device chosen; `None` for the system default.
    pub chosen: Option<String>,
    /// The device playing instead while the chosen one is missing.
    pub fallback: Option<String>,
}

/// What the player shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub state: PlayState,
    /// The current track's file.
    pub current: Option<PathBuf>,
    pub position: Duration,
    pub duration: Option<Duration>,
    /// 0.0 to 1.0, as the volume slider.
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub shuffle_kind: ShuffleKind,
    pub repeat: Repeat,
    pub output: OutputDevices,
    pub sleep_timer: SleepTimer,
    pub context: Option<Context>,
    /// The files in Up next, then the context's next ones, in play order.
    pub up_next: Vec<PathBuf>,
    pub upcoming: Vec<PathBuf>,
    /// How many tracks the context holds. With repeat all, or true random shuffle,
    /// `upcoming` goes on past them, to the queue panel's limit.
    pub context_len: usize,
}

/// The most minutes the sleep timer counts down from, as the window's longest choice.
const MAX_SLEEP_MINUTES: u32 = 90;
/// The most queue entries `get_now_playing` lists after Up next by default.
const DEFAULT_UPCOMING: u32 = 10;
/// The most entries the queue panel lists after Up next.
const MAX_UPCOMING: u32 = 100;
/// The most reasons an error for an unplayable request lists.
const MAX_REASONS: usize = 10;

#[derive(Deserialize, JsonSchema)]
pub struct PlayTracksArgs {
    /// In the order to play them, at most 500.
    track_ids: Vec<i64>,
    /// The track to start with (default: the first, or a random one when shuffling).
    start_track_id: Option<i64>,
    /// true turns shuffle on, false turns it off; left out, it stays as it is.
    shuffle: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PlayAlbumArgs {
    album_id: i64,
    /// The track to start with (default: the first, or a random one when shuffling).
    start_track_id: Option<i64>,
    /// true turns shuffle on, false turns it off; left out, it stays as it is.
    shuffle: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PlayPlaylistArgs {
    playlist_id: i64,
    /// The entry to start with, by its position as get_playlist lists it (default:
    /// the first, or a random one when shuffling).
    start_position: Option<u32>,
    /// true turns shuffle on, false turns it off; left out, it stays as it is.
    shuffle: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PlayLabelArgs {
    /// The label's name, ignoring case, as list_labels lists it.
    label: String,
    /// The track to start with (default: the first, or a random one when shuffling).
    start_track_id: Option<i64>,
    /// true turns shuffle on, false turns it off; left out, it stays as it is.
    shuffle: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PlayArtistArgs {
    artist_id: i64,
    /// The track to start with (default: the first, or a random one when shuffling).
    start_track_id: Option<i64>,
    /// true turns shuffle on, false turns it off; left out, it stays as it is.
    shuffle: Option<bool>,
}

#[derive(Clone, Copy, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueueAt {
    /// At the front of Up next: they play after the current track.
    Next,
    /// At the end of Up next.
    #[default]
    End,
}

#[derive(Deserialize, JsonSchema)]
pub struct QueueArgs {
    /// In the order to play them, at most 500.
    track_ids: Vec<i64>,
    /// next or end (default end).
    #[serde(default)]
    at: QueueAt,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Play,
    Pause,
    /// Plays when paused or stopped, pauses when playing.
    Toggle,
    /// Stops; the track stays, and play starts it from its beginning.
    Stop,
    Next,
    /// The previous track, or the start of this one when more than 3 seconds of it
    /// have played.
    Previous,
    /// To position_seconds in the current track.
    Seek,
    /// Brings VCMP's window to the front.
    ShowWindow,
}

#[derive(Deserialize, JsonSchema)]
pub struct ControlArgs {
    action: Action,
    /// For seek: where to go in the current track.
    position_seconds: Option<f64>,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueueEdit {
    /// Plays the entry at position; the entries before it are dropped.
    SkipTo,
    /// Removes the Up next entry at position.
    Remove,
    /// Moves the Up next entry at position to the position to.
    Move,
    /// Empties Up next.
    ClearUpNext,
}

#[derive(Deserialize, JsonSchema)]
pub struct EditQueueArgs {
    action: QueueEdit,
    /// An entry's position as get_now_playing lists it: Up next first, from 0, then
    /// what follows.
    position: Option<u32>,
    /// For move: the Up next position the entry goes to.
    to: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct OptionsArgs {
    /// Shuffle on or off.
    shuffle: Option<bool>,
    /// How shuffle picks: fewer_repeats (every track once per cycle, the same artist
    /// and album kept apart) or true_random.
    shuffle_kind: Option<ShuffleKind>,
    /// off, all (the context starts over) or one (the track repeats).
    repeat: Option<Repeat>,
    /// 0 to 100, as VCMP's volume slider.
    volume_percent: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DeviceArgs {
    /// A name as list_output_devices lists it; left out or null for the system's
    /// default device.
    device: Option<String>,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimerChoice {
    Off,
    /// Stops once the track playing now ends.
    EndOfTrack,
    /// Stops after the track that plays once minutes have passed.
    Minutes,
}

#[derive(Deserialize, JsonSchema)]
pub struct SleepArgs {
    timer: TimerChoice,
    /// For minutes: 1 to 90.
    minutes: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct NowPlayingArgs {
    /// How many entries after Up next to list (default 10, at most 100).
    upcoming_limit: Option<u32>,
}

/// A track asked for, as found in the library.
struct Candidate {
    id: i64,
    /// Its playlist entry's position.
    position: Option<u32>,
    track: Option<Track>,
}

impl Candidate {
    fn of(track: Track) -> Self {
        Self {
            id: track.id.0,
            position: None,
            track: Some(track),
        }
    }
}

/// The tracks to play, where to start, and those left out with the reasons.
struct Chosen {
    tracks: Vec<Track>,
    start: Option<usize>,
    left_out: Vec<serde_json::Value>,
}

/// Why `track` can't be played, if it can't.
fn unplayable(track: &Track) -> Option<String> {
    if !track.available || !track.path.exists() {
        return Some(format!("its file is missing: {}", track.path.display()));
    }
    if track.path.to_str().is_none() {
        return Some(format!(
            "its path is not valid Unicode, which VCMP can't play: {}",
            track.path.display()
        ));
    }
    None
}

/// Sorts `candidates` into those that play and those left out. Starting at one left
/// out starts at the next that plays, as the window does for playlists.
fn choose(candidates: Vec<Candidate>, start: Option<usize>) -> Result<Chosen, String> {
    let mut chosen = Chosen {
        tracks: Vec::with_capacity(candidates.len()),
        start: None,
        left_out: Vec::new(),
    };
    let mut reasons = Vec::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        if start == Some(index) {
            chosen.start = Some(chosen.tracks.len());
        }
        let reason = match &candidate.track {
            None => Some(format!("there is no track {}", candidate.id)),
            Some(track) => unplayable(track),
        };
        let Some(reason) = reason else {
            chosen.tracks.extend(candidate.track);
            continue;
        };
        reasons.push(format!("track {}: {reason}", candidate.id));
        let mut out = serde_json::json!({
            "track_id": candidate.id,
            "title": candidate.track.as_ref().map(|track| &track.title),
            "artist": candidate.track.as_ref().and_then(|track| track.artist.as_ref()),
            "reason": reason,
        });
        if let Some(position) = candidate.position {
            out["position"] = serde_json::json!(position);
        }
        chosen.left_out.push(out);
    }
    if chosen.tracks.is_empty() {
        let more = reasons.len().saturating_sub(MAX_REASONS);
        reasons.truncate(MAX_REASONS);
        if more > 0 {
            reasons.push(format!("and {more} more"));
        }
        return Err(format!(
            "none of these tracks can be played: {}",
            reasons.join("; ")
        ));
    }
    if chosen
        .start
        .is_some_and(|start| start >= chosen.tracks.len())
    {
        return Err(
            "no track from the one to start with on can be played; see left_out in a request without it"
                .into(),
        );
    }
    Ok(chosen)
}

/// The candidate `start_track_id` names.
fn start_at(
    candidates: &[Candidate],
    track: Option<i64>,
    of: &str,
) -> Result<Option<usize>, String> {
    track
        .map(|id| {
            candidates
                .iter()
                .position(|candidate| candidate.id == id)
                .ok_or_else(|| format!("track {id} is not {of}"))
        })
        .transpose()
}

/// The library's tracks by id, in the order given.
fn by_id(library: &Library, ids: &[i64]) -> Result<Vec<Candidate>, String> {
    ids.iter()
        .map(|&id| {
            Ok(Candidate {
                id,
                position: None,
                track: library.track(TrackId(id)).map_err(failed)?,
            })
        })
        .collect()
}

/// Everything by an artist in the artist page's order: their albums, then the tracks
/// they appear on elsewhere.
fn artist_tracks(library: &Library, artist: ArtistId) -> Result<Vec<Track>, String> {
    let mut tracks = Vec::new();
    for album in library.artist_albums(artist).map_err(failed)? {
        tracks.extend(library.album_tracks(album.id).map_err(failed)?);
    }
    tracks.extend(library.artist_appearances(artist).map_err(failed)?);
    Ok(tracks)
}

fn check_count(ids: &[i64]) -> Result<(), String> {
    if ids.is_empty() {
        return Err("name at least one track".into());
    }
    if ids.len() > MAX_FILES {
        return Err(format!("name at most {MAX_FILES} tracks at a time"));
    }
    Ok(())
}

/// What a play request answers: how many tracks play, the one to start with when
/// known, and those left out.
fn played_out(library: &Library, chosen: &Chosen) -> Result<serde_json::Value, String> {
    let first = chosen
        .start
        .map(|start| track_out(library, &chosen.tracks[start], None))
        .transpose()?;
    Ok(serde_json::json!({
        "playing": chosen.tracks.len(),
        "starts_with": first,
        "left_out": chosen.left_out,
    }))
}

fn state_name(state: PlayState) -> &'static str {
    match state {
        PlayState::Playing => "playing",
        PlayState::Paused => "paused",
        PlayState::Stopped => "stopped",
    }
}

fn repeat_name(repeat: Repeat) -> &'static str {
    match repeat {
        Repeat::Off => "off",
        Repeat::All => "all",
        Repeat::One => "one",
    }
}

fn percent(volume: f32) -> u32 {
    (volume.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// A queued file as a track the library knows, or by its path alone.
fn queued_out(library: &Library, path: &Path) -> Result<serde_json::Value, String> {
    let track = match library.track_id_for_path(path).map_err(failed)? {
        Some(id) => library.track(id).map_err(failed)?,
        None => None,
    };
    match track {
        Some(track) => track_out(library, &track, None),
        None => Ok(serde_json::json!({ "path": path.to_string_lossy() })),
    }
}

fn context_out(library: &Library, context: &Context) -> Result<serde_json::Value, String> {
    let (kind, id, name) = match context {
        Context::Agent(name) => ("agent", None, Some(name.clone())),
        Context::Album(id) => (
            "album",
            Some(id.0),
            library.album(*id).map_err(failed)?.map(|album| album.title),
        ),
        Context::Artist(id) => (
            "artist",
            Some(id.0),
            library
                .artist(*id)
                .map_err(failed)?
                .map(|artist| artist.name),
        ),
        Context::Playlist(id) => (
            "playlist",
            Some(id.0),
            library
                .playlist(*id)
                .map_err(failed)?
                .map(|playlist| playlist.name),
        ),
        Context::Label(id) => (
            "label",
            None,
            library.label(*id).map_err(failed)?.map(|label| label.name),
        ),
        Context::Folder(path) => ("folder", None, Some(path.to_string_lossy().into_owned())),
        Context::Search => ("search", None, None),
        Context::Songs => ("songs", None, None),
    };
    Ok(serde_json::json!({ "kind": kind, "id": id, "name": name }))
}

fn status_out(
    library: &Library,
    status: &Status,
    upcoming: usize,
) -> Result<serde_json::Value, String> {
    let entries = |paths: &[PathBuf], from: usize| -> Result<Vec<serde_json::Value>, String> {
        paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                Ok(serde_json::json!({
                    "position": from + index,
                    "track": queued_out(library, path)?,
                }))
            })
            .collect()
    };
    let (timer, minutes) = match status.sleep_timer {
        SleepTimer::Off => ("off", None),
        SleepTimer::EndOfTrack => ("end_of_track", None),
        SleepTimer::Minutes(minutes) => ("minutes", Some(minutes)),
    };
    // A context that never ends is listed for one pass, as many entries as it holds:
    // the rest repeats it, or draws from it at random.
    let repeats = status.context_len > 0
        && (status.repeat == Repeat::All
            || (status.shuffle && status.shuffle_kind == ShuffleKind::TrueRandom));
    let pass = if repeats {
        &status.upcoming[..status.context_len.min(status.upcoming.len())]
    } else {
        &status.upcoming[..]
    };
    let shown = &pass[..upcoming.min(pass.len())];
    Ok(serde_json::json!({
        "state": state_name(status.state),
        "track": status.current.as_ref().map(|path| queued_out(library, path)).transpose()?,
        "position_seconds": (status.position.as_secs_f64() * 10.0).round() / 10.0,
        "duration_seconds": status.duration.map(|duration| duration.as_secs()),
        "volume_percent": percent(status.volume),
        "muted": status.muted,
        "shuffle": status.shuffle,
        "shuffle_kind": match status.shuffle_kind {
            ShuffleKind::FewerRepeats => "fewer_repeats",
            ShuffleKind::TrueRandom => "true_random",
        },
        "repeat": repeat_name(status.repeat),
        "output_device": status.output.chosen,
        "playing_through_instead": status.output.fallback,
        "sleep_timer": { "timer": timer, "minutes_left": minutes },
        "context": status.context.as_ref().map(|context| context_out(library, context)).transpose()?,
        "up_next": entries(&status.up_next, 0)?,
        "upcoming": entries(shown, status.up_next.len())?,
        "more_upcoming": pass.len() - shown.len(),
        "repeats": repeats,
    }))
}

#[tool_router(router = playback_router, vis = "pub(crate)")]
impl VcmpServer {
    #[tool(
        description = "Play tracks by id, in the order given, in place of what plays now; VCMP's queue names you as where they come from. Tracks whose files are missing are left out and named in the answer."
    )]
    async fn play_tracks(
        &self,
        Parameters(args): Parameters<PlayTracksArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        check_count(&args.track_ids)?;
        let agent = agent(&context);
        self.play_from(move |library| {
            let candidates = by_id(library, &args.track_ids)?;
            let start = start_at(&candidates, args.start_track_id, "among track_ids")?;
            Ok((candidates, start, Context::Agent(agent), args.shuffle))
        })
        .await
    }

    #[tool(description = "Play an album, as its page's play button does.")]
    async fn play_album(
        &self,
        Parameters(args): Parameters<PlayAlbumArgs>,
    ) -> Result<String, String> {
        self.play_from(move |library| {
            let id = AlbumId(args.album_id);
            library
                .album(id)
                .map_err(failed)?
                .ok_or_else(|| format!("there is no album {}", args.album_id))?;
            let candidates: Vec<Candidate> = library
                .album_tracks(id)
                .map_err(failed)?
                .into_iter()
                .map(Candidate::of)
                .collect();
            let start = start_at(&candidates, args.start_track_id, "on this album")?;
            Ok((candidates, start, Context::Album(id), args.shuffle))
        })
        .await
    }

    #[tool(
        description = "Play a playlist, as its page's play button does; entries whose files are missing are left out and named in the answer."
    )]
    async fn play_playlist(
        &self,
        Parameters(args): Parameters<PlayPlaylistArgs>,
    ) -> Result<String, String> {
        self.play_from(move |library| {
            let id = PlaylistId(args.playlist_id);
            library
                .playlist(id)
                .map_err(failed)?
                .ok_or_else(|| format!("there is no playlist {}", args.playlist_id))?;
            let candidates: Vec<Candidate> = library
                .playlist_entries(id)
                .map_err(failed)?
                .into_iter()
                .map(|entry| Candidate {
                    id: entry.track.id.0,
                    position: Some(entry.position),
                    track: Some(entry.track),
                })
                .collect();
            if candidates.is_empty() {
                return Err(format!("playlist {} is empty", args.playlist_id));
            }
            let start = args
                .start_position
                .map(|position| {
                    candidates
                        .iter()
                        .position(|candidate| candidate.position == Some(position))
                        .ok_or_else(|| format!("the playlist has no entry at position {position}"))
                })
                .transpose()?;
            Ok((candidates, start, Context::Playlist(id), args.shuffle))
        })
        .await
    }

    #[tool(
        description = "Play the tracks holding one of the owner's labels, such as their chill music, as the label's page does."
    )]
    async fn play_label(
        &self,
        Parameters(args): Parameters<PlayLabelArgs>,
    ) -> Result<String, String> {
        self.play_from(move |library| {
            let label = library
                .label_named(&args.label)
                .map_err(failed)?
                .ok_or_else(|| {
                    format!("there is no label named {:?}; see list_labels", args.label)
                })?;
            let candidates: Vec<Candidate> = library
                .label_tracks(label.id)
                .map_err(failed)?
                .into_iter()
                .map(Candidate::of)
                .collect();
            if candidates.is_empty() {
                return Err(format!(
                    "no available track holds the label {:?}",
                    label.name
                ));
            }
            let start = start_at(&candidates, args.start_track_id, "under this label")?;
            Ok((candidates, start, Context::Label(label.id), args.shuffle))
        })
        .await
    }

    #[tool(
        description = "Play everything by an artist, as the artist page does: their albums, then the tracks they appear on elsewhere."
    )]
    async fn play_artist(
        &self,
        Parameters(args): Parameters<PlayArtistArgs>,
    ) -> Result<String, String> {
        self.play_from(move |library| {
            let id = ArtistId(args.artist_id);
            library
                .artist(id)
                .map_err(failed)?
                .ok_or_else(|| format!("there is no artist {}", args.artist_id))?;
            let candidates: Vec<Candidate> = artist_tracks(library, id)?
                .into_iter()
                .map(Candidate::of)
                .collect();
            if candidates.is_empty() {
                return Err(format!("artist {} has no available tracks", args.artist_id));
            }
            let start = start_at(&candidates, args.start_track_id, "by this artist")?;
            Ok((candidates, start, Context::Artist(id), args.shuffle))
        })
        .await
    }

    #[tool(
        description = "Put tracks into Up next, which plays before the rest of the queue: next (right after the current track) or at its end. Tracks whose files are missing are left out and named in the answer."
    )]
    async fn queue_tracks(
        &self,
        Parameters(args): Parameters<QueueArgs>,
    ) -> Result<String, String> {
        check_count(&args.track_ids)?;
        let chosen = self
            .read_raw(move |library| choose(by_id(library, &args.track_ids)?, None))
            .await?;
        let queued = chosen.tracks.len();
        let next = args.at == QueueAt::Next;
        self.play_on(move |playback| {
            playback.queue(chosen.tracks, next)?;
            Ok(serde_json::json!({
                "queued": queued,
                "at": if next { "next" } else { "end" },
                "left_out": chosen.left_out,
            }))
        })
        .await
    }

    #[tool(
        description = "The player's buttons: play, pause, toggle, stop, next, previous, seek (to position_seconds) or show_window (bring VCMP to the front)."
    )]
    async fn playback_control(
        &self,
        Parameters(args): Parameters<ControlArgs>,
    ) -> Result<String, String> {
        let control = match args.action {
            Action::Play => Control::Play,
            Action::Pause => Control::Pause,
            Action::Toggle => Control::TogglePlayPause,
            Action::Stop => Control::Stop,
            Action::Next => Control::Next,
            Action::Previous => Control::Previous,
            Action::Seek => {
                let seconds = args.position_seconds.ok_or("seek needs position_seconds")?;
                Control::Seek(
                    Duration::try_from_secs_f64(seconds)
                        .map_err(|_| "position_seconds must be 0 or more")?,
                )
            }
            Action::ShowWindow => Control::Raise,
        };
        self.play_on(move |playback| {
            playback.control(control)?;
            Ok("done; get_now_playing shows the result")
        })
        .await
    }

    #[tool(
        description = "Change the queue: skip_to plays the entry at position (entries before it are dropped), remove takes an Up next entry out, move moves one to the Up next position to, clear_up_next empties Up next. Positions are as get_now_playing lists them."
    )]
    async fn edit_queue(
        &self,
        Parameters(args): Parameters<EditQueueArgs>,
    ) -> Result<String, String> {
        let position = || {
            args.position
                .map(|position| position as usize)
                .ok_or_else(|| "this action needs position".to_owned())
        };
        let control = match args.action {
            QueueEdit::SkipTo => Control::SkipTo(position()?),
            QueueEdit::Remove => Control::RemoveFromUpNext(position()?),
            QueueEdit::Move => Control::MoveInUpNext {
                from: position()?,
                to: args.to.ok_or("move needs to")? as usize,
            },
            QueueEdit::ClearUpNext => Control::ClearUpNext,
        };
        self.play_on(move |playback| {
            playback.control(control)?;
            Ok("done")
        })
        .await
    }

    #[tool(
        description = "Set shuffle (on or off, and how it picks), repeat (off, all or one) and the volume (0 to 100); what is left out stays as it is."
    )]
    async fn set_playback_options(
        &self,
        Parameters(args): Parameters<OptionsArgs>,
    ) -> Result<String, String> {
        if args.volume_percent.is_some_and(|volume| volume > 100) {
            return Err("volume_percent goes from 0 to 100".into());
        }
        let options = Options {
            shuffle: args.shuffle,
            shuffle_kind: args.shuffle_kind,
            repeat: args.repeat,
            volume: args.volume_percent.map(|volume| volume as f32 / 100.0),
        };
        if options == Options::default() {
            return Err("name at least one option to set".into());
        }
        self.play_on(move |playback| {
            playback.set_options(options)?;
            Ok("set")
        })
        .await
    }

    #[tool(
        description = "The output devices VCMP can play through, the one chosen (null: the system's default) and, when the chosen one is missing, the device playing instead."
    )]
    async fn list_output_devices(&self) -> Result<String, String> {
        self.play_on(|playback| {
            let devices = playback.output_devices()?;
            Ok(serde_json::json!({
                "devices": devices.devices,
                "chosen": devices.chosen,
                "playing_through_instead": devices.fallback,
            }))
        })
        .await
    }

    #[tool(
        description = "Play through an output device, by its name from list_output_devices, or the system's default with no device. VCMP remembers the choice."
    )]
    async fn set_output_device(
        &self,
        Parameters(args): Parameters<DeviceArgs>,
    ) -> Result<String, String> {
        let device = args.device.filter(|device| !device.trim().is_empty());
        self.play_on(move |playback| {
            playback.set_output_device(device)?;
            Ok("set")
        })
        .await
    }

    #[tool(
        description = "The sleep timer: off, end_of_track (stop once this track ends), or minutes (stop after the track that plays once 1 to 90 minutes have passed)."
    )]
    async fn set_sleep_timer(
        &self,
        Parameters(args): Parameters<SleepArgs>,
    ) -> Result<String, String> {
        let timer = match args.timer {
            TimerChoice::Off => SleepTimer::Off,
            TimerChoice::EndOfTrack => SleepTimer::EndOfTrack,
            TimerChoice::Minutes => match args.minutes {
                Some(minutes @ 1..=MAX_SLEEP_MINUTES) => SleepTimer::Minutes(minutes),
                _ => return Err(format!("minutes goes from 1 to {MAX_SLEEP_MINUTES}")),
            },
        };
        self.play_on(move |playback| {
            playback.set_sleep_timer(timer)?;
            Ok("set")
        })
        .await
    }

    #[tool(
        description = "What plays now: the track, playing, paused or stopped, the position, volume, shuffle, repeat, output device and sleep timer, what it plays from, Up next, and the tracks after it. With repeats true (repeat all, or true_random shuffle) the tracks after it go on without end: they are listed for one pass, as many as the context holds."
    )]
    async fn get_now_playing(
        &self,
        Parameters(args): Parameters<NowPlayingArgs>,
    ) -> Result<String, String> {
        let upcoming = args
            .upcoming_limit
            .unwrap_or(DEFAULT_UPCOMING)
            .min(MAX_UPCOMING) as usize;
        let playback = self.playback.clone();
        let status = tokio::task::spawn_blocking(move || playback.status())
            .await
            .map_err(|error| error.to_string())??;
        self.read(move |library| status_out(library, &status, upcoming))
            .await
    }
}

impl VcmpServer {
    /// Reads what to play off the server's thread with `find`, which gives the
    /// candidates, where to start, the context and shuffle, then has the app play it.
    async fn play_from(
        &self,
        find: impl FnOnce(
            &Library,
        )
            -> Result<(Vec<Candidate>, Option<usize>, Context, Option<bool>), String>
        + Send
        + 'static,
    ) -> Result<String, String> {
        let (chosen, context, shuffle, answer) = self
            .read_raw(move |library| {
                let (candidates, start, context, shuffle) = find(library)?;
                let chosen = choose(candidates, start)?;
                let answer = played_out(library, &chosen)?;
                Ok((chosen, context, shuffle, answer))
            })
            .await?;
        self.play_on(move |playback| {
            playback.play(Play {
                tracks: chosen.tracks,
                context,
                start: chosen.start,
                shuffle,
            })?;
            Ok(answer)
        })
        .await
    }
}
