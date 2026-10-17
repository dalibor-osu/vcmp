//! Writing tags back to files. Each test copies the fixtures into a temporary
//! directory, tags the copies, scans them and edits them through the library.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};
use tempfile::TempDir;
use vcmp_library::{FieldEdit, InvalidTagEdit, Library, TagEdit, TagWriteError, TrackId};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const FORMATS: [(&str, &str); 4] = [
    ("silence.mp3", "mp3"),
    ("silence.flac", "flac"),
    ("silence.ogg", "ogg"),
    ("silence.wav", "wav"),
];

struct Setup {
    // The database closes before the directory is deleted, which Windows requires.
    library: Library,
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
        library.add_source(&base.join("music"), true).unwrap();
        Self {
            _root: root,
            base,
            library,
        }
    }

    fn music(&self) -> PathBuf {
        self.base.join("music")
    }

    fn scan(&mut self) {
        let report = self.library.scan(&AtomicBool::new(false), |_| {}).unwrap();
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    fn id(&self, path: &Path) -> TrackId {
        self.library.track_id_for_path(path).unwrap().unwrap()
    }

    fn write(&mut self, edits: &[(TrackId, &TagEdit)]) -> vcmp_library::TagWriteReport {
        self.library
            .write_tags(edits, &AtomicBool::new(false), |_, _| {})
            .unwrap()
    }
}

/// Copies a fixture to `dir/name` and gives it a full set of tags, including fields
/// the editor doesn't know and a back cover.
fn add_file(dir: &Path, name: &str, fixture: &str, title: &str, album: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::copy(Path::new(FIXTURES).join(fixture), &path).unwrap();
    let file = lofty::read_from_path(&path).unwrap();
    let mut tag = Tag::new(tag_type_for(&file));
    tag.set_title(title.into());
    tag.set_artist("Artist".into());
    tag.set_album(album.into());
    tag.insert_text(ItemKey::AlbumArtist, "Album Artist".into());
    tag.set_track(3);
    tag.set_track_total(12);
    tag.set_disk(1);
    tag.insert_text(ItemKey::RecordingDate, "1999".into());
    tag.set_genre("Jazz".into());
    tag.insert_text(ItemKey::Composer, "Composer".into());
    tag.set_comment("A comment".into());
    tag.insert_text(
        ItemKey::MusicBrainzRecordingId,
        "0c3a2b0e-8d6a-4c1e-9a43-1f2c3d4e5f60".into(),
    );
    tag.push_picture(picture(PictureType::CoverBack, png([10, 20, 30])));
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    path
}

fn tag_type_for(file: &TaggedFile) -> TagType {
    match file.file_type() {
        FileType::Wav => TagType::Id3v2,
        _ => file.primary_tag_type(),
    }
}

/// The tag VCMP reads and writes in this file.
fn read_tag(path: &Path) -> (Tag, Duration) {
    let mut file = lofty::read_from_path(path).unwrap();
    let tag_type = tag_type_for(&file);
    let duration = file.properties().duration();
    (file.remove(tag_type).unwrap(), duration)
}

fn picture(kind: PictureType, data: Vec<u8>) -> Picture {
    Picture::unchecked(data)
        .pic_type(kind)
        .mime_type(MimeType::Png)
        .build()
}

fn png(colour: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(32, 32, image::Rgb(colour));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn jpeg(colour: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(32, 32, image::Rgb(colour));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
    bytes.into_inner()
}

fn pictures(tag: &Tag) -> Vec<(PictureType, Vec<u8>)> {
    tag.pictures()
        .iter()
        .map(|picture| (picture.pic_type(), picture.data().to_vec()))
        .collect()
}

/// The audio data of a file, without its tags: what an edit must never change.
fn audio_payload(path: &Path) -> Vec<u8> {
    let bytes = fs::read(path).unwrap();
    match path.extension().unwrap().to_str().unwrap() {
        "mp3" => {
            let mut start = 0;
            if bytes.starts_with(b"ID3") {
                let size = bytes[6..10]
                    .iter()
                    .fold(0usize, |size, byte| (size << 7) | usize::from(*byte));
                let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
                start = 10 + size + footer;
            }
            let mut end = bytes.len();
            if end >= 128 && &bytes[end - 128..end - 125] == b"TAG" {
                end -= 128;
            }
            bytes[start..end].to_vec()
        }
        "flac" => {
            assert_eq!(&bytes[..4], b"fLaC");
            let mut payload = Vec::new();
            let mut position = 4;
            loop {
                let header = &bytes[position..position + 4];
                let length = (usize::from(header[1]) << 16)
                    | (usize::from(header[2]) << 8)
                    | usize::from(header[3]);
                // STREAMINFO describes the audio, so it counts as audio here.
                if header[0] & 0x7f == 0 {
                    payload.extend_from_slice(&bytes[position + 4..position + 4 + length]);
                }
                let last = header[0] & 0x80 != 0;
                position += 4 + length;
                if last {
                    payload.extend_from_slice(&bytes[position..]);
                    return payload;
                }
            }
        }
        "wav" => {
            let mut payload = Vec::new();
            let mut position = 12;
            while position + 8 <= bytes.len() {
                let id = &bytes[position..position + 4];
                let length =
                    u32::from_le_bytes(bytes[position + 4..position + 8].try_into().unwrap())
                        as usize;
                if id == b"fmt " || id == b"data" {
                    payload.extend_from_slice(&bytes[position + 8..position + 8 + length]);
                }
                position += 8 + length + length % 2;
            }
            payload
        }
        "ogg" => {
            // Writing comments renumbers the pages, so compare the packets; the
            // second one holds the comments.
            let packets = ogg_pager::Packets::read(&mut std::io::Cursor::new(bytes)).unwrap();
            packets
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != 1)
                .flat_map(|(_, packet)| packet.to_vec())
                .collect()
        }
        other => panic!("no payload reader for {other}"),
    }
}

/// Files in `dir` besides the ones a test made: leftovers of a write.
fn leftovers(dir: &Path, expected: &[&str]) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !expected.contains(&name.as_str()))
        .collect()
}

fn set(value: &str) -> FieldEdit<String> {
    FieldEdit::Set(value.to_owned())
}

#[test]
fn round_trip_every_format_with_unicode_values() {
    for (fixture, extension) in FORMATS {
        let mut setup = Setup::new();
        let dir = setup.music().join("Album");
        let name = format!("track.{extension}");
        let path = add_file(&dir, &name, fixture, "Old title", "Old album");
        setup.scan();
        let id = setup.id(&path);
        let audio_before = audio_payload(&path);
        let (_, duration_before) = read_tag(&path);
        let cover = jpeg([200, 30, 90]);

        let edit = TagEdit {
            title: set("Příliš žluťoučký kůň 🎵"),
            artist: set("  Антонин Дворжак  "),
            album: set("交響曲第9番 «Z Nového světa»"),
            album_artist: set("Orchestre de la Suisse Romande"),
            track_number: FieldEdit::Set(7),
            disc_number: FieldEdit::Set(2),
            year: FieldEdit::Set(2024),
            genre: set("Klasická"),
            cover: FieldEdit::Set(cover.clone()),
        };
        let report = setup.write(&[(id, &edit)]);
        assert_eq!(report.written, [id], "{extension}: {:?}", report.failed);

        let (tag, duration) = read_tag(&path);
        assert_eq!(
            tag.title().as_deref(),
            Some("Příliš žluťoučký kůň 🎵"),
            "{extension}"
        );
        assert_eq!(
            tag.artist().as_deref(),
            Some("Антонин Дворжак"),
            "{extension}"
        );
        assert_eq!(
            tag.album().as_deref(),
            Some("交響曲第9番 «Z Nového světa»"),
            "{extension}"
        );
        assert_eq!(
            tag.get_string(ItemKey::AlbumArtist),
            Some("Orchestre de la Suisse Romande"),
            "{extension}"
        );
        assert_eq!(tag.track(), Some(7), "{extension}");
        assert_eq!(tag.track_total(), Some(12), "{extension}: the total stays");
        assert_eq!(tag.disk(), Some(2), "{extension}");
        assert_eq!(tag.date().map(|date| date.year), Some(2024), "{extension}");
        assert_eq!(tag.genre().as_deref(), Some("Klasická"), "{extension}");

        // Fields the editor doesn't know survive.
        assert_eq!(
            tag.get_string(ItemKey::Composer),
            Some("Composer"),
            "{extension}"
        );
        assert_eq!(tag.comment().as_deref(), Some("A comment"), "{extension}");
        assert_eq!(
            tag.get_string(ItemKey::MusicBrainzRecordingId),
            Some("0c3a2b0e-8d6a-4c1e-9a43-1f2c3d4e5f60"),
            "{extension}"
        );
        let mut found = pictures(&tag);
        found.sort_by_key(|(kind, _)| kind.as_u8());
        assert_eq!(
            found,
            [
                (PictureType::CoverFront, cover.clone()),
                (PictureType::CoverBack, png([10, 20, 30]))
            ],
            "{extension}"
        );

        // The audio is untouched.
        assert_eq!(duration, duration_before, "{extension}");
        assert!(
            audio_payload(&path) == audio_before,
            "{extension}: audio changed"
        );
        assert!(leftovers(&dir, &[&name]).is_empty(), "{extension}");

        // The database follows: track, album, artist, artwork and search.
        let track = setup.library.track(id).unwrap().unwrap();
        assert_eq!(track.title, "Příliš žluťoučký kůň 🎵");
        assert_eq!(track.artist.as_deref(), Some("Антонин Дворжак"));
        assert_eq!(track.track_number, Some(7));
        assert_eq!(track.disc_number, Some(2));
        assert_eq!(track.year, Some(2024));
        assert_eq!(track.genre.as_deref(), Some("Klasická"));
        let album = setup
            .library
            .album(track.album_id.unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(album.title, "交響曲第9番 «Z Nového světa»");
        assert_eq!(
            album.artist.as_deref(),
            Some("Orchestre de la Suisse Romande")
        );
        assert_eq!(album.year, Some(2024));
        let artwork = album.artwork.expect("the new cover becomes the artwork");
        assert_eq!(fs::read(&artwork.original).unwrap(), cover);
        assert_eq!(
            setup.library.albums().unwrap().len(),
            1,
            "old album removed"
        );
        assert!(
            setup
                .library
                .artists()
                .unwrap()
                .iter()
                .all(|artist| artist.name != "Album Artist"),
            "{extension}"
        );
        let results = setup.library.search("dvorzak zlutoucky", 10).unwrap();
        assert!(results.tracks.is_empty(), "words must all match one row");
        let results = setup.library.search("zlutoucky", 10).unwrap();
        assert_eq!(results.tracks.len(), 1, "{extension}");
        assert_eq!(setup.library.embedded_cover(id).unwrap(), Some(cover));
    }
}

#[test]
fn clearing_fields_and_the_cover() {
    for (fixture, extension) in FORMATS {
        let mut setup = Setup::new();
        let dir = setup.music().join("Album");
        let path = add_file(
            &dir,
            &format!("track.{extension}"),
            fixture,
            "Title",
            "Album",
        );
        let id = {
            // Start with a front cover to remove.
            let (mut tag, _) = read_tag(&path);
            tag.push_picture(picture(PictureType::CoverFront, png([1, 2, 3])));
            tag.save_to_path(&path, WriteOptions::default()).unwrap();
            setup.scan();
            setup.id(&path)
        };
        let audio_before = audio_payload(&path);

        let edit = TagEdit {
            title: FieldEdit::Clear,
            artist: FieldEdit::Clear,
            album: FieldEdit::Clear,
            album_artist: FieldEdit::Clear,
            track_number: FieldEdit::Clear,
            disc_number: FieldEdit::Clear,
            year: FieldEdit::Clear,
            // An empty value clears too.
            genre: set("   "),
            cover: FieldEdit::Clear,
        };
        let report = setup.write(&[(id, &edit)]);
        assert_eq!(report.written, [id], "{extension}: {:?}", report.failed);

        let (tag, _) = read_tag(&path);
        assert_eq!(tag.title(), None, "{extension}");
        assert_eq!(tag.artist(), None, "{extension}");
        assert_eq!(tag.album(), None, "{extension}");
        assert_eq!(tag.get_string(ItemKey::AlbumArtist), None, "{extension}");
        assert_eq!(tag.track(), None, "{extension}");
        assert_eq!(tag.track_total(), None, "{extension}");
        assert_eq!(tag.disk(), None, "{extension}");
        assert_eq!(tag.date(), None, "{extension}");
        assert_eq!(tag.genre(), None, "{extension}");
        assert_eq!(
            tag.get_string(ItemKey::Composer),
            Some("Composer"),
            "{extension}"
        );
        assert_eq!(
            pictures(&tag),
            [(PictureType::CoverBack, png([10, 20, 30]))],
            "{extension}: only the back cover stays"
        );
        assert!(
            audio_payload(&path) == audio_before,
            "{extension}: audio changed"
        );

        let track = setup.library.track(id).unwrap().unwrap();
        assert_eq!(
            track.title, "track",
            "the file name stands in for the title"
        );
        assert_eq!(track.album_id, None);
        assert!(setup.library.albums().unwrap().is_empty());
    }
}

#[test]
fn fields_left_at_keep_are_not_touched() {
    for (fixture, extension) in FORMATS {
        let mut setup = Setup::new();
        let path = add_file(
            &setup.music(),
            &format!("t.{extension}"),
            fixture,
            "Title",
            "Album",
        );
        setup.scan();
        let id = setup.id(&path);
        let (before, _) = read_tag(&path);

        let edit = TagEdit {
            genre: set("Blues"),
            ..TagEdit::default()
        };
        let report = setup.write(&[(id, &edit)]);
        assert_eq!(report.written, [id], "{extension}: {:?}", report.failed);

        let (after, _) = read_tag(&path);
        let without_genre = |tag: &Tag| {
            let mut items: Vec<String> = tag
                .items()
                .filter(|item| item.key() != ItemKey::Genre)
                .map(|item| format!("{item:?}"))
                .collect();
            items.sort();
            items
        };
        assert_eq!(without_genre(&after), without_genre(&before), "{extension}");
        assert_eq!(pictures(&after), pictures(&before), "{extension}");
        assert_eq!(after.genre().as_deref(), Some("Blues"), "{extension}");
    }
}

#[test]
fn batch_regroups_the_album_and_reports_the_failing_file() {
    let mut setup = Setup::new();
    let dir = setup.music().join("Album");
    let paths: Vec<PathBuf> = FORMATS
        .iter()
        .enumerate()
        .map(|(index, (fixture, extension))| {
            add_file(
                &dir,
                &format!("{index}.{extension}"),
                fixture,
                "Song",
                "Album",
            )
        })
        .collect();
    setup.scan();
    let ids: Vec<TrackId> = paths.iter().map(|path| setup.id(path)).collect();
    let old_album = setup.library.track(ids[0]).unwrap().unwrap().album_id;

    // The FLAC file is damaged after the scan; its bytes must stay exactly as they are.
    let broken = b"not a flac file any more".to_vec();
    fs::write(&paths[1], &broken).unwrap();

    let edit = TagEdit {
        album_artist: set("New Album Artist"),
        ..TagEdit::default()
    };
    let edits: Vec<(TrackId, &TagEdit)> = ids.iter().map(|&id| (id, &edit)).collect();
    let mut steps = Vec::new();
    let report = setup
        .library
        .write_tags(&edits, &AtomicBool::new(false), |done, total| {
            steps.push((done, total))
        })
        .unwrap();

    assert_eq!(report.written, [ids[0], ids[2], ids[3]]);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].track, ids[1]);
    assert!(
        matches!(report.failed[0].error, TagWriteError::Unreadable(_)),
        "{:?}",
        report.failed[0].error
    );
    assert_eq!(fs::read(&paths[1]).unwrap(), broken);
    assert_eq!(steps.last(), Some(&(4, 4)));

    let new_album = setup.library.track(ids[0]).unwrap().unwrap().album_id;
    assert_ne!(new_album, old_album, "a new album artist makes a new group");
    let album = setup.library.album(new_album.unwrap()).unwrap().unwrap();
    assert_eq!(album.artist.as_deref(), Some("New Album Artist"));
    assert_eq!(album.track_count, 3);
    // The damaged file still sits in the old group until the next scan.
    assert_eq!(
        setup.library.track(ids[1]).unwrap().unwrap().album_id,
        old_album
    );
    assert!(leftovers(&dir, &["0.mp3", "1.flac", "2.ogg", "3.wav"]).is_empty());
}

#[test]
fn a_read_only_file_is_left_alone() {
    let mut setup = Setup::new();
    let path = add_file(
        &setup.music(),
        "song.flac",
        "silence.flac",
        "Title",
        "Album",
    );
    setup.scan();
    let id = setup.id(&path);
    let before = fs::read(&path).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions.clone()).unwrap();

    let edit = TagEdit {
        title: set("New"),
        ..TagEdit::default()
    };
    let report = setup.write(&[(id, &edit)]);
    assert!(report.written.is_empty());
    assert!(matches!(report.failed[0].error, TagWriteError::ReadOnly));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(setup.library.track(id).unwrap().unwrap().title, "Title");
    assert!(leftovers(&setup.music(), &["song.flac"]).is_empty());

    // Windows can't delete a read-only file when the directory is cleaned up.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
}

#[test]
fn an_invalid_edit_touches_no_file() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "song.mp3", "silence.mp3", "Title", "Album");
    setup.scan();
    let id = setup.id(&path);
    let before = fs::read(&path).unwrap();

    let cases = [
        (
            TagEdit {
                title: set("Two\nlines"),
                ..TagEdit::default()
            },
            InvalidTagEdit::ControlCharacter(vcmp_library::TagField::Title),
        ),
        (
            TagEdit {
                year: FieldEdit::Set(10_000),
                ..TagEdit::default()
            },
            InvalidTagEdit::YearOutOfRange,
        ),
        (
            TagEdit {
                cover: FieldEdit::Set(b"GIF89a not a cover".to_vec()),
                ..TagEdit::default()
            },
            InvalidTagEdit::CoverFormat,
        ),
    ];
    let valid = TagEdit {
        title: set("Fine"),
        ..TagEdit::default()
    };
    for (edit, expected) in cases {
        assert_eq!(edit.validate(), Err(expected.clone()));
        let error = setup
            .library
            .write_tags(
                &[(id, &valid), (id, &edit)],
                &AtomicBool::new(false),
                |_, _| {},
            )
            .unwrap_err();
        assert!(
            matches!(&error, vcmp_library::Error::InvalidTagEdit(found) if *found == expected),
            "{error}"
        );
    }
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_missing_file_is_reported() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "song.ogg", "silence.ogg", "Title", "Album");
    setup.scan();
    let id = setup.id(&path);
    fs::remove_file(&path).unwrap();

    let edit = TagEdit {
        title: set("New"),
        ..TagEdit::default()
    };
    let report = setup.write(&[(id, &edit)]);
    assert!(matches!(report.failed[0].error, TagWriteError::Unavailable));
    assert_eq!(report.failed[0].path.as_deref(), Some(path.as_path()));
}

#[test]
fn cancelling_stops_before_the_next_file() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "song.ogg", "silence.ogg", "Title", "Album");
    setup.scan();
    let id = setup.id(&path);
    let before = fs::read(&path).unwrap();
    let edit = TagEdit {
        title: set("New"),
        ..TagEdit::default()
    };
    let report = setup
        .library
        .write_tags(&[(id, &edit)], &AtomicBool::new(true), |_, _| {})
        .unwrap();
    assert!(report.cancelled);
    assert!(report.written.is_empty() && report.failed.is_empty());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn an_id3v23_tag_stays_id3v23() {
    for (fixture, extension) in [("silence.mp3", "mp3"), ("silence.wav", "wav")] {
        let mut setup = Setup::new();
        let path = setup.music().join(format!("song.{extension}"));
        fs::copy(Path::new(FIXTURES).join(fixture), &path).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title("Title".into());
        tag.insert_text(ItemKey::RecordingDate, "1999".into());
        tag.save_to_path(&path, WriteOptions::new().use_id3v23(true))
            .unwrap();
        setup.scan();
        let id = setup.id(&path);

        let edit = TagEdit {
            year: FieldEdit::Set(2001),
            ..TagEdit::default()
        };
        let report = setup.write(&[(id, &edit)]);
        assert_eq!(report.written, [id], "{extension}: {:?}", report.failed);
        let bytes = fs::read(&path).unwrap();
        let has = |header: &[u8]| bytes.windows(header.len()).any(|window| window == header);
        assert!(has(b"ID3\x03\x00"), "{extension}: still ID3v2.3");
        assert!(!has(b"ID3\x04\x00"), "{extension}");
        assert_eq!(setup.library.track(id).unwrap().unwrap().year, Some(2001));
    }
}

#[test]
fn an_id3v23_tag_with_newer_frames_becomes_id3v24() {
    let mut setup = Setup::new();
    let path = setup.music().join("song.mp3");
    fs::copy(Path::new(FIXTURES).join("silence.mp3"), &path).unwrap();
    // Taggers write ID3v2.4 frames such as full release dates into ID3v2.3 tags,
    // which ID3v2.3 itself can only hold as a year.
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title("Title".into());
    tag.insert_text(ItemKey::OriginalReleaseDate, "2016-08-05".into());
    tag.insert_text(ItemKey::ReleaseDate, "2016-08-05".into());
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], b"ID3\x04");
    bytes[3] = 3;
    fs::write(&path, bytes).unwrap();
    setup.scan();
    let id = setup.id(&path);

    let edit = TagEdit {
        genre: set("House"),
        ..TagEdit::default()
    };
    let report = setup.write(&[(id, &edit)]);
    assert_eq!(report.written, [id], "{:?}", report.failed);
    assert_eq!(fs::read(&path).unwrap()[3], 4, "upgraded to keep the dates");
    let (tag, _) = read_tag(&path);
    assert_eq!(
        tag.get_string(ItemKey::OriginalReleaseDate),
        Some("2016-08-05")
    );
    assert_eq!(tag.get_string(ItemKey::ReleaseDate), Some("2016-08-05"));
}

#[test]
fn an_mp3_with_only_id3v1_keeps_its_values() {
    let mut setup = Setup::new();
    let path = setup.music().join("old.mp3");
    // The fixture comes with an empty ID3v2 tag, which would hide the ID3v1 tag.
    fs::write(
        &path,
        audio_payload(&Path::new(FIXTURES).join("silence.mp3")),
    )
    .unwrap();
    let mut tag = Tag::new(TagType::Id3v1);
    tag.set_title("Old title".into());
    tag.set_artist("Old artist".into());
    tag.set_album("Old album".into());
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    setup.scan();
    let id = setup.id(&path);
    assert_eq!(setup.library.track(id).unwrap().unwrap().title, "Old title");

    let edit = TagEdit {
        genre: set("Rock"),
        ..TagEdit::default()
    };
    let report = setup.write(&[(id, &edit)]);
    assert_eq!(report.written, [id], "{:?}", report.failed);
    let track = setup.library.track(id).unwrap().unwrap();
    assert_eq!(track.title, "Old title");
    assert_eq!(track.artist.as_deref(), Some("Old artist"));
    assert_eq!(track.album.as_deref(), Some("Old album"));
    assert_eq!(track.genre.as_deref(), Some("Rock"));
    let file = lofty::read_from_path(&path).unwrap();
    assert!(file.tag(TagType::Id3v1).is_some(), "the ID3v1 tag is kept");
}

#[test]
fn an_untyped_picture_counts_as_the_cover() {
    let mut setup = Setup::new();
    let path = setup.music().join("song.flac");
    fs::copy(Path::new(FIXTURES).join("silence.flac"), &path).unwrap();
    let mut tag = Tag::new(TagType::VorbisComments);
    tag.set_title("Title".into());
    tag.push_picture(picture(PictureType::Other, png([9, 9, 9])));
    tag.push_picture(picture(PictureType::Artist, png([8, 8, 8])));
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    setup.scan();
    let id = setup.id(&path);

    let cover = png([7, 7, 7]);
    let edit = TagEdit {
        cover: FieldEdit::Set(cover.clone()),
        ..TagEdit::default()
    };
    assert_eq!(setup.write(&[(id, &edit)]).written, [id]);
    let (tag, _) = read_tag(&path);
    let mut found = pictures(&tag);
    found.sort_by_key(|(kind, _)| kind.as_u8());
    assert_eq!(
        found,
        [
            (PictureType::CoverFront, cover),
            (PictureType::Artist, png([8, 8, 8]))
        ]
    );
}

#[test]
fn an_edit_in_another_case_renames_the_album_and_artist() {
    let mut setup = Setup::new();
    let dir = setup.music().join("Album");
    let paths: Vec<PathBuf> = ["a.ogg", "b.ogg"]
        .iter()
        .map(|name| add_file(&dir, name, "silence.ogg", "Song", "the wall"))
        .collect();
    setup.scan();
    let ids: Vec<TrackId> = paths.iter().map(|path| setup.id(path)).collect();

    let edit = TagEdit {
        album: set("The Wall"),
        album_artist: set("ALBUM ARTIST"),
        ..TagEdit::default()
    };
    let report = setup.write(&[(ids[0], &edit), (ids[1], &edit)]);
    assert_eq!(report.written.len(), 2);
    let albums = setup.library.albums().unwrap();
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].title, "The Wall");
    assert_eq!(albums[0].artist.as_deref(), Some("ALBUM ARTIST"));
}
