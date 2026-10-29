//! The public face of the engine: a [`Player`] handle that takes [`Command`]s and an
//! event channel that reports what happened. Neither side shares mutable state with
//! the other, and nothing here depends on the UI toolkit.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::engine::{Engine, Message};
use crate::error::PlaybackError;
use crate::output::{CpalOutput, Output};
use crate::queue::{QueueSnapshot, QueueView, RepeatMode, ShuffleMode};
use crate::source::{AudioFormat, Sources};
use crate::track::{QueueEntry, TrackRef};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PlaybackState {
    /// Nothing is loaded, or playback was stopped and will start from the beginning.
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// The track now audible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NowPlaying {
    pub entry: QueueEntry,
    pub duration: Option<Duration>,
}

/// The output device in use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputInfo {
    pub device: String,
    pub format: AudioFormat,
    /// The device asked for is missing, so the system default plays instead. The UI
    /// should say so.
    pub fell_back: bool,
}

/// The queue and the place in the current track, for resuming after a restart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaybackSnapshot {
    pub queue: QueueSnapshot,
    pub position: Duration,
}

/// What the UI asks the engine to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Plays an album, playlist or other list: it becomes the context. `start` is the
    /// index of the track to begin with (`None`: the first, or a random one when
    /// shuffling). `origin` is the app's own key for the context, handed back in
    /// [`QueueView::origin`].
    PlayContext {
        entries: Vec<QueueEntry>,
        origin: Option<String>,
        start: Option<usize>,
    },
    /// Puts tracks at the front of Up next.
    PlayNext(Vec<QueueEntry>),
    /// Appends tracks to Up next.
    AddToQueue(Vec<QueueEntry>),
    RemoveFromUpNext(usize),
    MoveInUpNext {
        from: usize,
        to: usize,
    },
    ClearUpNext,
    /// Plays an entry of the queue panel, counted over [`QueueView::up_next`]
    /// followed by [`QueueView::upcoming`]; the entries before it are dropped.
    SkipTo(usize),
    Play,
    Pause,
    TogglePlayPause,
    /// Stops; the current track stays and Play starts it from the beginning.
    Stop,
    Next,
    /// Goes to the previous track, or back to the start of the current one when more
    /// than [`RESTART_THRESHOLD`] of it has played.
    Previous,
    Seek(Duration),
    /// 0.0 to 1.0, as a volume slider; the engine applies a perceptual curve.
    SetVolume(f32),
    SetShuffle(ShuffleMode),
    SetRepeat(RepeatMode),
    /// Plays through the device with this name (from [`output_devices`]), or the
    /// system default for `None`. A missing device falls back to the default.
    ///
    /// [`output_devices`]: crate::output_devices
    SetOutputDevice(Option<String>),
    /// Brings back a saved queue with its current track loaded, paused at the saved
    /// position.
    Restore(PlaybackSnapshot),
    /// Stops at the end of whichever track is playing then, with the queue moved on to
    /// the next one, ready to play from its start; nothing of that track is heard.
    /// Skipping keeps it set for the new track. It is cleared once it has stopped,
    /// by [`Command::Stop`], and when the queue runs out.
    StopAfterCurrent(bool),
}

/// What the engine reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    StateChanged(PlaybackState),
    /// A track started playing, or `None` when the queue ran out. Seeking does not
    /// send this; a track repeating does. After [`Command::StopAfterCurrent`] it names
    /// the track that waits, stopped, with no duration until it plays.
    TrackChanged(Option<NowPlaying>),
    /// The position in the current track, a few times a second while playing and
    /// after every seek.
    Position(Duration),
    /// The track being decoded, or `None` when no track is. Its file stays open
    /// meanwhile: from the start of the track (or shortly before, when it follows
    /// gaplessly) until it is decoded to its end, a ring's length before it ends, or
    /// playback stops or moves elsewhere. Pausing keeps it open. Sent when the track
    /// changes, not on a seek. On Windows an open file can't be replaced, so the tag
    /// editor waits for this before writing the file.
    Decoding(Option<TrackRef>),
    QueueChanged(QueueView),
    /// A track could not be played and was skipped.
    TrackFailed {
        track: TrackRef,
        error: PlaybackError,
    },
    /// An output device was opened.
    OutputChanged(OutputInfo),
    /// No output device could be opened. Playback pauses; Play tries again.
    OutputFailed(PlaybackError),
    /// A [`Command::Restore`] snapshot did not fit together.
    RestoreFailed(PlaybackError),
    /// Whether playback stops after the current track, sent when that changes: when
    /// asked, and when it is cleared, which it is also once it has stopped.
    StopAfterCurrent(bool),
}

/// How much of a track must have played for Previous to restart it instead.
pub const RESTART_THRESHOLD: Duration = Duration::from_secs(3);

/// Settings the player starts with.
#[derive(Clone)]
pub struct PlayerOptions {
    pub sources: Sources,
    /// The device to play through, by name; `None` for the system default.
    pub output_device: Option<String>,
    /// 0.0 to 1.0, see [`Command::SetVolume`].
    pub volume: f32,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        PlayerOptions {
            sources: Sources::default(),
            output_device: None,
            volume: 1.0,
        }
    }
}

/// Handle to the playback engine, which runs on its own thread until the handle is
/// dropped.
pub struct Player {
    messages: Sender<Message>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Starts the engine on the system's audio output. A missing or failing device
    /// does not fail this: it is reported as [`Event::OutputFailed`].
    pub fn new(options: PlayerOptions) -> io::Result<(Player, Receiver<Event>)> {
        Player::with_output(options, || Box::new(CpalOutput::new()))
    }

    pub(crate) fn with_output(
        options: PlayerOptions,
        output: impl FnOnce() -> Box<dyn Output> + Send + 'static,
    ) -> io::Result<(Player, Receiver<Event>)> {
        let (messages, inbox) = mpsc::channel();
        let (events, outbox) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("vcmp-playback".into())
            .spawn(move || Engine::new(inbox, events, options, output()).run())?;
        Ok((
            Player {
                messages,
                thread: Some(thread),
            },
            outbox,
        ))
    }

    pub fn send(&self, command: Command) {
        // Only fails once the engine has stopped, when there is nothing left to do.
        let _ = self.messages.send(Message::Command(command));
    }

    /// The queue and position now, for saving on exit. Waits for the engine (a few
    /// milliseconds at most).
    pub fn snapshot(&self) -> Option<PlaybackSnapshot> {
        let (reply, answer) = mpsc::channel();
        self.messages.send(Message::Snapshot(reply)).ok()?;
        answer.recv().ok()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.messages.send(Message::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
