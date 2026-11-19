//! Exporting playlists as M3U files: the file's lines, paths relative to the playlist
//! file or absolute, names outside ASCII, repeated entries and missing files. And
//! importing them: paths of every form, tags when the path doesn't match, what
//! matched nothing, the encoding, the name, and an export read back. Each test copies
//! the fixtures into a temporary directory and writes the playlist file there.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, Tag, TagExt};
use tempfile::TempDir;
use vcmp_library::{Error, Library, M3uEntry, M3uExport, M3uImport, PlaylistId, TrackId};

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

    /// Writes `text` to `file` (under the temporary directory) and imports it.
    fn import(&self, file: &str, text: impl AsRef<[u8]>) -> M3uImport {
        let file = self.base.join(file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, text).unwrap();
        self.library.import_playlist(&file, "Playlist 1").unwrap()
    }

    fn entries(&self, id: Option<PlaylistId>) -> Vec<TrackId> {
        let id = id.expect("a playlist was made");
        self.library
            .playlist_entries(id)
            .unwrap()
            .into_iter()
            .map(|entry| entry.track.id)
            .collect()
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
            left_out: Vec::new(),
            from_servers: Vec::new(),
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

#[test]
fn an_exported_playlist_reads_back_the_same_repeats_included() {
    let mut setup = Setup::new();
    let one = setup.add("Band/01 One.flac", "Band", "One");
    let largo = setup.add("Dvořák/01 Largo.flac", "Antonín Dvořák", "Largo");
    setup.scan();
    let (one, largo) = (setup.id(&one), setup.id(&largo));
    let id = setup.playlist(&[one, largo, one]);

    let mut names = Vec::new();
    for file in ["music/Mix.m3u8", "Playlists/Shared/Mix.m3u8"] {
        setup.export(id, file);
        let imported = setup
            .library
            .import_playlist(&setup.base.join(file), "Playlist 1")
            .unwrap();
        assert_eq!(imported.entries, 3);
        assert!(imported.unmatched.is_empty(), "{:?}", imported.unmatched);
        assert_eq!(setup.entries(imported.playlist), [one, largo, one]);
        names.push(imported.name);
    }
    assert_eq!(names, ["Mix 2", "Mix 3"]);
}

#[test]
fn paths_match_relative_or_absolute_with_either_separator() {
    let mut setup = Setup::new();
    let one = setup.add("Band/1.flac", "Band", "One");
    let two = setup.add("Band/2.flac", "Band", "Two");
    let three = setup.add("Band/3.flac", "Band", "Three");
    setup.scan();

    let text = format!(
        "../music/Band/1.flac\n..\\music\\.\\Band\\2.flac\n{}\n",
        three.display()
    );
    let imported = setup.import("Lists/Mix.m3u", text);
    assert_eq!(imported.name, "Mix");
    assert!(imported.unmatched.is_empty(), "{:?}", imported.unmatched);
    assert_eq!(
        setup.entries(imported.playlist),
        [setup.id(&one), setup.id(&two), setup.id(&three)]
    );
}

#[test]
fn file_urls_and_a_byte_order_mark_are_understood() {
    let mut setup = Setup::new();
    let one = setup.add("My Band/01 One.flac", "Band", "One");
    setup.scan();

    let url = url::Url::from_file_path(&one).unwrap();
    let imported = setup.import("vlc.m3u", format!("\u{FEFF}#EXTM3U\n{url}\n"));
    assert_eq!(setup.entries(imported.playlist), [setup.id(&one)]);
}

#[test]
fn a_file_that_is_not_utf8_is_read_as_windows_1252() {
    let mut setup = Setup::new();
    let one = setup.add("Café/01.flac", "Band", "One");
    setup.scan();

    let imported = setup.import(
        "legacy.m3u",
        b"#EXTM3U\r\nmusic\\Caf\xe9\\01.flac\r\n".as_slice(),
    );
    assert!(imported.unmatched.is_empty(), "{:?}", imported.unmatched);
    assert_eq!(setup.entries(imported.playlist), [setup.id(&one)]);
}

#[test]
fn a_missing_track_matches_by_its_last_known_path() {
    let mut setup = Setup::new();
    let one = setup.add("1.flac", "Band", "One");
    setup.scan();
    let id = setup.id(&one);
    fs::remove_file(&one).unwrap();
    setup.scan();

    let imported = setup.import("music/Mix.m3u8", "1.flac\n");
    assert_eq!(setup.entries(imported.playlist), [id]);
}

#[test]
fn entries_whose_path_does_not_match_try_their_tags_and_the_rest_are_listed() {
    let mut setup = Setup::new();
    let largo = setup.add("1.flac", "Antonín Dvořák", "Largo");
    let one = setup.add("2.flac", "Band", "One");
    // The same song outside the library, as a playlist made on another copy names it.
    let copy = setup.base.join("copy/One.flac");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(&one, &copy).unwrap();
    setup.scan();

    let text = format!(
        "#EXTM3U\n\
         #EXTINF:1,ANTONIN  DVORAK - largo\n/phone/Music/Largo.mp3\n\
         #EXTINF:1,Band - One\n{}\n\
         #EXTINF:1,Band - Missing\nmissing.flac\n\
         stream.mp3\n\
         #EXTINF:-1,Radio\nhttp://example.com/stream\n",
        copy.display()
    );
    let imported = setup.import("Mix.m3u8", text);
    assert_eq!(
        setup.entries(imported.playlist),
        [setup.id(&largo), setup.id(&one)]
    );
    assert_eq!(imported.entries, 5);
    assert_eq!(
        imported.unmatched,
        [
            M3uEntry {
                location: "missing.flac".into(),
                title: Some("Band - Missing".into()),
                seconds: Some(1.0),
            },
            M3uEntry {
                location: "stream.mp3".into(),
                title: None,
                seconds: None,
            },
            M3uEntry {
                location: "http://example.com/stream".into(),
                title: Some("Radio".into()),
                seconds: None,
            },
        ]
    );
}

#[test]
fn nothing_is_made_when_no_entry_matches() {
    let mut setup = Setup::new();
    setup.add("1.flac", "Band", "One");
    setup.scan();

    let imported = setup.import("Nothing.m3u", "#EXTM3U\n#EXTINF:1,Band - Two\ntwo.flac\n");
    assert_eq!(imported.playlist, None);
    assert_eq!(imported.name, "");
    assert_eq!((imported.entries, imported.unmatched.len()), (1, 1));

    let empty = setup.import("Empty.m3u8", "#EXTM3U\n");
    assert_eq!((empty.playlist, empty.entries), (None, 0));
    assert!(setup.library.playlists().unwrap().is_empty());

    assert!(matches!(
        setup
            .library
            .import_playlist(&setup.base.join("none.m3u"), "Playlist 1"),
        Err(Error::Io { .. })
    ));
}

#[test]
fn a_taken_name_gets_a_number() {
    let mut setup = Setup::new();
    setup.add("1.flac", "Band", "One");
    setup.scan();
    setup.library.create_playlist("Road trip").unwrap();

    let first = setup.import("Road trip.m3u8", "music/1.flac\n");
    let second = setup.import("road TRIP.m3u", "music/1.flac\n");
    assert_eq!(
        (first.name.as_str(), second.name.as_str()),
        ("Road trip 2", "road TRIP 3")
    );
    let names: Vec<String> = setup
        .library
        .playlists()
        .unwrap()
        .into_iter()
        .map(|playlist| playlist.name)
        .collect();
    assert_eq!(names, ["Road trip", "Road trip 2", "road TRIP 3"]);
}

/// Windows paths ignore case, so a path in other case than the file's still matches.
#[cfg(windows)]
#[test]
fn a_path_in_other_case_matches_on_windows() {
    let mut setup = Setup::new();
    let one = setup.add("Band/1.flac", "Band", "One");
    setup.scan();

    let imported = setup.import("Mix.m3u", "MUSIC\\BAND\\1.FLAC\n");
    assert_eq!(setup.entries(imported.playlist), [setup.id(&one)]);
}
