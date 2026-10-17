//! Playback engine: decoding, the output stream and device selection, gapless
//! transitions, track sources, the play queue and shuffle. Must not depend on the UI
//! crate.
//!
//! Start a [`Player`], send it [`Command`]s and read [`Event`]s from the channel it
//! returns. The engine runs on its own thread; audio reaches the device through a
//! lock-free ring buffer that a real-time callback reads without allocating,
//! locking or waiting. `docs/playback.md` describes the design.
//!
//! ```no_run
//! use vcmp_playback::{Command, Event, Player, PlayerOptions, QueueEntry, TrackRef};
//!
//! let (player, events) = Player::new(PlayerOptions::default())?;
//! let album = vec![
//!     QueueEntry::new(TrackRef::local("/music/album/01.flac")),
//!     QueueEntry::new(TrackRef::local("/music/album/02.flac")),
//! ];
//! player.send(Command::PlayContext { entries: album, origin: None, start: None });
//! for event in events {
//!     if let Event::TrackChanged(Some(now)) = event {
//!         println!("now playing {}", now.entry.track.id);
//!     }
//! }
//! # Ok::<(), std::io::Error>(())
//! ```

#[cfg(test)]
mod alloc_counter;
mod convert;
mod engine;
mod error;
mod local;
mod output;
mod player;
mod queue;
mod ring;
mod shuffle;
mod source;
#[cfg(test)]
mod test_signal;
mod track;

pub use error::PlaybackError;
pub use local::LocalSource;
pub use output::output_devices;
pub use player::{
    Command, Event, NowPlaying, OutputInfo, PlaybackSnapshot, PlaybackState, Player, PlayerOptions,
    RESTART_THRESHOLD,
};
pub use queue::{
    Advance, InvalidSnapshot, Queue, QueueSlot, QueueSnapshot, QueueView, RepeatMode, ShuffleMode,
};
pub use ring::Renderer;
pub use source::{AudioChunk, AudioFormat, Sources, TrackSource, TrackStream};
pub use track::{QueueEntry, SourceKind, TrackRef, UnknownSource};

#[cfg(test)]
#[global_allocator]
static ALLOCATOR: alloc_counter::CountingAllocator = alloc_counter::CountingAllocator;
