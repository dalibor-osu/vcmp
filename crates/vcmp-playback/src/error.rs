use std::fmt;
use std::io;

/// Why a track could not be played or the output could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaybackError {
    /// Opening or reading a file failed (missing file, unplugged drive, no access).
    Io(io::ErrorKind, String),
    /// Not a supported format, or no audio track in it.
    Unsupported(String),
    /// The audio data is broken beyond skipping a damaged frame.
    Decode(String),
    /// No output device could be opened, or the stream failed.
    Output(String),
    /// A saved queue did not match its context.
    InvalidSnapshot,
}

impl fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaybackError::Io(_, message) => write!(f, "cannot read the file: {message}"),
            PlaybackError::Unsupported(message) => write!(f, "unsupported audio: {message}"),
            PlaybackError::Decode(message) => write!(f, "cannot decode the audio: {message}"),
            PlaybackError::Output(message) => write!(f, "audio output failed: {message}"),
            PlaybackError::InvalidSnapshot => {
                f.write_str("the saved queue does not match its tracks")
            }
        }
    }
}

impl std::error::Error for PlaybackError {}

impl From<io::Error> for PlaybackError {
    fn from(error: io::Error) -> Self {
        PlaybackError::Io(error.kind(), error.to_string())
    }
}

impl From<symphonia::core::errors::Error> for PlaybackError {
    fn from(error: symphonia::core::errors::Error) -> Self {
        use symphonia::core::errors::Error;
        match error {
            Error::IoError(error) => error.into(),
            Error::Unsupported(what) => PlaybackError::Unsupported(what.to_owned()),
            other => PlaybackError::Decode(other.to_string()),
        }
    }
}
