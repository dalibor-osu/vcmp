//! References to tracks, independent of where their audio comes from.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

/// Which [`TrackSource`](crate::TrackSource) a track belongs to.
///
/// v1 has only local files. A Spotify source is planned behind the same queue,
/// shuffle and controls, which is why queue entries carry the source at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceKind {
    /// Files on a local or mounted disk; the track id is the file path.
    #[default]
    Local,
}

impl SourceKind {
    /// Stable name for storing the source, for example in a saved queue.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Local => "local",
        }
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SourceKind {
    type Err = UnknownSource;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(SourceKind::Local),
            _ => Err(UnknownSource(s.to_owned())),
        }
    }
}

/// A stored source name that this version does not know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownSource(pub String);

impl fmt::Display for UnknownSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown track source \"{}\"", self.0)
    }
}

impl std::error::Error for UnknownSource {}

/// A playable track: a source plus an id that the source understands.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrackRef {
    pub source: SourceKind,
    /// For [`SourceKind::Local`], the absolute file path.
    pub id: Arc<str>,
}

impl TrackRef {
    /// A local file. The path must be valid UTF-8.
    pub fn local(path: impl Into<Arc<str>>) -> Self {
        TrackRef {
            source: SourceKind::Local,
            id: path.into(),
        }
    }
}

/// One track in a play context or in Up next.
///
/// `artist` and `album` are opaque grouping keys used by the "less repeats" shuffle:
/// tracks with equal keys are kept out of adjacent slots. Use something unique per
/// artist and per album (a database id works). `None` means unknown and never
/// matches another track.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QueueEntry {
    pub track: TrackRef,
    pub artist: Option<Arc<str>>,
    pub album: Option<Arc<str>>,
}

impl QueueEntry {
    pub fn new(track: TrackRef) -> Self {
        QueueEntry {
            track,
            artist: None,
            album: None,
        }
    }

    pub fn with_artist(mut self, artist: impl Into<Arc<str>>) -> Self {
        self.artist = Some(artist.into());
        self
    }

    pub fn with_album(mut self, album: impl Into<Arc<str>>) -> Self {
        self.album = Some(album.into());
        self
    }
}

impl From<TrackRef> for QueueEntry {
    fn from(track: TrackRef) -> Self {
        QueueEntry::new(track)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_round_trip() {
        assert_eq!("local".parse::<SourceKind>(), Ok(SourceKind::Local));
        assert_eq!(SourceKind::Local.to_string(), "local");
        assert!("spotify".parse::<SourceKind>().is_err());
    }
}
