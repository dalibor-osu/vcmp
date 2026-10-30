//! Artist photos the owner chose: found online or picked from a file. They are kept
//! under the data directory (`images/artists/`), since nothing could rebuild them,
//! and are keyed by the artist's name, which outlives the artist rows a scan removes
//! and adds.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use image::codecs::jpeg::JpegEncoder;
use rusqlite::{OptionalExtension, params};

use crate::animation;
use crate::error::io_error;
use crate::{ArtistId, Error, Library, Result};

/// Longest side of a photo as kept: the artist page shows it at 200 points.
const PHOTO_SIZE: u32 = 800;

impl Library {
    /// The photo chosen for an artist, if its file is there.
    pub fn artist_photo(&self, artist: ArtistId) -> Result<Option<PathBuf>> {
        let image: Option<String> = self
            .conn
            .query_row(
                "SELECT p.image FROM artists ar JOIN artist_photos p ON p.name = ar.name
                 WHERE ar.id = ?1",
                [artist.0],
                |row| row.get(0),
            )
            .optional()?;
        Ok(image
            .map(|image| self.artist_photo_dir().join(image))
            .filter(|path| path.is_file()))
    }

    /// Where the artist's photo came from: the web address it was downloaded from,
    /// `None` for one picked from a file (or no photo).
    pub fn artist_photo_source(&self, artist: ArtistId) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT p.source FROM artists ar JOIN artist_photos p ON p.name = ar.name
                 WHERE ar.id = ?1",
                [artist.0],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Every artist with a chosen photo whose file is there.
    pub fn artist_photos(&self) -> Result<HashMap<ArtistId, PathBuf>> {
        let mut statement = self.conn.prepare(
            "SELECT ar.id, p.image FROM artists ar JOIN artist_photos p ON p.name = ar.name",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((ArtistId(row.get(0)?), row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let dir = self.artist_photo_dir();
        Ok(rows
            .into_iter()
            .map(|(id, image)| (id, dir.join(image)))
            .filter(|(_, path)| path.is_file())
            .collect())
    }

    /// The MusicBrainz artist ID the files are tagged with, if any.
    pub fn artist_mbid(&self, artist: ArtistId) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT mbid FROM artists WHERE id = ?1",
                [artist.0],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Makes an image the artist's photo: a JPEG copy bounded to 800 px goes to the
    /// data directory, replacing an earlier photo. An animated one (GIF, WebP or APNG,
    /// within [`animation::MAX_ANIMATION_BYTES`]) is kept as it is beside that copy,
    /// which shows its first frame. `source` says where it came from
    /// (a web address), `None` for a file of the owner's. Returns the kept file.
    pub fn set_artist_photo(
        &self,
        artist: ArtistId,
        data: &[u8],
        source: Option<&str>,
    ) -> Result<PathBuf> {
        let name: String = self
            .conn
            .query_row(
                "SELECT name FROM artists WHERE id = ?1",
                [artist.0],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::UnreadableImage(PathBuf::from(format!("artist {}", artist.0))))?;
        let unreadable = || Error::UnreadableImage(PathBuf::from(source.unwrap_or("the image")));
        let image = image::load_from_memory(data).map_err(|_| unreadable())?;
        let photo = image.thumbnail(PHOTO_SIZE, PHOTO_SIZE).into_rgb8();
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 88)
            .encode_image(&photo)
            .map_err(|_| unreadable())?;
        let stem = blake3::hash(&jpeg).to_hex()[..16].to_string();
        let file = format!("{stem}.jpg");
        let dir = self.artist_photo_dir();
        fs::create_dir_all(&dir).map_err(io_error(&dir))?;
        let path = dir.join(&file);
        fs::write(&path, &jpeg).map_err(io_error(&path))?;
        if let Some(extension) = animation::animation_extension(data) {
            let moving = dir.join(format!("{stem}.{extension}"));
            fs::write(&moving, data).map_err(io_error(&moving))?;
        }
        let old = self.own_artist_image(&name)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        self.conn.execute(
            "INSERT INTO artist_photos (name, image, source, chosen_ms) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (name) DO UPDATE SET
                 image = excluded.image, source = excluded.source, chosen_ms = excluded.chosen_ms",
            params![name, file, source, now],
        )?;
        if let Some(old) = old
            && old != file
        {
            self.remove_unused_artist_image(&old)?;
        }
        Ok(path)
    }

    /// Goes back to showing the artist without a photo of their own.
    pub fn clear_artist_photo(&self, artist: ArtistId) -> Result<()> {
        let name: Option<String> = self
            .conn
            .query_row(
                "SELECT name FROM artists WHERE id = ?1",
                [artist.0],
                |row| row.get(0),
            )
            .optional()?;
        let Some(name) = name else {
            return Ok(());
        };
        let old = self.own_artist_image(&name)?;
        self.conn
            .execute("DELETE FROM artist_photos WHERE name = ?1", [&name])?;
        if let Some(old) = old {
            self.remove_unused_artist_image(&old)?;
        }
        Ok(())
    }

    fn own_artist_image(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT image FROM artist_photos WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Deletes an image file no artist uses any more (two artists can share one).
    fn remove_unused_artist_image(&self, file: &str) -> Result<()> {
        let used: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM artist_photos WHERE image = ?1)",
            [file],
            |row| row.get(0),
        )?;
        if !used {
            let still = self.artist_photo_dir().join(Path::new(file));
            if let Some(moving) = animation::beside(&still) {
                let _ = fs::remove_file(moving);
            }
            let _ = fs::remove_file(still);
        }
        Ok(())
    }

    fn artist_photo_dir(&self) -> PathBuf {
        self.images.join("artists")
    }
}
