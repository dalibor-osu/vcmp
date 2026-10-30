//! M3U export: a playlist as a UTF-8 `.m3u8` file that other players, phones and
//! backups can read.
//!
//! The file starts with `#EXTM3U`, and each entry is an `#EXTINF` line (length in
//! whole seconds, "Artist - Title") followed by the file's path. A path is relative
//! to the playlist file's folder, written with `/`, when both are on the same
//! drive: the same filesystem on Linux and macOS, the same drive letter or network
//! share on Windows. Otherwise it is absolute, in the platform's own form.
//!
//! Entries whose file is gone are written with the path the library last saw, so
//! the file is complete again once the file is back. A path that can't be written
//! in a UTF-8 text file of lines (not valid UTF-8, or holding a line break) is left
//! out and reported.

use std::fs;
use std::path::{Component, Path, PathBuf};

use rusqlite::OptionalExtension;

use crate::error::io_error;
use crate::{Error, Library, PlaylistId, Result, Track};

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
    Some(parts.join("/"))
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
}
