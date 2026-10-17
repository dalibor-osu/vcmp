//! Where decoded audio comes from. A source turns a track id into a stream of
//! decoded samples; the engine does everything after that (format conversion,
//! gapless joining, output) the same way for every source.

use std::sync::Arc;
use std::time::Duration;

use crate::error::PlaybackError;
use crate::local::LocalSource;
use crate::track::SourceKind;

/// Sample rate and channel count of decoded audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Decoded audio: interleaved `f32` samples in `format`, channels in the usual
/// WAVE/FLAC order (front left, front right, centre, LFE, back/side ...).
#[derive(Clone, Copy, Debug)]
pub struct AudioChunk<'a> {
    pub format: AudioFormat,
    pub samples: &'a [f32],
}

/// Opens tracks of one [`SourceKind`].
pub trait TrackSource: Send + Sync {
    fn open(&self, id: &str) -> Result<Box<dyn TrackStream>, PlaybackError>;
}

/// One opened track, decoded on demand by the engine's decoder thread.
pub trait TrackStream: Send {
    /// Length of the track, when the file declares it.
    fn duration(&self) -> Option<Duration>;

    /// The next decoded chunk, with encoder delay and padding already removed so
    /// consecutive tracks join without a gap. `Ok(None)` at the end of the track.
    fn next_chunk(&mut self) -> Result<Option<AudioChunk<'_>>, PlaybackError>;

    /// Moves to `position` exactly: the next chunk starts with the sample at that
    /// time. Returns the position reached, which is earlier only when `position` is
    /// past the end.
    fn seek(&mut self, position: Duration) -> Result<Duration, PlaybackError>;
}

/// The track sources the engine can play from.
#[derive(Clone)]
pub struct Sources {
    local: Arc<dyn TrackSource>,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            local: Arc::new(LocalSource),
        }
    }
}

impl Sources {
    /// Replaces the source used for `kind`.
    pub fn with(mut self, kind: SourceKind, source: Arc<dyn TrackSource>) -> Self {
        match kind {
            SourceKind::Local => self.local = source,
        }
        self
    }

    pub fn get(&self, kind: SourceKind) -> &dyn TrackSource {
        match kind {
            SourceKind::Local => self.local.as_ref(),
        }
    }
}
