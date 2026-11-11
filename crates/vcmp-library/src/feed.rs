use std::path::PathBuf;
use std::time::Duration;

use rusqlite::{Row, params};

use crate::artwork::Artwork;
use crate::tags::Format;
use crate::{Library, Result, path_codec};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeedCursor {
    pub revision: i64,
    pub track: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedTrack {
    pub cursor: FeedCursor,
    pub path: PathBuf,
    pub source_id: Option<i64>,
    pub available: bool,
    pub format: Option<Format>,
    pub file_size: u64,
    pub modified_ms: i64,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub duration: Duration,
    pub added_ms: i64,
    pub artwork: Option<String>,
    pub mb_release_id: Option<String>,
    pub mb_artist_id: Option<String>,
    pub mb_album_artist_id: Option<String>,
    pub mb_recording_id: Option<String>,
}

fn first_tag(key: &str) -> String {
    format!(
        "(SELECT TRIM(g.value) FROM track_tags g WHERE g.track_id = t.id AND g.key = '{key}'
            AND TRIM(g.value) != '' ORDER BY g.position LIMIT 1)"
    )
}

fn feed_track(row: &Row) -> rusqlite::Result<FeedTrack> {
    Ok(FeedTrack {
        cursor: FeedCursor {
            revision: row.get(0)?,
            track: row.get(1)?,
        },
        path: path_codec::decode(&row.get::<_, Vec<u8>>(2)?),
        source_id: row.get(3)?,
        available: row.get(4)?,
        format: Format::parse(&row.get::<_, String>(5)?),
        file_size: row.get::<_, i64>(6)?.max(0) as u64,
        modified_ms: row.get(7)?,
        title: row.get(8)?,
        artist: row.get(9)?,
        album: row.get(10)?,
        album_artist: row.get(11)?,
        track_number: row.get(12)?,
        disc_number: row.get(13)?,
        year: row.get(14)?,
        genre: row.get(15)?,
        duration: Duration::from_millis(row.get::<_, i64>(16)?.max(0) as u64),
        added_ms: row.get(17)?,
        artwork: row.get(18)?,
        mb_release_id: row.get(19)?,
        mb_artist_id: row.get(20)?,
        mb_album_artist_id: row.get(21)?,
        mb_recording_id: row.get(22)?,
    })
}

impl Library {
    pub fn track_feed(&self, after: FeedCursor, limit: usize) -> Result<Vec<FeedTrack>> {
        let sql = format!(
            "SELECT t.revision, t.id, t.path, t.source_id, t.available, t.format, t.file_size,
                    t.modified_ms, t.title, t.artist, t.album, t.album_artist, t.track_number,
                    t.disc_number, t.year, t.genre, t.duration_ms, t.added_ms, a.artwork_hash,
                    {}, {}, {}, t.mb_recording_id
             FROM tracks t LEFT JOIN albums a ON a.id = t.album_id
             WHERE (t.revision, t.id) > (?1, ?2)
             ORDER BY t.revision, t.id LIMIT ?3",
            first_tag("MusicBrainzReleaseId"),
            first_tag("MusicBrainzArtistId"),
            first_tag("MusicBrainzReleaseArtistId"),
        );
        let mut statement = self.conn.prepare_cached(&sql)?;
        let tracks = statement
            .query_map(
                params![after.revision, after.track, limit as i64],
                feed_track,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(tracks)
    }

    pub fn feed_head(&self) -> Result<FeedCursor> {
        Ok(self
            .conn
            .query_row(
                "SELECT revision, id FROM tracks ORDER BY revision DESC, id DESC LIMIT 1",
                [],
                |row| {
                    Ok(FeedCursor {
                        revision: row.get(0)?,
                        track: row.get(1)?,
                    })
                },
            )
            .unwrap_or_default())
    }

    pub fn artwork_by_key(&self, key: &str) -> Option<Artwork> {
        let (stem, extension) = key.split_once('.')?;
        let valid = !stem.is_empty()
            && stem.bytes().all(|b| b.is_ascii_hexdigit())
            && !extension.is_empty()
            && extension.bytes().all(|b| b.is_ascii_lowercase());
        if !valid {
            return None;
        }
        self.artwork.get(key)
    }
}
