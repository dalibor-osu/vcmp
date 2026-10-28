//! Labels: the owner's own names for tracks ("chill", "workout"), each with a colour.
//! They live only in the database, never in the files.
//!
//! Labels belong to tracks; labelling an album labels the tracks it holds now. Links
//! point at tracks, which are never deleted, so a label stays on a track while its
//! file is gone and shows again with it, like a playlist entry. Names are unique
//! ignoring case, so people and agents can refer to a label by its name.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, Row, params};

use crate::query::{self, TrackFilter};
use crate::{AlbumId, Error, Library, Result, Track, TrackId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LabelId(pub i64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub id: LabelId,
    pub name: String,
    /// `palette:` and one of [`LABEL_PALETTE`], or `#rrggbb`.
    pub colour: String,
    /// Available tracks holding it.
    pub track_count: u32,
}

/// The colours a label can take by name; the UI draws each in a light and a dark
/// variant. Any other colour is stored as `#rrggbb` and drawn as it is.
pub const LABEL_PALETTE: [&str; 10] = [
    "pink", "mauve", "red", "peach", "yellow", "green", "teal", "sky", "blue", "lavender",
];

/// The longest label name, in characters.
pub const LABEL_NAME_MAX: usize = 40;

/// Whether `colour` is one a label can be stored with.
pub fn valid_label_colour(colour: &str) -> bool {
    match colour.strip_prefix("palette:") {
        Some(name) => LABEL_PALETTE.contains(&name),
        None => {
            colour.len() == 7
                && colour.starts_with('#')
                && colour[1..]
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
        }
    }
}

/// The name as stored: trimmed, and checked for length.
pub(crate) fn label_name(name: &str) -> Result<String> {
    let name = name.trim();
    let length = name.chars().count();
    if length == 0 || length > LABEL_NAME_MAX {
        return Err(Error::InvalidLabelName(name.to_owned()));
    }
    Ok(name.to_owned())
}

impl Library {
    /// Every label by name.
    pub fn labels(&self) -> Result<Vec<Label>> {
        let mut statement = self.conn.prepare(&format!(
            "{LABEL_QUERY} GROUP BY l.id ORDER BY l.name, l.id"
        ))?;
        Ok(statement
            .query_map([], label_from)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn label(&self, id: LabelId) -> Result<Option<Label>> {
        Ok(self
            .conn
            .query_row(
                &format!("{LABEL_QUERY} WHERE l.id = ?1 GROUP BY l.id"),
                [id.0],
                label_from,
            )
            .optional()?)
    }

    /// The label of that name, ignoring case (beyond ASCII too).
    pub fn label_named(&self, name: &str) -> Result<Option<Label>> {
        let wanted = name.trim().to_lowercase();
        Ok(self
            .labels()?
            .into_iter()
            .find(|label| label.name.to_lowercase() == wanted))
    }

    pub fn create_label(&self, name: &str, colour: &str) -> Result<LabelId> {
        let name = label_name(name)?;
        self.check_name_free(&name, None)?;
        if !valid_label_colour(colour) {
            return Err(Error::InvalidLabelColour(colour.to_owned()));
        }
        self.conn.execute(
            "INSERT INTO labels (name, colour, created_ms) VALUES (?1, ?2, ?3)",
            params![name, colour, now_ms()],
        )?;
        Ok(LabelId(self.conn.last_insert_rowid()))
    }

    pub fn rename_label(&self, id: LabelId, name: &str) -> Result<()> {
        let name = label_name(name)?;
        self.check_name_free(&name, Some(id))?;
        self.conn.execute(
            "UPDATE labels SET name = ?2 WHERE id = ?1",
            params![id.0, name],
        )?;
        Ok(())
    }

    pub fn set_label_colour(&self, id: LabelId, colour: &str) -> Result<()> {
        if !valid_label_colour(colour) {
            return Err(Error::InvalidLabelColour(colour.to_owned()));
        }
        self.conn.execute(
            "UPDATE labels SET colour = ?2 WHERE id = ?1",
            params![id.0, colour],
        )?;
        Ok(())
    }

    /// Deletes the label and takes it off every track.
    pub fn delete_label(&self, id: LabelId) -> Result<()> {
        self.conn
            .execute("DELETE FROM labels WHERE id = ?1", [id.0])?;
        Ok(())
    }

    /// Puts the label on the tracks; those that hold it already keep it.
    pub fn add_label(&self, id: LabelId, tracks: &[TrackId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for track in tracks {
            self.set_link(id, *track, true)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Takes the label off the tracks.
    pub fn remove_label(&self, id: LabelId, tracks: &[TrackId]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for track in tracks {
            self.set_link(id, *track, false)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Puts the label on one track or takes it off, inside the caller's transaction.
    pub(crate) fn set_link(&self, id: LabelId, track: TrackId, on: bool) -> Result<()> {
        if on {
            self.conn
                .prepare_cached(
                    "INSERT OR IGNORE INTO track_labels (track_id, label_id, added_ms)
                     VALUES (?1, ?2, ?3)",
                )?
                .execute(params![track.0, id.0, now_ms()])?;
        } else {
            self.conn
                .prepare_cached("DELETE FROM track_labels WHERE track_id = ?1 AND label_id = ?2")?
                .execute(params![track.0, id.0])?;
        }
        Ok(())
    }

    /// The labels on one track, by name.
    pub fn track_labels_of(&self, track: TrackId) -> Result<Vec<Label>> {
        let mut statement = self.conn.prepare_cached(&format!(
            "{LABEL_QUERY} WHERE l.id IN (SELECT label_id FROM track_labels WHERE track_id = ?1)
             GROUP BY l.id ORDER BY l.name, l.id"
        ))?;
        Ok(statement
            .query_map([track.0], label_from)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Every link, available tracks or not: each labelled track with its labels.
    pub fn label_links(&self) -> Result<HashMap<TrackId, Vec<LabelId>>> {
        let mut statement = self
            .conn
            .prepare("SELECT track_id, label_id FROM track_labels ORDER BY track_id")?;
        let mut links: HashMap<TrackId, Vec<LabelId>> = HashMap::new();
        for row in
            statement.query_map([], |row| Ok((TrackId(row.get(0)?), LabelId(row.get(1)?))))?
        {
            let (track, label) = row?;
            links.entry(track).or_default().push(label);
        }
        Ok(links)
    }

    /// For each album, the labels at least one of its available tracks holds.
    pub fn album_labels(&self) -> Result<HashMap<AlbumId, Vec<LabelId>>> {
        let mut statement = self.conn.prepare(
            "SELECT DISTINCT t.album_id, tl.label_id FROM track_labels tl
             JOIN tracks t ON t.id = tl.track_id
             WHERE t.available = 1 AND t.album_id IS NOT NULL",
        )?;
        let mut albums: HashMap<AlbumId, Vec<LabelId>> = HashMap::new();
        for row in
            statement.query_map([], |row| Ok((AlbumId(row.get(0)?), LabelId(row.get(1)?))))?
        {
            let (album, label) = row?;
            albums.entry(album).or_default().push(label);
        }
        Ok(albums)
    }

    /// The available tracks holding the label, in the library's order (artist,
    /// album, disc, track).
    pub fn label_tracks(&self, id: LabelId) -> Result<Vec<Track>> {
        query::tracks(&self.conn, TrackFilter::Label(id))
    }

    /// The available tracks holding every one of `labels`, by id, `limit` of them
    /// from `offset`.
    pub fn tracks_with_labels(
        &self,
        labels: &[LabelId],
        offset: usize,
        limit: usize,
    ) -> Result<Vec<TrackId>> {
        if labels.is_empty() {
            return Ok(Vec::new());
        }
        let list = labels
            .iter()
            .map(|label| label.0.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut statement = self.conn.prepare(&format!(
            "SELECT t.id FROM tracks t JOIN track_labels tl ON tl.track_id = t.id
             WHERE t.available = 1 AND tl.label_id IN ({list})
             GROUP BY t.id HAVING COUNT(DISTINCT tl.label_id) = ?1
             ORDER BY t.id LIMIT ?2 OFFSET ?3"
        ))?;
        let wanted = labels
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        Ok(statement
            .query_map(params![wanted as i64, limit as i64, offset as i64], |row| {
                Ok(TrackId(row.get(0)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Fails when another label (not `except`) has the name, ignoring case.
    fn check_name_free(&self, name: &str, except: Option<LabelId>) -> Result<()> {
        match self.label_named(name)? {
            Some(label) if Some(label.id) != except => Err(Error::LabelNameTaken(label.name)),
            _ => Ok(()),
        }
    }
}

const LABEL_QUERY: &str = "SELECT l.id, l.name, l.colour, COUNT(t.id) FROM labels l
    LEFT JOIN track_labels tl ON tl.label_id = l.id
    LEFT JOIN tracks t ON t.id = tl.track_id AND t.available = 1";

fn label_from(row: &Row) -> rusqlite::Result<Label> {
    Ok(Label {
        id: LabelId(row.get(0)?),
        name: row.get(1)?,
        colour: row.get(2)?,
        track_count: row.get(3)?,
    })
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
