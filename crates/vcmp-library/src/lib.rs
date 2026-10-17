//! Music library: source directories, scanning, tag reading, the SQLite database and
//! artwork caching. Must not depend on the UI crate.
//!
//! The database is a cache of the files' tags (the files are the source of truth) plus
//! what only VCMP knows: source directories and settings. Music files are only ever
//! read by this crate.

mod artwork;
mod error;
mod lookups;
mod path_codec;
mod query;
mod scan;
mod schema;
mod tags;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use rusqlite::{Connection, OptionalExtension, params};

pub use artwork::{Artwork, THUMBNAIL_SIZE};
pub use error::{Error, Result};
pub use lookups::{CoverArtId, CoverLookup};
pub use query::{Album, AlbumId, Artist, ArtistId, SearchResults, Track, TrackId};
pub use scan::{ScanPhase, ScanProgress, ScanReport, SkippedFile};
pub use tags::Format;
pub use watch::LibraryWatcher;

use artwork::ArtworkStore;
use error::io_error;

/// Setting key: rescan at startup and watch the source directories for changes.
/// Stored as "true" or "false"; absent means true.
pub const SETTING_AUTO_UPDATE: &str = "library.auto_update";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDir {
    pub id: i64,
    pub path: PathBuf,
    /// Whether subdirectories are included.
    pub recursive: bool,
}

/// One connection to the library database. Open one per thread; SQLite's WAL mode
/// lets a scan write while other connections read.
pub struct Library {
    conn: Connection,
    artwork: ArtworkStore,
}

impl Library {
    /// Opens (creating if needed) the database at `db_path`, upgrading its schema.
    /// Artwork is cached under `cache_dir`.
    pub fn open(db_path: &Path, cache_dir: &Path) -> Result<Self> {
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir).map_err(io_error(dir))?;
        }
        let mut conn = Connection::open(db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&mut conn)?;
        Ok(Self {
            conn,
            artwork: ArtworkStore::new(cache_dir),
        })
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    /// Whether the library rescans at startup and watches its sources (default on).
    pub fn auto_update(&self) -> Result<bool> {
        Ok(self
            .setting(SETTING_AUTO_UPDATE)?
            .is_none_or(|value| value != "false"))
    }

    pub fn set_auto_update(&self, enabled: bool) -> Result<()> {
        self.set_setting(SETTING_AUTO_UPDATE, if enabled { "true" } else { "false" })
    }

    pub fn sources(&self) -> Result<Vec<SourceDir>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, path, recursive FROM source_dirs ORDER BY id")?;
        let sources = statement
            .query_map([], |row| {
                Ok(SourceDir {
                    id: row.get(0)?,
                    path: path_codec::decode(&row.get::<_, Vec<u8>>(1)?),
                    recursive: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(sources)
    }

    /// Adds a source directory, stored with symlinks and short names resolved (and on
    /// Windows without the `\\?\` prefix). Adding the same directory twice returns the
    /// existing entry with the new `recursive` value.
    pub fn add_source(&self, path: &Path, recursive: bool) -> Result<SourceDir> {
        let path = dunce::canonicalize(path).map_err(io_error(path))?;
        if !path.is_dir() {
            return Err(Error::NotADirectory(path));
        }
        let id = self.conn.query_row(
            "INSERT INTO source_dirs (path, recursive) VALUES (?1, ?2)
             ON CONFLICT (path) DO UPDATE SET recursive = excluded.recursive
             RETURNING id",
            params![path_codec::encode(&path), recursive],
            |row| row.get(0),
        )?;
        Ok(SourceDir {
            id,
            path,
            recursive,
        })
    }

    pub fn set_source_recursive(&self, id: i64, recursive: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE source_dirs SET recursive = ?2 WHERE id = ?1",
            params![id, recursive],
        )?;
        Ok(())
    }

    /// Removes a source. Its tracks stay in the database as unavailable, so playlist
    /// entries survive and come back if the directory is added again.
    pub fn remove_source(&self, id: i64) -> Result<()> {
        // A shared reference is enough: the connection is never used from two places
        // at once, and the transaction ends before this returns.
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE tracks SET available = 0, source_id = NULL WHERE source_id = ?1",
            [id],
        )?;
        tx.execute("DELETE FROM source_dirs WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Scans all sources. `cancel` stops the scan at the next file; everything done
    /// up to that point is kept. `progress` is called from this thread.
    pub fn scan(
        &mut self,
        cancel: &AtomicBool,
        mut progress: impl FnMut(ScanProgress),
    ) -> Result<ScanReport> {
        scan::scan(&mut self.conn, &self.artwork, cancel, &mut progress)
    }

    /// The id of the track stored for a file path, available or not.
    pub fn track_id_for_path(&self, path: &Path) -> Result<Option<TrackId>> {
        // Stored paths lie under canonical source directories, so the folder is
        // resolved the same way; the file name is kept, in case the file is a symlink.
        let resolved = match (path.parent().map(dunce::canonicalize), path.file_name()) {
            (Some(Ok(folder)), Some(name)) => folder.join(name),
            _ => path.to_path_buf(),
        };
        Ok(scan::track_id_for_path(&self.conn, &resolved)?.map(TrackId))
    }

    /// How many tracks are available (their files were found by the last scan).
    pub fn available_track_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tracks WHERE available = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn albums(&self) -> Result<Vec<Album>> {
        query::albums(&self.conn, &self.artwork, None)
    }

    /// One album, if it still has available tracks.
    pub fn album(&self, id: AlbumId) -> Result<Option<Album>> {
        query::album(&self.conn, &self.artwork, id)
    }

    pub fn artist_albums(&self, artist: ArtistId) -> Result<Vec<Album>> {
        query::albums(&self.conn, &self.artwork, Some(artist))
    }

    pub fn artists(&self) -> Result<Vec<Artist>> {
        query::artists(&self.conn)
    }

    /// Every available track, ordered by artist, album, disc and track number.
    pub fn tracks(&self) -> Result<Vec<Track>> {
        query::tracks(&self.conn, query::TrackFilter::All)
    }

    pub fn album_tracks(&self, album: AlbumId) -> Result<Vec<Track>> {
        query::tracks(&self.conn, query::TrackFilter::Album(album))
    }

    /// A single track, including unavailable ones (playlists show those greyed out).
    pub fn track(&self, id: TrackId) -> Result<Option<Track>> {
        query::track(&self.conn, id)
    }

    /// Prefix search over titles, artists and albums, ignoring case and diacritics.
    pub fn search(&self, text: &str, limit: usize) -> Result<SearchResults> {
        query::search(&self.conn, &self.artwork, text, limit)
    }
}
