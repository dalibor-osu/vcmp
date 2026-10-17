//! What VCMP found online for an album, kept so that each album is looked up once.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, params};

use crate::{AlbumId, Library, Result};

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
        let (kind, mbid) = match cover {
            Some(CoverArtId::Release(mbid)) => (Some("release"), Some(mbid.as_str())),
            Some(CoverArtId::ReleaseGroup(mbid)) => (Some("release-group"), Some(mbid.as_str())),
            None => (None, None),
        };
        let at = at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        self.conn.execute(
            "INSERT INTO cover_lookups (group_key, cover_kind, cover_mbid, looked_up_ms)
             SELECT group_key, ?2, ?3, ?4 FROM albums WHERE id = ?1
             ON CONFLICT (group_key) DO UPDATE SET
                 cover_kind = excluded.cover_kind,
                 cover_mbid = excluded.cover_mbid,
                 looked_up_ms = excluded.looked_up_ms",
            params![album.0, kind, mbid, at],
        )?;
        Ok(())
    }
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
