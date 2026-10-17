//! Database schema and its migrations. `PRAGMA user_version` holds the number of
//! migrations applied; a database from a newer build is refused rather than altered.

use rusqlite::Connection;

use crate::{Error, Result};

const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

CREATE TABLE source_dirs (
    id INTEGER PRIMARY KEY,
    path BLOB NOT NULL UNIQUE,
    recursive INTEGER NOT NULL DEFAULT 1
) STRICT;

CREATE TABLE artists (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    mbid TEXT
) STRICT;

-- artwork_state: 0 = not looked for yet, 1 = found, 2 = none found.
CREATE TABLE albums (
    id INTEGER PRIMARY KEY,
    group_key TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    artist_id INTEGER REFERENCES artists(id),
    mbid TEXT,
    year INTEGER,
    artwork_hash TEXT,
    artwork_state INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE tracks (
    id INTEGER PRIMARY KEY,
    path BLOB NOT NULL UNIQUE,
    source_id INTEGER REFERENCES source_dirs(id) ON DELETE SET NULL,
    file_size INTEGER NOT NULL,
    modified_ms INTEGER NOT NULL,
    available INTEGER NOT NULL DEFAULT 1,
    format TEXT NOT NULL,
    title TEXT NOT NULL,
    artist TEXT,
    artist_id INTEGER REFERENCES artists(id),
    album TEXT,
    album_artist TEXT,
    album_id INTEGER REFERENCES albums(id),
    track_number INTEGER,
    disc_number INTEGER,
    year INTEGER,
    genre TEXT,
    duration_ms INTEGER NOT NULL,
    mb_recording_id TEXT,
    added_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX tracks_album ON tracks(album_id);
CREATE INDEX tracks_artist ON tracks(artist_id);
CREATE INDEX tracks_available_duration ON tracks(available, duration_ms);

-- Full-text indexes; rowid is the id in the matching table. Tracks are indexed as
-- they are written, artists and albums are rebuilt at the end of each scan.
CREATE VIRTUAL TABLE track_search USING fts5(
    title, artist, album,
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE artist_search USING fts5(
    name,
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE album_search USING fts5(
    title, artist,
    tokenize = 'unicode61 remove_diacritics 2'
);
"#,
    r#"
-- What an online lookup found for albums whose files carry no MusicBrainz release
-- ID: a Cover Art Archive cover (its kind and MBID), or NULLs when nothing matched.
-- Keyed by the album's group_key, which outlives album rows a scan removes and adds.
CREATE TABLE cover_lookups (
    group_key TEXT PRIMARY KEY,
    cover_kind TEXT CHECK (cover_kind IN ('release', 'release-group')),
    cover_mbid TEXT,
    looked_up_ms INTEGER NOT NULL
) STRICT;
"#,
];

pub(crate) const VERSION: i64 = MIGRATIONS.len() as i64;

pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > VERSION {
        return Err(Error::SchemaTooNew {
            found,
            supported: VERSION,
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
