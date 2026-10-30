//! Playlists: a name, ordered entries that reference tracks, and a picture.
//!
//! Entries keep pointing at their track when its file goes away: tracks are never
//! deleted, only marked unavailable, so an entry shows greyed out while its file is
//! gone (an unplugged drive) and comes back with it. The picture is the owner's own
//! image (kept under the data directory, since it cannot be rebuilt), or one made
//! from the covers of the first four albums in the playlist (cached).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::codecs::jpeg::JpegEncoder;
use rusqlite::{OptionalExtension, Row, params};

use crate::animation;
use crate::error::io_error;
use crate::query::{TRACK_COLUMNS, track_from};
use crate::{Error, Library, Result, Track, TrackId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlaylistId(pub i64);

#[derive(Clone, Debug, PartialEq)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    /// Every entry, available or not.
    pub track_count: u32,
    /// Entries whose file is there now.
    pub available_count: u32,
    /// Of the available entries.
    pub duration: Duration,
    /// The owner's own picture, or one made from the first four album covers (or
    /// the first cover, with fewer albums); `None` without any cover.
    pub picture: Option<PathBuf>,
    /// The picture is the owner's own.
    pub own_picture: bool,
}

/// One entry of a playlist, in order; the track may be unavailable.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaylistEntry {
    pub position: u32,
    pub track: Track,
}

/// Longest side of an owner's picture as kept, and of a made one.
const PICTURE_SIZE: u32 = 600;

impl Library {
    /// Every playlist, by name.
    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        let mut statement = self.conn.prepare(&format!(
            "{PLAYLIST_QUERY} GROUP BY p.id ORDER BY p.name COLLATE NOCASE, p.id"
        ))?;
        let rows = statement
            .query_map([], playlist_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|row| self.with_picture(row)).collect()
    }

    pub fn playlist(&self, id: PlaylistId) -> Result<Option<Playlist>> {
        let row = self
            .conn
            .query_row(
                &format!("{PLAYLIST_QUERY} WHERE p.id = ?1 GROUP BY p.id"),
                [id.0],
                playlist_row,
            )
            .optional()?;
        row.map(|row| self.with_picture(row)).transpose()
    }

    /// Playlists whose name holds every word of `text`, ignoring case.
    pub fn search_playlists(&self, text: &str) -> Result<Vec<Playlist>> {
        let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .playlists()?
            .into_iter()
            .filter(|playlist| {
                let name = playlist.name.to_lowercase();
                words.iter().all(|word| name.contains(word.as_str()))
            })
            .collect())
    }

    /// The entries in order, unavailable ones included.
    pub fn playlist_entries(&self, id: PlaylistId) -> Result<Vec<PlaylistEntry>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {TRACK_COLUMNS}, e.position FROM playlist_entries e
             JOIN tracks t ON t.id = e.track_id
             WHERE e.playlist_id = ?1 ORDER BY e.position"
        ))?;
        Ok(statement
            .query_map([id.0], |row| {
                Ok(PlaylistEntry {
                    position: row.get(15)?,
                    track: track_from(row)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn create_playlist(&self, name: &str) -> Result<PlaylistId> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        self.conn.execute(
            "INSERT INTO playlists (name, created_ms) VALUES (?1, ?2)",
            params![name.trim(), now],
        )?;
        Ok(PlaylistId(self.conn.last_insert_rowid()))
    }

    pub fn rename_playlist(&self, id: PlaylistId, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE playlists SET name = ?2 WHERE id = ?1",
            params![id.0, name.trim()],
        )?;
        Ok(())
    }

    /// Deletes the playlist, its entries and the owner's picture for it.
    pub fn delete_playlist(&self, id: PlaylistId) -> Result<()> {
        let image = self.own_image(id)?;
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", [id.0])?;
        if let Some(image) = image {
            remove_picture(&image);
        }
        Ok(())
    }

    /// Appends tracks to the end of the playlist.
    pub fn add_to_playlist(&self, id: PlaylistId, tracks: &[TrackId]) -> Result<()> {
        let mut entries = self.entry_tracks(id)?;
        entries.extend_from_slice(tracks);
        self.write_entries(id, &entries)
    }

    /// Removes the entries at `positions` (counted from 0); the rest close up.
    pub fn remove_from_playlist(&self, id: PlaylistId, positions: &[u32]) -> Result<()> {
        let remove: HashSet<u32> = positions.iter().copied().collect();
        let entries: Vec<TrackId> = self
            .entry_tracks(id)?
            .into_iter()
            .enumerate()
            .filter(|(position, _)| !remove.contains(&(*position as u32)))
            .map(|(_, track)| track)
            .collect();
        self.write_entries(id, &entries)
    }

    /// Moves the entry at `from` to `to`, shifting those in between.
    pub fn move_in_playlist(&self, id: PlaylistId, from: u32, to: u32) -> Result<()> {
        let mut entries = self.entry_tracks(id)?;
        let (from, to) = (from as usize, to as usize);
        if from >= entries.len() || to >= entries.len() || from == to {
            return Ok(());
        }
        let track = entries.remove(from);
        entries.insert(to, track);
        self.write_entries(id, &entries)
    }

    /// Makes an image the playlist's picture: a copy, bounded to 600 px, goes to the
    /// data directory, replacing an earlier one. An animated picture (GIF, WebP or
    /// APNG, within [`animation::MAX_ANIMATION_BYTES`]) is kept as it is beside that
    /// copy, which shows its first frame.
    pub fn set_playlist_image(&self, id: PlaylistId, source: &Path) -> Result<()> {
        let data = fs::read(source).map_err(io_error(source))?;
        let image = image::load_from_memory(&data)
            .map_err(|_| Error::UnreadableImage(source.to_path_buf()))?;
        let picture = image.thumbnail(PICTURE_SIZE, PICTURE_SIZE).into_rgb8();
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 88)
            .encode_image(&picture)
            .map_err(|_| Error::UnreadableImage(source.to_path_buf()))?;
        let stem = format!("{}-{}", id.0, &blake3::hash(&jpeg).to_hex()[..16]);
        let dir = self.images.join("playlists");
        fs::create_dir_all(&dir).map_err(io_error(&dir))?;
        let path = dir.join(format!("{stem}.jpg"));
        fs::write(&path, &jpeg).map_err(io_error(&path))?;
        if let Some(extension) = animation::animation_extension(&data) {
            let moving = dir.join(format!("{stem}.{extension}"));
            fs::write(&moving, &data).map_err(io_error(&moving))?;
        }
        let old = self.own_image(id)?;
        self.conn.execute(
            "UPDATE playlists SET image = ?2 WHERE id = ?1",
            params![id.0, format!("{stem}.jpg")],
        )?;
        if let Some(old) = old
            && old != path
        {
            remove_picture(&old);
        }
        Ok(())
    }

    /// Goes back to the picture made from album covers.
    pub fn clear_playlist_image(&self, id: PlaylistId) -> Result<()> {
        let old = self.own_image(id)?;
        self.conn
            .execute("UPDATE playlists SET image = NULL WHERE id = ?1", [id.0])?;
        if let Some(old) = old {
            remove_picture(&old);
        }
        Ok(())
    }

    fn own_image(&self, id: PlaylistId) -> Result<Option<PathBuf>> {
        let name: Option<String> = self
            .conn
            .query_row("SELECT image FROM playlists WHERE id = ?1", [id.0], |row| {
                row.get(0)
            })
            .optional()?
            .flatten();
        Ok(name.map(|name| self.images.join("playlists").join(name)))
    }

    fn entry_tracks(&self, id: PlaylistId) -> Result<Vec<TrackId>> {
        let mut statement = self.conn.prepare(
            "SELECT track_id FROM playlist_entries WHERE playlist_id = ?1 ORDER BY position",
        )?;
        Ok(statement
            .query_map([id.0], |row| Ok(TrackId(row.get(0)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Rewrites the playlist's entries in order. Playlists are small, and a rewrite
    /// avoids shifting positions under the (playlist, position) key one by one.
    fn write_entries(&self, id: PlaylistId, tracks: &[TrackId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM playlist_entries WHERE playlist_id = ?1",
            [id.0],
        )?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO playlist_entries (playlist_id, position, track_id) VALUES (?1, ?2, ?3)",
            )?;
            for (position, track) in tracks.iter().enumerate() {
                insert.execute(params![id.0, position as i64, track.0])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn with_picture(&self, row: PlaylistRow) -> Result<Playlist> {
        let own = row
            .image
            .map(|name| self.images.join("playlists").join(name))
            .filter(|path| path.is_file());
        let picture = match &own {
            Some(path) => Some(path.clone()),
            None => self.cover_mosaic(row.id)?,
        };
        Ok(Playlist {
            id: row.id,
            name: row.name,
            track_count: row.track_count,
            available_count: row.available_count,
            duration: row.duration,
            own_picture: own.is_some(),
            picture,
        })
    }

    /// The picture made from the first four different album covers (the files' own
    /// or a fetched one), in playlist order: a 2 x 2 grid, or the first cover with
    /// fewer albums.
    fn cover_mosaic(&self, id: PlaylistId) -> Result<Option<PathBuf>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT COALESCE(a.artwork_hash, fc.artwork_hash) AS cover FROM playlist_entries e
             JOIN tracks t ON t.id = e.track_id
             JOIN albums a ON a.id = t.album_id
             LEFT JOIN fetched_covers fc ON fc.group_key = a.group_key
             WHERE e.playlist_id = ?1 AND cover IS NOT NULL
             ORDER BY e.position",
        )?;
        let mut keys: Vec<String> = Vec::new();
        for key in statement.query_map([id.0], |row| row.get::<_, String>(0))? {
            let key = key?;
            if !keys.contains(&key) && self.artwork.get(&key).is_some() {
                keys.push(key);
                if keys.len() == 4 {
                    break;
                }
            }
        }
        self.artwork.mosaic(&keys)
    }
}

/// Deletes an owner's picture and the animation kept beside it.
fn remove_picture(still: &Path) {
    if let Some(moving) = animation::beside(still) {
        let _ = fs::remove_file(moving);
    }
    let _ = fs::remove_file(still);
}

const PLAYLIST_QUERY: &str = "SELECT p.id, p.name, p.image, COUNT(e.track_id),
        COALESCE(SUM(t.available), 0),
        COALESCE(SUM(CASE WHEN t.available = 1 THEN t.duration_ms ELSE 0 END), 0)
    FROM playlists p
    LEFT JOIN playlist_entries e ON e.playlist_id = p.id
    LEFT JOIN tracks t ON t.id = e.track_id";

struct PlaylistRow {
    id: PlaylistId,
    name: String,
    image: Option<String>,
    track_count: u32,
    available_count: u32,
    duration: Duration,
}

fn playlist_row(row: &Row) -> rusqlite::Result<PlaylistRow> {
    Ok(PlaylistRow {
        id: PlaylistId(row.get(0)?),
        name: row.get(1)?,
        image: row.get(2)?,
        track_count: row.get(3)?,
        available_count: row.get(4)?,
        duration: Duration::from_millis(row.get::<_, i64>(5)?.max(0) as u64),
    })
}
