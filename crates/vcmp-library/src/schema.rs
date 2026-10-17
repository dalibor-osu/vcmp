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
    r#"
-- Search finds any part of a name, not only word starts: titles in Japanese and
-- Chinese have no spaces, so "ライブ" has to match inside "ラブライブ!". The
-- trigram tokenizer indexes every three-character run, ignoring case and diacritics.
DROP TABLE track_search;
DROP TABLE artist_search;
DROP TABLE album_search;
CREATE VIRTUAL TABLE track_search USING fts5(
    title, artist, album,
    tokenize = 'trigram remove_diacritics 1'
);
CREATE VIRTUAL TABLE artist_search USING fts5(
    name,
    tokenize = 'trigram remove_diacritics 1'
);
CREATE VIRTUAL TABLE album_search USING fts5(
    title, artist,
    tokenize = 'trigram remove_diacritics 1'
);
INSERT INTO track_search (rowid, title, artist, album)
    SELECT id, title, TRIM(IFNULL(artist, '') || ' ' || IFNULL(album_artist, '')), IFNULL(album, '')
    FROM tracks;
INSERT INTO artist_search (rowid, name) SELECT id, name FROM artists;
INSERT INTO album_search (rowid, title, artist)
    SELECT albums.id, albums.title, IFNULL(artists.name, '')
    FROM albums LEFT JOIN artists ON artists.id = albums.artist_id;
"#,
    r#"
-- Plays Last.fm has not received yet, oldest first. They hold the tags as they were
-- when the track played, not track ids: a later rescan or tag edit must not change
-- what was listened to. played_ms is when the play started (Unix time).
CREATE TABLE scrobble_queue (
    id INTEGER PRIMARY KEY,
    artist TEXT NOT NULL,
    title TEXT NOT NULL,
    album TEXT,
    album_artist TEXT,
    track_number INTEGER,
    duration_ms INTEGER,
    played_ms INTEGER NOT NULL
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
