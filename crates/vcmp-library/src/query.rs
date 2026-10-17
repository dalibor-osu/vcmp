//! Read-side queries. Library views list only available tracks and the albums and
//! artists that still have some; a single track can be fetched regardless.

use std::path::PathBuf;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, params_from_iter};

use crate::artwork::{Artwork, ArtworkStore};
use crate::tags::Format;
use crate::{Result, path_codec};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AlbumId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArtistId(pub i64);

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub path: PathBuf,
    pub available: bool,
    pub format: Option<Format>,
    pub title: String,
    pub artist: Option<String>,
    pub artist_id: Option<ArtistId>,
    pub album: Option<String>,
    pub album_id: Option<AlbumId>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub duration: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Album {
    pub id: AlbumId,
    pub title: String,
    /// Album artist, or the track artist when the files name no album artist.
    pub artist: Option<String>,
    pub artist_id: Option<ArtistId>,
    pub year: Option<i32>,
    pub track_count: u32,
    pub artwork: Option<Artwork>,
    /// MusicBrainz release ID, when the files are tagged with one.
    pub musicbrainz_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artist {
    pub id: ArtistId,
    pub name: String,
    pub album_count: u32,
    pub track_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchResults {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
}

pub(crate) enum TrackFilter {
    All,
    Album(AlbumId),
    /// Tracks credited to the artist outside the artist's own albums.
    AppearsOn(ArtistId),
}

pub(crate) const TRACK_COLUMNS: &str =
    "t.id, t.path, t.available, t.format, t.title, t.artist, t.artist_id, t.album,
    t.album_id, t.album_artist, t.track_number, t.disc_number, t.year, t.genre, t.duration_ms";

pub(crate) fn track_from(row: &Row) -> rusqlite::Result<Track> {
    Ok(Track {
        id: TrackId(row.get(0)?),
        path: path_codec::decode(&row.get::<_, Vec<u8>>(1)?),
        available: row.get(2)?,
        format: Format::parse(&row.get::<_, String>(3)?),
        title: row.get(4)?,
        artist: row.get(5)?,
        artist_id: row.get::<_, Option<i64>>(6)?.map(ArtistId),
        album: row.get(7)?,
        album_id: row.get::<_, Option<i64>>(8)?.map(AlbumId),
        album_artist: row.get(9)?,
        track_number: row.get(10)?,
        disc_number: row.get(11)?,
        year: row.get(12)?,
        genre: row.get(13)?,
        duration: Duration::from_millis(row.get::<_, i64>(14)?.max(0) as u64),
    })
}

pub(crate) fn tracks(conn: &Connection, filter: TrackFilter) -> Result<Vec<Track>> {
    let (condition, album) = match filter {
        TrackFilter::All => ("", None),
        TrackFilter::Album(AlbumId(id)) => ("AND t.album_id = ?1", Some(id)),
        TrackFilter::AppearsOn(ArtistId(id)) => (
            "AND t.artist_id = ?1 AND (a.artist_id IS NULL OR a.artist_id != ?1)",
            Some(id),
        ),
    };
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks t
         LEFT JOIN albums a ON a.id = t.album_id
         LEFT JOIN artists aa ON aa.id = a.artist_id
         WHERE t.available = 1 {condition}
         ORDER BY aa.name IS NULL, aa.name COLLATE NOCASE, a.year, a.title COLLATE NOCASE,
                  t.disc_number, t.track_number, t.title COLLATE NOCASE"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = match album {
        Some(id) => statement
            .query_map([id], track_from)?
            .collect::<rusqlite::Result<_>>()?,
        None => statement
            .query_map([], track_from)?
            .collect::<rusqlite::Result<_>>()?,
    };
    Ok(rows)
}

pub(crate) fn track(conn: &Connection, TrackId(id): TrackId) -> Result<Option<Track>> {
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks t WHERE t.id = ?1");
    Ok(conn.query_row(&sql, [id], track_from).optional()?)
}

const ALBUM_QUERY: &str =
    "SELECT a.id, a.title, ar.name, a.artist_id, a.year, COUNT(t.id), a.artwork_hash, a.mbid
    FROM albums a
    JOIN tracks t ON t.album_id = a.id AND t.available = 1
    LEFT JOIN artists ar ON ar.id = a.artist_id";

fn album_from(row: &Row, store: &ArtworkStore) -> rusqlite::Result<Album> {
    Ok(Album {
        id: AlbumId(row.get(0)?),
        title: row.get(1)?,
        artist: row.get(2)?,
        artist_id: row.get::<_, Option<i64>>(3)?.map(ArtistId),
        year: row.get(4)?,
        track_count: row.get(5)?,
        artwork: row
            .get::<_, Option<String>>(6)?
            .and_then(|key| store.get(&key)),
        musicbrainz_id: row.get(7)?,
    })
}

/// One album, if it still has available tracks.
pub(crate) fn album(
    conn: &Connection,
    store: &ArtworkStore,
    AlbumId(id): AlbumId,
) -> Result<Option<Album>> {
    let sql = format!("{ALBUM_QUERY} WHERE a.id = ?1 GROUP BY a.id");
    Ok(conn
        .query_row(&sql, [id], |row| album_from(row, store))
        .optional()?)
}

pub(crate) fn albums(
    conn: &Connection,
    store: &ArtworkStore,
    artist: Option<ArtistId>,
) -> Result<Vec<Album>> {
    let condition = if artist.is_some() {
        "WHERE a.artist_id = ?1"
    } else {
        ""
    };
    let sql = format!(
        "{ALBUM_QUERY} {condition} GROUP BY a.id
         ORDER BY ar.name IS NULL, ar.name COLLATE NOCASE, a.year, a.title COLLATE NOCASE"
    );
    let mut statement = conn.prepare(&sql)?;
    let map = |row: &Row| album_from(row, store);
    let albums = match artist {
        Some(ArtistId(id)) => statement
            .query_map([id], map)?
            .collect::<rusqlite::Result<_>>()?,
        None => statement
            .query_map([], map)?
            .collect::<rusqlite::Result<_>>()?,
    };
    Ok(albums)
}

/// Album and track counts for `ar`, counting available tracks only.
const ARTIST_STATS: &str = "(SELECT COUNT(*) FROM albums a WHERE a.artist_id = ar.id
            AND EXISTS (SELECT 1 FROM tracks t WHERE t.album_id = a.id AND t.available = 1)) AS album_count,
        (SELECT COUNT(*) FROM tracks t WHERE t.artist_id = ar.id AND t.available = 1) AS track_count";

fn artist_from(row: &Row) -> rusqlite::Result<Artist> {
    Ok(Artist {
        id: ArtistId(row.get(0)?),
        name: row.get(1)?,
        album_count: row.get(2)?,
        track_count: row.get(3)?,
    })
}

pub(crate) fn artist(conn: &Connection, ArtistId(id): ArtistId) -> Result<Option<Artist>> {
    let sql = format!("SELECT ar.id, ar.name, {ARTIST_STATS} FROM artists ar WHERE ar.id = ?1");
    Ok(conn.query_row(&sql, [id], artist_from).optional()?)
}

pub(crate) fn artists(conn: &Connection) -> Result<Vec<Artist>> {
    let sql = format!(
        "SELECT id, name, album_count, track_count FROM (
            SELECT ar.id AS id, ar.name AS name, {ARTIST_STATS} FROM artists ar)
         WHERE album_count + track_count > 0
         ORDER BY name COLLATE NOCASE"
    );
    let mut statement = conn.prepare(&sql)?;
    Ok(statement
        .query_map([], artist_from)?
        .collect::<rusqlite::Result<_>>()?)
}

/// Typed text as conditions on a trigram search table. Words of three characters
/// or more go into one full-text query, which finds them anywhere in a name and
/// ranks the rows. A trigram index cannot look up shorter words ("高坂"), so those
/// become LIKE conditions over the table's columns.
#[derive(Debug, PartialEq, Eq)]
struct TextQuery {
    full_text: Option<String>,
    short: Vec<String>,
}

impl TextQuery {
    fn parse(text: &str) -> Option<Self> {
        let words: Vec<String> = text
            .split_whitespace()
            .map(|word| word.replace('"', ""))
            .filter(|word| !word.is_empty())
            .collect();
        if words.is_empty() {
            return None;
        }
        let (long, short): (Vec<String>, Vec<String>) = words
            .into_iter()
            .partition(|word| word.chars().count() >= 3);
        let full_text = (!long.is_empty()).then(|| {
            long.iter()
                .map(|word| format!("\"{word}\""))
                .collect::<Vec<_>>()
                .join(" AND ")
        });
        Some(Self { full_text, short })
    }

    /// The WHERE clause for `table` (aliased `s`) and its parameters, and whether
    /// `s.rank` can order the rows.
    fn condition(&self, table: &str, columns: &[&str]) -> (String, Vec<String>, bool) {
        let mut parts = Vec::new();
        let mut params = Vec::new();
        if let Some(full_text) = &self.full_text {
            params.push(full_text.clone());
            parts.push(format!("{table} MATCH ?{}", params.len()));
        }
        for word in &self.short {
            params.push(format!("%{}%", escape_like(word)));
            let n = params.len();
            let any = columns
                .iter()
                .map(|column| format!("s.{column} LIKE ?{n} ESCAPE '\\'"))
                .collect::<Vec<_>>()
                .join(" OR ");
            parts.push(format!("({any})"));
        }
        (parts.join(" AND "), params, self.full_text.is_some())
    }
}

fn escape_like(word: &str) -> String {
    let mut escaped = String::with_capacity(word.len());
    for c in word.chars() {
        if matches!(c, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

pub(crate) fn search(
    conn: &Connection,
    store: &ArtworkStore,
    text: &str,
    limit: usize,
) -> Result<SearchResults> {
    let Some(query) = TextQuery::parse(text) else {
        return Ok(SearchResults::default());
    };
    // Without a full-text query there is no rank; NULL leaves the order to the name.
    let rank = |ranked: bool| if ranked { "s.rank" } else { "NULL" };
    // The matches are found first, once: joined straight to the other tables, the
    // planner may run the full-text query again for every album row (237 ms
    // instead of a few on the owner's library).
    let found = |table: &str, columns: &[&str]| {
        let (condition, params, ranked) = query.condition(table, columns);
        let sql = format!(
            "WITH found AS MATERIALIZED (
                SELECT s.rowid AS id, {} AS rank FROM {table} s WHERE {condition})",
            rank(ranked)
        );
        (sql, params)
    };

    let (with, params) = found("artist_search", &["name"]);
    let artist_sql = format!(
        "{with}
         SELECT id, name, album_count, track_count FROM (
            SELECT ar.id AS id, ar.name AS name, f.rank AS rank, {ARTIST_STATS}
            FROM found f JOIN artists ar ON ar.id = f.id)
         WHERE album_count + track_count > 0
         ORDER BY rank, name COLLATE NOCASE LIMIT {limit}"
    );
    let artists = conn
        .prepare(&artist_sql)?
        .query_map(params_from_iter(&params), artist_from)?
        .collect::<rusqlite::Result<_>>()?;

    let (with, params) = found("album_search", &["title", "artist"]);
    let album_sql = format!(
        "{with}
         {ALBUM_QUERY} JOIN found f ON f.id = a.id
         GROUP BY a.id ORDER BY MIN(f.rank), a.title COLLATE NOCASE LIMIT {limit}"
    );
    let albums = conn
        .prepare(&album_sql)?
        .query_map(params_from_iter(&params), |row| album_from(row, store))?
        .collect::<rusqlite::Result<_>>()?;

    let (with, params) = found("track_search", &["title", "artist", "album"]);
    let track_sql = format!(
        "{with}
         SELECT {TRACK_COLUMNS} FROM found f JOIN tracks t ON t.id = f.id
         WHERE t.available = 1
         ORDER BY f.rank, t.title COLLATE NOCASE LIMIT {limit}"
    );
    let tracks = conn
        .prepare(&track_sql)?
        .query_map(params_from_iter(&params), track_from)?
        .collect::<rusqlite::Result<_>>()?;

    Ok(SearchResults {
        artists,
        albums,
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_words_are_searched_short_ones_filtered() {
        let query = TextQuery::parse("  pink  fl \"say\" 高坂 ").unwrap();
        assert_eq!(
            query,
            TextQuery {
                full_text: Some("\"pink\" AND \"say\"".into()),
                short: vec!["fl".into(), "高坂".into()],
            }
        );
        let (condition, params, ranked) = query.condition("t", &["a", "b"]);
        assert_eq!(
            condition,
            "t MATCH ?1 AND (s.a LIKE ?2 ESCAPE '\\' OR s.b LIKE ?2 ESCAPE '\\') \
             AND (s.a LIKE ?3 ESCAPE '\\' OR s.b LIKE ?3 ESCAPE '\\')"
        );
        assert_eq!(params, ["\"pink\" AND \"say\"", "%fl%", "%高坂%"]);
        assert!(ranked);
        assert_eq!(TextQuery::parse("   "), None);
        assert_eq!(TextQuery::parse("\"\""), None);
        assert_eq!(escape_like("50%_a\\b"), "50\\%\\_a\\\\b");
    }
}
