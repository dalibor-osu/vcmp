//! Scanning, and what builds on the scanned tracks (search, playlists), against real
//! (tiny) audio files. Each test copies fixtures into a temporary directory and
//! writes the tags it needs there.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use lofty::config::WriteOptions;
use lofty::file::{FileType, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};
use tempfile::TempDir;
use vcmp_library::{
    Artwork, CoverArtId, Error, FieldEdit, Format, Library, LibraryWatcher, PlaylistId, ScanReport,
    TagEdit, TrackId,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Setup {
    // Fields drop in order: the database closes before the directory is deleted,
    // which Windows requires.
    library: Library,
    // The temporary directory with symlinks resolved (macOS keeps temporary files
    // under a symlink, Windows may hand out short 8.3 names), matching how sources
    // are stored.
    base: PathBuf,
    _root: TempDir,
}

impl Setup {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(root.path()).unwrap();
        fs::create_dir(base.join("music")).unwrap();
        let library =
            Library::open(&base.join("data/library.sqlite3"), &base.join("cache")).unwrap();
        Self {
            _root: root,
            base,
            library,
        }
    }

    fn music(&self) -> PathBuf {
        self.base.as_path().join("music")
    }

    fn cache(&self) -> PathBuf {
        self.base.as_path().join("cache")
    }

    fn add_music_source(&self) {
        self.library.add_source(&self.music(), true).unwrap();
    }

    fn scan(&mut self) -> ScanReport {
        let report = self.library.scan(&AtomicBool::new(false), |_| {}).unwrap();
        assert!(!report.cancelled);
        report
    }
}

#[derive(Default)]
struct Tags<'a> {
    title: Option<&'a str>,
    artist: Option<&'a str>,
    album: Option<&'a str>,
    album_artist: Option<&'a str>,
    track: Option<u32>,
    year: Option<&'a str>,
    picture: Option<Vec<u8>>,
}

/// Copies a fixture to `dir/name` and writes `tags` into the copy.
fn add_file(dir: &Path, name: &str, fixture: &str, tags: Tags) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::copy(Path::new(FIXTURES).join(fixture), &path).unwrap();
    write_tags(&path, tags);
    path
}

fn write_tags(path: &Path, tags: Tags) {
    let file = lofty::read_from_path(path).unwrap();
    let tag_type = match file.file_type() {
        FileType::Wav => TagType::Id3v2,
        _ => file.primary_tag_type(),
    };
    let mut tag = Tag::new(tag_type);
    if let Some(value) = tags.title {
        tag.set_title(value.into());
    }
    if let Some(value) = tags.artist {
        tag.set_artist(value.into());
    }
    if let Some(value) = tags.album {
        tag.set_album(value.into());
    }
    if let Some(value) = tags.album_artist {
        tag.insert_text(ItemKey::AlbumArtist, value.into());
    }
    if let Some(value) = tags.track {
        tag.set_track(value);
    }
    if let Some(value) = tags.year {
        tag.insert_text(ItemKey::Year, value.into());
    }
    if let Some(data) = tags.picture {
        tag.push_picture(
            Picture::unchecked(data)
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
    }
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

fn song<'a>(title: &'a str, artist: &'a str, album: &'a str) -> Tags<'a> {
    Tags {
        title: Some(title),
        artist: Some(artist),
        album: Some(album),
        ..Tags::default()
    }
}

fn png() -> Vec<u8> {
    png_in([245, 194, 231])
}

fn png_in(colour: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(64, 64, image::Rgb(colour));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

#[test]
fn reads_every_supported_format_and_skips_opus() {
    let mut setup = Setup::new();
    let album = setup.music().join("Album");
    for (index, (fixture, name)) in [
        ("silence.mp3", "1.mp3"),
        ("silence.flac", "2.flac"),
        ("silence.ogg", "3.ogg"),
        ("silence.wav", "4.wav"),
    ]
    .into_iter()
    .enumerate()
    {
        let title = format!("Song {}", index + 1);
        add_file(
            &album,
            name,
            fixture,
            Tags {
                track: Some(index as u32 + 1),
                ..song(&title, "Artist", "Album")
            },
        );
    }
    fs::copy(
        Path::new(FIXTURES).join("opus-in-ogg.ogg"),
        album.join("opus.ogg"),
    )
    .unwrap();
    fs::write(album.join("notes.txt"), "not music").unwrap();
    setup.add_music_source();

    let report = setup.scan();
    assert_eq!(report.added, 4);
    assert_eq!(report.skipped.len(), 1);
    assert!(report.skipped[0].path.ends_with("opus.ogg"));
    assert!(
        report.skipped[0].reason.contains("unsupported"),
        "{}",
        report.skipped[0].reason
    );

    let tracks = setup.library.tracks().unwrap();
    let formats: Vec<_> = tracks.iter().map(|track| track.format).collect();
    assert_eq!(
        formats,
        [
            Some(Format::Mp3),
            Some(Format::Flac),
            Some(Format::OggVorbis),
            Some(Format::Wav)
        ]
    );
    for track in &tracks {
        // MP3 durations count whole frames including encoder padding (1.152 s here).
        assert!(
            (900..=1200).contains(&track.duration.as_millis()),
            "{}: {:?}",
            track.title,
            track.duration
        );
        assert_eq!(track.artist.as_deref(), Some("Artist"));
    }
    let albums = setup.library.albums().unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].track_count, 4);
    assert_eq!(setup.library.artists().unwrap().len(), 1);
}

#[test]
fn untagged_file_is_titled_by_its_name() {
    let mut setup = Setup::new();
    add_file(
        &setup.music(),
        "Intro Jam.flac",
        "silence.flac",
        Tags::default(),
    );
    setup.add_music_source();
    setup.scan();

    let tracks = setup.library.tracks().unwrap();
    assert_eq!(tracks[0].title, "Intro Jam");
    assert_eq!(
        (tracks[0].artist.as_deref(), tracks[0].album.as_deref()),
        (None, None)
    );
    assert!(setup.library.albums().unwrap().is_empty());
}

#[test]
fn rescanning_an_unchanged_library_changes_nothing() {
    let mut setup = Setup::new();
    add_file(&setup.music(), "a.mp3", "silence.mp3", song("A", "X", "Y"));
    add_file(
        &setup.music(),
        "b.flac",
        "silence.flac",
        song("B", "X", "Y"),
    );
    setup.add_music_source();
    setup.scan();
    let before = setup.library.tracks().unwrap();

    let report = setup.scan();
    assert_eq!(
        (
            report.unchanged,
            report.added,
            report.updated,
            report.relinked
        ),
        (2, 0, 0, 0)
    );
    assert_eq!(setup.library.tracks().unwrap(), before);
}

#[test]
fn changed_tags_update_the_track_and_drop_the_empty_album() {
    let mut setup = Setup::new();
    let path = add_file(
        &setup.music(),
        "a.mp3",
        "silence.mp3",
        song("Old title", "X", "Old album"),
    );
    setup.add_music_source();
    setup.scan();
    let id = setup.library.track_id_for_path(&path).unwrap().unwrap();

    // Make sure the modification time moves even on coarse file systems.
    std::thread::sleep(Duration::from_millis(1100));
    write_tags(&path, song("New title", "X", "New album"));
    let report = setup.scan();

    assert_eq!(report.updated, 1);
    let track = setup.library.track(id).unwrap().unwrap();
    assert_eq!(track.title, "New title");
    let albums: Vec<String> = setup
        .library
        .albums()
        .unwrap()
        .into_iter()
        .map(|album| album.title)
        .collect();
    assert_eq!(albums, ["New album"]);
}

#[test]
fn moved_file_keeps_its_track_id_and_playlist_entry() {
    let mut setup = Setup::new();
    let old = add_file(
        &setup.music(),
        "a.flac",
        "silence.flac",
        song("Largo", "Dvořák", "New World"),
    );
    setup.add_music_source();
    setup.scan();
    let id = setup.library.track_id_for_path(&old).unwrap().unwrap();
    let playlist = setup.library.create_playlist("Calm").unwrap();
    setup.library.add_to_playlist(playlist, &[id]).unwrap();

    let new = setup.music().join("Dvořák/New World/02 Largo.flac");
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    fs::rename(&old, &new).unwrap();
    let report = setup.scan();

    assert_eq!(
        (report.relinked, report.added, report.became_unavailable),
        (1, 0, 1)
    );
    let track = setup.library.track(id).unwrap().unwrap();
    assert_eq!(track.path, new);
    assert!(track.available);
    assert_eq!(setup.library.tracks().unwrap().len(), 1);
    let entries = setup.library.playlist_entries(playlist).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].track.id, &entries[0].track.path), (id, &new));
}

#[test]
fn ambiguous_move_creates_a_new_track() {
    let mut setup = Setup::new();
    let first = add_file(
        &setup.music().join("one"),
        "a.flac",
        "silence.flac",
        song("Same", "X", "Y"),
    );
    let second = add_file(
        &setup.music().join("two"),
        "a.flac",
        "silence.flac",
        song("Same", "X", "Y"),
    );
    setup.add_music_source();
    setup.scan();
    let ids = [first.clone(), second.clone()]
        .map(|path| setup.library.track_id_for_path(&path).unwrap().unwrap());

    let moved = setup.music().join("three/a.flac");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(&first, &moved).unwrap();
    fs::remove_file(&second).unwrap();
    let report = setup.scan();

    assert_eq!((report.relinked, report.added), (0, 1));
    let new_id = setup.library.track_id_for_path(&moved).unwrap().unwrap();
    assert!(!ids.contains(&new_id));
}

#[test]
fn deleted_file_becomes_unavailable_and_returns() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "a.ogg", "silence.ogg", song("A", "X", "Y"));
    setup.add_music_source();
    setup.scan();
    let id = setup.library.track_id_for_path(&path).unwrap().unwrap();

    let parked = setup.base.as_path().join("parked.ogg");
    fs::rename(&path, &parked).unwrap();
    assert_eq!(setup.scan().became_unavailable, 1);
    assert!(setup.library.tracks().unwrap().is_empty());
    assert!(setup.library.albums().unwrap().is_empty());
    assert!(!setup.library.track(id).unwrap().unwrap().available);

    fs::rename(&parked, &path).unwrap();
    assert_eq!(setup.scan().became_available, 1);
    assert!(setup.library.track(id).unwrap().unwrap().available);
}

#[test]
fn removing_a_source_keeps_its_tracks_as_unavailable() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "a.wav", "silence.wav", song("A", "X", "Y"));
    let source = setup.library.add_source(&setup.music(), true).unwrap();
    setup.scan();
    let id = setup.library.track_id_for_path(&path).unwrap().unwrap();

    setup.library.remove_source(source.id).unwrap();
    assert!(setup.library.sources().unwrap().is_empty());
    assert!(setup.library.tracks().unwrap().is_empty());
    assert!(!setup.library.track(id).unwrap().unwrap().available);

    setup.add_music_source();
    let report = setup.scan();
    assert_eq!(report.became_available, 1);
    assert_eq!(setup.library.track_id_for_path(&path).unwrap(), Some(id));
}

#[test]
fn unreachable_source_is_reported_and_its_tracks_go_unavailable() {
    let mut setup = Setup::new();
    let drive = setup.base.as_path().join("drive");
    add_file(&drive, "a.mp3", "silence.mp3", song("A", "X", "Y"));
    setup.library.add_source(&drive, true).unwrap();
    setup.scan();

    let unplugged = setup.base.as_path().join("unplugged");
    fs::rename(&drive, &unplugged).unwrap();
    let report = setup.scan();
    assert_eq!(report.unreachable_sources.len(), 1);
    assert_eq!(report.became_unavailable, 1);
}

#[test]
fn non_recursive_source_ignores_subdirectories() {
    let mut setup = Setup::new();
    add_file(
        &setup.music(),
        "top.mp3",
        "silence.mp3",
        song("Top", "X", "Y"),
    );
    add_file(
        &setup.music().join("sub"),
        "deep.mp3",
        "silence.mp3",
        song("Deep", "X", "Y"),
    );
    let source = setup.library.add_source(&setup.music(), false).unwrap();
    setup.scan();
    assert_eq!(setup.library.tracks().unwrap().len(), 1);

    setup.library.set_source_recursive(source.id, true).unwrap();
    assert_eq!(setup.scan().added, 1);
    assert_eq!(setup.library.tracks().unwrap().len(), 2);
}

#[test]
fn albums_group_by_album_artist_and_title() {
    let mut setup = Setup::new();
    let dir = setup.music();
    // Same title, different artists: two albums.
    add_file(
        &dir,
        "1.mp3",
        "silence.mp3",
        song("A", "Band One", "Greatest Hits"),
    );
    add_file(
        &dir,
        "2.mp3",
        "silence.mp3",
        song("B", "Band Two", "Greatest Hits"),
    );
    // A compilation: one album despite different track artists.
    for (index, artist) in ["Singer", "Rapper"].into_iter().enumerate() {
        let tags = Tags {
            album_artist: Some("Various Artists"),
            ..song("C", artist, "Summer Mix")
        };
        add_file(&dir, &format!("mix{index}.flac"), "silence.flac", tags);
    }
    // Case differences in tags do not split an album.
    add_file(
        &dir,
        "w1.ogg",
        "silence.ogg",
        song("D", "Pink Floyd", "The Wall"),
    );
    add_file(
        &dir,
        "w2.ogg",
        "silence.ogg",
        song("E", "pink floyd", "the wall"),
    );
    setup.add_music_source();
    setup.scan();

    let albums = setup.library.albums().unwrap();
    let summary: Vec<(String, Option<String>, u32)> = albums
        .into_iter()
        .map(|album| (album.title, album.artist, album.track_count))
        .collect();
    assert_eq!(
        summary,
        [
            ("Greatest Hits".to_string(), Some("Band One".to_string()), 1),
            ("Greatest Hits".to_string(), Some("Band Two".to_string()), 1),
            ("The Wall".to_string(), Some("Pink Floyd".to_string()), 2),
            (
                "Summer Mix".to_string(),
                Some("Various Artists".to_string()),
                2
            ),
        ]
    );
    let compilation = setup
        .library
        .artists()
        .unwrap()
        .into_iter()
        .find(|artist| artist.name == "Various Artists");
    assert_eq!(
        compilation.map(|artist| (artist.album_count, artist.track_count)),
        Some((1, 0))
    );
}

#[test]
fn album_year_is_the_earliest_track_year() {
    let mut setup = Setup::new();
    add_file(
        &setup.music(),
        "1.flac",
        "silence.flac",
        Tags {
            year: Some("1994"),
            ..song("A", "X", "Y")
        },
    );
    add_file(
        &setup.music(),
        "2.flac",
        "silence.flac",
        Tags {
            year: Some("1993-05-01"),
            ..song("B", "X", "Y")
        },
    );
    setup.add_music_source();
    setup.scan();
    let album = &setup.library.albums().unwrap()[0];
    assert_eq!(album.year, Some(1993));
    assert_eq!(setup.library.album(album.id).unwrap().as_ref(), Some(album));
    assert_eq!(setup.library.available_track_count().unwrap(), 2);
}

#[test]
fn artwork_comes_from_embedded_pictures_or_folder_images() {
    let mut setup = Setup::new();
    add_file(
        &setup.music().join("embedded"),
        "a.mp3",
        "silence.mp3",
        Tags {
            picture: Some(png()),
            ..song("A", "X", "Embedded")
        },
    );
    let folder = setup.music().join("folder");
    add_file(&folder, "a.flac", "silence.flac", song("B", "X", "Folder"));
    fs::write(folder.join("Cover.PNG"), png()).unwrap();
    add_file(
        &setup.music().join("none"),
        "a.ogg",
        "silence.ogg",
        song("C", "X", "None"),
    );
    setup.add_music_source();
    setup.scan();

    for title in ["Embedded", "Folder"] {
        let found =
            album_artwork(&setup.library, title).unwrap_or_else(|| panic!("{title} has artwork"));
        assert!(found.thumbnail.starts_with(setup.cache()));
        assert!(image::open(&found.thumbnail).is_ok());
    }
    assert_eq!(album_artwork(&setup.library, "None"), None);

    // A deleted cache is rebuilt by the next scan.
    fs::remove_dir_all(setup.cache()).unwrap();
    assert_eq!(album_artwork(&setup.library, "Embedded"), None);
    setup.scan();
    assert!(album_artwork(&setup.library, "Embedded").is_some());
    assert!(album_artwork(&setup.library, "Folder").is_some());
}

#[test]
fn a_fetched_cover_stays_until_the_files_bring_their_own() {
    let mut setup = Setup::new();
    let path = add_file(
        &setup.music(),
        "a.flac",
        "silence.flac",
        song("A", "X", "Bare"),
    );
    setup.add_music_source();
    setup.scan();
    let album = |library: &Library| {
        library
            .albums()
            .unwrap()
            .into_iter()
            .find(|album| album.title == "Bare")
            .map(|album| album.id)
    };
    assert_eq!(album_artwork(&setup.library, "Bare"), None);
    let fetched = setup
        .library
        .set_fetched_cover(
            album(&setup.library).unwrap(),
            &png_in([30, 102, 245]),
            &CoverArtId::Release("r-1".into()),
        )
        .unwrap()
        .expect("a png is accepted");
    assert!(fetched.thumbnail.starts_with(setup.cache()));
    assert_eq!(album_artwork(&setup.library, "Bare"), Some(fetched.clone()));
    // A playlist's picture uses it like the files' own art.
    let playlist = setup.library.create_playlist("Mix").unwrap();
    let track = setup.library.track_id_for_path(&path).unwrap().unwrap();
    setup.library.add_to_playlist(playlist, &[track]).unwrap();
    let picture = setup.library.playlist(playlist).unwrap().unwrap().picture;
    assert_eq!(picture, Some(fetched.thumbnail.clone()));

    // Rescans keep it, also when a file changed without gaining art.
    setup.scan();
    assert_eq!(album_artwork(&setup.library, "Bare"), Some(fetched.clone()));
    let retag = |tags: Tags| {
        // Make sure the modification time moves even on coarse file systems.
        std::thread::sleep(Duration::from_millis(1100));
        write_tags(&path, tags);
    };
    retag(Tags {
        year: Some("2001"),
        ..song("A", "X", "Bare")
    });
    assert_eq!(setup.scan().updated, 1);
    assert_eq!(album_artwork(&setup.library, "Bare"), Some(fetched.clone()));

    // It belongs to the album, not its row: moving the file to another album and
    // back makes a new row, which finds it again.
    retag(song("A", "X", "Elsewhere"));
    setup.scan();
    assert_eq!(album(&setup.library), None);
    retag(song("A", "X", "Bare"));
    setup.scan();
    assert_eq!(album_artwork(&setup.library, "Bare"), Some(fetched.clone()));

    // Art of the files' own wins, and the fetched cover comes back when the tag
    // editor removes it.
    retag(Tags {
        picture: Some(png()),
        ..song("A", "X", "Bare")
    });
    setup.scan();
    let own = album_artwork(&setup.library, "Bare").expect("embedded art");
    assert_ne!(own, fetched);
    let track = setup.library.track_id_for_path(&path).unwrap().unwrap();
    let clear = TagEdit {
        cover: FieldEdit::Clear,
        ..TagEdit::default()
    };
    let report = setup
        .library
        .write_tags(&[(track, &clear)], &AtomicBool::new(false), |_, _| {})
        .unwrap();
    assert_eq!(report.written, [track]);
    assert_eq!(album_artwork(&setup.library, "Bare"), Some(fetched));
}

fn album_artwork(library: &Library, title: &str) -> Option<Artwork> {
    library
        .albums()
        .unwrap()
        .into_iter()
        .find(|album| album.title == title)
        .unwrap()
        .artwork
}

#[test]
fn search_ignores_case_and_diacritics_and_matches_prefixes() {
    let mut setup = Setup::new();
    add_file(
        &setup.music(),
        "1.flac",
        "silence.flac",
        song("Largo", "Antonín Dvořák", "Symphony No. 9"),
    );
    add_file(
        &setup.music(),
        "2.flac",
        "silence.flac",
        song("Comfortably Numb", "Pink Floyd", "The Wall"),
    );
    setup.add_music_source();
    setup.scan();

    let results = setup.library.search("dvorak", 10).unwrap();
    assert_eq!(
        results
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Antonín Dvořák"]
    );
    assert_eq!(results.tracks.len(), 1);
    assert_eq!(results.albums.len(), 1, "albums match on their artist too");

    let results = setup.library.search("LAR", 10).unwrap();
    assert_eq!(
        results
            .tracks
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>(),
        ["Largo"]
    );

    let results = setup.library.search("floyd wall", 10).unwrap();
    assert_eq!(
        results
            .albums
            .iter()
            .map(|a| a.title.as_str())
            .collect::<Vec<_>>(),
        ["The Wall"]
    );
    assert!(
        results.artists.is_empty(),
        "no artist is named both floyd and wall"
    );

    assert_eq!(setup.library.search("  ", 10).unwrap(), Default::default());
    assert!(
        setup.library.search("\"*:(", 10).is_ok(),
        "query syntax in the text is not interpreted"
    );
}

#[test]
fn settings_persist_and_auto_update_defaults_on() {
    let setup = Setup::new();
    assert!(setup.library.auto_update().unwrap());
    setup.library.set_auto_update(false).unwrap();
    setup.library.set_setting("ui.theme", "dark").unwrap();
    setup.library.set_setting("ui.theme", "light").unwrap();

    let reopened = Library::open(
        &setup.base.as_path().join("data/library.sqlite3"),
        &setup.cache(),
    )
    .unwrap();
    assert!(!reopened.auto_update().unwrap());
    assert_eq!(
        reopened.setting("ui.theme").unwrap().as_deref(),
        Some("light")
    );
    assert_eq!(reopened.setting("missing").unwrap(), None);
}

#[test]
fn cancelled_scan_stops_without_adding() {
    let mut setup = Setup::new();
    add_file(&setup.music(), "a.mp3", "silence.mp3", song("A", "X", "Y"));
    setup.add_music_source();
    let report = setup.library.scan(&AtomicBool::new(true), |_| {}).unwrap();
    assert!(report.cancelled);
    assert!(setup.library.tracks().unwrap().is_empty());
}

#[test]
fn database_from_a_newer_build_is_refused() {
    let setup = Setup::new();
    let db = setup.base.as_path().join("data/library.sqlite3");
    drop(setup.library);
    rusqlite::Connection::open(&db)
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    match Library::open(&db, &setup.base.as_path().join("cache")) {
        Err(Error::SchemaTooNew { found: 99, .. }) => {}
        other => panic!("expected SchemaTooNew, got {:?}", other.err()),
    }
}

#[test]
fn adding_a_missing_directory_fails() {
    let setup = Setup::new();
    assert!(
        setup
            .library
            .add_source(&setup.base.as_path().join("nope"), true)
            .is_err()
    );
    let file = setup.base.as_path().join("file");
    fs::write(&file, "x").unwrap();
    assert!(matches!(
        setup.library.add_source(&file, true),
        Err(Error::NotADirectory(_))
    ));
}

// macOS (APFS) and Windows only allow file names that are valid Unicode.
#[cfg(target_os = "linux")]
#[test]
fn file_names_that_are_not_utf8_survive() {
    use std::os::unix::ffi::OsStrExt;
    let mut setup = Setup::new();
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.mp3");
    let path = setup.music().join(name);
    fs::copy(Path::new(FIXTURES).join("silence.mp3"), &path).unwrap();
    setup.add_music_source();
    setup.scan();
    assert_eq!(setup.library.tracks().unwrap()[0].path, path);
}

#[test]
fn watcher_reports_a_new_file_once_things_are_quiet() {
    let setup = Setup::new();
    setup.add_music_source();
    let (sender, receiver) = mpsc::channel();
    let _watcher = LibraryWatcher::start(
        &setup.library.sources().unwrap(),
        Duration::from_millis(300),
        move || {
            let _ = sender.send(());
        },
    )
    .unwrap();

    for index in 0..3 {
        add_file(
            &setup.music(),
            &format!("{index}.mp3"),
            "silence.mp3",
            Tags::default(),
        );
    }
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("a change notification");
    // The burst of writes is reported once.
    assert!(receiver.recv_timeout(Duration::from_millis(800)).is_err());
}

#[test]
fn an_artist_page_has_own_albums_and_appearances() {
    let mut setup = Setup::new();
    let dir = setup.music();
    add_file(
        &dir,
        "1.flac",
        "silence.flac",
        song("Own", "Singer", "Solo"),
    );
    add_file(
        &dir,
        "2.flac",
        "silence.flac",
        Tags {
            album_artist: Some("Band"),
            ..song("Guest spot", "Singer", "Band Album")
        },
    );
    add_file(
        &dir,
        "3.flac",
        "silence.flac",
        Tags {
            album_artist: Some("Band"),
            ..song("Band song", "Band", "Band Album")
        },
    );
    add_file(
        &dir,
        "4.flac",
        "silence.flac",
        Tags {
            title: Some("Loose"),
            artist: Some("Singer"),
            ..Tags::default()
        },
    );
    setup.add_music_source();
    setup.scan();

    let singer = setup
        .library
        .artists()
        .unwrap()
        .into_iter()
        .find(|artist| artist.name == "Singer")
        .unwrap();
    assert_eq!(
        setup.library.artist(singer.id).unwrap(),
        Some(singer.clone())
    );
    assert_eq!(
        (singer.album_count, singer.track_count),
        (1, 3),
        "its own album, and every credited track"
    );
    let own: Vec<String> = setup
        .library
        .artist_albums(singer.id)
        .unwrap()
        .into_iter()
        .map(|album| album.title)
        .collect();
    assert_eq!(own, ["Solo"]);
    let mut appearances: Vec<String> = setup
        .library
        .artist_appearances(singer.id)
        .unwrap()
        .into_iter()
        .map(|track| track.title)
        .collect();
    appearances.sort();
    assert_eq!(appearances, ["Guest spot", "Loose"]);
}

#[test]
fn search_finds_parts_of_words_in_any_script() {
    let mut setup = Setup::new();
    add_file(
        &setup.music(),
        "1.flac",
        "silence.flac",
        Tags {
            album_artist: Some("μ's"),
            ..song("Music S.T.A.R.T!!", "高坂穂乃果", "ラブライブ! Solo Live!")
        },
    );
    add_file(
        &setup.music(),
        "2.flac",
        "silence.flac",
        song(
            "Butterfly Effect",
            "Koven",
            "Butterfly Effect (Deluxe Edition)",
        ),
    );
    setup.add_music_source();
    setup.scan();
    let titles = |text: &str| -> Vec<String> {
        let results = setup.library.search(text, 10).unwrap();
        results
            .tracks
            .into_iter()
            .map(|track| track.title)
            .collect()
    };

    // Inside a word without spaces, at three characters and more.
    assert_eq!(titles("ライブ"), ["Music S.T.A.R.T!!"]);
    assert_eq!(titles("穂乃果"), ["Music S.T.A.R.T!!"]);
    // Shorter words are matched too, without the index.
    assert_eq!(titles("高坂"), ["Music S.T.A.R.T!!"]);
    assert_eq!(titles("μ"), ["Music S.T.A.R.T!!"], "by album artist");
    // Inside Latin words, combined with a short word.
    assert_eq!(titles("terfly ko"), ["Butterfly Effect"]);
    // Wildcards in the text are plain characters.
    assert!(titles("%").is_empty());
    let results = setup.library.search("ライブ", 10).unwrap();
    assert_eq!(results.albums.len(), 1);
}

/// Three songs on one album with a cover, by title.
fn three_songs(setup: &mut Setup) -> [TrackId; 3] {
    let album = setup.music().join("Album");
    for (index, title) in ["One", "Two", "Three"].into_iter().enumerate() {
        add_file(
            &album,
            &format!("{}.flac", index + 1),
            "silence.flac",
            Tags {
                track: Some(index as u32 + 1),
                picture: Some(png()),
                ..song(title, "Artist", "Album")
            },
        );
    }
    setup.add_music_source();
    setup.scan();
    let id = |title: &str| {
        let tracks = setup.library.tracks().unwrap();
        tracks.iter().find(|track| track.title == title).unwrap().id
    };
    [id("One"), id("Two"), id("Three")]
}

fn entry_titles(library: &Library, playlist: PlaylistId) -> Vec<String> {
    library
        .playlist_entries(playlist)
        .unwrap()
        .into_iter()
        .map(|entry| entry.track.title)
        .collect()
}

#[test]
fn a_playlist_keeps_its_order_through_edits() {
    let mut setup = Setup::new();
    let [one, two, three] = three_songs(&mut setup);
    let library = &setup.library;
    let road = library.create_playlist("  Road trip ").unwrap();
    library.add_to_playlist(road, &[one, two]).unwrap();
    // A track may be in a playlist more than once.
    library.add_to_playlist(road, &[three, one]).unwrap();
    assert_eq!(entry_titles(library, road), ["One", "Two", "Three", "One"]);

    library.move_in_playlist(road, 0, 2).unwrap();
    assert_eq!(entry_titles(library, road), ["Two", "Three", "One", "One"]);
    library.move_in_playlist(road, 3, 0).unwrap();
    assert_eq!(entry_titles(library, road), ["One", "Two", "Three", "One"]);
    library.remove_from_playlist(road, &[1, 3]).unwrap();
    assert_eq!(entry_titles(library, road), ["One", "Three"]);
    let positions: Vec<u32> = library
        .playlist_entries(road)
        .unwrap()
        .iter()
        .map(|entry| entry.position)
        .collect();
    assert_eq!(positions, [0, 1]);

    library.rename_playlist(road, "Commute").unwrap();
    let quiet = library.create_playlist("calm").unwrap();
    let names: Vec<String> = library
        .playlists()
        .unwrap()
        .into_iter()
        .map(|playlist| playlist.name)
        .collect();
    assert_eq!(names, ["calm", "Commute"]);
    let commute = library.playlist(road).unwrap().unwrap();
    assert_eq!((commute.track_count, commute.available_count), (2, 2));
    let empty = library.playlist(quiet).unwrap().unwrap();
    assert_eq!((empty.track_count, empty.picture), (0, None));

    library.delete_playlist(road).unwrap();
    assert_eq!(library.playlist(road).unwrap(), None);
    assert!(library.playlist_entries(road).unwrap().is_empty());
}

#[test]
fn a_missing_file_stays_in_its_playlists_and_comes_back() {
    let mut setup = Setup::new();
    let [one, two, _] = three_songs(&mut setup);
    let playlist = setup.library.create_playlist("Mix").unwrap();
    setup
        .library
        .add_to_playlist(playlist, &[one, two])
        .unwrap();
    let full = setup.library.playlist(playlist).unwrap().unwrap();

    let moved = setup.base.join("away.flac");
    fs::rename(setup.music().join("Album/2.flac"), &moved).unwrap();
    setup.scan();
    let entries = setup.library.playlist_entries(playlist).unwrap();
    let available: Vec<bool> = entries.iter().map(|entry| entry.track.available).collect();
    assert_eq!(available, [true, false]);
    let partial = setup.library.playlist(playlist).unwrap().unwrap();
    assert_eq!((partial.track_count, partial.available_count), (2, 1));
    assert!(partial.duration < full.duration || full.duration.is_zero());

    fs::rename(&moved, setup.music().join("Album/2.flac")).unwrap();
    setup.scan();
    let back = setup.library.playlist(playlist).unwrap().unwrap();
    assert_eq!(back.available_count, 2);
}

#[test]
fn a_playlist_picture_is_its_own_image_or_the_album_cover() {
    let mut setup = Setup::new();
    let [one, ..] = three_songs(&mut setup);
    let library = &setup.library;
    let playlist = library.create_playlist("Mix").unwrap();
    library.add_to_playlist(playlist, &[one]).unwrap();
    let cover = album_artwork(library, "Album").unwrap().thumbnail;
    let made = library.playlist(playlist).unwrap().unwrap();
    assert_eq!(
        (made.picture, made.own_picture),
        (Some(cover.clone()), false)
    );

    let source = setup.base.join("picked.png");
    fs::write(&source, png()).unwrap();
    library.set_playlist_image(playlist, &source).unwrap();
    let own = library.playlist(playlist).unwrap().unwrap();
    let picture = own.picture.unwrap();
    assert!(own.own_picture);
    assert!(picture.starts_with(setup.base.join("data/images/playlists")));
    assert!(image::open(&picture).is_ok());

    // The cache can go; the owner's picture is not in it.
    fs::remove_dir_all(setup.cache()).unwrap();
    assert!(picture.is_file());

    library.clear_playlist_image(playlist).unwrap();
    assert!(!picture.exists());
    assert!(!library.playlist(playlist).unwrap().unwrap().own_picture);

    library.set_playlist_image(playlist, &source).unwrap();
    let picture = library
        .playlist(playlist)
        .unwrap()
        .unwrap()
        .picture
        .unwrap();
    library.delete_playlist(playlist).unwrap();
    assert!(!picture.exists());

    fs::write(&source, b"not an image").unwrap();
    let other = library.create_playlist("Other").unwrap();
    assert!(matches!(
        library.set_playlist_image(other, &source),
        Err(Error::UnreadableImage(_))
    ));
}

#[test]
fn playlists_are_found_by_every_word_of_their_name() {
    let setup = Setup::new();
    let library = &setup.library;
    library.create_playlist("Summer Evenings").unwrap();
    library.create_playlist("Winter").unwrap();
    let found = |text: &str| -> Vec<String> {
        library
            .search_playlists(text)
            .unwrap()
            .into_iter()
            .map(|playlist| playlist.name)
            .collect()
    };
    assert_eq!(found("even sum"), ["Summer Evenings"]);
    assert_eq!(found("WIN"), ["Winter"]);
    assert!(found("summer winter").is_empty());
    assert!(found("  ").is_empty());
}

fn png_of(size: u32, colour: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(size, size, image::Rgb(colour));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

#[test]
fn art_another_program_replaces_or_removes_in_the_files_is_noticed() {
    let mut setup = Setup::new();
    let album = setup.music().join("Album");
    let picture = |data: Vec<u8>| Tags {
        picture: Some(data),
        ..song("One", "Band", "Album")
    };
    let path = add_file(
        &album,
        "1.flac",
        "silence.flac",
        picture(png_of(64, [200, 0, 0])),
    );
    setup.add_music_source();
    setup.scan();
    let first = album_artwork(&setup.library, "Album").expect("the embedded cover");

    // Another program puts a new cover in the file.
    write_tags(&path, picture(png_of(80, [0, 0, 200])));
    setup.scan();
    let second = album_artwork(&setup.library, "Album").expect("the new cover");
    assert_ne!(first, second);

    // And then takes it out.
    write_tags(&path, song("One", "Band", "Album"));
    setup.scan();
    assert_eq!(album_artwork(&setup.library, "Album"), None);
}

#[test]
fn a_folder_image_added_replaced_or_removed_is_noticed() {
    let mut setup = Setup::new();
    let album = setup.music().join("Album");
    add_file(
        &album,
        "1.flac",
        "silence.flac",
        song("One", "Band", "Album"),
    );
    setup.add_music_source();
    setup.scan();
    assert_eq!(album_artwork(&setup.library, "Album"), None);

    let cover = album.join("Cover.PNG");
    fs::write(&cover, png_of(64, [200, 0, 0])).unwrap();
    let report = setup.scan();
    assert_eq!(report.updated, 0, "no music file changed");
    let first = album_artwork(&setup.library, "Album").expect("the folder's cover");

    fs::write(&cover, png_of(80, [0, 0, 200])).unwrap();
    setup.scan();
    let second = album_artwork(&setup.library, "Album").expect("the replaced cover");
    assert_ne!(first, second);

    fs::remove_file(&cover).unwrap();
    setup.scan();
    assert_eq!(album_artwork(&setup.library, "Album"), None);

    // Images that aren't covers don't count.
    fs::write(album.join("booklet.png"), png_of(64, [0, 200, 0])).unwrap();
    setup.scan();
    assert_eq!(album_artwork(&setup.library, "Album"), None);
}

#[test]
fn a_changed_later_track_keeps_the_album_cover_of_its_first_tracks() {
    let mut setup = Setup::new();
    let album = setup.music().join("Album");
    fn with(title: &str, track: u32, data: Vec<u8>) -> Tags<'_> {
        Tags {
            track: Some(track),
            picture: Some(data),
            ..song(title, "Band", "Album")
        }
    }
    add_file(
        &album,
        "1.flac",
        "silence.flac",
        with("One", 1, png_of(64, [200, 0, 0])),
    );
    add_file(
        &album,
        "2.flac",
        "silence.flac",
        with("Two", 2, png_of(64, [200, 0, 0])),
    );
    let third = add_file(
        &album,
        "3.flac",
        "silence.flac",
        with("Three", 3, png_of(64, [200, 0, 0])),
    );
    setup.add_music_source();
    setup.scan();
    let cover = album_artwork(&setup.library, "Album").unwrap();

    write_tags(&third, with("Three", 3, png_of(80, [0, 0, 200])));
    setup.scan();
    assert_eq!(album_artwork(&setup.library, "Album"), Some(cover));
}

#[test]
fn an_artist_photo_is_kept_by_name_and_replaced_or_removed() {
    let mut setup = Setup::new();
    add_file(
        &setup.music(),
        "1.flac",
        "silence.flac",
        song("One", "Band", "Album"),
    );
    setup.add_music_source();
    setup.scan();
    let band = setup.library.artists().unwrap()[0].id;
    assert_eq!(setup.library.artist_photo(band).unwrap(), None);
    assert_eq!(setup.library.artist_mbid(band).unwrap(), None);

    let first = setup
        .library
        .set_artist_photo(
            band,
            &png_of(1200, [200, 0, 0]),
            Some("https://example.org/a.jpg"),
        )
        .unwrap();
    assert!(first.starts_with(setup.base.join("data/images/artists")));
    let kept = image::open(&first).unwrap();
    assert_eq!((kept.width(), kept.height()), (800, 800), "bounded");
    assert_eq!(
        setup.library.artist_photo(band).unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        setup.library.artist_photos().unwrap().get(&band),
        Some(&first)
    );

    // A rescan keeps it, and so does deleting the cache.
    let _ = fs::remove_dir_all(setup.cache());
    setup.scan();
    assert_eq!(
        setup.library.artist_photo(band).unwrap(),
        Some(first.clone())
    );

    let second = setup
        .library
        .set_artist_photo(band, &png_of(64, [0, 0, 200]), None)
        .unwrap();
    assert_ne!(first, second);
    assert!(!first.exists(), "the replaced file goes");

    setup.library.clear_artist_photo(band).unwrap();
    assert_eq!(setup.library.artist_photo(band).unwrap(), None);
    assert!(!second.exists());
    assert!(matches!(
        setup.library.set_artist_photo(band, b"not an image", None),
        Err(Error::UnreadableImage(_))
    ));
}

#[test]
fn a_fetched_cover_follows_its_album_through_a_renaming_edit() {
    let mut setup = Setup::new();
    let path = add_file(
        &setup.music(),
        "a.flac",
        "silence.flac",
        song("A", "X", "Bare"),
    );
    setup.add_music_source();
    setup.scan();
    let album = setup.library.albums().unwrap()[0].id;
    let fetched = setup
        .library
        .set_fetched_cover(
            album,
            &png_of(64, [30, 102, 245]),
            &CoverArtId::Release("r-1".into()),
        )
        .unwrap()
        .unwrap();

    let track = setup.library.track_id_for_path(&path).unwrap().unwrap();
    let edit = TagEdit {
        album: FieldEdit::Set("Bare (Remastered)".into()),
        album_artist: FieldEdit::Set("The X".into()),
        ..TagEdit::default()
    };
    let report = setup
        .library
        .write_tags(&[(track, &edit)], &AtomicBool::new(false), |_, _| {})
        .unwrap();
    assert!(report.failed.is_empty());
    let albums = setup.library.albums().unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].title, "Bare (Remastered)");
    assert_eq!(albums[0].artwork, Some(fetched));
    assert!(albums[0].fetched_cover);
}
