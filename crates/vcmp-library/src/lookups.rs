//! What VCMP found online for an album: the Discord presence's cover lookups, kept so
//! that each album is looked up once, and covers the user fetched for albums whose
//! files have no art.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, params};

use crate::{AlbumId, Artwork, Library, Result};

/// An album cover on the Cover Art Archive, found through MusicBrainz.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverArtId {
    /// The cover of one release (one edition of the album), by its MBID.
    Release(String),
    /// The cover the archive picks for a release group (every edition), by its MBID.
    ReleaseGroup(String),
}

/// The outcome of looking an album up, and when that happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverLookup {
    /// `None` when nothing matched.
    pub cover: Option<CoverArtId>,
    pub looked_up: SystemTime,
}

impl Library {
    /// The stored outcome of looking `album` up, if it was looked up.
    pub fn cover_lookup(&self, album: AlbumId) -> Result<Option<CoverLookup>> {
        let row = self
            .conn
            .query_row(
                "SELECT l.cover_kind, l.cover_mbid, l.looked_up_ms
                   FROM cover_lookups l JOIN albums a ON a.group_key = l.group_key
                  WHERE a.id = ?1",
                [album.0],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(kind, mbid, at)| CoverLookup {
            cover: match (kind.as_deref(), mbid) {
                (Some("release"), Some(mbid)) => Some(CoverArtId::Release(mbid)),
                (Some("release-group"), Some(mbid)) => Some(CoverArtId::ReleaseGroup(mbid)),
                _ => None,
            },
            looked_up: UNIX_EPOCH + Duration::from_millis(at.max(0) as u64),
        }))
    }

    /// Stores the outcome of looking `album` up at `at`; `cover` is `None` when
    /// nothing matched. Replaces an earlier outcome.
    pub fn set_cover_lookup(
        &self,
        album: AlbumId,
        cover: Option<&CoverArtId>,
        at: SystemTime,
    ) -> Result<()> {
        let (kind, mbid) = cover.map(CoverArtId::kind_and_mbid).unzip();
        self.conn.execute(
            "INSERT INTO cover_lookups (group_key, cover_kind, cover_mbid, looked_up_ms)
             SELECT group_key, ?2, ?3, ?4 FROM albums WHERE id = ?1
             ON CONFLICT (group_key) DO UPDATE SET
                 cover_kind = excluded.cover_kind,
                 cover_mbid = excluded.cover_mbid,
                 looked_up_ms = excluded.looked_up_ms",
            params![album.0, kind, mbid, millis(at)],
        )?;
        Ok(())
    }

    /// Stores `image`, downloaded from the Cover Art Archive's `source`, as the cover
    /// of `album`, whose files have no art of their own. The image is cached like
    /// extracted artwork and kept by the album's grouping key, so rescans keep it;
    /// art the files gain later is shown instead. Replaces an earlier fetched cover.
    ///
    /// Returns the cached files, or `None` when `image` is not an image VCMP can
    /// decode or the album is gone.
    pub fn set_fetched_cover(
        &self,
        album: AlbumId,
        image: &[u8],
        source: &CoverArtId,
    ) -> Result<Option<Artwork>> {
        let Some(key) = self.artwork.store(image)? else {
            return Ok(None);
        };
        let (kind, mbid) = source.kind_and_mbid();
        let stored = self.conn.execute(
            "INSERT INTO fetched_covers (group_key, artwork_hash, cover_kind, cover_mbid, fetched_ms)
             SELECT group_key, ?2, ?3, ?4, ?5 FROM albums WHERE id = ?1
             ON CONFLICT (group_key) DO UPDATE SET
                 artwork_hash = excluded.artwork_hash,
                 cover_kind = excluded.cover_kind,
                 cover_mbid = excluded.cover_mbid,
                 fetched_ms = excluded.fetched_ms",
            params![album.0, key, kind, mbid, millis(SystemTime::now())],
        )?;
        Ok(if stored == 0 {
            None
        } else {
            self.artwork.get(&key)
        })
    }
}

impl Library {
    /// Forgets the cover the user fetched for `album`, and what the automatic lookup
    /// found for it, so that lookup may run again.
    pub fn clear_fetched_cover(&self, album: AlbumId) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for table in ["fetched_covers", "cover_lookups"] {
            tx.execute(
                &format!(
                    "DELETE FROM {table}
                     WHERE group_key = (SELECT group_key FROM albums WHERE id = ?1)"
                ),
                [album.0],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

impl CoverArtId {
    /// The kind as the database names it, and the MBID.
    fn kind_and_mbid(&self) -> (&'static str, &str) {
        match self {
            CoverArtId::Release(mbid) => ("release", mbid),
            CoverArtId::ReleaseGroup(mbid) => ("release-group", mbid),
        }
    }
}

fn millis(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(
            &dir.path().join("library.sqlite3"),
            &dir.path().join("cache"),
        )
        .unwrap();
        (dir, library)
    }

    fn add_album(library: &Library, key: &str) -> AlbumId {
        library
            .conn
            .execute(
                "INSERT INTO albums (group_key, title) VALUES (?1, 'Title')",
                [key],
            )
            .unwrap();
        AlbumId(library.conn.last_insert_rowid())
    }

    #[test]
    fn an_album_is_not_looked_up_until_it_is() {
        let (_dir, library) = library();
        let album = add_album(&library, "band\u{1f}title");
        assert_eq!(library.cover_lookup(album).unwrap(), None);
    }

    #[test]
    fn found_and_not_found_are_both_kept() {
        let (_dir, library) = library();
        let found = add_album(&library, "a\u{1f}one");
        let missing = add_album(&library, "a\u{1f}two");
        let at = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let cover = CoverArtId::ReleaseGroup("rg-1".into());
        library.set_cover_lookup(found, Some(&cover), at).unwrap();
        library.set_cover_lookup(missing, None, at).unwrap();

        assert_eq!(
            library.cover_lookup(found).unwrap(),
            Some(CoverLookup {
                cover: Some(cover),
                looked_up: at
            })
        );
        assert_eq!(
            library.cover_lookup(missing).unwrap(),
            Some(CoverLookup {
                cover: None,
                looked_up: at
            })
        );

        let later = at + Duration::from_secs(60);
        let release = CoverArtId::Release("r-2".into());
        library
            .set_cover_lookup(missing, Some(&release), later)
            .unwrap();
        assert_eq!(
            library.cover_lookup(missing).unwrap().unwrap().cover,
            Some(release),
            "a new outcome replaces the old one"
        );
    }

    #[test]
    fn a_fetched_cover_must_be_an_image_for_an_album_that_exists() {
        let (_dir, library) = library();
        let album = add_album(&library, "band\u{1f}title");
        let source = CoverArtId::Release("r-1".into());
        assert_eq!(
            library
                .set_fetched_cover(album, b"not an image", &source)
                .unwrap(),
            None
        );

        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let png = png.into_inner();
        assert_eq!(
            library
                .set_fetched_cover(AlbumId(album.0 + 1), &png, &source)
                .unwrap(),
            None,
            "no such album"
        );
        let artwork = library
            .set_fetched_cover(album, &png, &source)
            .unwrap()
            .expect("a png is accepted");
        assert!(artwork.thumbnail.is_file());
    }

    #[test]
    fn the_outcome_outlives_the_album_row() {
        let (_dir, library) = library();
        let album = add_album(&library, "band\u{1f}title");
        let cover = CoverArtId::Release("r-1".into());
        library
            .set_cover_lookup(album, Some(&cover), SystemTime::now())
            .unwrap();

        // A scan that loses every track removes the album; finding them again adds a
        // new row under the same key (with another album in between, a new id).
        add_album(&library, "other\u{1f}album");
        library
            .conn
            .execute("DELETE FROM albums WHERE id = ?1", [album.0])
            .unwrap();
        let again = add_album(&library, "band\u{1f}title");
        assert_ne!(again, album);
        assert_eq!(
            library.cover_lookup(again).unwrap().unwrap().cover,
            Some(cover)
        );
    }
}
