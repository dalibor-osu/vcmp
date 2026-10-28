//! Change sets: the tag edits, label edits and playlist deletions AI agents ask for,
//! kept as a log the owner reviews and can undo.
//!
//! A tag set records, for every file, what was asked and the values of every tag the
//! edit may change as the database had them, so accepting it can leave alone a file
//! that changed since (a conflict) instead of overwriting it. Undoing builds a new
//! set that writes those values back raw, all of them, checked against what the
//! first set wrote. Covers a set replaces or brings are kept under `images/undo/`.
//! A label set works the same way on the database alone: per track, whether it held
//! each label it changes, under the key `label:` and the name. Finished sets are kept
//! for 30 days.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::edit::{self, Expected, FieldEdit, TagEdit, TagWriteError};
use crate::error::io_error;
use crate::labels::{self, LabelId, valid_label_colour};
use crate::other_tags;
use crate::tags;
use crate::{Error, Library, PlaylistId, Result, TrackId, path_codec, scan};

/// How long a finished set is kept, so it can still be undone.
pub const CHANGE_SETS_KEPT: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChangeSetId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeStatus {
    /// Waiting for the owner.
    Pending,
    /// Every file written (or the playlist deleted).
    Accepted,
    /// Accepted, but one or more files were not written.
    PartlyWritten,
    Rejected,
    /// Still pending when VCMP closed.
    Discarded,
    /// Accepted, and later reversed by an undo set.
    Undone,
}

impl ChangeStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::PartlyWritten => "partly_written",
            Self::Rejected => "rejected",
            Self::Discarded => "discarded",
            Self::Undone => "undone",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "accepted" => Self::Accepted,
            "partly_written" => Self::PartlyWritten,
            "rejected" => Self::Rejected,
            "discarded" => Self::Discarded,
            "undone" => Self::Undone,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Tags,
    /// Deleting a playlist, which can't be undone. The name is as it was asked.
    PlaylistDeletion {
        playlist: PlaylistId,
        name: String,
    },
    /// Labels put on and taken off tracks, in the database only.
    Labels,
}

/// What a label set asks for one track: labels to put on and take off, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelEdit {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

/// A label an agent asks to create along with a label set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewLabel {
    pub name: String,
    /// As [`crate::Label::colour`].
    pub colour: String,
}

/// A label a set creates once accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposedLabel {
    pub name: String,
    pub colour: String,
    /// None until accepted; then whether the set made it, or found a label of that
    /// name (ignoring case) and used it as it was.
    pub created: Option<bool>,
}

/// A set as the history lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSetSummary {
    pub id: ChangeSetId,
    /// The agent that asked, as it named itself.
    pub agent: String,
    pub kind: ChangeKind,
    pub status: ChangeStatus,
    /// The set this one undoes.
    pub undoes: Option<ChangeSetId>,
    pub created: SystemTime,
    pub finished: Option<SystemTime>,
    pub file_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSet {
    pub summary: ChangeSetSummary,
    pub files: Vec<ChangedFile>,
    /// The labels a label set creates.
    pub new_labels: Vec<ProposedLabel>,
}

/// One file of a tag set, or one track of a label set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    pub track: TrackId,
    pub path: PathBuf,
    /// What the set asks, by target: one of the nine fields (`title`, `artist`,
    /// `album`, `album_artist`, `track_number`, `disc_number`, `year`, `genre`,
    /// `cover`) or `tag:` and a [`crate::Library::track_tags`] key. None removes.
    /// A cover's value is the image's path. In a label set: `label:` and the label's
    /// name, with the name as the value to put it on and None to take it off.
    pub edits: Vec<(String, Option<Vec<String>>)>,
    /// Every tag the edits may change.
    pub values: Vec<ValueChange>,
    /// The cover before and after, when the set changes it: an image path, or None
    /// for no cover. `cover_after` is known once the file is written.
    pub cover_before: Option<Option<PathBuf>>,
    pub cover_after: Option<Option<PathBuf>>,
    /// None until the set is accepted.
    pub result: Option<FileResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueChange {
    /// A tag key, or in a label set `label:` and the name; the label's values are
    /// its name when the track holds it and nothing when it doesn't.
    pub key: String,
    /// As the database had them when the set was made.
    pub before: Vec<String>,
    /// As they read back once written.
    pub after: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileResult {
    Written,
    /// The file no longer held what the set was made against, so it was left alone.
    Conflict(String),
    Failed(String),
}

impl Library {
    /// Records a tag set for review: every edit is checked, every track must be in
    /// the library and available, and every key must fit its file's kind of tag.
    /// Nothing is written until [`Library::accept_change_set`].
    pub fn submit_tag_changes(
        &self,
        agent: &str,
        edits: &[(TrackId, TagEdit)],
    ) -> Result<ChangeSetId> {
        if edits.is_empty() {
            return Err(Error::EmptyChangeSet);
        }
        for (_, edit) in edits {
            edit.validate()?;
        }
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for (id, edit) in edits {
            if !seen.insert(*id) {
                return Err(Error::RepeatedTrack(*id));
            }
            let track = self.track(*id)?.ok_or(Error::UnknownTrack(*id))?;
            let format = track
                .format
                .filter(|_| track.available)
                .ok_or(Error::UnavailableTrack(*id))?;
            let resolved = other_tags::resolve(&edit.other, format, false)
                .map_err(|key| Error::UnsupportedKey { track: *id, key })?;
            let entries = self.track_tags(*id)?;
            let keys = other_tags::touched_keys(
                edit,
                &resolved,
                entries.iter().map(|entry| entry.key.as_str()),
            );
            let values: Vec<(String, Vec<String>)> = keys
                .into_iter()
                .map(|key| {
                    let found = entries
                        .iter()
                        .filter(|entry| entry.key == key)
                        .map(|entry| entry.value.clone())
                        .collect();
                    (key, found)
                })
                .collect();
            let cover_before = if edit.cover.is_keep() {
                None
            } else {
                Some(match tags::embedded_cover(&track.path) {
                    Some(data) => self.keep_cover(&data)?,
                    None => String::new(),
                })
            };
            files.push(NewFile {
                track: *id,
                path: track.path,
                edits: self.targets(edit)?,
                values,
                cover_before,
            });
        }
        self.insert_set(agent, "tags", None, None, &files, &[])
    }

    /// Records a label set for review: every label named must exist or be among
    /// `new_labels`, and every track must be in the library (its file may be gone).
    /// Edits that change nothing (a label the track holds already) are left out, and
    /// only the new labels some track gets are kept. Nothing changes until
    /// [`Library::accept_change_set`].
    pub fn submit_label_changes(
        &self,
        agent: &str,
        edits: &[(TrackId, LabelEdit)],
        new_labels: &[NewLabel],
    ) -> Result<ChangeSetId> {
        let existing = self.labels()?;
        let mut created: Vec<(String, String)> = Vec::new();
        for label in new_labels {
            let name = labels::label_name(&label.name)?;
            if !valid_label_colour(&label.colour) {
                return Err(Error::InvalidLabelColour(label.colour.clone()));
            }
            let lower = name.to_lowercase();
            if let Some(taken) = existing
                .iter()
                .map(|label| &label.name)
                .chain(created.iter().map(|(name, _)| name))
                .find(|taken| taken.to_lowercase() == lower)
            {
                return Err(Error::LabelNameTaken(taken.clone()));
            }
            created.push((name, label.colour.clone()));
        }
        let canonical = |name: &str| -> Result<String> {
            let wanted = name.trim().to_lowercase();
            existing
                .iter()
                .map(|label| &label.name)
                .chain(created.iter().map(|(name, _)| name))
                .find(|known| known.to_lowercase() == wanted)
                .cloned()
                .ok_or_else(|| Error::UnknownLabel(name.trim().to_owned()))
        };
        let names: HashMap<LabelId, String> = existing
            .iter()
            .map(|label| (label.id, label.name.to_lowercase()))
            .collect();
        let links = self.label_links()?;
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for (id, edit) in edits {
            if !seen.insert(*id) {
                return Err(Error::RepeatedTrack(*id));
            }
            let track = self.track(*id)?.ok_or(Error::UnknownTrack(*id))?;
            let held: HashSet<&str> = links
                .get(id)
                .into_iter()
                .flatten()
                .filter_map(|label| names.get(label).map(String::as_str))
                .collect();
            let mut wanted: BTreeMap<String, bool> = BTreeMap::new();
            for name in &edit.add {
                wanted.insert(canonical(name)?, true);
            }
            for name in &edit.remove {
                let name = canonical(name)?;
                if wanted.get(&name) == Some(&true) {
                    return Err(Error::ContradictoryLabelEdit {
                        track: *id,
                        label: name,
                    });
                }
                wanted.insert(name, false);
            }
            let changes: Vec<(String, bool)> = wanted
                .into_iter()
                .filter(|(name, on)| held.contains(name.to_lowercase().as_str()) != *on)
                .collect();
            if changes.is_empty() {
                continue;
            }
            files.push(NewFile {
                track: *id,
                path: track.path,
                edits: changes
                    .iter()
                    .map(|(name, on)| (format!("label:{name}"), on.then(|| vec![name.clone()])))
                    .collect(),
                values: changes
                    .iter()
                    .map(|(name, on)| {
                        let before = if *on { Vec::new() } else { vec![name.clone()] };
                        (format!("label:{name}"), before)
                    })
                    .collect(),
                cover_before: None,
            });
        }
        if files.is_empty() {
            return Err(Error::EmptyChangeSet);
        }
        created.retain(|(name, _)| {
            let target = format!("label:{name}");
            files.iter().any(|file| {
                file.edits
                    .iter()
                    .any(|(edit, values)| *edit == target && values.is_some())
            })
        });
        self.insert_set(agent, "labels", None, None, &files, &created)
    }

    /// Records a request to delete a playlist, for review. Deleting can't be undone.
    pub fn submit_playlist_deletion(
        &self,
        agent: &str,
        playlist: PlaylistId,
    ) -> Result<ChangeSetId> {
        let found = self
            .playlist(playlist)?
            .ok_or(Error::UnknownPlaylist(playlist))?;
        self.insert_set(
            agent,
            "playlist_deletion",
            Some(playlist),
            Some(&found.name),
            &[],
            &[],
        )
    }

    /// Records the undo of an accepted tag or label set, for review like any set. It
    /// puts back the values every written file (or labels every changed track) had
    /// before, provided it still holds what the set wrote. Undoing a label set also
    /// deletes the labels it created that no track holds any more.
    pub fn submit_undo(&self, id: ChangeSetId, agent: &str) -> Result<ChangeSetId> {
        let set = self.change_set(id)?.ok_or(Error::UnknownChangeSet(id))?;
        let labels = set.summary.kind == ChangeKind::Labels;
        let undoable = matches!(set.summary.kind, ChangeKind::Tags | ChangeKind::Labels)
            && matches!(
                set.summary.status,
                ChangeStatus::Accepted | ChangeStatus::PartlyWritten
            );
        let undo_waiting: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM change_sets WHERE undoes = ?1 AND status = 'pending')",
            [id.0],
            |row| row.get(0),
        )?;
        if !undoable || undo_waiting {
            return Err(Error::NotUndoable(id));
        }
        let files: Vec<NewFile> = set
            .files
            .into_iter()
            .filter(|file| file.result == Some(FileResult::Written))
            .map(|file| {
                let mut edits: Vec<(String, Option<Vec<String>>)> = file
                    .values
                    .iter()
                    .map(|value| {
                        let before = (!value.before.is_empty()).then(|| value.before.clone());
                        let target = if labels {
                            value.key.clone()
                        } else {
                            format!("tag:{}", value.key)
                        };
                        (target, before)
                    })
                    .collect();
                if let Some(before) = &file.cover_before {
                    edits.push((
                        "cover".into(),
                        before.as_ref().map(|path| vec![file_name(path)]),
                    ));
                }
                NewFile {
                    track: file.track,
                    path: file.path,
                    edits,
                    values: file
                        .values
                        .into_iter()
                        .map(|value| (value.key, value.after.unwrap_or_default()))
                        .collect(),
                    cover_before: file
                        .cover_after
                        .map(|after| after.map(|path| file_name(&path)).unwrap_or_default()),
                }
            })
            .collect();
        if files.is_empty() {
            return Err(Error::NotUndoable(id));
        }
        let kind = if labels { "labels" } else { "tags" };
        let undo = self.insert_set(agent, kind, None, None, &files, &[])?;
        self.conn.execute(
            "UPDATE change_sets SET undoes = ?2 WHERE id = ?1",
            params![undo.0, id.0],
        )?;
        Ok(undo)
    }

    /// Writes a pending set: each file through the tag writer, after checking it still
    /// holds what the set was made against. Returns the set with each file's result.
    /// `progress` gets (files done, total).
    pub fn accept_change_set(
        &mut self,
        id: ChangeSetId,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<ChangeSet> {
        let set = self.pending_set(id)?;
        if let ChangeKind::PlaylistDeletion { playlist, .. } = set.summary.kind {
            self.delete_playlist(playlist)?;
            self.finish(id, ChangeStatus::Accepted)?;
            return self.change_set(id)?.ok_or(Error::UnknownChangeSet(id));
        }
        if set.summary.kind == ChangeKind::Labels {
            return self.accept_labels(set, progress);
        }
        // An undo puts back the values the nine fields' keys had, raw.
        let raw = set.summary.undoes.is_some();
        let mut written: Vec<(i64, TagEdit)> = Vec::new();
        let mut all_written = true;
        let total = set.files.len();
        for (done, file) in set.files.iter().enumerate() {
            progress(done, total);
            let outcome = self.write_change(file, raw)?;
            let (result, detail) = match &outcome {
                Ok(_) => ("written", None),
                Err(TagWriteError::Conflict(detail)) => ("conflict", Some(detail.clone())),
                Err(error) => ("failed", Some(error.to_string())),
            };
            self.conn.execute(
                "UPDATE change_files SET result = ?3, detail = ?4
                 WHERE set_id = ?1 AND track_id = ?2",
                params![id.0, file.track.0, result, detail],
            )?;
            match outcome {
                Ok(edit) => written.push((file.track.0, spelling(edit))),
                Err(_) => all_written = false,
            }
        }
        progress(total, total);
        if !written.is_empty() {
            let edited: Vec<(i64, &TagEdit)> =
                written.iter().map(|(track, edit)| (*track, edit)).collect();
            scan::reread_edited(&mut self.conn, &self.artwork, &edited)?;
        }
        for (track, _) in &written {
            self.record_after(id, TrackId(*track))?;
        }
        let status = if all_written {
            ChangeStatus::Accepted
        } else {
            ChangeStatus::PartlyWritten
        };
        self.finish(id, status)?;
        if let Some(undone) = set.summary.undoes.filter(|_| all_written) {
            self.conn.execute(
                "UPDATE change_sets SET status = 'undone' WHERE id = ?1",
                [undone.0],
            )?;
        }
        self.change_set(id)?.ok_or(Error::UnknownChangeSet(id))
    }

    pub fn reject_change_set(&self, id: ChangeSetId) -> Result<()> {
        self.pending_set(id)?;
        self.finish(id, ChangeStatus::Rejected)
    }

    /// Marks every set still pending as discarded: VCMP keeps no set waiting across a
    /// restart. Called when the library opens for the app. Returns how many there were.
    pub fn discard_pending_change_sets(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE change_sets SET status = 'discarded', finished_ms = ?1
             WHERE status = 'pending'",
            [now_ms()],
        )?)
    }

    /// Every set, newest first.
    pub fn change_sets(&self) -> Result<Vec<ChangeSetSummary>> {
        let mut statement = self
            .conn
            .prepare(&format!("{SUMMARY_SELECT} ORDER BY s.id DESC"))?;
        let rows = statement.query_map([], summary_from)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn change_set(&self, id: ChangeSetId) -> Result<Option<ChangeSet>> {
        let summary = self
            .conn
            .query_row(
                &format!("{SUMMARY_SELECT} WHERE s.id = ?1"),
                [id.0],
                summary_from,
            )
            .optional()?;
        let Some(summary) = summary else {
            return Ok(None);
        };
        let mut statement = self.conn.prepare(
            "SELECT id, track_id, path, cover_before, cover_after, result, detail
             FROM change_files WHERE set_id = ?1 ORDER BY id",
        )?;
        let rows: Vec<(i64, ChangedFile)> = statement
            .query_map([id.0], |row| {
                let cover = |name: Option<String>| {
                    name.map(|name| (!name.is_empty()).then(|| self.undo_dir().join(name)))
                };
                let result = match (
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ) {
                    (Some(result), detail) => Some(match result.as_str() {
                        "written" => FileResult::Written,
                        "conflict" => FileResult::Conflict(detail.unwrap_or_default()),
                        _ => FileResult::Failed(detail.unwrap_or_default()),
                    }),
                    (None, _) => None,
                };
                Ok((
                    row.get(0)?,
                    ChangedFile {
                        track: TrackId(row.get(1)?),
                        path: path_codec::decode(&row.get::<_, Vec<u8>>(2)?),
                        edits: Vec::new(),
                        values: Vec::new(),
                        cover_before: cover(row.get(3)?),
                        cover_after: cover(row.get(4)?),
                        result,
                    },
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut files = Vec::with_capacity(rows.len());
        for (file_id, mut file) in rows {
            file.edits = self.file_edits(file_id)?;
            file.values = self.file_values(file_id)?;
            files.push(file);
        }
        let mut statement = self.conn.prepare_cached(
            "SELECT name, colour, created FROM change_labels WHERE set_id = ?1 ORDER BY name",
        )?;
        let new_labels = statement
            .query_map([id.0], |row| {
                Ok(ProposedLabel {
                    name: row.get(0)?,
                    colour: row.get(1)?,
                    created: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(ChangeSet {
            summary,
            files,
            new_labels,
        }))
    }

    /// Deletes the sets finished longer ago than [`CHANGE_SETS_KEPT`], with the covers
    /// only they kept. Pending sets are never touched.
    pub fn expire_change_sets(&self) -> Result<()> {
        let cutoff = now_ms() - CHANGE_SETS_KEPT.as_millis() as i64;
        self.conn.execute(
            "DELETE FROM change_sets WHERE status <> 'pending' AND finished_ms < ?1",
            [cutoff],
        )?;
        self.prune_kept_covers()
    }

    /// The bytes the covers kept for finished sets take: what
    /// [`Library::delete_finished_change_sets`] frees on disk, the database rows
    /// aside.
    pub fn finished_change_sets_size(&self) -> Result<u64> {
        let pending = self.kept_covers("WHERE s.status = 'pending'")?;
        let finished = self.kept_covers("WHERE s.status <> 'pending'")?;
        Ok(finished
            .difference(&pending)
            .filter_map(|name| fs::metadata(self.undo_dir().join(name)).ok())
            .map(|metadata| metadata.len())
            .sum())
    }

    /// Deletes every finished set and the covers only they kept, so none of them can
    /// be undone any more. Returns the bytes the covers took.
    pub fn delete_finished_change_sets(&self) -> Result<u64> {
        let size = self.finished_change_sets_size()?;
        self.conn
            .execute("DELETE FROM change_sets WHERE status <> 'pending'", [])?;
        self.prune_kept_covers()?;
        Ok(size)
    }

    fn undo_dir(&self) -> PathBuf {
        self.images.join("undo")
    }

    /// Keeps a cover's bytes under `images/undo/`, named by their hash, and returns the
    /// file name.
    fn keep_cover(&self, data: &[u8]) -> Result<String> {
        let extension = match image::guess_format(data) {
            Ok(image::ImageFormat::Png) => "png",
            Ok(image::ImageFormat::Jpeg) => "jpg",
            _ => "bin",
        };
        let name = format!("{}.{extension}", edit::cover_hash(data));
        let dir = self.undo_dir();
        let path = dir.join(&name);
        if !path.exists() {
            fs::create_dir_all(&dir).map_err(io_error(&dir))?;
            fs::write(&path, data).map_err(io_error(&path))?;
        }
        Ok(name)
    }

    /// The edit's changes by target, its new cover kept as a file.
    fn targets(&self, edit: &TagEdit) -> Result<Vec<(String, Option<Vec<String>>)>> {
        fn target<T: ToString>(
            targets: &mut Vec<(String, Option<Vec<String>>)>,
            name: &str,
            edit: &FieldEdit<T>,
        ) {
            match edit {
                FieldEdit::Keep => {}
                FieldEdit::Set(value) => targets.push((name.into(), Some(vec![value.to_string()]))),
                FieldEdit::Clear => targets.push((name.into(), None)),
            }
        }
        let mut targets = Vec::new();
        target(&mut targets, "title", &edit.title);
        target(&mut targets, "artist", &edit.artist);
        target(&mut targets, "album", &edit.album);
        target(&mut targets, "album_artist", &edit.album_artist);
        target(&mut targets, "track_number", &edit.track_number);
        target(&mut targets, "disc_number", &edit.disc_number);
        target(&mut targets, "year", &edit.year);
        target(&mut targets, "genre", &edit.genre);
        match &edit.cover {
            FieldEdit::Keep => {}
            FieldEdit::Set(data) => {
                targets.push(("cover".into(), Some(vec![self.keep_cover(data)?])))
            }
            FieldEdit::Clear => targets.push(("cover".into(), None)),
        }
        for (key, edit) in &edit.other {
            match edit {
                FieldEdit::Keep => {}
                FieldEdit::Set(values) if !values.is_empty() => {
                    targets.push((format!("tag:{key}"), Some(values.clone())));
                }
                FieldEdit::Set(_) | FieldEdit::Clear => targets.push((format!("tag:{key}"), None)),
            }
        }
        Ok(targets)
    }

    fn insert_set(
        &self,
        agent: &str,
        kind: &str,
        playlist: Option<PlaylistId>,
        playlist_name: Option<&str>,
        files: &[NewFile],
        new_labels: &[(String, String)],
    ) -> Result<ChangeSetId> {
        // Unchecked: &self keeps the app's shared connection usable; nothing else
        // runs a transaction on this connection meanwhile.
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO change_sets (agent, kind, playlist_id, playlist_name, status, created_ms)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
            params![
                agent,
                kind,
                playlist.map(|id| id.0),
                playlist_name,
                now_ms()
            ],
        )?;
        let id = tx.last_insert_rowid();
        for file in files {
            tx.execute(
                "INSERT INTO change_files (set_id, track_id, path, cover_before)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    id,
                    file.track.0,
                    path_codec::encode(&file.path),
                    file.cover_before
                ],
            )?;
            let file_id = tx.last_insert_rowid();
            for (target, values) in &file.edits {
                tx.execute(
                    "INSERT INTO change_edits (file_id, target, vals) VALUES (?1, ?2, ?3)",
                    params![file_id, target, values.as_deref().map(to_json)],
                )?;
            }
            for (key, before) in &file.values {
                tx.execute(
                    "INSERT INTO change_values (file_id, key, before) VALUES (?1, ?2, ?3)",
                    params![file_id, key, to_json(before)],
                )?;
            }
        }
        for (name, colour) in new_labels {
            tx.execute(
                "INSERT INTO change_labels (set_id, name, colour) VALUES (?1, ?2, ?3)",
                params![id, name, colour],
            )?;
        }
        tx.commit()?;
        Ok(ChangeSetId(id))
    }

    fn pending_set(&self, id: ChangeSetId) -> Result<ChangeSet> {
        let set = self.change_set(id)?.ok_or(Error::UnknownChangeSet(id))?;
        if set.summary.status != ChangeStatus::Pending {
            return Err(Error::ChangeSetNotPending(id));
        }
        Ok(set)
    }

    fn finish(&self, id: ChangeSetId, status: ChangeStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE change_sets SET status = ?2, finished_ms = ?3 WHERE id = ?1",
            params![id.0, status.as_str(), now_ms()],
        )?;
        Ok(())
    }

    /// Writes one file of a set. The outer error is the database's; the inner one is
    /// why this file wasn't written.
    fn write_change(
        &self,
        file: &ChangedFile,
        raw: bool,
    ) -> Result<std::result::Result<TagEdit, TagWriteError>> {
        let track = self.track(file.track)?.filter(|track| track.available);
        let Some(track) = track else {
            return Ok(Err(TagWriteError::Unavailable));
        };
        let edit = match edit_from(&file.edits) {
            Ok(edit) => edit,
            Err(reason) => return Ok(Err(TagWriteError::Write(reason))),
        };
        let expected = Expected {
            values: file
                .values
                .iter()
                .map(|value| (value.key.clone(), value.before.clone()))
                .collect(),
            cover: file.cover_before.as_ref().map(|cover| {
                cover
                    .as_ref()
                    .and_then(|path| path.file_stem())
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
        };
        Ok(edit::write(&track.path, &edit, raw, Some(&expected)).map(|()| edit))
    }

    /// Accepts a pending label set: creates its new labels (or takes a label of the
    /// same name that exists by then), then changes each track whose labels are still
    /// as the set found them. A track whose labels changed since, or that would get a
    /// label deleted since, is a conflict and left alone.
    fn accept_labels(
        &self,
        set: ChangeSet,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<ChangeSet> {
        let id = set.summary.id;
        // Unchecked: &self keeps the app's shared connection usable; nothing else
        // runs a transaction on this connection meanwhile.
        let tx = self.conn.unchecked_transaction()?;
        for label in &set.new_labels {
            let created = match self.label_named(&label.name)? {
                Some(_) => false,
                None => {
                    self.create_label(&label.name, &label.colour)?;
                    true
                }
            };
            tx.execute(
                "UPDATE change_labels SET created = ?3 WHERE set_id = ?1 AND name = ?2",
                params![id.0, label.name, created],
            )?;
        }
        let known: HashMap<String, LabelId> = self
            .labels()?
            .into_iter()
            .map(|label| (label.name.to_lowercase(), label.id))
            .collect();
        let find = |key: &str| {
            let name = key.strip_prefix("label:").unwrap_or(key);
            known.get(&name.to_lowercase()).copied()
        };
        let links = self.label_links()?;
        let total = set.files.len();
        let mut all_written = true;
        for (done, file) in set.files.iter().enumerate() {
            progress(done, total);
            let held: HashSet<LabelId> = links
                .get(&file.track)
                .into_iter()
                .flatten()
                .copied()
                .collect();
            let holds = |key: &str| find(key).is_some_and(|label| held.contains(&label));
            let changed = file
                .values
                .iter()
                .any(|value| holds(&value.key) == value.before.is_empty());
            let gone = file
                .edits
                .iter()
                .find(|(target, values)| values.is_some() && find(target).is_none());
            let conflict = if changed {
                Some("its labels changed since the set was made".to_owned())
            } else {
                gone.map(|(target, _)| {
                    let name = target.strip_prefix("label:").unwrap_or(target);
                    format!("the label {name:?} no longer exists")
                })
            };
            let (result, detail) = match &conflict {
                Some(detail) => ("conflict", Some(detail.as_str())),
                None => ("written", None),
            };
            if conflict.is_some() {
                all_written = false;
            } else {
                for (target, values) in &file.edits {
                    let Some(label) = find(target) else { continue };
                    self.set_link(label, file.track, values.is_some())?;
                }
            }
            tx.execute(
                "UPDATE change_files SET result = ?3, detail = ?4
                 WHERE set_id = ?1 AND track_id = ?2",
                params![id.0, file.track.0, result, detail],
            )?;
            if conflict.is_none() {
                let file_id: i64 = tx.query_row(
                    "SELECT id FROM change_files WHERE set_id = ?1 AND track_id = ?2",
                    params![id.0, file.track.0],
                    |row| row.get(0),
                )?;
                for (target, values) in &file.edits {
                    tx.execute(
                        "UPDATE change_values SET after = ?3 WHERE file_id = ?1 AND key = ?2",
                        params![file_id, target, to_json(values.as_deref().unwrap_or(&[]))],
                    )?;
                }
            }
        }
        progress(total, total);
        let status = if all_written {
            ChangeStatus::Accepted
        } else {
            ChangeStatus::PartlyWritten
        };
        tx.execute(
            "UPDATE change_sets SET status = ?2, finished_ms = ?3 WHERE id = ?1",
            params![id.0, status.as_str(), now_ms()],
        )?;
        if let Some(undone) = set.summary.undoes {
            if all_written {
                tx.execute(
                    "UPDATE change_sets SET status = 'undone' WHERE id = ?1",
                    [undone.0],
                )?;
            }
            // The labels the undone set made go with it, unless a track holds them.
            tx.execute(
                "DELETE FROM labels WHERE id IN (
                     SELECT l.id FROM labels l JOIN change_labels c ON c.name = l.name
                     WHERE c.set_id = ?1 AND c.created = 1)
                 AND NOT EXISTS (SELECT 1 FROM track_labels tl WHERE tl.label_id = labels.id)",
                [undone.0],
            )?;
        }
        tx.commit()?;
        self.change_set(id)?.ok_or(Error::UnknownChangeSet(id))
    }

    /// Stores what a written file's tags read back as.
    fn record_after(&self, id: ChangeSetId, track: TrackId) -> Result<()> {
        let file_id: i64 = self.conn.query_row(
            "SELECT id FROM change_files WHERE set_id = ?1 AND track_id = ?2",
            params![id.0, track.0],
            |row| row.get(0),
        )?;
        let entries = self.track_tags(track)?;
        for value in self.file_values(file_id)? {
            let after: Vec<String> = entries
                .iter()
                .filter(|entry| entry.key == value.key)
                .map(|entry| entry.value.clone())
                .collect();
            self.conn.execute(
                "UPDATE change_values SET after = ?3 WHERE file_id = ?1 AND key = ?2",
                params![file_id, value.key, to_json(&after)],
            )?;
        }
        // The cover written is the one asked for, or none.
        let cover: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT vals FROM change_edits WHERE file_id = ?1 AND target = 'cover'",
                [file_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(cover) = cover {
            let name = match cover {
                Some(json) => from_json(&self.conn, &json)?
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
                None => String::new(),
            };
            self.conn.execute(
                "UPDATE change_files SET cover_after = ?2 WHERE id = ?1",
                params![file_id, name],
            )?;
        }
        Ok(())
    }

    fn file_edits(&self, file_id: i64) -> Result<Vec<(String, Option<Vec<String>>)>> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT target, vals FROM change_edits WHERE file_id = ?1")?;
        let rows: Vec<(String, Option<String>)> = statement
            .query_map([file_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut edits = Vec::with_capacity(rows.len());
        for (target, vals) in rows {
            let mut values = vals.map(|json| from_json(&self.conn, &json)).transpose()?;
            if target == "cover"
                && let Some(names) = &mut values
            {
                for name in names.iter_mut() {
                    *name = self.undo_dir().join(&*name).to_string_lossy().into_owned();
                }
            }
            edits.push((target, values));
        }
        // The nine fields in form order, then the other tags by key.
        edits.sort_by_key(|(target, _)| {
            (
                FIELD_ORDER
                    .iter()
                    .position(|field| field == target)
                    .unwrap_or(FIELD_ORDER.len()),
                target.clone(),
            )
        });
        Ok(edits)
    }

    fn file_values(&self, file_id: i64) -> Result<Vec<ValueChange>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT key, before, after FROM change_values WHERE file_id = ?1 ORDER BY key",
        )?;
        let rows: Vec<(String, String, Option<String>)> = statement
            .query_map([file_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(key, before, after)| {
                Ok(ValueChange {
                    key,
                    before: from_json(&self.conn, &before)?,
                    after: after.map(|json| from_json(&self.conn, &json)).transpose()?,
                })
            })
            .collect()
    }

    /// The cover files the sets matching `filter` refer to.
    fn kept_covers(&self, filter: &str) -> Result<HashSet<String>> {
        let mut names = HashSet::new();
        let mut statement = self.conn.prepare(&format!(
            "SELECT f.cover_before, f.cover_after FROM change_files f
             JOIN change_sets s ON s.id = f.set_id {filter}"
        ))?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })?;
        for row in rows {
            let (before, after) = row?;
            names.extend(
                before
                    .into_iter()
                    .chain(after)
                    .filter(|name| !name.is_empty()),
            );
        }
        let mut statement = self.conn.prepare(&format!(
            "SELECT e.vals FROM change_edits e JOIN change_files f ON f.id = e.file_id
             JOIN change_sets s ON s.id = f.set_id {filter}
             {} e.target = 'cover' AND e.vals IS NOT NULL",
            if filter.is_empty() { "WHERE" } else { "AND" }
        ))?;
        let rows: Vec<String> = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for json in rows {
            names.extend(from_json(&self.conn, &json)?);
        }
        Ok(names)
    }

    /// Deletes the kept covers no set refers to any more.
    fn prune_kept_covers(&self) -> Result<()> {
        let dir = self.undo_dir();
        let Ok(files) = fs::read_dir(&dir) else {
            return Ok(());
        };
        let kept = self.kept_covers("")?;
        for file in files.flatten() {
            if !kept.contains(&*file.file_name().to_string_lossy()) {
                let _ = fs::remove_file(file.path());
            }
        }
        Ok(())
    }
}

/// A file of a set about to be recorded.
struct NewFile {
    track: TrackId,
    path: PathBuf,
    edits: Vec<(String, Option<Vec<String>>)>,
    values: Vec<(String, Vec<String>)>,
    cover_before: Option<String>,
}

const FIELD_ORDER: [&str; 9] = [
    "title",
    "artist",
    "album",
    "album_artist",
    "track_number",
    "disc_number",
    "year",
    "genre",
    "cover",
];

const SUMMARY_SELECT: &str =
    "SELECT s.id, s.agent, s.kind, s.playlist_id, s.playlist_name, s.status,
        s.undoes, s.created_ms, s.finished_ms,
        (SELECT COUNT(*) FROM change_files f WHERE f.set_id = s.id)
    FROM change_sets s";

fn summary_from(row: &rusqlite::Row) -> rusqlite::Result<ChangeSetSummary> {
    let kind = match row.get::<_, String>(2)?.as_str() {
        "playlist_deletion" => ChangeKind::PlaylistDeletion {
            playlist: PlaylistId(row.get::<_, Option<i64>>(3)?.unwrap_or_default()),
            name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        },
        "labels" => ChangeKind::Labels,
        _ => ChangeKind::Tags,
    };
    Ok(ChangeSetSummary {
        id: ChangeSetId(row.get(0)?),
        agent: row.get(1)?,
        kind,
        status: ChangeStatus::parse(&row.get::<_, String>(5)?),
        undoes: row.get::<_, Option<i64>>(6)?.map(ChangeSetId),
        created: time(row.get(7)?),
        finished: row.get::<_, Option<i64>>(8)?.map(time),
        file_count: row.get::<_, i64>(9)? as usize,
    })
}

/// The edit a file's targets describe. Fails when a value doesn't parse or a kept
/// cover is gone.
fn edit_from(targets: &[(String, Option<Vec<String>>)]) -> std::result::Result<TagEdit, String> {
    fn text(values: &Option<Vec<String>>) -> FieldEdit<String> {
        match values.as_ref().and_then(|values| values.first()) {
            Some(value) => FieldEdit::Set(value.clone()),
            None => FieldEdit::Clear,
        }
    }
    fn number<T: std::str::FromStr>(
        target: &str,
        values: &Option<Vec<String>>,
    ) -> std::result::Result<FieldEdit<T>, String> {
        match values.as_ref().and_then(|values| values.first()) {
            Some(value) => value
                .parse()
                .map(FieldEdit::Set)
                .map_err(|_| format!("{target} {value:?} is not a number")),
            None => Ok(FieldEdit::Clear),
        }
    }
    let mut edit = TagEdit::default();
    for (target, values) in targets {
        match target.as_str() {
            "title" => edit.title = text(values),
            "artist" => edit.artist = text(values),
            "album" => edit.album = text(values),
            "album_artist" => edit.album_artist = text(values),
            "genre" => edit.genre = text(values),
            "track_number" => edit.track_number = number(target, values)?,
            "disc_number" => edit.disc_number = number(target, values)?,
            "year" => edit.year = number(target, values)?,
            "cover" => {
                edit.cover = match values.as_ref().and_then(|values| values.first()) {
                    Some(path) => FieldEdit::Set(
                        fs::read(path).map_err(|error| format!("the kept cover: {error}"))?,
                    ),
                    None => FieldEdit::Clear,
                }
            }
            _ => {
                let key = target
                    .strip_prefix("tag:")
                    .ok_or_else(|| format!("unknown target {target:?}"))?;
                let edit_values = match values {
                    Some(values) => FieldEdit::Set(values.clone()),
                    None => FieldEdit::Clear,
                };
                edit.other.insert(key.to_owned(), edit_values);
            }
        }
    }
    Ok(edit)
}

/// The edit the database re-read gets, which only uses the album, artist and album
/// artist to keep the spelling written. An undo writes those raw under their keys.
fn spelling(mut edit: TagEdit) -> TagEdit {
    let raw: BTreeMap<String, FieldEdit<Vec<String>>> = std::mem::take(&mut edit.other);
    for (key, field) in [
        ("AlbumTitle", &mut edit.album),
        ("TrackArtist", &mut edit.artist),
        ("AlbumArtist", &mut edit.album_artist),
    ] {
        if let Some(FieldEdit::Set(values)) = raw.get(key)
            && field.is_keep()
        {
            *field = FieldEdit::Set(values.join(", "));
        }
    }
    edit
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A list of strings as a JSON array.
fn to_json(values: &[String]) -> String {
    let mut json = String::from("[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push('"');
        for c in value.chars() {
            match c {
                '"' => json.push_str("\\\""),
                '\\' => json.push_str("\\\\"),
                c if u32::from(c) < 0x20 => json.push_str(&format!("\\u{:04x}", u32::from(c))),
                c => json.push(c),
            }
        }
        json.push('"');
    }
    json.push(']');
    json
}

/// A JSON array of strings, read by SQLite's own JSON functions.
fn from_json(conn: &Connection, json: &str) -> Result<Vec<String>> {
    let mut statement = conn.prepare_cached("SELECT value FROM json_each(?1) ORDER BY key")?;
    Ok(statement
        .query_map([json], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn time(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64)
}
