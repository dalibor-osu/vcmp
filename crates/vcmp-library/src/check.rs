//! The library check: duplicate tracks, tracks with missing tags and albums without a
//! cover. Every query here only reads; the one thing this module writes is the
//! "Not duplicates" marks in the database. It never touches a music file.
//!
//! Duplicates are available tracks with a real title and a track artist whose title
//! and artist match once case, accents (of the Latin, Greek and Cyrillic scripts) and
//! runs of spaces are ignored, and whose lengths lie within 2 s of each other: sorted
//! by length, a group runs on while each track is at most 2 s longer than the one
//! before, so a group can span more than 2 s.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use rusqlite::{Transaction, TransactionBehavior, params};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::query::{AlbumId, TRACK_COLUMNS, track_from};
use crate::{Album, Library, Result, Track, TrackId};

/// The widest gap between the lengths of two neighbouring tracks in a duplicate group.
pub const DUPLICATE_WINDOW: Duration = Duration::from_secs(2);

/// Tracks the check takes for the same recording.
#[derive(Clone, Debug, PartialEq)]
pub struct DuplicateGroup {
    /// Shortest first.
    pub tracks: Vec<Track>,
    /// Marked "Not duplicates": every track is in one group the owner marked.
    pub hidden: bool,
}

/// The fields a track lacks. The title counts as missing when the file holds none and
/// the track shows its file name instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissingFields {
    pub title: bool,
    pub artist: bool,
    pub album: bool,
    pub track_number: bool,
    pub year: bool,
}

impl MissingFields {
    pub fn any(self) -> bool {
        self.title || self.artist || self.album || self.track_number || self.year
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackGaps {
    pub track: Track,
    pub missing: MissingFields,
}

/// An album's tracks that lack tags, in album order.
#[derive(Clone, Debug, PartialEq)]
pub struct AlbumGaps {
    pub album: Album,
    pub tracks: Vec<TrackGaps>,
}

impl Library {
    /// Every duplicate group among the available tracks, hidden ones included, by
    /// artist and title.
    pub fn duplicate_groups(&self) -> Result<Vec<DuplicateGroup>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {TRACK_COLUMNS} FROM tracks t
             WHERE t.available = 1 AND t.artist IS NOT NULL AND TRIM(t.artist) != ''
               AND {HAS_TITLE}"
        ))?;
        let tracks: Vec<Track> = statement
            .query_map([], track_from)?
            .collect::<rusqlite::Result<_>>()?;
        let marked = self.marked_groups()?;
        Ok(group_duplicates(tracks)
            .into_iter()
            .map(|tracks| DuplicateGroup {
                hidden: is_marked(&marked, &tracks),
                tracks,
            })
            .collect())
    }

    /// Marks the tracks as not duplicates of each other. A group found later hides
    /// while all its tracks are among these.
    pub fn mark_not_duplicates(&self, tracks: &[TrackId]) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let group: i64 = tx.query_row(
            "SELECT IFNULL(MAX(group_id), 0) + 1 FROM not_duplicates",
            [],
            |row| row.get(0),
        )?;
        for track in tracks {
            tx.prepare_cached(
                "INSERT OR IGNORE INTO not_duplicates (group_id, track_id) VALUES (?1, ?2)",
            )?
            .execute(params![group, track.0])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Undoes "Not duplicates" for the tracks: forgets every marked group that holds
    /// all of them, so a group they form shows again.
    pub fn unmark_not_duplicates(&self, tracks: &[TrackId]) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        for (group, members) in self.marked_groups()? {
            if tracks.iter().all(|track| members.contains(track)) {
                tx.prepare_cached("DELETE FROM not_duplicates WHERE group_id = ?1")?
                    .execute([group])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Available tracks lacking a title, artist, album, track number or year, by
    /// album in library order.
    pub fn missing_tags(&self) -> Result<Vec<AlbumGaps>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {TRACK_COLUMNS}, NOT {HAS_TITLE} FROM tracks t
             LEFT JOIN albums a ON a.id = t.album_id
             LEFT JOIN artists aa ON aa.id = a.artist_id
             WHERE t.available = 1 AND (NOT {HAS_TITLE}
                OR t.artist IS NULL OR TRIM(t.artist) = ''
                OR t.album IS NULL OR TRIM(t.album) = ''
                OR t.track_number IS NULL OR t.year IS NULL)
             ORDER BY aa.name IS NULL, aa.name COLLATE NOCASE, a.unknown, a.year,
                      a.title COLLATE NOCASE, t.album_id, t.disc_number, t.track_number,
                      t.title COLLATE NOCASE"
        ))?;
        let found: Vec<TrackGaps> = statement
            .query_map([], |row| {
                let track = track_from(row)?;
                let blank = |text: &Option<String>| text.as_deref().is_none_or(is_blank);
                Ok(TrackGaps {
                    missing: MissingFields {
                        title: row.get(16)?,
                        artist: blank(&track.artist),
                        album: blank(&track.album),
                        track_number: track.track_number.is_none(),
                        year: track.year.is_none(),
                    },
                    track,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut albums: HashMap<AlbumId, Album> = self
            .albums()?
            .into_iter()
            .map(|album| (album.id, album))
            .collect();
        let mut gaps: Vec<AlbumGaps> = Vec::new();
        for track in found {
            let id = track.track.album_id;
            match gaps.last_mut() {
                Some(last) if Some(last.album.id) == id => last.tracks.push(track),
                _ => {
                    let Some(album) = id.and_then(|id| albums.remove(&id)) else {
                        continue;
                    };
                    gaps.push(AlbumGaps {
                        album,
                        tracks: vec![track],
                    });
                }
            }
        }
        Ok(gaps)
    }

    /// Albums with available tracks whose files and folders hold no cover, that a
    /// scan has looked for one, and that have no cover fetched online. "Unknown"
    /// albums are left out: they are no release a cover could be found for.
    pub fn albums_without_cover(&self) -> Result<Vec<Album>> {
        self.albums()?
            .into_iter()
            .filter(|album| !album.unknown && album.artwork.is_none())
            .filter_map(|album| match self.artwork_looked_for(album.id) {
                Ok(true) => Some(Ok(album)),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    /// Whether a scan looked for the album's art and found none.
    fn artwork_looked_for(&self, album: AlbumId) -> Result<bool> {
        let state: i64 = self
            .conn
            .prepare_cached("SELECT artwork_state FROM albums WHERE id = ?1")?
            .query_row([album.0], |row| row.get(0))?;
        Ok(state == 2)
    }

    fn marked_groups(&self) -> Result<BTreeMap<i64, HashSet<TrackId>>> {
        let mut statement = self
            .conn
            .prepare("SELECT group_id, track_id FROM not_duplicates")?;
        let mut groups: BTreeMap<i64, HashSet<TrackId>> = BTreeMap::new();
        for row in statement.query_map([], |row| Ok((row.get(0)?, TrackId(row.get(1)?))))? {
            let (group, track) = row?;
            groups.entry(group).or_default().insert(track);
        }
        Ok(groups)
    }
}

/// The condition on `tracks t` for a title the file holds, not the file-name fallback.
const HAS_TITLE: &str = "EXISTS (SELECT 1 FROM track_tags g WHERE g.track_id = t.id
    AND g.key = 'TrackTitle' AND TRIM(g.value) != '')";

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

fn is_marked(marked: &BTreeMap<i64, HashSet<TrackId>>, tracks: &[Track]) -> bool {
    marked
        .values()
        .any(|members| tracks.iter().all(|track| members.contains(&track.id)))
}

/// `text` for comparing: compatibility-decomposed, accents dropped, lower case, runs
/// of white space made one space. Only the accents of the Latin, Greek and Cyrillic
/// scripts go; other marks stay and compose again, so the Japanese voicing marks
/// keep "が" apart from "か".
pub(crate) fn fold(text: &str) -> String {
    let stripped: String = text
        .nfkd()
        .filter(|c| !is_accent(*c))
        .flat_map(char::to_lowercase)
        .nfc()
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A combining accent of the diacritical mark blocks.
fn is_accent(c: char) -> bool {
    is_combining_mark(c)
        && matches!(c,
            '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}')
}

/// Groups tracks whose folded artist and title match and whose lengths chain within
/// [`DUPLICATE_WINDOW`]. Groups of one are dropped.
pub(crate) fn group_duplicates(tracks: Vec<Track>) -> Vec<Vec<Track>> {
    let mut by_name: BTreeMap<(String, String), Vec<Track>> = BTreeMap::new();
    for track in tracks {
        let artist = fold(track.artist.as_deref().unwrap_or_default());
        let title = fold(&track.title);
        if artist.is_empty() || title.is_empty() {
            continue;
        }
        by_name.entry((artist, title)).or_default().push(track);
    }
    let mut groups = Vec::new();
    for (_, mut same) in by_name {
        same.sort_by(|a, b| a.duration.cmp(&b.duration).then(a.path.cmp(&b.path)));
        let mut group: Vec<Track> = Vec::new();
        for track in same {
            if let Some(last) = group.last()
                && track.duration - last.duration > DUPLICATE_WINDOW
            {
                if group.len() > 1 {
                    groups.push(std::mem::take(&mut group));
                } else {
                    group.clear();
                }
            }
            group.push(track);
        }
        if group.len() > 1 {
            groups.push(group);
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn track(id: i64, artist: &str, title: &str, millis: u64) -> Track {
        Track {
            id: TrackId(id),
            path: PathBuf::from(format!("{id}.flac")),
            available: true,
            format: None,
            title: title.into(),
            artist: Some(artist.into()),
            artist_id: None,
            album: None,
            album_id: None,
            album_artist: None,
            track_number: None,
            disc_number: None,
            year: None,
            genre: None,
            duration: Duration::from_millis(millis),
            added: std::time::UNIX_EPOCH,
        }
    }

    fn ids(groups: &[Vec<Track>]) -> Vec<Vec<i64>> {
        groups
            .iter()
            .map(|group| group.iter().map(|track| track.id.0).collect())
            .collect()
    }

    #[test]
    fn folding_ignores_case_accents_and_spacing() {
        assert_eq!(fold("  Beyoncé  Knowles "), "beyonce knowles");
        assert_eq!(fold("ÁÉÍÓÚ ñ"), "aeiou n");
        assert_eq!(fold("Ｆｕｌｌ　Width"), "full width");
        assert_eq!(fold("Crème\tBrûlée"), "creme brulee");
        assert_eq!(fold("高坂"), "高坂");
        // Voicing marks are no accents.
        assert_eq!(fold("ガイド"), "ガイド");
        assert_ne!(fold("が"), fold("か"));
        // Half-width katakana widen and compose.
        assert_eq!(fold("ｶﾞｲﾄﾞ"), "ガイド");
    }

    #[test]
    fn case_and_accents_do_not_split_a_group() {
        let groups = group_duplicates(vec![
            track(1, "Sigur Rós", "Hoppípolla", 260_000),
            track(2, "sigur ros", "HOPPIPOLLA", 261_000),
            track(3, "Sigur Rós", "Glósóli", 260_000),
        ]);
        assert_eq!(ids(&groups), [vec![1, 2]]);
    }

    #[test]
    fn lengths_chain_within_two_seconds() {
        let groups = group_duplicates(vec![
            track(3, "A", "Song", 183_000),
            track(1, "A", "Song", 180_000),
            track(2, "A", "Song", 181_500),
            // 2.001 s after the last one: a group of its own, and alone, dropped.
            track(4, "A", "Song", 185_001),
            // Exactly 2 s apart: together.
            track(5, "A", "Song", 300_000),
            track(6, "A", "Song", 302_000),
        ]);
        assert_eq!(ids(&groups), [vec![1, 2, 3], vec![5, 6]]);
    }

    #[test]
    fn different_artists_are_not_duplicates() {
        let groups = group_duplicates(vec![
            track(1, "A", "Song", 180_000),
            track(2, "B", "Song", 180_000),
        ]);
        assert!(groups.is_empty());
    }

    #[test]
    fn a_marked_group_hides_while_it_holds_every_track() {
        let marked = BTreeMap::from([(1, HashSet::from([TrackId(1), TrackId(2), TrackId(3)]))]);
        let group = |list: &[i64]| -> Vec<Track> {
            list.iter()
                .map(|&id| track(id, "A", "Song", 1000))
                .collect()
        };
        assert!(is_marked(&marked, &group(&[1, 2, 3])));
        // A track went missing: still hidden.
        assert!(is_marked(&marked, &group(&[1, 3])));
        // A track joined: shown again.
        assert!(!is_marked(&marked, &group(&[1, 2, 3, 4])));
    }
}
