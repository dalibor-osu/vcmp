//! Change sets: recording what agents ask for, writing it once accepted, conflicts,
//! undo, and how long sets are kept. Each test copies the fixtures into a temporary
//! directory and tags the copies.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};
use tempfile::TempDir;
use vcmp_library::{
    ChangeKind, ChangeSetId, ChangeStatus, Error, FieldEdit, FileResult, Library, TagEdit, TrackId,
};

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

    fn undo_dir(&self) -> PathBuf {
        self.base.join("data/images/undo")
    }

    fn scan(&mut self) {
        let report = self.library.scan(&AtomicBool::new(false), |_| {}).unwrap();
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    fn id(&self, path: &Path) -> TrackId {
        self.library.track_id_for_path(path).unwrap().unwrap()
    }

    fn stored(&self, id: TrackId, key: &str) -> Vec<String> {
        self.library
            .track_tags(id)
            .unwrap()
            .into_iter()
            .filter(|entry| entry.key == key)
            .map(|entry| entry.value)
            .collect()
    }

    fn submit(&mut self, edits: &[(TrackId, TagEdit)]) -> ChangeSetId {
        self.library
            .submit_tag_changes("Test agent", edits)
            .unwrap()
    }

    fn accept(&mut self, id: ChangeSetId) -> vcmp_library::ChangeSet {
        self.library.accept_change_set(id, |_, _| {}).unwrap()
    }

    fn status(&self, id: ChangeSetId) -> ChangeStatus {
        self.library.change_set(id).unwrap().unwrap().summary.status
    }

    /// Edits a file the way another program would: VCMP's own writer, not a set.
    fn edit_elsewhere(&mut self, id: TrackId, edit: &TagEdit) {
        let report = self
            .library
            .write_tags(&[(id, edit)], &AtomicBool::new(false), |_, _| {})
            .unwrap();
        assert_eq!(report.written, [id], "{:?}", report.failed);
    }
}

/// Copies a fixture to `dir/name` and tags it with a title, an artist, an album, a
/// composer and a front cover.
fn add_file(dir: &Path, name: &str, fixture: &str, title: &str) -> PathBuf {
    let path = dir.join(name);
    fs::copy(Path::new(FIXTURES).join(fixture), &path).unwrap();
    let file = lofty::read_from_path(&path).unwrap();
    let tag_type = match file.file_type() {
        FileType::Wav => TagType::Id3v2,
        _ => file.primary_tag_type(),
    };
    let mut tag = Tag::new(tag_type);
    tag.set_title(title.into());
    tag.set_artist("Artist".into());
    tag.set_album("Album".into());
    tag.insert_text(ItemKey::Composer, "Writer".into());
    tag.push_picture(
        Picture::unchecked(png([10, 20, 30]))
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Png)
            .build(),
    );
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    path
}

fn png(colour: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(16, 16, image::Rgb(colour));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn cover(path: &Path) -> Option<Vec<u8>> {
    let file = lofty::read_from_path(path).unwrap();
    file.tags()
        .iter()
        .flat_map(|tag| tag.pictures())
        .find(|picture| picture.pic_type() == PictureType::CoverFront)
        .map(|picture| picture.data().to_vec())
}

fn set(value: &str) -> FieldEdit<String> {
    FieldEdit::Set(value.to_owned())
}

fn values(values: &[&str]) -> FieldEdit<Vec<String>> {
    FieldEdit::Set(values.iter().map(|value| value.to_string()).collect())
}

fn title(value: &str) -> TagEdit {
    TagEdit {
        title: set(value),
        ..TagEdit::default()
    }
}

#[test]
fn a_set_is_written_only_once_accepted() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "a.flac", "silence.flac", "Old");
    setup.scan();
    let id = setup.id(&path);
    let before = fs::read(&path).unwrap();

    let edit = TagEdit {
        title: set("New"),
        other: [("Composer".to_owned(), values(&["Someone else"]))].into(),
        ..TagEdit::default()
    };
    let set_id = setup.submit(&[(id, edit)]);
    assert_eq!(fs::read(&path).unwrap(), before, "nothing is written yet");

    let pending = setup.library.change_set(set_id).unwrap().unwrap();
    assert_eq!(pending.summary.status, ChangeStatus::Pending);
    assert_eq!(pending.summary.agent, "Test agent");
    assert_eq!(pending.summary.kind, ChangeKind::Tags);
    let file = &pending.files[0];
    assert_eq!(file.track, id);
    assert_eq!(file.result, None);
    assert_eq!(
        file.edits,
        [
            ("title".to_owned(), Some(vec!["New".to_owned()])),
            (
                "tag:Composer".to_owned(),
                Some(vec!["Someone else".to_owned()])
            ),
        ]
    );
    let before_of = |key: &str| {
        file.values
            .iter()
            .find(|value| value.key == key)
            .map(|value| value.before.clone())
    };
    assert_eq!(before_of("TrackTitle"), Some(vec!["Old".to_owned()]));
    assert_eq!(before_of("Composer"), Some(vec!["Writer".to_owned()]));

    let accepted = setup.accept(set_id);
    assert_eq!(accepted.summary.status, ChangeStatus::Accepted);
    assert!(accepted.summary.finished.is_some());
    assert_eq!(accepted.files[0].result, Some(FileResult::Written));
    let after_of = |key: &str| {
        accepted.files[0]
            .values
            .iter()
            .find(|value| value.key == key)
            .and_then(|value| value.after.clone())
    };
    assert_eq!(after_of("TrackTitle"), Some(vec!["New".to_owned()]));
    assert_eq!(setup.library.track(id).unwrap().unwrap().title, "New");
    assert_eq!(setup.stored(id, "Composer"), ["Someone else"]);

    let error = setup
        .library
        .accept_change_set(set_id, |_, _| {})
        .unwrap_err();
    assert!(matches!(error, Error::ChangeSetNotPending(found) if found == set_id));
}

#[test]
fn a_rejected_set_writes_nothing() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "a.mp3", "silence.mp3", "Old");
    setup.scan();
    let id = setup.id(&path);
    let before = fs::read(&path).unwrap();

    let set_id = setup.submit(&[(id, title("New"))]);
    setup.library.reject_change_set(set_id).unwrap();
    assert_eq!(setup.status(set_id), ChangeStatus::Rejected);
    assert!(setup.library.accept_change_set(set_id, |_, _| {}).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_file_changed_since_the_set_was_made_is_left_alone() {
    let mut setup = Setup::new();
    let a = add_file(&setup.music(), "a.ogg", "silence.ogg", "A");
    let b = add_file(&setup.music(), "b.ogg", "silence.ogg", "B");
    setup.scan();
    let (a, b) = (setup.id(&a), setup.id(&b));

    let set_id = setup.submit(&[(a, title("Agent A")), (b, title("Agent B"))]);
    setup.edit_elsewhere(a, &title("Owner A"));

    let accepted = setup.accept(set_id);
    assert_eq!(accepted.summary.status, ChangeStatus::PartlyWritten);
    assert!(
        matches!(&accepted.files[0].result, Some(FileResult::Conflict(detail)) if detail.contains("TrackTitle")),
        "{:?}",
        accepted.files[0].result
    );
    assert_eq!(accepted.files[1].result, Some(FileResult::Written));
    assert_eq!(setup.library.track(a).unwrap().unwrap().title, "Owner A");
    assert_eq!(setup.library.track(b).unwrap().unwrap().title, "Agent B");
}

#[test]
fn undo_puts_back_every_value_as_it_was() {
    for (fixture, extension) in FORMATS {
        let mut setup = Setup::new();
        let path = add_file(&setup.music(), &format!("a.{extension}"), fixture, "Title");
        let original_cover = cover(&path).unwrap();
        setup.scan();
        let id = setup.id(&path);
        let entries_before = setup.library.track_tags(id).unwrap();

        let edit = TagEdit {
            title: set("New title"),
            artist: set("New artist"),
            year: FieldEdit::Set(2001),
            cover: FieldEdit::Set(png([200, 0, 0])),
            other: [
                ("Composer".to_owned(), FieldEdit::Clear),
                ("Label".to_owned(), values(&["New label"])),
            ]
            .into(),
            ..TagEdit::default()
        };
        let set_id = setup.submit(&[(id, edit)]);
        let accepted = setup.accept(set_id);
        assert_eq!(
            accepted.files[0].result,
            Some(FileResult::Written),
            "{extension}"
        );
        assert_eq!(cover(&path), Some(png([200, 0, 0])), "{extension}");

        let undo = setup.library.submit_undo(set_id, "Test agent").unwrap();
        let pending = setup.library.change_set(undo).unwrap().unwrap();
        assert_eq!(pending.summary.undoes, Some(set_id));
        let undone = setup.accept(undo);
        assert_eq!(
            undone.files[0].result,
            Some(FileResult::Written),
            "{extension}: {:?}",
            undone.files[0].result
        );
        assert_eq!(setup.status(set_id), ChangeStatus::Undone, "{extension}");

        let mut entries_after = setup.library.track_tags(id).unwrap();
        let mut entries_before = entries_before.clone();
        let order = |entries: &mut Vec<vcmp_library::TagEntry>| {
            entries.sort_by(|a, b| a.key.cmp(&b.key).then(a.value.cmp(&b.value)));
        };
        order(&mut entries_after);
        order(&mut entries_before);
        assert_eq!(entries_after, entries_before, "{extension}");
        assert_eq!(cover(&path), Some(original_cover), "{extension}");
    }
}

#[test]
fn undo_restores_several_values_and_a_full_date() {
    let mut setup = Setup::new();
    let path = setup.music().join("a.flac");
    fs::copy(Path::new(FIXTURES).join("silence.flac"), &path).unwrap();
    let mut comments =
        FlacFile::read_from(&mut fs::File::open(&path).unwrap(), ParseOptions::new())
            .unwrap()
            .vorbis_comments()
            .cloned()
            .unwrap_or_default();
    for (field, value) in [
        ("TITLE", "Title"),
        ("ARTIST", "First"),
        ("ARTIST", "Second"),
        ("DATE", "2012-12-26"),
    ] {
        comments.push(field.into(), value.into());
    }
    comments
        .save_to_path(&path, WriteOptions::default())
        .unwrap();
    setup.scan();
    let id = setup.id(&path);

    let edit = TagEdit {
        artist: set("One artist"),
        year: FieldEdit::Set(2001),
        ..TagEdit::default()
    };
    let set_id = setup.submit(&[(id, edit)]);
    setup.accept(set_id);
    assert_eq!(setup.stored(id, "TrackArtist"), ["One artist"]);
    assert_eq!(setup.stored(id, "RecordingDate"), ["2001"]);

    let undo = setup.library.submit_undo(set_id, "Test agent").unwrap();
    setup.accept(undo);
    assert_eq!(setup.stored(id, "TrackArtist"), ["First", "Second"]);
    assert_eq!(setup.stored(id, "RecordingDate"), ["2012-12-26"]);
    let track = setup.library.track(id).unwrap().unwrap();
    assert_eq!(track.artist.as_deref(), Some("First, Second"));
    assert_eq!(track.year, Some(2012));
}

#[test]
fn an_undo_leaves_a_file_changed_after_the_set_alone() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "a.mp3", "silence.mp3", "Old");
    setup.scan();
    let id = setup.id(&path);

    let set_id = setup.submit(&[(id, title("Agent"))]);
    setup.accept(set_id);
    setup.edit_elsewhere(id, &title("Owner"));

    let undo = setup.library.submit_undo(set_id, "Test agent").unwrap();
    let undone = setup.accept(undo);
    assert!(matches!(
        undone.files[0].result,
        Some(FileResult::Conflict(_))
    ));
    assert_eq!(undone.summary.status, ChangeStatus::PartlyWritten);
    assert_eq!(setup.status(set_id), ChangeStatus::Accepted);
    assert_eq!(setup.library.track(id).unwrap().unwrap().title, "Owner");
}

#[test]
fn a_set_is_undone_once() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "a.flac", "silence.flac", "Old");
    setup.scan();
    let id = setup.id(&path);

    let pending = setup.submit(&[(id, title("New"))]);
    assert!(matches!(
        setup.library.submit_undo(pending, "Test agent"),
        Err(Error::NotUndoable(_))
    ));
    setup.accept(pending);
    let undo = setup.library.submit_undo(pending, "Test agent").unwrap();
    assert!(
        matches!(
            setup.library.submit_undo(pending, "Test agent"),
            Err(Error::NotUndoable(_))
        ),
        "an undo is already waiting"
    );
    setup.accept(undo);
    assert!(matches!(
        setup.library.submit_undo(pending, "Test agent"),
        Err(Error::NotUndoable(_))
    ));
    assert_eq!(setup.library.track(id).unwrap().unwrap().title, "Old");
}

#[test]
fn a_set_is_checked_when_it_is_made() {
    let mut setup = Setup::new();
    let flac = add_file(&setup.music(), "a.flac", "silence.flac", "A");
    let gone = add_file(&setup.music(), "b.flac", "silence.flac", "B");
    setup.scan();
    let (flac, gone_id) = (setup.id(&flac), setup.id(&gone));
    fs::remove_file(&gone).unwrap();
    setup.scan();

    let library = &mut setup.library;
    assert!(matches!(
        library.submit_tag_changes("Agent", &[]),
        Err(Error::EmptyChangeSet)
    ));
    assert!(matches!(
        library.submit_tag_changes("Agent", &[(TrackId(999), title("x"))]),
        Err(Error::UnknownTrack(TrackId(999)))
    ));
    assert!(matches!(
        library.submit_tag_changes("Agent", &[(gone_id, title("x"))]),
        Err(Error::UnavailableTrack(found)) if found == gone_id
    ));
    assert!(matches!(
        library.submit_tag_changes("Agent", &[(flac, title("x")), (flac, title("y"))]),
        Err(Error::RepeatedTrack(found)) if found == flac
    ));
    let id3_key = TagEdit {
        other: [("id3:TXXX:X".to_owned(), values(&["x"]))].into(),
        ..TagEdit::default()
    };
    assert!(matches!(
        library.submit_tag_changes("Agent", &[(flac, id3_key)]),
        Err(Error::UnsupportedKey { track, ref key }) if track == flac && key == "id3:TXXX:X"
    ));
    assert!(matches!(
        library.submit_tag_changes("Agent", &[(flac, title("Two\nlines"))]),
        Err(Error::InvalidTagEdit(_))
    ));
    assert!(library.change_sets().unwrap().is_empty());
}

#[test]
fn sets_left_pending_are_discarded() {
    let mut setup = Setup::new();
    let path = add_file(&setup.music(), "a.flac", "silence.flac", "A");
    setup.scan();
    let id = setup.id(&path);
    let set_id = setup.submit(&[(id, title("x"))]);

    assert_eq!(setup.library.discard_pending_change_sets().unwrap(), 1);
    let set = setup.library.change_set(set_id).unwrap().unwrap();
    assert_eq!(set.summary.status, ChangeStatus::Discarded);
    assert!(set.summary.finished.is_some());
}

#[test]
fn finished_sets_expire_after_30_days_with_their_covers() {
    let mut setup = Setup::new();
    let a = add_file(&setup.music(), "a.flac", "silence.flac", "A");
    let b = add_file(&setup.music(), "b.flac", "silence.flac", "B");
    setup.scan();
    let (a, b) = (setup.id(&a), setup.id(&b));

    let with_cover = TagEdit {
        cover: FieldEdit::Set(png([0, 200, 0])),
        ..TagEdit::default()
    };
    let old = setup.submit(&[(a, with_cover)]);
    setup.accept(old);
    let recent = setup.submit(&[(b, title("Recent"))]);
    setup.accept(recent);
    let pending = setup.submit(&[(b, title("Pending"))]);
    // The old cover and the new one are kept for the accepted set.
    assert_eq!(fs::read_dir(setup.undo_dir()).unwrap().count(), 2);

    let db = setup.base.join("data/library.sqlite3");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let month_ago = vcmp_library::CHANGE_SETS_KEPT + Duration::from_secs(60);
    conn.execute(
        "UPDATE change_sets SET finished_ms = finished_ms - ?2 WHERE id = ?1",
        rusqlite::params![old.0, month_ago.as_millis() as i64],
    )
    .unwrap();
    drop(conn);

    setup.library.expire_change_sets().unwrap();
    assert!(setup.library.change_set(old).unwrap().is_none());
    assert!(setup.library.change_set(recent).unwrap().is_some());
    assert_eq!(setup.status(pending), ChangeStatus::Pending);
    assert_eq!(fs::read_dir(setup.undo_dir()).unwrap().count(), 0);
    assert!(matches!(
        setup.library.submit_undo(old, "Agent"),
        Err(Error::UnknownChangeSet(_))
    ));
}

#[test]
fn deleting_finished_sets_frees_their_covers() {
    let mut setup = Setup::new();
    let a = add_file(&setup.music(), "a.mp3", "silence.mp3", "A");
    let b = add_file(&setup.music(), "b.mp3", "silence.mp3", "B");
    setup.scan();
    let (a, b) = (setup.id(&a), setup.id(&b));

    let finished = setup.submit(&[(
        a,
        TagEdit {
            cover: FieldEdit::Set(png([0, 0, 200])),
            ..TagEdit::default()
        },
    )]);
    setup.accept(finished);
    let covers: u64 = fs::read_dir(setup.undo_dir())
        .unwrap()
        .map(|file| file.unwrap().metadata().unwrap().len())
        .sum();
    assert!(covers > 0);
    assert_eq!(setup.library.finished_change_sets_size().unwrap(), covers);

    let pending = setup.submit(&[(b, title("Waiting"))]);
    assert_eq!(setup.library.delete_finished_change_sets().unwrap(), covers);
    assert!(setup.library.change_set(finished).unwrap().is_none());
    assert_eq!(setup.status(pending), ChangeStatus::Pending);
    assert_eq!(fs::read_dir(setup.undo_dir()).unwrap().count(), 0);
    assert_eq!(setup.library.finished_change_sets_size().unwrap(), 0);
}

#[test]
fn a_playlist_deletion_waits_for_review_and_cannot_be_undone() {
    let mut setup = Setup::new();
    let playlist = setup.library.create_playlist("Road trip").unwrap();
    let set_id = setup
        .library
        .submit_playlist_deletion("Test agent", playlist)
        .unwrap();
    assert!(setup.library.playlist(playlist).unwrap().is_some());
    let pending = setup.library.change_set(set_id).unwrap().unwrap();
    assert_eq!(
        pending.summary.kind,
        ChangeKind::PlaylistDeletion {
            playlist,
            name: "Road trip".into()
        }
    );

    let accepted = setup.accept(set_id);
    assert_eq!(accepted.summary.status, ChangeStatus::Accepted);
    assert!(setup.library.playlist(playlist).unwrap().is_none());
    assert!(matches!(
        setup.library.submit_undo(set_id, "Test agent"),
        Err(Error::NotUndoable(_))
    ));
}
