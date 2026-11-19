//! Favourites: the tracks the owner liked, with when. They live only in the
//! database, never in the files.
//!
//! A like points at a track, and tracks are never deleted, so a heart stays on a
//! track while its file is gone and follows a moved file, like a playlist entry.
//! Liking a track that is liked already keeps its first time. The list is newest
//! first; tracks liked together (an album) keep their album order.

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::params;

use crate::labels::now_ms;
use crate::query::{TRACK_COLUMNS, track_from};
use crate::{Library, Result, Track, TrackId};

/// One liked track.
#[derive(Clone, Debug, PartialEq)]
pub struct Favourite {
    pub track: Track,
    pub liked: SystemTime,
}

/// The favourites as a whole, for the card and the page's heading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FavouritesSummary {
    /// Every liked track, available or not.
    pub track_count: u32,
    /// Liked tracks whose file is there now.
    pub available_count: u32,
    /// Of the available ones.
    pub duration: Duration,
}

impl Library {
    /// Likes the tracks; those liked already keep the time they were first liked.
    pub fn like(&self, tracks: &[TrackId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let now = now_ms();
        for track in tracks {
            self.conn
                .prepare_cached(
                    "INSERT OR IGNORE INTO favourites (track_id, liked_ms) VALUES (?1, ?2)",
                )?
                .execute(params![track.0, now])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Likes each track as of its time (Last.fm's love date, for an import); those
    /// liked already keep their own time. Returns how many were liked now.
    pub fn like_at(&self, tracks: &[(TrackId, SystemTime)]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut liked = 0;
        for (track, at) in tracks {
            let at_ms = at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            liked += self
                .conn
                .prepare_cached(
                    "INSERT OR IGNORE INTO favourites (track_id, liked_ms)
                     SELECT id, ?2 FROM tracks WHERE id = ?1",
                )?
                .execute(params![track.0, at_ms])?;
        }
        tx.commit()?;
        Ok(liked)
    }

    /// Every track's id, artist and title, available or not, for tracks that have an
    /// artist: what a Last.fm import matches loved tracks against.
    pub fn track_names(&self) -> Result<Vec<(TrackId, String, String)>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, artist, title FROM tracks WHERE artist IS NOT NULL")?;
        Ok(statement
            .query_map([], |row| {
                Ok((TrackId(row.get(0)?), row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Takes the tracks out of the favourites; those not liked are left alone.
    pub fn unlike(&self, tracks: &[TrackId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for track in tracks {
            self.conn
                .prepare_cached("DELETE FROM favourites WHERE track_id = ?1")?
                .execute([track.0])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every liked track, available or not.
    pub fn liked_tracks(&self) -> Result<HashSet<TrackId>> {
        let mut statement = self.conn.prepare("SELECT track_id FROM favourites")?;
        Ok(statement
            .query_map([], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// The liked tracks, newest first, unavailable ones included.
    pub fn favourites(&self) -> Result<Vec<Favourite>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {TRACK_COLUMNS}, f.liked_ms FROM favourites f
             JOIN tracks t ON t.id = f.track_id
             ORDER BY f.liked_ms DESC, t.album_id, t.disc_number, t.track_number, t.id"
        ))?;
        Ok(statement
            .query_map([], |row| {
                let liked: i64 = row.get(crate::query::TRACK_COLUMN_COUNT)?;
                Ok(Favourite {
                    track: track_from(row)?,
                    liked: UNIX_EPOCH + Duration::from_millis(liked.max(0) as u64),
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// How many tracks are liked, how many of them can play, and for how long.
    pub fn favourites_summary(&self) -> Result<FavouritesSummary> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(t.available), 0),
                 COALESCE(SUM(CASE WHEN t.available = 1 THEN t.duration_ms END), 0)
             FROM favourites f JOIN tracks t ON t.id = f.track_id",
            [],
            |row| {
                Ok(FavouritesSummary {
                    track_count: row.get(0)?,
                    available_count: row.get(1)?,
                    duration: Duration::from_millis(row.get::<_, i64>(2)?.max(0) as u64),
                })
            },
        )?)
    }
}
