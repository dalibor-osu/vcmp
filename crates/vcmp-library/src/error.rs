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
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub(crate) fn io_error(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}
