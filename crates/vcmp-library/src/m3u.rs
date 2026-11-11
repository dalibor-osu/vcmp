//! M3U files: a playlist exported as a UTF-8 `.m3u8` file that other players, phones
//! and backups can read, and a playlist imported from one.
//!
//! The exported file starts with `#EXTM3U`, and each entry is an `#EXTINF` line
//! (length in whole seconds, "Artist - Title") followed by the file's path. A path
//! is relative to the playlist file's folder, written with `/`, when both are on the
//! same drive: the same filesystem on Linux and macOS, the same drive letter or
//! network share on Windows. Otherwise it is absolute, in the platform's own form.
//!
//! Entries whose file is gone are written with the path the library last saw, so
//! the file is complete again once the file is back. A path that can't be written
//! in a UTF-8 text file of lines (not valid UTF-8, or holding a line break) is left
//! out and reported.
//!
//! An imported file is read as UTF-8, or as Windows-1252 when its bytes aren't
//! UTF-8. Each entry is matched to a library track by its path (a `file:` URL, or a
//! path absolute or relative to the playlist file, with `/` or `\`), and failing
//! that by its `#EXTINF` artist and title, ignoring case, diacritics and extra
//! spaces, the closest length winning. The matched tracks become a new playlist
//! named after the file; the rest are reported.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::error::io_error;
use crate::{Error, Library, PlaylistId, Result, Track, TrackId};

/// What [`Library::export_playlist`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct M3uExport {
    /// Entries in the file, in playlist order.
    pub written: usize,
    /// Of those, entries whose file the library can't find now. They point at the
    /// file's last known path.
    pub missing: usize,
    /// Paths left out of the file, because it can't hold them.
    pub left_out: Vec<PathBuf>,
}

/// What [`Library::import_playlist`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct M3uImport {
    /// The playlist made, or `None` when no entry matched a library track.
    pub playlist: Option<PlaylistId>,
    /// The name the playlist got; empty without one.
    pub name: String,
    /// Entries in the file.
    pub entries: usize,
    /// Entries that matched no library track, in file order. The others are in the
    /// playlist, in file order, repeats included.
    pub unmatched: Vec<M3uEntry>,
}

/// One entry of an M3U file, as the file wrote it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct M3uEntry {
    /// The path or URL.
    pub location: String,
    /// The text of its `#EXTINF` line after the length, normally "Artist - Title".
    pub title: Option<String>,
    /// The length in seconds from its `#EXTINF` line, when known.
    pub seconds: Option<f64>,
}

/// The file name an export of the playlist called `name` suggests: the name with
/// the characters Windows, macOS or Linux don't allow in a file name replaced, and
/// `.m3u8`.
pub fn m3u_file_name(name: &str) -> String {
    let stem: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows drops trailing dots and spaces, and a name of dots alone is a folder.
    let stem = stem.trim_end_matches(['.', ' ']);
    let stem = if stem.is_empty() { "Playlist" } else { stem };
    format!("{stem}.m3u8")
}

impl Library {
    /// Writes the playlist's entries, unavailable ones included, to `file` as an
    /// M3U file (see the module's documentation), replacing a file already there.
    pub fn export_playlist(&self, id: PlaylistId, file: &Path) -> Result<M3uExport> {
        let exists = self
            .conn
            .query_row("SELECT 1 FROM playlists WHERE id = ?1", [id.0], |_| Ok(()))
            .optional()?;
        if exists.is_none() {
            return Err(Error::UnknownPlaylist(id));
        }
        let tracks: Vec<Track> = self
            .playlist_entries(id)?
            .into_iter()
            .map(|entry| entry.track)
            .collect();
        write(file, &tracks)
    }

    /// Writes the favourites, newest first and unavailable ones included, to `file`
    /// as an M3U file, like [`Library::export_playlist`].
    pub fn export_favourites(&self, file: &Path) -> Result<M3uExport> {
        let tracks: Vec<Track> = self
            .favourites()?
            .into_iter()
            .map(|favourite| favourite.track)
            .collect();
        write(file, &tracks)
    }

    /// Makes a playlist of the library's tracks an M3U file lists (see the module's
    /// documentation), named after the file, or `fallback_name` when the file's name
    /// is blank, with a number added when another playlist has that name. Nothing is
    /// made when no entry matches.
    pub fn import_playlist(&self, file: &Path, fallback_name: &str) -> Result<M3uImport> {
        let bytes = fs::read(file).map_err(io_error(file))?;
        let entries = parse(&decode(&bytes));
        let folder = match file.parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder,
            _ => Path::new("."),
        };
        let folder = dunce::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        let mut index: Option<TagIndex> = None;
        let mut tracks = Vec::new();
        let mut unmatched = Vec::new();
        for entry in &entries {
            let mut track = self.track_at(&folder, &entry.location)?;
            if track.is_none()
                && let Some(title) = &entry.title
            {
                if index.is_none() {
                    index = Some(TagIndex::load(&self.conn)?);
                }
                track = index
                    .as_ref()
                    .and_then(|index| index.find(title, entry.seconds));
            }
            match track {
                Some(track) => tracks.push(track),
                None => unmatched.push(entry.clone()),
            }
        }
        let mut report = M3uImport {
            entries: entries.len(),
            unmatched,
            ..M3uImport::default()
        };
        if !tracks.is_empty() {
            let (id, name) =
                self.create_filled_playlist(&playlist_name(file, fallback_name), &tracks)?;
            report.playlist = Some(id);
            report.name = name;
        }
        Ok(report)
    }

    /// The library's track at the path an entry names, available or not.
    fn track_at(&self, folder: &Path, location: &str) -> Result<Option<TrackId>> {
        let Some(path) = local_path(folder, location) else {
            return Ok(None);
        };
        if !on_this_computer(&path) && !self.under_a_source(&path)? {
            return Ok(None);
        }
        if let Some(track) = self.track_id_for_path(&path)? {
            return Ok(Some(track));
        }
        // The same file spelled another way: in other letter case on Windows, or
        // through a link.
        match dunce::canonicalize(&path) {
            Ok(real) if real != path => self.track_id_for_path(&real),
            _ => Ok(None),
        }
    }
}

impl Library {
    fn under_a_source(&self, path: &Path) -> Result<bool> {
        let path = path.to_string_lossy().to_lowercase();
        Ok(self.sources()?.iter().any(|source| {
            let source = source.path.to_string_lossy().to_lowercase();
            path.strip_prefix(&source)
                .is_some_and(|rest| rest.starts_with(['\\', '/']))
        }))
    }
}

fn on_this_computer(path: &Path) -> bool {
    use std::path::Prefix;
    match path.components().next() {
        Some(Component::Prefix(prefix)) => {
            matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        }
        _ => true,
    }
}

/// An M3U file's text: UTF-8 without its byte order mark, or Windows-1252 when the
/// bytes aren't UTF-8.
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&byte| windows_1252(byte)).collect(),
    }
}

/// A Windows-1252 byte as a character. The five bytes the code page leaves undefined
/// become the control characters of the same number, as browsers read them.
fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}',
        '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}',
        '\u{2014}', '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}',
        '\u{178}',
    ];
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        byte => char::from(byte),
    }
}

/// The entries of an M3U file's text, in order: each line that isn't blank and
/// doesn't start with `#`, with the length and title of the `#EXTINF` line before
/// it. Other `#` lines are skipped.
fn parse(text: &str) -> Vec<M3uEntry> {
    let mut entries = Vec::new();
    let mut info = None;
    for line in text.split(['\r', '\n']) {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            info = Some(extinf(rest));
        } else if !line.is_empty() && !line.starts_with('#') {
            let (seconds, title) = info.take().unwrap_or_default();
            entries.push(M3uEntry {
                location: line.to_owned(),
                title,
                seconds,
            });
        }
    }
    entries
}

/// The length and title of an `#EXTINF` line, given what follows `#EXTINF:`. The
/// length ends at the first space or comma (some players put attributes after it),
/// the title follows the first comma. A negative length is unknown.
fn extinf(info: &str) -> (Option<f64>, Option<String>) {
    let (head, title) = match info.split_once(',') {
        Some((head, title)) => (head, Some(title.trim())),
        None => (info, None),
    };
    let seconds = head
        .split_whitespace()
        .next()
        .and_then(|length| length.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
    let title = title.filter(|title| !title.is_empty()).map(str::to_owned);
    (seconds, title)
}

/// The local path an entry's location names: a `file:` URL, or a path, absolute or
/// relative to `folder`, with `/` or `\` between its parts. `None` for other URLs.
fn local_path(folder: &Path, location: &str) -> Option<PathBuf> {
    if let Some((scheme, rest)) = location.split_once(':')
        && scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        if scheme.eq_ignore_ascii_case("file") {
            let path = url::Url::parse(location).ok()?.to_file_path().ok()?;
            return Some(lexical(&path));
        }
        if rest.starts_with("//") {
            return None;
        }
    }
    let path = if cfg!(windows) {
        location.replace('/', "\\")
    } else {
        location.replace('\\', "/")
    };
    Some(lexical(&folder.join(path)))
}

/// `path` with its `.` and `..` parts resolved, without asking the filesystem.
fn lexical(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            component => resolved.push(component),
        }
    }
    resolved
}

/// The name an imported playlist asks for: the file's name without its extension,
/// or `fallback` when that is blank.
fn playlist_name(file: &Path, fallback: &str) -> String {
    let stem = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_owned())
        .unwrap_or_default();
    if stem.is_empty() {
        fallback.trim().to_owned()
    } else {
        stem
    }
}

/// Text as tag matching compares it: lowercase, without diacritics, with one space
/// between words.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !folded.is_empty() {
            folded.push(' ');
        }
        folded.extend(word.to_lowercase().nfd().filter(|c| !is_combining_mark(*c)));
    }
    folded
}

/// The library's tracks by folded artist and title, and by folded title alone, each
/// list in library order.
struct TagIndex {
    by_artist_and_title: HashMap<(String, String), Vec<Candidate>>,
    by_title: HashMap<String, Vec<Candidate>>,
}

#[derive(Clone, Copy)]
struct Candidate {
    track: TrackId,
    millis: i64,
    available: bool,
}

impl TagIndex {
    fn load(conn: &Connection) -> Result<Self> {
        let mut index = Self {
            by_artist_and_title: HashMap::new(),
            by_title: HashMap::new(),
        };
        let mut statement = conn
            .prepare("SELECT id, artist, title, duration_ms, available FROM tracks ORDER BY id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let candidate = Candidate {
                track: TrackId(row.get(0)?),
                millis: row.get(3)?,
                available: row.get(4)?,
            };
            let artist = row.get::<_, Option<String>>(1)?.map(|artist| fold(&artist));
            let title = fold(&row.get::<_, String>(2)?);
            if let Some(artist) = artist.filter(|artist| !artist.is_empty()) {
                index
                    .by_artist_and_title
                    .entry((artist, title.clone()))
                    .or_default()
                    .push(candidate);
            }
            index.by_title.entry(title).or_default().push(candidate);
        }
        Ok(index)
    }

    /// The track an `#EXTINF` title names: "Artist - Title" split at the first
    /// " - ", or failing that the whole text as a title. Of several, the one whose
    /// length is closest to `seconds` wins, then an available one, then the first in
    /// the library.
    fn find(&self, text: &str, seconds: Option<f64>) -> Option<TrackId> {
        let candidates = text
            .split_once(" - ")
            .and_then(|(artist, title)| self.by_artist_and_title.get(&(fold(artist), fold(title))))
            .or_else(|| self.by_title.get(&fold(text)))?;
        let wanted = seconds.map(|seconds| (seconds * 1000.0).round() as i64);
        candidates
            .iter()
            .min_by_key(|candidate| {
                let distance = wanted.map_or(0, |wanted| candidate.millis.abs_diff(wanted));
                (distance, !candidate.available)
            })
            .map(|candidate| candidate.track)
    }
}

fn write(file: &Path, tracks: &[Track]) -> Result<M3uExport> {
    let folder = match file.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    // Track paths have their folders' symlinks resolved, so the playlist's folder
    // is resolved too before paths are made relative to it.
    let folder = dunce::canonicalize(folder).map_err(io_error(folder))?;
    let (text, report) = contents(&folder, tracks, |track| same_root(&folder, track));
    fs::write(file, text).map_err(io_error(file))?;
    Ok(report)
}

/// The file's text for a playlist file in `folder`; `same_root` tells whether a
/// track's file is on the folder's drive.
fn contents(
    folder: &Path,
    tracks: &[Track],
    same_root: impl Fn(&Path) -> bool,
) -> (String, M3uExport) {
    let mut text = String::from("#EXTM3U\n");
    let mut report = M3uExport::default();
    for track in tracks {
        let Some(path) = entry_path(folder, &track.path, same_root(&track.path)) else {
            report.left_out.push(track.path.clone());
            continue;
        };
        let seconds = match track.duration.as_secs_f64().round() as i64 {
            0 => -1,
            seconds => seconds,
        };
        let title = match track.artist.as_deref().filter(|artist| !artist.is_empty()) {
            Some(artist) => format!("{artist} - {}", track.title),
            None => track.title.clone(),
        };
        let title = title.replace(['\r', '\n'], " ");
        text.push_str(&format!("#EXTINF:{seconds},{title}\n{path}\n"));
        report.written += 1;
        if !track.available {
            report.missing += 1;
        }
    }
    (text, report)
}

/// The track's path as the file holds it, or `None` when a line can't hold it.
fn entry_path(folder: &Path, track: &Path, same_root: bool) -> Option<String> {
    let absolute = track.to_str()?;
    if absolute.contains(['\r', '\n']) {
        return None;
    }
    let relative = if same_root {
        relative(folder, track)
    } else {
        None
    };
    Some(relative.unwrap_or_else(|| absolute.to_owned()))
}

/// `track` relative to `folder`, both absolute, with `/` between the parts.
fn relative(folder: &Path, track: &Path) -> Option<String> {
    if !folder.is_absolute() || !track.is_absolute() {
        return None;
    }
    let folder: Vec<Component> = folder.components().collect();
    let track: Vec<Component> = track.components().collect();
    let shared = folder
        .iter()
        .zip(&track)
        .take_while(|(a, b)| a == b)
        .count();
    if shared == 0 {
        return None;
    }
    let mut parts: Vec<&str> = vec![".."; folder.len() - shared];
    for part in &track[shared..] {
        parts.push(part.as_os_str().to_str()?);
    }
    let joined = parts.join("/");
    Some(if joined.starts_with('#') {
        format!("./{joined}")
    } else {
        joined
    })
}

/// Whether `track` is on the same filesystem as `folder`. A missing file counts
/// from its nearest folder that exists.
#[cfg(unix)]
fn same_root(folder: &Path, track: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let device = |path: &Path| {
        path.ancestors()
            .find_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.dev())
    };
    matches!((device(folder), device(track)), (Some(a), Some(b)) if a == b)
}

/// Whether `track` has the same drive letter or network share as `folder`.
#[cfg(not(unix))]
fn same_root(folder: &Path, track: &Path) -> bool {
    let prefix = |path: &Path| match path.components().next() {
        Some(Component::Prefix(prefix)) => {
            Some(prefix.as_os_str().to_string_lossy().to_lowercase())
        }
        _ => None,
    };
    prefix(folder) == prefix(track)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::TrackId;

    fn track(path: &str, artist: Option<&str>, title: &str, seconds: f64) -> Track {
        Track {
            id: TrackId(1),
            path: PathBuf::from(path),
            available: true,
            format: None,
            title: title.to_owned(),
            artist: artist.map(str::to_owned),
            artist_id: None,
            album: None,
            album_id: None,
            album_artist: None,
            track_number: None,
            disc_number: None,
            year: None,
            genre: None,
            duration: Duration::from_secs_f64(seconds),
            added: std::time::UNIX_EPOCH,
        }
    }

    #[cfg(unix)]
    #[test]
    fn writes_relative_paths_on_the_same_drive_and_absolute_ones_elsewhere() {
        let tracks = [
            track("/music/Band/01.flac", Some("Band"), "One", 200.4),
            track("/music/Band/02.flac", Some("Band"), "Two", 61.6),
            track("/elsewhere/03.flac", None, "Three", 0.0),
        ];
        let (text, report) = contents(Path::new("/music/Lists"), &tracks, |path| {
            path.starts_with("/music")
        });
        assert_eq!(
            text,
            "#EXTM3U\n\
             #EXTINF:200,Band - One\n../Band/01.flac\n\
             #EXTINF:62,Band - Two\n../Band/02.flac\n\
             #EXTINF:-1,Three\n/elsewhere/03.flac\n"
        );
        assert_eq!(report.written, 3);
        assert_eq!(report.missing, 0);
    }

    #[cfg(windows)]
    #[test]
    fn writes_absolute_paths_on_another_drive() {
        let tracks = [
            track(r"C:\Music\Band\01.flac", Some("Band"), "One", 1.0),
            track(r"D:\Music\02.flac", Some("Band"), "Two", 1.0),
        ];
        let folder = Path::new(r"C:\Music\Lists");
        let (text, _) = contents(folder, &tracks, |path| same_root(folder, path));
        assert_eq!(
            text,
            "#EXTM3U\n\
             #EXTINF:1,Band - One\n../Band/01.flac\n\
             #EXTINF:1,Band - Two\nD:\\Music\\02.flac\n"
        );
    }

    #[test]
    fn a_relative_path_starting_with_a_hash_is_not_a_comment() {
        let folder = std::env::temp_dir();
        let path = folder.join("#1 Hits").join("a.flac");
        let tracks = [track(path.to_str().unwrap(), None, "A", 1.0)];
        let (text, _) = contents(&folder, &tracks, |_| true);
        assert_eq!(text, "#EXTM3U\n#EXTINF:1,A\n./#1 Hits/a.flac\n");
        assert_eq!(local_path(&folder, "./#1 Hits/a.flac"), Some(path));
    }

    #[cfg(windows)]
    #[test]
    fn network_and_device_paths_are_not_this_computers() {
        assert!(on_this_computer(Path::new(r"C:\Music\a.flac")));
        assert!(!on_this_computer(Path::new(r"\\host\share\a.flac")));
        assert!(!on_this_computer(Path::new(r"\\.\PhysicalDrive0")));
        let folder = Path::new(r"C:\Music");
        assert!(
            local_path(folder, "file://host/share/a.flac")
                .is_none_or(|path| !on_this_computer(&path))
        );
    }

    #[test]
    fn a_line_break_in_a_title_becomes_a_space() {
        let folder = std::env::temp_dir();
        let path = folder.join("a.flac");
        let tracks = [track(path.to_str().unwrap(), Some("A\nB"), "C\r\nD", 1.0)];
        let (text, _) = contents(&folder, &tracks, |_| true);
        assert_eq!(text, "#EXTM3U\n#EXTINF:1,A B - C  D\na.flac\n");
    }

    #[cfg(unix)]
    #[test]
    fn leaves_out_paths_a_line_cannot_hold() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let mut not_utf8 = track("/music/x", None, "Café", 1.0);
        not_utf8.path = PathBuf::from(OsStr::from_bytes(b"/music/caf\xe9.flac"));
        let tracks = [
            not_utf8,
            track("/music/two\nlines.flac", None, "Two", 1.0),
            track("/music/ok.flac", None, "Ok", 1.0),
        ];
        let (text, report) = contents(Path::new("/music"), &tracks, |_| true);
        assert_eq!(text, "#EXTM3U\n#EXTINF:1,Ok\nok.flac\n");
        assert_eq!(report.written, 1);
        assert_eq!(report.left_out.len(), 2);
    }

    #[test]
    fn suggests_a_file_name_every_system_allows() {
        assert_eq!(m3u_file_name("Road trip"), "Road trip.m3u8");
        assert_eq!(m3u_file_name("AC/DC: best?"), "AC_DC_ best_.m3u8");
        assert_eq!(m3u_file_name("Dvořák "), "Dvořák.m3u8");
        assert_eq!(m3u_file_name("..."), "Playlist.m3u8");
        assert_eq!(m3u_file_name("  "), "Playlist.m3u8");
    }

    #[test]
    fn reads_utf8_without_its_byte_order_mark_and_anything_else_as_windows_1252() {
        assert_eq!(decode("Dvořák\n".as_bytes()), "Dvořák\n");
        assert_eq!(decode(b"\xEF\xBB\xBF#EXTM3U\n"), "#EXTM3U\n");
        assert_eq!(
            decode(b"Caf\xe9 \x80 \x96 \x81"),
            "Café \u{20AC} \u{2013} \u{81}"
        );
    }

    #[test]
    fn entries_take_the_extinf_line_before_them() {
        let text = "#EXTM3U\r\n\
                    #EXTINF:200,Band - One\r\n\
                    Band/01.flac\r\n\
                    \r\n\
                    # a comment\n\
                    #EXTGRP:Rock\n\
                    two.mp3\n\
                    #EXTINF:-1,Three\n\
                    #EXTINF:61.6 tvg-id=\"x\",Band - Four\n\
                    four.ogg";
        assert_eq!(
            parse(text),
            [
                M3uEntry {
                    location: "Band/01.flac".into(),
                    title: Some("Band - One".into()),
                    seconds: Some(200.0),
                },
                M3uEntry {
                    location: "two.mp3".into(),
                    title: None,
                    seconds: None,
                },
                M3uEntry {
                    location: "four.ogg".into(),
                    title: Some("Band - Four".into()),
                    seconds: Some(61.6),
                },
            ]
        );
        assert_eq!(extinf("-1,Three"), (None, Some("Three".into())));
        assert_eq!(extinf("12"), (Some(12.0), None));
        assert_eq!(extinf("x, "), (None, None));
    }

    #[test]
    fn relative_paths_resolve_against_the_folder_with_either_separator() {
        let folder = std::env::temp_dir().join("Lists");
        let music = std::env::temp_dir()
            .join("music")
            .join("Band")
            .join("01.flac");
        assert_eq!(
            local_path(&folder, "../music/Band/01.flac").as_deref(),
            Some(music.as_path())
        );
        assert_eq!(
            local_path(&folder, r"..\music\.\Band\01.flac").as_deref(),
            Some(music.as_path())
        );
        assert_eq!(
            local_path(&folder, "Band: Live.flac").as_deref(),
            Some(folder.join("Band: Live.flac").as_path())
        );
        assert_eq!(local_path(&folder, "http://example.com/a.mp3"), None);
        assert_eq!(
            local_path(&folder, "content://media/external/audio/1"),
            None
        );
    }

    #[test]
    fn file_urls_are_decoded() {
        let folder = std::env::temp_dir();
        let path = folder.join("Band").join("01 One.flac");
        let url = url::Url::from_file_path(&path).unwrap();
        assert!(url.as_str().contains("01%20One.flac"), "{url}");
        assert_eq!(
            local_path(Path::new("elsewhere"), url.as_str()).as_deref(),
            Some(path.as_path())
        );
    }

    #[cfg(windows)]
    #[test]
    fn absolute_windows_paths_stay_absolute() {
        let folder = Path::new(r"C:\Music\Lists");
        assert_eq!(
            local_path(folder, "D:/Music/01.flac").as_deref(),
            Some(Path::new(r"D:\Music\01.flac"))
        );
        assert_eq!(
            local_path(folder, r"\Music\01.flac").as_deref(),
            Some(Path::new(r"C:\Music\01.flac"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn absolute_unix_paths_stay_absolute() {
        assert_eq!(
            local_path(Path::new("/music/Lists"), "/elsewhere/01.flac").as_deref(),
            Some(Path::new("/elsewhere/01.flac"))
        );
    }

    #[test]
    fn folding_ignores_case_diacritics_and_extra_spaces() {
        assert_eq!(fold("  Antonín   DVOŘÁK "), "antonin dvorak");
        assert_eq!(fold("Sigur Rós"), fold("sigur ros"));
        assert_ne!(fold("One"), fold("Ones"));
    }

    #[test]
    fn tags_pick_the_closest_length_then_an_available_track() {
        let candidate = |id, seconds: i64, available| Candidate {
            track: TrackId(id),
            millis: seconds * 1000,
            available,
        };
        let mut index = TagIndex {
            by_artist_and_title: HashMap::new(),
            by_title: HashMap::new(),
        };
        index.by_artist_and_title.insert(
            (fold("Band"), fold("One")),
            vec![
                candidate(1, 300, true),
                candidate(2, 200, false),
                candidate(3, 200, true),
            ],
        );
        index
            .by_title
            .insert(fold("Band - Live"), vec![candidate(4, 10, true)]);
        index.by_title.insert(
            fold("One"),
            vec![candidate(5, 100, false), candidate(6, 100, true)],
        );

        assert_eq!(index.find("BAND - one", Some(210.0)), Some(TrackId(3)));
        assert_eq!(index.find("Band - One", Some(1000.0)), Some(TrackId(1)));
        assert_eq!(index.find("Band - One", None), Some(TrackId(1)));
        assert_eq!(index.find("Band - Live", None), Some(TrackId(4)));
        assert_eq!(index.find("One", None), Some(TrackId(6)));
        assert_eq!(index.find("Band - Two", Some(1.0)), None);
    }

    #[test]
    fn a_playlist_is_named_after_its_file() {
        assert_eq!(
            playlist_name(Path::new("a/Road trip.m3u8"), "Playlist 3"),
            "Road trip"
        );
        assert_eq!(
            playlist_name(Path::new("a/ .m3u"), "Playlist 3"),
            "Playlist 3"
        );
    }
}
