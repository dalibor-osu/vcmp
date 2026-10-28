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
-- Playlists. image: file name of the owner's own picture under the data directory's
-- images/playlists/, NULL for one made from album covers. Entries point at tracks,
-- which are never deleted, so an entry outlives its file going missing.
CREATE TABLE playlists (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    image TEXT,
    created_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE playlist_entries (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    track_id INTEGER NOT NULL REFERENCES tracks(id),
    PRIMARY KEY (playlist_id, position)
) STRICT, WITHOUT ROWID;

CREATE INDEX playlist_entries_track ON playlist_entries(track_id);
"#,
    r#"
-- Covers the user fetched from the Cover Art Archive for albums whose files have no
-- art: the cached image's key, the cover it came from (its kind and MBID) and when.
-- Keyed by the album's group_key like cover_lookups, so scans that remove and add
-- album rows keep it. The files' own art is shown instead whenever they have some.
CREATE TABLE fetched_covers (
    group_key TEXT PRIMARY KEY,
    artwork_hash TEXT NOT NULL,
    cover_kind TEXT NOT NULL CHECK (cover_kind IN ('release', 'release-group')),
    cover_mbid TEXT NOT NULL,
    fetched_ms INTEGER NOT NULL
) STRICT;
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
    r#"
-- The cover images in each music folder (cover.jpg and the like) as a fingerprint
-- of their names, sizes and modification times. Adding, replacing or removing such
-- an image changes no music file, so a scan compares fingerprints to know when the
-- folder's albums need their artwork looked for again.
CREATE TABLE folder_art (
    folder BLOB PRIMARY KEY,
    fingerprint TEXT NOT NULL
) STRICT;
"#,
    r#"
-- Artist photos the owner chose. image: file name under the data directory's
-- images/artists/; source: the web address it came from, NULL for a file of the
-- owner's. Keyed by the artist's name, which outlives artist rows a scan removes and
-- adds, like albums' group keys.
CREATE TABLE artist_photos (
    name TEXT PRIMARY KEY COLLATE NOCASE,
    image TEXT NOT NULL,
    source TEXT,
    chosen_ms INTEGER NOT NULL
) STRICT;
"#,
    r#"
-- Every track belongs to an album. One without an album tag goes into an "Unknown"
-- album of its album artist, else its artist; one naming neither into the single
-- Unknown album with no artist. unknown = 1 marks these; their title is empty (the
-- UI names them) and their group key is "unknown:" with the artist's id.
ALTER TABLE albums ADD COLUMN unknown INTEGER NOT NULL DEFAULT 0;

-- An album artist only ever named as such has no artist row yet.
INSERT OR IGNORE INTO artists (name)
    SELECT album_artist FROM tracks WHERE album IS NULL AND album_artist IS NOT NULL;

INSERT OR IGNORE INTO albums (group_key, title, artist_id, unknown)
    SELECT 'unknown:' || IFNULL(ar.id, ''), '', ar.id, 1
    FROM tracks t
    LEFT JOIN artists ar ON ar.name = COALESCE(t.album_artist, t.artist)
    WHERE t.album IS NULL;

UPDATE tracks SET album_id = (
    SELECT a.id FROM albums a
    WHERE a.group_key = 'unknown:' || IFNULL(
        (SELECT ar.id FROM artists ar
         WHERE ar.name = COALESCE(tracks.album_artist, tracks.artist)), ''))
WHERE album IS NULL;

DELETE FROM album_search;
INSERT INTO album_search (rowid, title, artist)
    SELECT albums.id, albums.title, IFNULL(artists.name, '')
    FROM albums LEFT JOIN artists ON artists.id = albums.artist_id;
"#,
    r#"
-- Every text item of the tag VCMP reads, as the file holds it, so tags VCMP doesn't
-- show can be looked at and searched (by AI agents through MCP, for one). key is
-- lofty's common name ("Composer") or the format's own key behind a prefix
-- ("id3:TXXX:MY FIELD", "vorbis:MYFIELD"); position is the item's place in the
-- track's list, so repeated keys keep their order.
CREATE TABLE track_tags (
    track_id INTEGER NOT NULL REFERENCES tracks(id),
    position INTEGER NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (track_id, position)
) STRICT, WITHOUT ROWID;

CREATE INDEX track_tags_key ON track_tags(key);

-- Fill the table: a modification time no file has makes the next scan re-read every
-- file, as if it had changed.
UPDATE tracks SET modified_ms = -1;
"#,
    r#"
-- Change sets: tag edits and playlist deletions AI agents ask for. A set waits for
-- the owner (pending), who accepts or rejects it; a pending set VCMP closed on is
-- discarded. Accepted is every file written, partly_written that some were not. An
-- undo is a set of its own (undoes); once it is written, the set it reverses is
-- undone. Finished sets are kept for 30 days after finished_ms.
CREATE TABLE change_sets (
    id INTEGER PRIMARY KEY,
    agent TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('tags', 'playlist_deletion')),
    playlist_id INTEGER,
    playlist_name TEXT,
    status TEXT NOT NULL CHECK (status IN
        ('pending', 'accepted', 'partly_written', 'rejected', 'discarded', 'undone')),
    undoes INTEGER REFERENCES change_sets(id) ON DELETE SET NULL,
    created_ms INTEGER NOT NULL,
    finished_ms INTEGER
) STRICT;

-- The files of a tag set. result stays NULL until the set is accepted: written,
-- conflict (the file no longer held what the set was made against) or failed, with
-- detail saying why. The covers are file names under images/undo/ in the data
-- directory, '' for no cover, NULL when the set leaves the cover alone: before is
-- the file's when the set was made, after the one written.
CREATE TABLE change_files (
    id INTEGER PRIMARY KEY,
    set_id INTEGER NOT NULL REFERENCES change_sets(id) ON DELETE CASCADE,
    track_id INTEGER NOT NULL REFERENCES tracks(id),
    path BLOB NOT NULL,
    cover_before TEXT,
    cover_after TEXT,
    result TEXT CHECK (result IN ('written', 'conflict', 'failed')),
    detail TEXT
) STRICT;

CREATE INDEX change_files_set ON change_files(set_id);

-- What a set asks for each file: one of the nine fields by name ('title', 'artist',
-- 'album', 'album_artist', 'track_number', 'disc_number', 'year', 'genre', 'cover')
-- or 'tag:' and a track_tags key. vals is a JSON array of strings, NULL to remove;
-- the cover's one value is a file name under images/undo/.
CREATE TABLE change_edits (
    file_id INTEGER NOT NULL REFERENCES change_files(id) ON DELETE CASCADE,
    target TEXT NOT NULL,
    vals TEXT,
    PRIMARY KEY (file_id, target)
) STRICT, WITHOUT ROWID;

-- The values of every tag a set may change in a file, by track_tags key, as JSON
-- arrays of strings: before as the database had them when the set was made, after
-- as they read back once written (NULL until then).
CREATE TABLE change_values (
    file_id INTEGER NOT NULL REFERENCES change_files(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    before TEXT NOT NULL,
    after TEXT,
    PRIMARY KEY (file_id, key)
) STRICT, WITHOUT ROWID;
"#,
    r#"
-- Labels: the owner's own names for tracks ("chill"), with a colour, kept only here
-- and never in the files. colour is 'palette:' and a name from the UI's palette, or
-- '#rrggbb'. Links point at tracks, which are never deleted, so they outlive a file
-- going missing like playlist entries.
CREATE TABLE labels (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    colour TEXT NOT NULL,
    created_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE track_labels (
    track_id INTEGER NOT NULL REFERENCES tracks(id),
    label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    added_ms INTEGER NOT NULL,
    PRIMARY KEY (track_id, label_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX track_labels_label ON track_labels(label_id);

-- Agents' label edits are change sets of kind 'labels'. The CHECK on kind can't be
-- altered in place, so change_sets is rebuilt; migrations run with foreign keys off,
-- so dropping the old table leaves the files of its sets alone.
CREATE TABLE change_sets_new (
    id INTEGER PRIMARY KEY,
    agent TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('tags', 'playlist_deletion', 'labels')),
    playlist_id INTEGER,
    playlist_name TEXT,
    status TEXT NOT NULL CHECK (status IN
        ('pending', 'accepted', 'partly_written', 'rejected', 'discarded', 'undone')),
    undoes INTEGER REFERENCES change_sets(id) ON DELETE SET NULL,
    created_ms INTEGER NOT NULL,
    finished_ms INTEGER
) STRICT;
INSERT INTO change_sets_new (id, agent, kind, playlist_id, playlist_name, status, undoes,
        created_ms, finished_ms)
    SELECT id, agent, kind, playlist_id, playlist_name, status, undoes, created_ms,
        finished_ms
    FROM change_sets;
DROP TABLE change_sets;
ALTER TABLE change_sets_new RENAME TO change_sets;

-- The labels a label set creates when accepted. created: NULL until then, 1 when
-- the set made it, 0 when a label of that name existed by then (and was used as is).
CREATE TABLE change_labels (
    set_id INTEGER NOT NULL REFERENCES change_sets(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    colour TEXT NOT NULL,
    created INTEGER,
    PRIMARY KEY (set_id, name)
) STRICT, WITHOUT ROWID;
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
    // Off while migrating: rebuilding a table drops the old one, which would delete
    // the rows that refer to it. Library::open turns them on afterwards.
    conn.pragma_update(None, "foreign_keys", false)?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
