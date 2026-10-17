//! Plays waiting to be sent to Last.fm. They stay in the database until Last.fm has
//! taken them, so they survive a restart and time offline.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::params_from_iter;

use crate::{Library, Result};

/// One play, with the track's tags as they were when it played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scrobble {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub duration: Option<Duration>,
    /// When the play started.
    pub played_at: SystemTime,
}

/// A play in the queue, with the id [`Library::remove_scrobbles`] takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedScrobble {
    pub id: i64,
    pub scrobble: Scrobble,
}

impl Library {
    /// Adds a play to the end of the queue.
    pub fn queue_scrobble(&self, scrobble: &Scrobble) -> Result<i64> {
        let played_ms = scrobble
            .played_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        self.conn.execute(
            "INSERT INTO scrobble_queue
                 (artist, title, album, album_artist, track_number, duration_ms, played_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                scrobble.artist,
                scrobble.title,
                scrobble.album,
                scrobble.album_artist,
                scrobble.track_number,
                scrobble
                    .duration
                    .map(|duration| duration.as_millis() as i64),
                played_ms,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The oldest `limit` plays in the queue, oldest first.
    pub fn queued_scrobbles(&self, limit: usize) -> Result<Vec<QueuedScrobble>> {
        let mut statement = self.conn.prepare(
            "SELECT id, artist, title, album, album_artist, track_number, duration_ms, played_ms
               FROM scrobble_queue ORDER BY played_ms, id LIMIT ?1",
        )?;
        let queued = statement
            .query_map([limit as i64], |row| {
                Ok(QueuedScrobble {
                    id: row.get(0)?,
                    scrobble: Scrobble {
                        artist: row.get(1)?,
                        title: row.get(2)?,
                        album: row.get(3)?,
                        album_artist: row.get(4)?,
                        track_number: row.get(5)?,
                        duration: row
                            .get::<_, Option<i64>>(6)?
                            .map(|ms| Duration::from_millis(ms.max(0) as u64)),
                        played_at: UNIX_EPOCH
                            + Duration::from_millis(row.get::<_, i64>(7)?.max(0) as u64),
                    },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(queued)
    }

    /// How many plays wait to be sent.
    pub fn queued_scrobble_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM scrobble_queue", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Takes plays out of the queue, once Last.fm has them. Unknown ids are skipped.
    pub fn remove_scrobbles(&self, ids: &[i64]) -> Result<()> {
        // SQLite takes up to 32766 parameters; batches are far smaller.
        for chunk in ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(", ");
            self.conn.execute(
                &format!("DELETE FROM scrobble_queue WHERE id IN ({marks})"),
                params_from_iter(chunk),
            )?;
        }
        Ok(())
    }

    /// Empties the queue.
    pub fn clear_scrobbles(&self) -> Result<()> {
        self.conn.execute("DELETE FROM scrobble_queue", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(dir: &std::path::Path) -> Library {
        Library::open(&dir.join("library.sqlite3"), &dir.join("cache")).unwrap()
    }

    fn play(title: &str, at_secs: u64) -> Scrobble {
        Scrobble {
            artist: "Band".into(),
            title: title.into(),
            album: Some("Record".into()),
            album_artist: None,
            track_number: Some(3),
            duration: Some(Duration::from_millis(201_500)),
            played_at: UNIX_EPOCH + Duration::from_secs(at_secs),
        }
    }

    #[test]
    fn the_queue_survives_reopening_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let first = play("Opening", 1_700_000_000);
        let second = Scrobble {
            album: None,
            track_number: None,
            duration: None,
            ..play("Closing", 1_700_000_300)
        };
        {
            let library = open(dir.path());
            library.queue_scrobble(&first).unwrap();
            library.queue_scrobble(&second).unwrap();
        }
        let library = open(dir.path());
        let queued: Vec<Scrobble> = library
            .queued_scrobbles(50)
            .unwrap()
            .into_iter()
            .map(|queued| queued.scrobble)
            .collect();
        assert_eq!(queued, [first, second]);
        assert_eq!(library.queued_scrobble_count().unwrap(), 2);
    }

    #[test]
    fn plays_come_out_oldest_first_in_batches() {
        let dir = tempfile::tempdir().unwrap();
        let library = open(dir.path());
        for minute in (0..120).rev() {
            library
                .queue_scrobble(&play(&format!("Track {minute}"), 60 * minute))
                .unwrap();
        }
        let batch = library.queued_scrobbles(50).unwrap();
        assert_eq!(batch.len(), 50);
        assert_eq!(batch[0].scrobble.title, "Track 0");
        assert_eq!(batch[49].scrobble.title, "Track 49");

        let ids: Vec<i64> = batch.iter().map(|queued| queued.id).collect();
        library.remove_scrobbles(&ids).unwrap();
        assert_eq!(library.queued_scrobble_count().unwrap(), 70);
        assert_eq!(
            library.queued_scrobbles(1).unwrap()[0].scrobble.title,
            "Track 50"
        );

        library.clear_scrobbles().unwrap();
        assert_eq!(library.queued_scrobbles(50).unwrap(), []);
    }
}
