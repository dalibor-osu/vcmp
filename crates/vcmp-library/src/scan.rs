//! Bringing the database in line with the source directories.
//!
//! A scan walks every source, compares each file's size and modification time with
//! the database, reads tags only for new or changed files, and marks vanished files
//! unavailable instead of deleting them. A new file that matches exactly one
//! unavailable track (same title, artist and album, duration within a second) takes
//! over that track's row, so playlists and history follow a moved file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use walkdir::WalkDir;

use crate::artwork::{self, ArtworkStore};
use crate::edit::{FieldEdit, TagEdit};
use crate::tags::{self, Format, ReadTrack};
use crate::{Result, path_codec};

/// Tolerance when matching a moved file to its old row.
const RELINK_DURATION_TOLERANCE_MS: i64 = 1000;
/// Files written at most per transaction, so a long first scan shows up gradually.
const CHUNK: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanPhase {
    /// Walking the source directories; `total` is not known yet.
    Discovering,
    /// Reading tags of new and changed files.
    Reading,
    /// Looking for album artwork.
    Artwork,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub added: usize,
    pub updated: usize,
    pub relinked: usize,
    /// Tracks whose files were gone in this scan.
    pub became_unavailable: usize,
    /// Unavailable tracks whose unchanged files are back.
    pub became_available: usize,
    pub unchanged: usize,
    pub skipped: Vec<SkippedFile>,
    /// Sources that could not be read (missing directory, unplugged drive).
    pub unreachable_sources: Vec<PathBuf>,
    pub cancelled: bool,
}

#[derive(Debug)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: String,
}

struct Found {
    path: PathBuf,
    source_id: i64,
    size: i64,
    modified_ms: i64,
}

struct ReadJob<'a> {
    file: &'a Found,
    existing: Option<i64>,
    with_cover: bool,
}

/// What a scan walked: the music files by encoded path, and each folder's cover images
/// as `name size modified` lines.
struct Discovered {
    files: HashMap<Vec<u8>, Found>,
    folder_images: HashMap<PathBuf, Vec<String>>,
}

struct Known {
    id: i64,
    size: i64,
    modified_ms: i64,
    available: bool,
    source_id: Option<i64>,
}

pub(crate) fn scan(
    conn: &mut Connection,
    artwork: &ArtworkStore,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(ScanProgress),
) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let Discovered {
        files: found,
        folder_images,
    } = discover(conn, &mut report, cancel, progress)?;
    if cancel.load(Ordering::Relaxed) {
        report.cancelled = true;
        return Ok(report);
    }
    // Albums made from here on are new to this scan.
    let first_new_album: i64 =
        conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM albums", [], |row| {
            row.get(0)
        })?;
    let known = known_tracks(conn)?;

    let mut to_read = Vec::new();
    let tx = conn.transaction()?;
    for file in found.values() {
        match known.get(&path_codec::encode(&file.path)) {
            Some(row) if row.size == file.size && row.modified_ms == file.modified_ms => {
                if row.available {
                    report.unchanged += 1;
                } else {
                    tx.execute("UPDATE tracks SET available = 1 WHERE id = ?1", [row.id])?;
                    report.became_available += 1;
                }
                if row.source_id != Some(file.source_id) {
                    tx.execute(
                        "UPDATE tracks SET source_id = ?1 WHERE id = ?2",
                        [file.source_id, row.id],
                    )?;
                }
            }
            Some(row) => to_read.push((file, Some(row.id))),
            None => to_read.push((file, None)),
        }
    }
    let found_keys: HashSet<Vec<u8>> = found.keys().cloned().collect();
    for (key, row) in &known {
        if row.available && !found_keys.contains(key) {
            tx.execute("UPDATE tracks SET available = 0 WHERE id = ?1", [row.id])?;
            report.became_unavailable += 1;
        }
    }
    tx.commit()?;

    // Stable order makes re-linking and album titles deterministic, and on hard disks
    // it roughly follows the order the files were copied in.
    to_read.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    // The first file of each folder also brings its embedded cover for the album.
    let mut folders = HashSet::new();
    let jobs: Vec<ReadJob> = to_read
        .iter()
        .map(|&(file, existing)| ReadJob {
            file,
            existing,
            with_cover: file
                .path
                .parent()
                .is_some_and(|dir| folders.insert(dir.to_path_buf())),
        })
        .collect();
    let total = jobs.len();

    // One thread reads the files in order, ahead of the database writes. Eight
    // reader threads measured no faster on a hard disk, and one keeps disk access in
    // path order.
    let ((), stored_covers) = with_cover_pool(artwork, |covers| {
        let (read_sender, read_receiver) = mpsc::sync_channel(64);
        thread::scope(|scope| {
            let jobs = &jobs;
            scope.spawn(move || {
                for job in jobs {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = tags::read(&job.file.path, job.with_cover);
                    if read_sender.send(result).is_err() {
                        break;
                    }
                }
            });

            let mut claimed = HashSet::new();
            let mut cover_sent = HashSet::new();
            // The reader sends a result per job, in order, until it is cancelled.
            write_as_received(conn, &read_receiver, |tx, index, result| {
                let job = &jobs[index];
                progress(ScanProgress {
                    phase: ScanPhase::Reading,
                    done: index,
                    total,
                });
                match result {
                    Ok(track) => {
                        let album = apply(
                            tx,
                            job.file,
                            job.existing,
                            &track,
                            &mut claimed,
                            &mut report,
                        )?;
                        // A new album takes the cover of its first file read. One that
                        // had art before is left to the artwork pass, which looks at
                        // its first tracks, so a changed later track can't swap it.
                        if let (Some(album), Some(cover)) = (album, track.cover)
                            && album >= first_new_album
                            && !cover_sent.contains(&album)
                            && album_needs_artwork(tx, album)?
                        {
                            cover_sent.insert(album);
                            let _ = covers.send((album, cover));
                        }
                    }
                    Err(reason) => report.skipped.push(SkippedFile {
                        path: job.file.path.clone(),
                        reason,
                    }),
                }
                Ok(())
            })
        })
    })?;
    save_artwork(conn, &stored_covers)?;
    if cancel.load(Ordering::Relaxed) {
        report.cancelled = true;
        return Ok(report);
    }
    progress(ScanProgress {
        phase: ScanPhase::Reading,
        done: total,
        total,
    });

    recheck_changed_folder_art(conn, &found, &folder_images)?;
    finish_groups(conn)?;
    find_artwork(conn, artwork, cancel, progress)?;
    report.cancelled = cancel.load(Ordering::Relaxed);
    Ok(report)
}

/// Sends the albums of every folder whose cover images were added, replaced or
/// removed since the last scan back to the artwork pass, and records what each
/// folder holds now. A folder seen for the first time is only recorded: its albums
/// are new, or were looked at when their files were.
fn recheck_changed_folder_art(
    conn: &mut Connection,
    found: &HashMap<Vec<u8>, Found>,
    folder_images: &HashMap<PathBuf, Vec<String>>,
) -> Result<()> {
    let mut files_by_folder: HashMap<&Path, Vec<&Found>> = HashMap::new();
    for file in found.values() {
        if let Some(folder) = file.path.parent() {
            files_by_folder.entry(folder).or_default().push(file);
        }
    }
    let stored: HashMap<Vec<u8>, String> = conn
        .prepare("SELECT folder, fingerprint FROM folder_art")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;

    let tx = conn.transaction()?;
    for (folder, files) in &files_by_folder {
        let mut images = folder_images.get(*folder).cloned().unwrap_or_default();
        images.sort();
        let fingerprint = images.join("\n");
        let key = path_codec::encode(folder);
        match stored.get(&key) {
            Some(old) if *old == fingerprint => continue,
            Some(_) => {
                for file in files {
                    tx.execute(
                        "UPDATE albums SET artwork_state = 0
                         WHERE id = (SELECT album_id FROM tracks WHERE path = ?1)",
                        [path_codec::encode(&file.path)],
                    )?;
                }
            }
            None => {}
        }
        tx.execute(
            "INSERT INTO folder_art (folder, fingerprint) VALUES (?1, ?2)
             ON CONFLICT (folder) DO UPDATE SET fingerprint = excluded.fingerprint",
            params![key, fingerprint],
        )?;
    }
    // Folders without music any more.
    let current: HashSet<Vec<u8>> = files_by_folder
        .keys()
        .map(|folder| path_codec::encode(folder))
        .collect();
    for folder in stored.keys().filter(|folder| !current.contains(*folder)) {
        tx.execute("DELETE FROM folder_art WHERE folder = ?1", [folder])?;
    }
    tx.commit()?;
    Ok(())
}

/// Re-reads files whose tags VCMP just wrote, the way a scan reads changed files, so
/// albums, artists, artwork and search follow the edit. A file that can't be read
/// now is left to the next scan, which sees its new modification time.
pub(crate) fn reread_edited(
    conn: &mut Connection,
    artwork: &ArtworkStore,
    edited: &[(i64, &TagEdit)],
) -> Result<()> {
    // The files are read before the transaction, so the write lock isn't held while
    // the disk works (see `write_as_received`).
    let mut read = Vec::with_capacity(edited.len());
    for &(id, edit) in edited {
        let Some((path, source_id, old_album)) = conn
            .query_row(
                "SELECT path, source_id, album_id FROM tracks WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        path_codec::decode(&row.get::<_, Vec<u8>>(0)?),
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()?
        else {
            continue;
        };
        let (Some(source_id), Ok(metadata)) = (source_id, std::fs::metadata(&path)) else {
            continue;
        };
        let track = match tags::read(&path, false) {
            Ok(track) => track,
            Err(reason) => {
                log::warn!(
                    "{}: not re-read after writing tags: {reason}",
                    path.display()
                );
                continue;
            }
        };
        let file = Found {
            path,
            source_id,
            size: metadata.len() as i64,
            modified_ms: metadata.modified().map(millis).unwrap_or(0),
        };
        read.push((id, edit, file, track, old_album));
    }

    let tx = conn.transaction()?;
    let mut albums = HashSet::new();
    for (id, edit, file, track, old_album) in read {
        let new_album = apply(
            &tx,
            &file,
            Some(id),
            &track,
            &mut HashSet::new(),
            &mut ScanReport::default(),
        )?;
        albums.extend(old_album);
        albums.extend(new_album);

        // A cover the user fetched belongs to the album's group key, which a new
        // title or album artist changes: it moves with the edit, unless the album
        // the track joined has one already.
        if let (Some(old_album), Some(new_album)) = (old_album, new_album)
            && old_album != new_album
        {
            tx.execute(
                "INSERT OR IGNORE INTO fetched_covers
                     (group_key, artwork_hash, cover_kind, cover_mbid, fetched_ms)
                 SELECT new.group_key, f.artwork_hash, f.cover_kind, f.cover_mbid, f.fetched_ms
                 FROM albums old JOIN fetched_covers f ON f.group_key = old.group_key,
                      albums new
                 WHERE old.id = ?1 AND new.id = ?2",
                params![old_album, new_album],
            )?;
        }

        // Albums and artists are matched ignoring case, so a scan keeps the spelling
        // it met first. An edit is the user's spelling, so it wins.
        if let (FieldEdit::Set(_), Some(album), Some(title)) =
            (&edit.album, new_album, &track.tags.album)
        {
            tx.execute(
                "UPDATE albums SET title = ?2 WHERE id = ?1",
                params![album, title],
            )?;
        }
        for (changed, name) in [
            (&edit.artist, &track.tags.artist),
            (&edit.album_artist, &track.tags.album_artist),
        ] {
            if let (FieldEdit::Set(_), Some(name)) = (changed, name) {
                tx.execute(
                    "UPDATE artists SET name = ?1 WHERE name = ?1 COLLATE NOCASE AND name <> ?1 COLLATE BINARY",
                    [name],
                )?;
            }
        }
    }
    // The cover may have changed, or the album may have lost the track its artwork
    // came from: look for the artwork of every album involved again.
    for album in albums {
        tx.execute("UPDATE albums SET artwork_state = 0 WHERE id = ?1", [album])?;
    }
    tx.commit()?;
    finish_groups(conn)?;
    find_artwork(conn, artwork, &AtomicBool::new(false), &mut |_| {})
}

fn discover(
    conn: &Connection,
    report: &mut ScanReport,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(ScanProgress),
) -> Result<Discovered> {
    let mut statement = conn.prepare("SELECT id, path, recursive FROM source_dirs ORDER BY id")?;
    let sources: Vec<(i64, PathBuf, bool)> = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                path_codec::decode(&row.get::<_, Vec<u8>>(1)?),
                row.get(2)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;

    let mut found = HashMap::new();
    let mut folder_images: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for (source_id, root, recursive) in sources {
        if !root.is_dir() {
            report.unreachable_sources.push(root);
            continue;
        }
        let walker = WalkDir::new(&root).max_depth(if recursive { usize::MAX } else { 1 });
        for entry in walker.into_iter().filter_map(|entry| entry.ok()) {
            if cancel.load(Ordering::Relaxed) {
                return Ok(Discovered {
                    files: found,
                    folder_images,
                });
            }
            if !entry.file_type().is_file() {
                continue;
            }
            if artwork::is_folder_image(entry.path()) {
                if let (Some(folder), Some(name), Ok(metadata)) = (
                    entry.path().parent(),
                    entry.path().file_name(),
                    entry.metadata(),
                ) {
                    let modified = metadata.modified().map(millis).unwrap_or(0);
                    folder_images
                        .entry(folder.to_path_buf())
                        .or_default()
                        .push(format!(
                            "{} {} {modified}",
                            name.to_string_lossy(),
                            metadata.len()
                        ));
                }
                continue;
            }
            if Format::from_extension(entry.path()).is_none() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let key = path_codec::encode(entry.path());
            // A file under two sources belongs to the first one.
            found.entry(key).or_insert_with(|| Found {
                path: entry.path().to_path_buf(),
                source_id,
                size: metadata.len() as i64,
                modified_ms: metadata.modified().map(millis).unwrap_or(0),
            });
            if found.len().is_multiple_of(500) {
                progress(ScanProgress {
                    phase: ScanPhase::Discovering,
                    done: found.len(),
                    total: 0,
                });
            }
        }
    }
    progress(ScanProgress {
        phase: ScanPhase::Discovering,
        done: found.len(),
        total: 0,
    });
    Ok(Discovered {
        files: found,
        folder_images,
    })
}

fn known_tracks(conn: &Connection) -> Result<HashMap<Vec<u8>, Known>> {
    let mut statement =
        conn.prepare("SELECT path, id, file_size, modified_ms, available, source_id FROM tracks")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            Known {
                id: row.get(1)?,
                size: row.get(2)?,
                modified_ms: row.get(3)?,
                available: row.get(4)?,
                source_id: row.get(5)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Writes one read file: updates its row, re-links it to a vanished row, or inserts it.
/// Returns the file's album.
fn apply(
    tx: &Transaction,
    file: &Found,
    existing: Option<i64>,
    track: &ReadTrack,
    claimed: &mut HashSet<i64>,
    report: &mut ScanReport,
) -> Result<Option<i64>> {
    let tags = &track.tags;
    let title = tags.title.clone().unwrap_or_else(|| {
        file.path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let duration_ms = track.duration.as_millis() as i64;

    let artist_id = tags
        .artist
        .as_deref()
        .map(|name| upsert_artist(tx, name, tags.mb_artist_id.as_deref()))
        .transpose()?;
    // The album belongs to its album artist, or else the track artist.
    let (owner, owner_mbid) = match &tags.album_artist {
        Some(name) => (Some(name.as_str()), tags.mb_album_artist_id.as_deref()),
        None => (tags.artist.as_deref(), tags.mb_artist_id.as_deref()),
    };
    let owner_id = owner
        .map(|name| upsert_artist(tx, name, owner_mbid))
        .transpose()?;
    let album_id = Some(match &tags.album {
        Some(album) => upsert_album(tx, album, owner, owner_id, tags.mb_album_id.as_deref())?,
        // Without an album tag: the owner's Unknown album.
        None => upsert_unknown_album(tx, owner_id)?,
    });

    let id = match existing {
        Some(id) => {
            report.updated += 1;
            id
        }
        None => match relink_candidate(
            tx,
            &title,
            tags.artist.as_deref(),
            tags.album.as_deref(),
            duration_ms,
            claimed,
        )? {
            Some(id) => {
                report.relinked += 1;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO tracks (path, file_size, modified_ms, format, title, duration_ms, added_ms)
                     VALUES (?1, 0, 0, '', '', 0, ?2)",
                    params![path_codec::encode(&file.path), millis(SystemTime::now())],
                )?;
                report.added += 1;
                tx.last_insert_rowid()
            }
        },
    };
    claimed.insert(id);

    tx.execute(
        "UPDATE tracks SET path = ?2, source_id = ?3, file_size = ?4, modified_ms = ?5, available = 1,
             format = ?6, title = ?7, artist = ?8, artist_id = ?9, album = ?10, album_artist = ?11,
             album_id = ?12, track_number = ?13, disc_number = ?14, year = ?15, genre = ?16,
             duration_ms = ?17, mb_recording_id = ?18
         WHERE id = ?1",
        params![
            id,
            path_codec::encode(&file.path),
            file.source_id,
            file.size,
            file.modified_ms,
            track.format.as_str(),
            title,
            tags.artist,
            artist_id,
            tags.album,
            tags.album_artist,
            album_id,
            tags.track_number,
            tags.disc_number,
            tags.year,
            tags.genre,
            duration_ms,
            tags.mb_recording_id,
        ],
    )?;
    tx.execute("DELETE FROM track_tags WHERE track_id = ?1", [id])?;
    let mut insert_tag = tx.prepare_cached(
        "INSERT INTO track_tags (track_id, position, key, value) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (position, entry) in track.entries.iter().enumerate() {
        insert_tag.execute(params![id, position as i64, entry.key, entry.value])?;
    }
    if let Some(album_id) = album_id {
        // New or changed tracks may bring artwork the album did not have, or another
        // program may have replaced or removed the art it has: look again.
        tx.execute(
            "UPDATE albums SET artwork_state = 0 WHERE id = ?1 AND artwork_state != 0",
            [album_id],
        )?;
    }

    let search_artist = [tags.artist.as_deref(), tags.album_artist.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    tx.execute("DELETE FROM track_search WHERE rowid = ?1", [id])?;
    tx.execute(
        "INSERT INTO track_search (rowid, title, artist, album) VALUES (?1, ?2, ?3, ?4)",
        params![
            id,
            title,
            search_artist,
            tags.album.as_deref().unwrap_or_default()
        ],
    )?;
    Ok(album_id)
}

fn relink_candidate(
    tx: &Transaction,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    duration_ms: i64,
    claimed: &HashSet<i64>,
) -> Result<Option<i64>> {
    let mut statement = tx.prepare_cached(
        "SELECT id FROM tracks
         WHERE available = 0
           AND duration_ms BETWEEN ?4 - ?5 AND ?4 + ?5
           AND title = ?1 COLLATE NOCASE
           AND IFNULL(artist, '') = ?2 COLLATE NOCASE
           AND IFNULL(album, '') = ?3 COLLATE NOCASE",
    )?;
    let candidates: Vec<i64> = statement
        .query_map(
            params![
                title,
                artist.unwrap_or_default(),
                album.unwrap_or_default(),
                duration_ms,
                RELINK_DURATION_TOLERANCE_MS
            ],
            |row| row.get(0),
        )?
        .collect::<rusqlite::Result<Vec<i64>>>()?
        .into_iter()
        .filter(|id| !claimed.contains(id))
        .collect();
    // Only an unambiguous match is trusted.
    Ok(match candidates.as_slice() {
        [id] => Some(*id),
        _ => None,
    })
}

fn upsert_artist(tx: &Transaction, name: &str, mbid: Option<&str>) -> Result<i64> {
    let mut statement = tx.prepare_cached(
        "INSERT INTO artists (name, mbid) VALUES (?1, ?2)
         ON CONFLICT (name) DO UPDATE SET mbid = COALESCE(artists.mbid, excluded.mbid)
         RETURNING id",
    )?;
    Ok(statement.query_row(params![name, mbid], |row| row.get(0))?)
}

/// Albums are keyed by MusicBrainz release ID when tagged, otherwise by album artist
/// (or track artist) plus title, compared case-insensitively.
fn upsert_album(
    tx: &Transaction,
    title: &str,
    owner: Option<&str>,
    owner_id: Option<i64>,
    mbid: Option<&str>,
) -> Result<i64> {
    let key = match mbid {
        Some(mbid) => format!("mb:{}", mbid.to_lowercase()),
        None => format!(
            "{}\u{1f}{}",
            owner.unwrap_or_default().to_lowercase(),
            title.to_lowercase()
        ),
    };
    let mut statement = tx.prepare_cached(
        "INSERT INTO albums (group_key, title, artist_id, mbid) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (group_key) DO UPDATE SET artist_id = COALESCE(albums.artist_id, excluded.artist_id)
         RETURNING id",
    )?;
    Ok(statement.query_row(params![key, title, owner_id, mbid], |row| row.get(0))?)
}

/// The "Unknown" album of the artist `owner_id` (or of no artist), which holds the
/// tracks without an album tag. Keyed by the artist's id, so the schema migration
/// that first made them builds the same keys.
fn upsert_unknown_album(tx: &Transaction, owner_id: Option<i64>) -> Result<i64> {
    let key = format!(
        "unknown:{}",
        owner_id.map(|id| id.to_string()).unwrap_or_default()
    );
    let mut statement = tx.prepare_cached(
        "INSERT INTO albums (group_key, title, artist_id, unknown) VALUES (?1, '', ?2, 1)
         ON CONFLICT (group_key) DO UPDATE SET unknown = 1
         RETURNING id",
    )?;
    Ok(statement.query_row(params![key, owner_id], |row| row.get(0))?)
}

/// Removes albums and artists nothing refers to any more, refreshes album years and
/// rebuilds the artist and album search indexes.
fn finish_groups(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(
        "DELETE FROM albums WHERE id NOT IN (SELECT album_id FROM tracks WHERE album_id IS NOT NULL);
         DELETE FROM artists
          WHERE id NOT IN (SELECT artist_id FROM tracks WHERE artist_id IS NOT NULL)
            AND id NOT IN (SELECT artist_id FROM albums WHERE artist_id IS NOT NULL);
         UPDATE albums SET year = (SELECT MIN(year) FROM tracks WHERE album_id = albums.id);
         DELETE FROM artist_search;
         INSERT INTO artist_search (rowid, name) SELECT id, name FROM artists;
         DELETE FROM album_search;
         INSERT INTO album_search (rowid, title, artist)
             SELECT albums.id, albums.title, IFNULL(artists.name, '')
             FROM albums LEFT JOIN artists ON artists.id = albums.artist_id;",
    )?;
    tx.commit()?;
    Ok(())
}

/// Finds artwork for albums that have none yet, or whose cached files were deleted:
/// first a picture embedded in one of the album's files, then a cover image in their
/// folders. Covers the user fetched are not the files' art and are left alone.
fn find_artwork(
    conn: &mut Connection,
    store: &ArtworkStore,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(ScanProgress),
) -> Result<()> {
    let albums: Vec<(i64, Option<String>, i64)> = conn
        .prepare(
            "SELECT id, artwork_hash, artwork_state FROM albums
             WHERE unknown = 0
               AND EXISTS (SELECT 1 FROM tracks WHERE album_id = albums.id AND available = 1)",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let pending: Vec<i64> = albums
        .into_iter()
        .filter(|(_, hash, state)| match state {
            0 => true,
            1 => hash.as_deref().and_then(|key| store.get(key)).is_none(),
            _ => false,
        })
        .map(|(id, _, _)| id)
        .collect();

    let mut work = Vec::with_capacity(pending.len());
    for album_id in pending {
        let files: Vec<PathBuf> = conn
            .prepare_cached(
                "SELECT path FROM tracks WHERE album_id = ?1 AND available = 1
                 ORDER BY disc_number, track_number, path",
            )?
            .query_map([album_id], |row| {
                Ok(path_codec::decode(&row.get::<_, Vec<u8>>(0)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        work.push((album_id, files));
    }

    let total = work.len();
    let mut without_artwork = Vec::new();
    let ((), found) = with_cover_pool(store, |covers| {
        for (done, (album_id, files)) in work.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            progress(ScanProgress {
                phase: ScanPhase::Artwork,
                done,
                total,
            });
            match artwork_candidate(files) {
                Some(data) => {
                    let _ = covers.send((*album_id, data));
                }
                None => without_artwork.push(*album_id),
            }
        }
        Ok(())
    })?;
    save_artwork(conn, &found)?;
    // Albums checked without success are not checked again until a track or a
    // folder image changes. They lose art their files no longer have, so a fetched
    // cover shows instead.
    let failed = found
        .iter()
        .filter(|(_, key)| key.is_none())
        .map(|(id, _)| *id);
    let tx = conn.transaction()?;
    for album_id in without_artwork.iter().copied().chain(failed) {
        tx.execute(
            "UPDATE albums SET artwork_state = 2, artwork_hash = NULL WHERE id = ?1",
            [album_id],
        )?;
    }
    tx.commit()?;
    progress(ScanProgress {
        phase: ScanPhase::Artwork,
        done: total,
        total,
    });
    Ok(())
}

fn album_needs_artwork(tx: &Transaction, album: i64) -> Result<bool> {
    let state: i64 = tx
        .prepare_cached("SELECT artwork_state FROM albums WHERE id = ?1")?
        .query_row([album], |row| row.get(0))?;
    Ok(state == 0)
}

/// Records found artwork; albums without a key are left for the artwork pass.
fn save_artwork(conn: &mut Connection, found: &[(i64, Option<String>)]) -> Result<()> {
    let tx = conn.transaction()?;
    for (album_id, key) in found {
        if let Some(key) = key {
            tx.execute(
                "UPDATE albums SET artwork_hash = ?2, artwork_state = 1 WHERE id = ?1",
                params![album_id, key],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Stores an image; a cache write failure costs the album its artwork, not the scan.
fn store_artwork(store: &ArtworkStore, data: &[u8]) -> Option<String> {
    store.store(data).unwrap_or_else(|error| {
        log::warn!("could not cache artwork: {error}");
        None
    })
}

/// The image bytes to use for an album: a picture embedded in one of its first two
/// tracks, otherwise a cover image in its folders. Only reads files; decoding happens
/// in the cover pool.
fn artwork_candidate(files: &[PathBuf]) -> Option<Vec<u8>> {
    let is_image = |data: &[u8]| image::guess_format(data).is_ok();
    // Albums rarely embed art in only some files, so two tracks are enough to try.
    for file in files.iter().take(2) {
        if let Some(data) = tags::embedded_cover(file)
            && is_image(&data)
        {
            return Some(data);
        }
    }
    let mut seen = HashSet::new();
    let folders = files
        .iter()
        .filter_map(|file| file.parent())
        .filter(|dir| seen.insert(*dir));
    for folder in folders {
        if let Some(image) = artwork::folder_image(folder)
            && let Ok(data) = std::fs::read(&image)
            && is_image(&data)
        {
            return Some(data);
        }
    }
    None
}

/// Writes each result as it arrives, at most `CHUNK` to a transaction, until the
/// sender is gone. The write lock is held only while results already here are
/// written: before waiting for the next one, the transaction is committed. Held while
/// a hard disk read the files, it kept the window's writes (a setting, a playlist)
/// waiting past their busy timeout, and they failed.
fn write_as_received<T>(
    conn: &Connection,
    results: &mpsc::Receiver<T>,
    mut write: impl FnMut(&Transaction, usize, T) -> Result<()>,
) -> Result<()> {
    let mut open: Option<Transaction> = None;
    let mut index = 0;
    loop {
        let next = match results.try_recv() {
            Ok(next) => next,
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {
                if let Some(tx) = open.take() {
                    tx.commit()?;
                }
                let Ok(next) = results.recv() else { break };
                next
            }
        };
        let tx = match open.take() {
            Some(tx) => tx,
            // Unchecked: no other transaction is open on this connection meanwhile.
            None => conn.unchecked_transaction()?,
        };
        write(&tx, index, next)?;
        index += 1;
        if index % CHUNK == 0 {
            tx.commit()?;
        } else {
            open = Some(tx);
        }
    }
    if let Some(tx) = open {
        tx.commit()?;
    }
    Ok(())
}

/// Threads for image work (decoding, shrinking, encoding covers).
fn worker_count() -> usize {
    thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 8))
}

type StoredCovers = Vec<(i64, Option<String>)>;

/// Runs `produce` on this thread while a pool of threads decodes, shrinks and caches
/// the covers it sends as `(album, image bytes)`. Disk reads stay with `produce`, in
/// order; only CPU work is spread out. Returns `produce`'s result and every album's
/// artwork key (`None` when the image could not be used).
fn with_cover_pool<R>(
    store: &ArtworkStore,
    produce: impl FnOnce(&mpsc::Sender<(i64, Vec<u8>)>) -> Result<R>,
) -> Result<(R, StoredCovers)> {
    let (sender, receiver) = mpsc::channel::<(i64, Vec<u8>)>();
    let receiver = Mutex::new(receiver);
    thread::scope(|scope| {
        let (stored_sender, stored_receiver) = mpsc::channel();
        for _ in 0..worker_count() {
            let (queue, stored) = (&receiver, stored_sender.clone());
            scope.spawn(move || {
                loop {
                    let next = queue.lock().expect("cover queue lock").recv();
                    let Ok((album, data)) = next else { break };
                    let _ = stored.send((album, store_artwork(store, &data)));
                }
            });
        }
        drop(stored_sender);
        let result = produce(&sender);
        // Closing the queue lets the pool finish, also when `produce` failed.
        drop(sender);
        Ok((result?, stored_receiver.iter().collect()))
    })
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}

/// Track id of a path, for tests and callers that hold a path.
pub(crate) fn track_id_for_path(conn: &Connection, path: &Path) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM tracks WHERE path = ?1",
            [path_codec::encode(path)],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// While the scan waits for the next file to be read, another connection can
    /// write: the window's settings and playlists don't wait for the disk.
    #[test]
    fn waiting_for_a_file_holds_no_write_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite3");
        let open = || {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            conn
        };
        let scanner = open();
        scanner
            .execute("CREATE TABLE written (value INTEGER)", [])
            .unwrap();
        let window = open();
        // Long enough for a commit, short enough to fail fast on a lock held until
        // the next file comes.
        window.busy_timeout(Duration::from_secs(2)).unwrap();

        let (send, receive) = mpsc::channel();
        let (wrote, written) = mpsc::channel();
        let scan = thread::spawn(move || {
            write_as_received(&scanner, &receive, |tx, _, value: i64| {
                tx.execute("INSERT INTO written (value) VALUES (?1)", [value])?;
                wrote.send(()).unwrap();
                Ok(())
            })
        });
        send.send(1).unwrap();
        written.recv().unwrap();
        window
            .execute("INSERT INTO written (value) VALUES (2)", [])
            .expect("the window writes while the scan waits for a file");
        send.send(3).unwrap();
        drop(send);
        scan.join().unwrap().unwrap();

        let values: Vec<i64> = window
            .prepare("SELECT value FROM written ORDER BY rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(values, [1, 2, 3]);
    }
}
