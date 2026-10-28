use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is not a directory")]
    NotADirectory(PathBuf),
    #[error(
        "the database was written by a newer VCMP (schema {found}, this build knows {supported})"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("invalid tag edit: {0}")]
    InvalidTagEdit(#[from] crate::edit::InvalidTagEdit),
    #[error("file watcher: {0}")]
    Watch(#[from] notify::Error),
    #[error("{0} is not an image VCMP can read")]
    UnreadableImage(PathBuf),
    #[error("a change set needs at least one change")]
    EmptyChangeSet,
    #[error("track {0:?} is not in the library")]
    UnknownTrack(crate::TrackId),
    #[error("the file of track {0:?} is not available")]
    UnavailableTrack(crate::TrackId),
    #[error("track {0:?} appears twice in one change set")]
    RepeatedTrack(crate::TrackId),
    #[error("the tag of track {track:?} can't hold {key:?} as given")]
    UnsupportedKey { track: crate::TrackId, key: String },
    #[error("playlist {0:?} does not exist")]
    UnknownPlaylist(crate::PlaylistId),
    #[error("change set {0:?} does not exist (any more)")]
    UnknownChangeSet(crate::ChangeSetId),
    #[error("change set {0:?} is not waiting for review")]
    ChangeSetNotPending(crate::ChangeSetId),
    #[error(
        "change set {0:?} can't be undone: only an accepted tag or label set with written files can, once"
    )]
    NotUndoable(crate::ChangeSetId),
    #[error("{0:?} can't be a label's name: it needs 1 to 40 characters")]
    InvalidLabelName(String),
    #[error("a label named {0:?} exists already")]
    LabelNameTaken(String),
    #[error("{0:?} is not a label colour: use palette: and a palette name, or #rrggbb")]
    InvalidLabelColour(String),
    #[error("there is no label named {0:?}")]
    UnknownLabel(String),
    #[error("track {track:?} would both get and lose the label {label:?}")]
    ContradictoryLabelEdit {
        track: crate::TrackId,
        label: String,
    },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn io_error(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}
