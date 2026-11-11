use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use lofty::config::WriteOptions;
use lofty::file::{FileType, TaggedFileExt};
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};
use tempfile::TempDir;
use vcmp_library::{FeedCursor, FeedTrack, Format, Library};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Setup {
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
            library,
            base,
            _root: root,
        }
    }

    fn music(&self) -> PathBuf {
        self.base.join("music")
    }

    fn scan(&mut self) {
        let report = self.library.scan(&AtomicBool::new(false), |_| {}).unwrap();
        assert!(!report.cancelled);
    }

    fn changes_after(&self, after: FeedCursor) -> Vec<FeedTrack> {
        let mut all = Vec::new();
        let mut cursor = after;
        loop {
            let page = self.library.track_feed(cursor, 2).unwrap();
            let Some(last) = page.last() else {
                return all;
            };
            cursor = last.cursor;
            all.extend(page);
        }
    }
}

fn add_song(dir: &Path, name: &str, fixture: &str, title: &str, album: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::copy(Path::new(FIXTURES).join(fixture), &path).unwrap();
    tag_song(&path, title, album, None);
    path
}

fn tag_song(path: &Path, title: &str, album: &str, release: Option<&str>) {
    let file = lofty::read_from_path(path).unwrap();
    let tag_type = match file.file_type() {
        FileType::Wav | FileType::Aiff => TagType::Id3v2,
        _ => file.primary_tag_type(),
    };
    let mut tag = Tag::new(tag_type);
    tag.set_title(title.into());
    tag.set_artist("Artist".into());
    tag.set_album(album.into());
    tag.set_track(1);
    if let Some(release) = release {
        tag.insert_text(ItemKey::MusicBrainzReleaseId, release.into());
    }
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

fn png() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(32, 32, image::Rgb([245, 194, 231]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn titles(tracks: &[FeedTrack]) -> Vec<&str> {
    tracks.iter().map(|track| track.title.as_str()).collect()
}

#[test]
fn the_feed_lists_every_track_once_and_an_unchanged_rescan_adds_nothing() {
    let mut setup = Setup::new();
    let album = setup.music().join("Album");
    add_song(&album, "1.flac", "silence.flac", "One", "Album");
    add_song(&album, "2.mp3", "silence.mp3", "Two", "Album");
    add_song(&album, "3.ogg", "silence.ogg", "Three", "Album");
    setup.scan();

    let all = setup.changes_after(FeedCursor::default());
    let mut listed = titles(&all);
    listed.sort();
    assert_eq!(listed, ["One", "Three", "Two"]);
    let cursors: Vec<FeedCursor> = all.iter().map(|track| track.cursor).collect();
    let mut sorted = cursors.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        cursors, sorted,
        "pages come in cursor order, each track once"
    );

    let head = setup.library.feed_head().unwrap();
    assert_eq!(head, *cursors.last().unwrap());
    let one = all.iter().find(|track| track.title == "One").unwrap();
    assert!(one.available);
    assert_eq!(one.format, Some(Format::Flac));
    assert_eq!(one.artist.as_deref(), Some("Artist"));
    assert_eq!(one.album.as_deref(), Some("Album"));
    assert_eq!(one.track_number, Some(1));
    assert_eq!(one.path, album.join("1.flac"));
    assert!(one.source_id.is_some());
    assert!(one.file_size > 0);
    assert!(one.duration > Duration::ZERO);

    setup.scan();
    assert!(setup.changes_after(head).is_empty());
    assert_eq!(setup.library.feed_head().unwrap(), head);
}

#[test]
fn a_changed_file_shows_again_with_its_new_tags() {
    let mut setup = Setup::new();
    let album = setup.music().join("Album");
    let one = add_song(&album, "1.flac", "silence.flac", "One", "Album");
    add_song(&album, "2.flac", "silence.flac", "Two", "Album");
    setup.scan();
    let head = setup.library.feed_head().unwrap();

    tag_song(
        &one,
        "Uno",
        "Album",
        Some("8d4e2bd0-d4a4-4d4c-8e48-8d2a6f5e4b11"),
    );
    setup.scan();
    let changed = setup.changes_after(head);
    assert_eq!(titles(&changed), ["Uno"]);
    assert_eq!(
        changed[0].mb_release_id.as_deref(),
        Some("8d4e2bd0-d4a4-4d4c-8e48-8d2a6f5e4b11")
    );
    assert!(changed[0].cursor > head);
}

#[test]
fn a_file_that_goes_shows_as_unavailable_and_again_when_it_is_back() {
    let mut setup = Setup::new();
    let album = setup.music().join("Album");
    let one = add_song(&album, "1.flac", "silence.flac", "One", "Album");
    add_song(&album, "2.flac", "silence.flac", "Two", "Album");
    setup.scan();
    let head = setup.library.feed_head().unwrap();

    let aside = setup.base.join("aside.flac");
    fs::rename(&one, &aside).unwrap();
    setup.scan();
    let gone = setup.changes_after(head);
    assert_eq!(titles(&gone), ["One"]);
    assert!(!gone[0].available);

    let head = setup.library.feed_head().unwrap();
    fs::rename(&aside, &one).unwrap();
    setup.scan();
    let back = setup.changes_after(head);
    assert_eq!(titles(&back), ["One"]);
    assert!(back[0].available);
}

#[test]
fn new_folder_art_shows_every_track_of_the_album_again() {
    let mut setup = Setup::new();
    let album = setup.music().join("Album");
    add_song(&album, "1.flac", "silence.flac", "One", "Album");
    add_song(&album, "2.flac", "silence.flac", "Two", "Album");
    add_song(
        &setup.music().join("Other"),
        "3.flac",
        "silence.flac",
        "Three",
        "Other",
    );
    setup.scan();
    let head = setup.library.feed_head().unwrap();
    assert!(
        setup
            .changes_after(FeedCursor::default())
            .iter()
            .all(|track| track.artwork.is_none())
    );

    fs::write(album.join("cover.png"), png()).unwrap();
    setup.scan();
    let changed = setup.changes_after(head);
    let mut listed = titles(&changed);
    listed.sort();
    assert_eq!(listed, ["One", "Two"]);
    let key = changed[0].artwork.clone().expect("the album's artwork key");
    assert_eq!(changed[1].artwork.as_deref(), Some(key.as_str()));

    let artwork = setup.library.artwork_by_key(&key).unwrap();
    assert!(artwork.original.is_file());
    assert!(artwork.thumbnail.is_file());
}

#[test]
fn artwork_keys_that_are_not_keys_are_refused() {
    let setup = Setup::new();
    for key in [
        "",
        "..",
        "../library.sqlite3",
        "abc/def.png",
        "abc.PNG",
        "abc",
        ".png",
        "zz.png",
        "0123456789abcdef.png",
    ] {
        assert_eq!(setup.library.artwork_by_key(key), None, "{key:?}");
    }
}
