use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, Row, params};

use crate::artwork::Artwork;
use crate::feed::valid_artwork_key;
use crate::query::{ServerId, TrackId};
use crate::tags::Format;
use crate::{Error, FeedCursor, Library, Result, scan};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub id: ServerId,
    pub server_id: String,
    pub name: String,
    pub address: String,
    pub cursor: FeedCursor,
    pub synced: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MirroredTrack {
    pub remote_id: i64,
    pub available: bool,
    pub folder: Option<i64>,
    pub path: String,
    pub format: Option<Format>,
    pub size: u64,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerFolder {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    pub track: TrackId,
    pub server: ServerId,
    pub remote_id: i64,
    pub size: u64,
    pub modified_ms: i64,
    pub format: Option<Format>,
    pub available: bool,
}

const SERVER_COLUMNS: &str =
    "id, server_id, name, address, cursor_revision, cursor_track, synced_ms";

fn server_from(row: &Row) -> rusqlite::Result<Server> {
    Ok(Server {
        id: ServerId(row.get(0)?),
        server_id: row.get(1)?,
        name: row.get(2)?,
        address: row.get(3)?,
        cursor: FeedCursor {
            revision: row.get(4)?,
            track: row.get(5)?,
        },
        synced: row
            .get::<_, Option<i64>>(6)?
            .map(|ms| UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64)),
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl Library {
    pub fn servers(&self) -> Result<Vec<Server>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {SERVER_COLUMNS} FROM servers WHERE connected = 1 ORDER BY id"
        ))?;
        Ok(statement
            .query_map([], server_from)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn server(&self, id: ServerId) -> Result<Option<Server>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {SERVER_COLUMNS} FROM servers WHERE id = ?1 AND connected = 1"),
                [id.0],
                server_from,
            )
            .optional()?)
    }

    pub fn add_server(&self, server_id: &str, name: &str, address: &str) -> Result<Server> {
        let id: i64 = self.conn.query_row(
            "INSERT INTO servers (server_id, name, address, added_ms) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (server_id) DO UPDATE SET name = excluded.name,
                 address = excluded.address, connected = 1,
                 cursor_revision = CASE WHEN servers.connected = 1
                     THEN servers.cursor_revision ELSE 0 END,
                 cursor_track = CASE WHEN servers.connected = 1
                     THEN servers.cursor_track ELSE 0 END
             RETURNING id",
            params![server_id, name, address, now_ms()],
            |row| row.get(0),
        )?;
        self.server(ServerId(id))?
            .ok_or(Error::UnknownServer(ServerId(id)))
    }

    pub fn rename_server(&self, id: ServerId, name: &str, address: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE servers SET name = ?2, address = ?3 WHERE id = ?1",
            params![id.0, name, address],
        )?;
        Ok(())
    }

    pub fn remove_server(&self, id: ServerId) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE servers SET connected = 0, cursor_revision = 0, cursor_track = 0
             WHERE id = ?1",
            [id.0],
        )?;
        tx.execute(
            "UPDATE tracks SET available = 0
             WHERE id IN (SELECT track_id FROM remote_tracks WHERE server = ?1)",
            [id.0],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn restart_server_feed(&self, id: ServerId) -> Result<()> {
        self.conn.execute(
            "UPDATE servers SET cursor_revision = 0, cursor_track = 0 WHERE id = ?1",
            [id.0],
        )?;
        Ok(())
    }

    pub fn apply_server_page(
        &mut self,
        server: ServerId,
        tracks: &[MirroredTrack],
        next: FeedCursor,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        for track in tracks {
            scan::apply_remote(&tx, server, track)?;
        }
        tx.execute(
            "UPDATE servers SET cursor_revision = ?2, cursor_track = ?3 WHERE id = ?1",
            params![server.0, next.revision, next.track],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn forget_unseen_server_tracks(
        &mut self,
        server: ServerId,
        seen: &HashSet<i64>,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let rows: Vec<(i64, i64)> = tx
            .prepare("SELECT track_id, remote_id FROM remote_tracks WHERE server = ?1")?
            .query_map([server.0], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut gone = 0;
        {
            let mut update =
                tx.prepare("UPDATE tracks SET available = 0 WHERE id = ?1 AND available = 1")?;
            for (track, remote) in rows {
                if !seen.contains(&remote) {
                    gone += update.execute([track])?;
                }
            }
        }
        tx.commit()?;
        Ok(gone)
    }

    pub fn finish_server_sync(&mut self, server: ServerId) -> Result<()> {
        scan::finish_groups(&mut self.conn)?;
        self.conn.execute(
            "UPDATE servers SET synced_ms = ?2 WHERE id = ?1",
            params![server.0, now_ms()],
        )?;
        Ok(())
    }

    pub fn set_server_folders(&mut self, server: ServerId, folders: &[ServerFolder]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM server_folders WHERE server = ?1", [server.0])?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO server_folders (server, remote_id, name) VALUES (?1, ?2, ?3)",
            )?;
            for folder in folders {
                insert.execute(params![server.0, folder.id, folder.name])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn server_folders(&self, server: ServerId) -> Result<Vec<ServerFolder>> {
        let mut statement = self.conn.prepare(
            "SELECT remote_id, name FROM server_folders WHERE server = ?1 ORDER BY name, remote_id",
        )?;
        Ok(statement
            .query_map([server.0], |row| {
                Ok(ServerFolder {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn remote_track(&self, server: ServerId, remote_id: i64) -> Result<Option<TrackId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT track_id FROM remote_tracks WHERE server = ?1 AND remote_id = ?2",
                params![server.0, remote_id],
                |row| row.get(0).map(TrackId),
            )
            .optional()?)
    }

    pub fn remote_file(&self, track: TrackId) -> Result<Option<RemoteFile>> {
        Ok(self
            .conn
            .query_row(
                "SELECT rt.track_id, rt.server, rt.remote_id, t.file_size, t.modified_ms,
                        t.format, t.available
                 FROM remote_tracks rt JOIN tracks t ON t.id = rt.track_id
                 WHERE rt.track_id = ?1",
                [track.0],
                |row| {
                    Ok(RemoteFile {
                        track: TrackId(row.get(0)?),
                        server: ServerId(row.get(1)?),
                        remote_id: row.get(2)?,
                        size: row.get::<_, i64>(3)?.max(0) as u64,
                        modified_ms: row.get(4)?,
                        format: Format::parse(&row.get::<_, String>(5)?),
                        available: row.get(6)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn missing_server_artwork(&self, server: ServerId) -> Result<Vec<String>> {
        let prefix = format!("{}%", crate::query::server_group_prefix(server));
        let keys: Vec<String> = self
            .conn
            .prepare(
                "SELECT DISTINCT artwork_hash FROM albums
                 WHERE group_key LIKE ?1 AND artwork_hash IS NOT NULL",
            )?
            .query_map([prefix], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(keys
            .into_iter()
            .filter(|key| valid_artwork_key(key) && self.artwork.get(key).is_none())
            .collect())
    }

    pub fn store_server_artwork(
        &self,
        key: &str,
        original: &[u8],
        thumbnail: &[u8],
    ) -> Result<Option<Artwork>> {
        if !valid_artwork_key(key) {
            return Ok(None);
        }
        self.artwork.put(key, original, thumbnail).map(Some)
    }
}
