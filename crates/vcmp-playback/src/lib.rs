//! Playback engine: decoding, the output stream and device selection, gapless
//! transitions, track sources, the play queue and shuffle. Must not depend on the UI
//! crate.

mod queue;
mod shuffle;
mod track;

pub use queue::{
    Advance, InvalidSnapshot, Queue, QueueSlot, QueueSnapshot, QueueView, RepeatMode, ShuffleMode,
};
pub use track::{QueueEntry, SourceKind, TrackRef, UnknownSource};
