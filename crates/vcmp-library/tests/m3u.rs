//! Exporting playlists as M3U files: the file's lines, paths relative to the playlist
//! file or absolute, names outside ASCII, repeated entries and missing files. Each
//! test copies the fixtures into a temporary directory and writes the playlist file
//! there.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, Tag, TagExt};
use tempfile::TempDir;
use vcmp_library::{Error, Library, M3uExport, PlaylistId, TrackId};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Setup {
    // The database closes before the directory is deleted, which Windows requires.
    library: Library,
    base: PathBuf,
    _root: TempDir,
}

impl Setup {
    fn new() -> Self {
        Self::with_music_in(tempfile::tempdir().unwrap())
    }

    /// The music folder, the database and the cache in `root`.
    fn with_music_in(root: TempDir) -> Self {
        let base = dunce::canonicalize(root.path()).unwrap();
        fs::create_dir(base.join("music")).unwrap();
        let library =
            Library::open(&base.join("data/library.sqlite3"), &base.join("cache")).unwrap();
        library.add_source(&base.join("music"), true).unwrap();
        Self {
            _root: root,
            base,
            library,
        }
    }

    /// Adds a one-second FLAC file at `name` under the music folder.
    fn add(&self, name: &str, artist: &str, title: &str) -> PathBuf {
        let path = self.base.join("music").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(Path::new(FIXTURES).join("silence.flac"), &path).unwrap();
        let file = lofty::read_from_path(&path).unwrap();
        let mut tag = Tag::new(file.primary_tag_type());
        tag.set_artist(artist.into());
        tag.set_title(title.into());
        tag.save_to_path(&path, WriteOptions::default()).unwrap();
        path
    }

    fn scan(&mut self) {
        let report = self.library.scan(&AtomicBool::new(false), |_| {}).unwrap();
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    fn id(&self, path: &Path) -> TrackId {
        self.library.track_id_for_path(path).unwrap().unwrap()
    }

    fn playlist(&self, tracks: &[TrackId]) -> PlaylistId {
        let id = self.library.create_playlist("Mix").unwrap();
        self.library.add_to_playlist(id, tracks).unwrap();
        id
    }

    /// Exports to `file` (under the temporary directory) and reads the file back.
    fn export(&self, id: PlaylistId, file: &str) -> (String, M3uExport) {
        let file = self.base.join(file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let report = self.library.export_playlist(id, &file).unwrap();
        (fs::read_to_string(&file).unwrap(), report)
    }
}

#[test]
fn paths_are_relative_to_the_playlist_file() {
    let mut setup = Setup::new();
    let one = setup.add("Band/01 One.flac", "Band", "One");
    let two = setup.add("Band/02 Two.flac", "Band", "Two");
    setup.scan();
    let id = setup.playlist(&[setup.id(&one), setup.id(&two)]);

    let (beside, report) = setup.export(id, "music/Mix.m3u8");
    assert_eq!(
        beside,
        "#EXTM3U\n\
         #EXTINF:1,Band - One\nBand/01 One.flac\n\
         #EXTINF:1,Band - Two\nBand/02 Two.flac\n"
    );
    assert_eq!(
        report,
        M3uExport {
            written: 2,
            missing: 0,
            left_out: Vec::new()
        }
    );

    let (below, _) = setup.export(id, "Playlists/Shared/Mix.m3u8");
    assert!(
        below.contains("\n../../music/Band/01 One.flac\n"),
        "{below}"
    );
}

#[test]
fn names_outside_ascii_are_written_as_utf8() {
    let mut setup = Setup::new();
    let largo = setup.add("Dvořák/01 Largo.flac", "Antonín Dvořák", "Largo");
    let song = setup.add("日本/歌.flac", "歌手", "歌");
    setup.scan();
    let id = setup.playlist(&[setup.id(&largo), setup.id(&song)]);

    let (text, _) = setup.export(id, "Smíšené/Výběr.m3u8");
    assert_eq!(
        text,
        "#EXTM3U\n\
         #EXTINF:1,Antonín Dvořák - Largo\n../music/Dvořák/01 Largo.flac\n\
         #EXTINF:1,歌手 - 歌\n../music/日本/歌.flac\n"
    );
}

#[test]
fn a_track_in_the_playlist_twice_is_written_twice_in_order() {
    let mut setup = Setup::new();
    let one = setup.add("1.flac", "Band", "One");
    let two = setup.add("2.flac", "Band", "Two");
    setup.scan();
    let (one, two) = (setup.id(&one), setup.id(&two));
    let id = setup.playlist(&[one, two, one]);

    let (text, report) = setup.export(id, "music/Mix.m3u8");
    let paths: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
    assert_eq!(paths, ["1.flac", "2.flac", "1.flac"]);
    assert_eq!(report.written, 3);
}

#[test]
fn a_missing_file_keeps_its_last_known_path() {
    let mut setup = Setup::new();
    let one = setup.add("1.flac", "Band", "One");
    let two = setup.add("2.flac", "Band", "Two");
    setup.scan();
    let id = setup.playlist(&[setup.id(&one), setup.id(&two)]);
    fs::remove_file(&one).unwrap();
    setup.scan();

    let (text, report) = setup.export(id, "music/Mix.m3u8");
    assert_eq!(
        text,
        "#EXTM3U\n#EXTINF:1,Band - One\n1.flac\n#EXTINF:1,Band - Two\n2.flac\n"
    );
    assert_eq!(report.written, 2);
    assert_eq!(report.missing, 1);
}

#[test]
fn an_empty_playlist_is_a_header_alone_and_an_unknown_one_is_an_error() {
    let setup = Setup::new();
    let id = setup.playlist(&[]);
    let (text, report) = setup.export(id, "Mix.m3u8");
    assert_eq!(text, "#EXTM3U\n");
    assert_eq!(report.written, 0);

    let gone = PlaylistId(id.0 + 1);
    assert!(matches!(
        setup
            .library
            .export_playlist(gone, &setup.base.join("Gone.m3u8")),
        Err(Error::UnknownPlaylist(found)) if found == gone
    ));
    assert!(!setup.base.join("Gone.m3u8").exists());
}

#[test]
fn exporting_again_replaces_the_file() {
    let mut setup = Setup::new();
    let one = setup.add("1.flac", "Band", "One");
    setup.scan();
    let id = setup.playlist(&[setup.id(&one)]);
    let (first, _) = setup.export(id, "Mix.m3u8");
    setup.library.remove_from_playlist(id, &[0]).unwrap();
    let (second, _) = setup.export(id, "Mix.m3u8");
    assert_ne!(first, second);
    assert_eq!(second, "#EXTM3U\n");
}

#[test]
fn a_folder_that_does_not_exist_is_an_error() {
    let setup = Setup::new();
    let id = setup.playlist(&[]);
    let file = setup.base.join("nowhere/Mix.m3u8");
    assert!(matches!(
        setup.library.export_playlist(id, &file),
        Err(Error::Io { .. })
    ));
}

/// Music on another filesystem than the playlist file: `/dev/shm` is a memory
/// filesystem of its own, and the playlist goes to the target directory on disk.
#[cfg(target_os = "linux")]
#[test]
fn a_track_on_another_filesystem_gets_its_absolute_path() {
    let shm = Path::new("/dev/shm");
    if !shm.is_dir() {
        return;
    }
    let mut setup = Setup::with_music_in(tempfile::tempdir_in(shm).unwrap());
    let one = setup.add("Band/1.flac", "Band", "One");
    setup.scan();
    let id = setup.playlist(&[setup.id(&one)]);

    let out = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let file = out.path().join("Mix.m3u8");
    setup.library.export_playlist(id, &file).unwrap();
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        format!("#EXTM3U\n#EXTINF:1,Band - One\n{}\n", one.display())
    );
}
