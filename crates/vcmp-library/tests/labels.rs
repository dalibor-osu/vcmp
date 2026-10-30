//! Labels: the owner's own names for tracks, kept in the database only, and the label
//! sets agents ask for (review, conflicts, undo). Each test copies the fixtures into a
//! temporary directory; nothing here writes a music file.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use lofty::config::WriteOptions;
use lofty::file::{FileType, TaggedFileExt};
use lofty::tag::{Accessor, Tag, TagExt, TagType};
use tempfile::TempDir;
use vcmp_library::{
    ChangeKind, ChangeSetId, ChangeStatus, Error, FileResult, LabelEdit, LabelId, Library,
    NewLabel, TrackId,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

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

    /// Adds a FLAC file with a title and album, and returns its path.
    fn add(&self, name: &str, title: &str, album: &str) -> PathBuf {
        add_file(&self.music(), name, title, album)
    }

    fn id(&self, path: &Path) -> TrackId {
        self.library.track_id_for_path(path).unwrap().unwrap()
    }

    /// Three tracks: two on "First", one on "Second".
    fn three(&mut self) -> [TrackId; 3] {
        let a = self.add("a.flac", "One", "First");
        let b = self.add("b.flac", "Two", "First");
        let c = self.add("c.flac", "Three", "Second");
        self.scan();
        [self.id(&a), self.id(&b), self.id(&c)]
    }

    fn label(&self, name: &str) -> LabelId {
        self.library.label_named(name).unwrap().unwrap().id
    }

    fn names_on(&self, track: TrackId) -> Vec<String> {
        self.library
            .track_labels_of(track)
            .unwrap()
            .into_iter()
            .map(|label| label.name)
            .collect()
    }

    fn submit(&self, edits: &[(TrackId, LabelEdit)], new: &[NewLabel]) -> ChangeSetId {
        self.library
            .submit_label_changes("Test agent", edits, new)
            .unwrap()
    }

    fn accept(&mut self, id: ChangeSetId) -> vcmp_library::ChangeSet {
        self.library.accept_change_set(id, |_, _| {}).unwrap()
    }

    fn status(&self, id: ChangeSetId) -> ChangeStatus {
        self.library.change_set(id).unwrap().unwrap().summary.status
    }
}

fn add_file(dir: &Path, name: &str, title: &str, album: &str) -> PathBuf {
    let path = dir.join(name);
    fs::copy(Path::new(FIXTURES).join("silence.flac"), &path).unwrap();
    let file = lofty::read_from_path(&path).unwrap();
    let tag_type = match file.file_type() {
        FileType::Wav => TagType::Id3v2,
        _ => file.primary_tag_type(),
    };
    let mut tag = Tag::new(tag_type);
    tag.set_title(title.into());
    tag.set_artist("Artist".into());
    tag.set_album(album.into());
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    path
}

fn add(names: &[&str]) -> LabelEdit {
    LabelEdit {
        add: names.iter().map(|name| name.to_string()).collect(),
        remove: Vec::new(),
    }
}

fn remove(names: &[&str]) -> LabelEdit {
    LabelEdit {
        add: Vec::new(),
        remove: names.iter().map(|name| name.to_string()).collect(),
    }
}

fn new_label(name: &str, colour: &str) -> NewLabel {
    NewLabel {
        name: name.into(),
        colour: colour.into(),
    }
}

#[test]
fn labels_are_made_renamed_recoloured_and_deleted() {
    let mut setup = Setup::new();
    let [a, b, _] = setup.three();
    let library = &setup.library;

    let chill = library.create_label("  chill ", "palette:green").unwrap();
    let ruhig = library.create_label("Ärger", "#a1b2c3").unwrap();
    // Names are unique ignoring case, beyond ASCII too.
    assert!(matches!(
        library.create_label("CHILL", "palette:blue"),
        Err(Error::LabelNameTaken(name)) if name == "chill"
    ));
    assert!(matches!(
        library.create_label("ärger", "palette:blue"),
        Err(Error::LabelNameTaken(_))
    ));
    assert!(matches!(
        library.create_label(" ", "palette:blue"),
        Err(Error::InvalidLabelName(_))
    ));
    assert!(matches!(
        library.create_label(&"x".repeat(41), "palette:blue"),
        Err(Error::InvalidLabelName(_))
    ));
    for colour in ["palette:beige", "#A1B2C3", "#abc", "green"] {
        assert!(matches!(
            library.create_label("Other", colour),
            Err(Error::InvalidLabelColour(_))
        ));
    }

    library.add_label(chill, &[a, b]).unwrap();
    library.add_label(chill, &[a]).unwrap();
    let found = library.label(chill).unwrap().unwrap();
    assert_eq!(
        (
            found.name.as_str(),
            found.colour.as_str(),
            found.track_count
        ),
        ("chill", "palette:green", 2)
    );

    // Renaming to its own name in another case is allowed, to another's is not.
    library.rename_label(chill, "Chill").unwrap();
    assert!(matches!(
        library.rename_label(chill, "ärger"),
        Err(Error::LabelNameTaken(_))
    ));
    library.set_label_colour(chill, "#102030").unwrap();
    let names: Vec<String> = library
        .labels()
        .unwrap()
        .into_iter()
        .map(|label| label.name)
        .collect();
    assert_eq!(names, ["Chill", "Ärger"]);
    assert_eq!(library.label(chill).unwrap().unwrap().colour, "#102030");

    library.remove_label(chill, &[b]).unwrap();
    assert_eq!(setup.names_on(a), ["Chill"]);
    assert!(setup.names_on(b).is_empty());

    setup.library.delete_label(chill).unwrap();
    assert!(setup.library.label(chill).unwrap().is_none());
    assert!(setup.names_on(a).is_empty());
    assert!(setup.library.label_links().unwrap().is_empty());
    assert!(setup.library.label(ruhig).unwrap().is_some());
}

#[test]
fn a_label_stays_on_a_moved_file_and_on_one_that_is_gone() {
    let mut setup = Setup::new();
    let [a, b, _] = setup.three();
    let chill = setup.library.create_label("chill", "palette:teal").unwrap();
    setup.library.add_label(chill, &[a, b]).unwrap();

    let moved = setup.music().join("elsewhere/a.flac");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(setup.music().join("a.flac"), &moved).unwrap();
    fs::rename(setup.music().join("b.flac"), setup.base.join("away.flac")).unwrap();
    setup.scan();

    assert_eq!(setup.id(&moved), a);
    // The file of b is gone: it keeps its label, but counts and lists leave it out.
    assert_eq!(setup.names_on(b), ["chill"]);
    assert_eq!(setup.library.label(chill).unwrap().unwrap().track_count, 1);
    let tracks: Vec<TrackId> = setup
        .library
        .label_tracks(chill)
        .unwrap()
        .into_iter()
        .map(|track| track.id)
        .collect();
    assert_eq!(tracks, [a]);

    fs::rename(setup.base.join("away.flac"), setup.music().join("b.flac")).unwrap();
    setup.scan();
    assert_eq!(setup.library.label(chill).unwrap().unwrap().track_count, 2);
}

#[test]
fn tracks_need_every_label_and_albums_any_track() {
    let mut setup = Setup::new();
    let [a, b, c] = setup.three();
    let chill = setup.library.create_label("chill", "palette:teal").unwrap();
    let rainy = setup.library.create_label("rainy", "palette:blue").unwrap();
    setup.library.add_label(chill, &[a, c]).unwrap();
    setup.library.add_label(rainy, &[a, b]).unwrap();

    let both = setup
        .library
        .tracks_with_labels(&[chill, rainy], 0, 100)
        .unwrap();
    assert_eq!(both, [a]);
    let chilled = setup.library.tracks_with_labels(&[chill], 0, 100).unwrap();
    assert_eq!(chilled, [a, c]);
    assert_eq!(
        setup.library.tracks_with_labels(&[chill], 1, 100).unwrap(),
        [c]
    );

    let first = setup.library.track(a).unwrap().unwrap().album_id.unwrap();
    let second = setup.library.track(c).unwrap().unwrap().album_id.unwrap();
    let albums = setup.library.album_labels().unwrap();
    let mut on_first = albums[&first].clone();
    on_first.sort();
    assert_eq!(on_first, [chill, rainy]);
    assert_eq!(albums[&second], [chill]);

    let links = setup.library.label_links().unwrap();
    assert_eq!(links[&b], [rainy]);
}

#[test]
fn a_label_set_changes_nothing_until_accepted() {
    let mut setup = Setup::new();
    let [a, b, c] = setup.three();
    let rainy = setup.library.create_label("Rainy", "palette:blue").unwrap();
    setup.library.add_label(rainy, &[c]).unwrap();

    let set = setup.submit(
        &[
            (a, add(&["chill", "rainy"])),
            (b, add(&["CHILL"])),
            (c, remove(&["Rainy"])),
        ],
        &[new_label("Chill", "palette:green")],
    );
    let pending = setup.library.change_set(set).unwrap().unwrap();
    assert_eq!(pending.summary.kind, ChangeKind::Labels);
    assert_eq!(pending.summary.status, ChangeStatus::Pending);
    assert_eq!(pending.summary.file_count, 3);
    assert_eq!(pending.new_labels[0].name, "Chill");
    assert_eq!(pending.new_labels[0].created, None);
    // Names are as the labels spell them; the values say whether a track holds one.
    assert_eq!(
        pending.files[0].edits,
        [
            ("label:Chill".to_string(), Some(vec!["Chill".to_string()])),
            ("label:Rainy".to_string(), Some(vec!["Rainy".to_string()])),
        ]
    );
    assert_eq!(pending.files[2].edits, [("label:Rainy".to_string(), None)]);
    assert_eq!(pending.files[2].values[0].before, ["Rainy"]);
    assert!(setup.library.label_named("chill").unwrap().is_none());
    assert_eq!(setup.names_on(c), ["Rainy"]);

    let done = setup.accept(set);
    assert_eq!(done.summary.status, ChangeStatus::Accepted);
    assert!(
        done.files
            .iter()
            .all(|file| file.result == Some(FileResult::Written))
    );
    assert_eq!(done.new_labels[0].created, Some(true));
    assert_eq!(done.files[0].values[0].after, Some(vec!["Chill".into()]));
    assert_eq!(done.files[2].values[0].after, Some(Vec::new()));
    assert_eq!(setup.names_on(a), ["Chill", "Rainy"]);
    assert_eq!(setup.names_on(b), ["Chill"]);
    assert!(setup.names_on(c).is_empty());
    let chill = setup.library.label_named("chill").unwrap().unwrap();
    assert_eq!(chill.colour, "palette:green");
}

#[test]
fn a_rejected_label_set_changes_nothing() {
    let mut setup = Setup::new();
    let [a, _, _] = setup.three();
    let set = setup.submit(
        &[(a, add(&["chill"]))],
        &[new_label("chill", "palette:green")],
    );
    setup.library.reject_change_set(set).unwrap();
    assert_eq!(setup.status(set), ChangeStatus::Rejected);
    assert!(setup.library.labels().unwrap().is_empty());
    assert!(matches!(
        setup.library.accept_change_set(set, |_, _| {}),
        Err(Error::ChangeSetNotPending(_))
    ));
}

#[test]
fn a_track_whose_labels_changed_since_is_left_alone() {
    let mut setup = Setup::new();
    let [a, b, c] = setup.three();
    let chill = setup.library.create_label("chill", "palette:teal").unwrap();
    let rainy = setup.library.create_label("rainy", "palette:blue").unwrap();
    setup.library.add_label(rainy, &[c]).unwrap();
    let set = setup.submit(
        &[
            (a, add(&["chill"])),
            (b, add(&["chill"])),
            (c, remove(&["rainy"])),
        ],
        &[],
    );
    // The owner labels b and takes rainy off c meanwhile.
    setup.library.add_label(chill, &[b]).unwrap();
    setup.library.remove_label(rainy, &[c]).unwrap();

    let done = setup.accept(set);
    assert_eq!(done.summary.status, ChangeStatus::PartlyWritten);
    let results: Vec<_> = done.files.iter().map(|file| file.result.clone()).collect();
    assert_eq!(results[0], Some(FileResult::Written));
    assert!(matches!(results[1], Some(FileResult::Conflict(_))));
    assert!(matches!(results[2], Some(FileResult::Conflict(_))));
    assert_eq!(done.files[1].values[0].after, None);
}

#[test]
fn a_label_deleted_while_the_set_waits_is_a_conflict() {
    let mut setup = Setup::new();
    let [a, _, _] = setup.three();
    let chill = setup.library.create_label("chill", "palette:teal").unwrap();
    let set = setup.submit(&[(a, add(&["chill"]))], &[]);
    setup.library.delete_label(chill).unwrap();

    let done = setup.accept(set);
    assert_eq!(done.summary.status, ChangeStatus::PartlyWritten);
    assert!(matches!(
        &done.files[0].result,
        Some(FileResult::Conflict(detail)) if detail.contains("chill")
    ));
    assert!(setup.library.labels().unwrap().is_empty());
}

#[test]
fn a_new_label_made_meanwhile_is_used_as_it_is() {
    let mut setup = Setup::new();
    let [a, _, _] = setup.three();
    let set = setup.submit(
        &[(a, add(&["chill"]))],
        &[new_label("chill", "palette:green")],
    );
    let mine = setup.library.create_label("Chill", "#123456").unwrap();

    let done = setup.accept(set);
    assert_eq!(done.summary.status, ChangeStatus::Accepted);
    assert_eq!(done.new_labels[0].created, Some(false));
    let labels = setup.library.labels().unwrap();
    assert_eq!(labels.len(), 1);
    assert_eq!((labels[0].id, labels[0].colour.as_str()), (mine, "#123456"));
    assert_eq!(setup.names_on(a), ["Chill"]);
}

#[test]
fn undo_puts_labels_back_and_removes_unused_new_ones() {
    let mut setup = Setup::new();
    let [a, b, c] = setup.three();
    let rainy = setup.library.create_label("rainy", "palette:blue").unwrap();
    setup.library.add_label(rainy, &[c]).unwrap();
    let set = setup.submit(
        &[
            (a, add(&["chill", "calm"])),
            (b, add(&["calm"])),
            (c, remove(&["rainy"])),
        ],
        &[
            new_label("chill", "palette:green"),
            new_label("calm", "palette:sky"),
        ],
    );
    setup.accept(set);
    // The owner puts "calm" on another track, so the undo must keep that label.
    let calm = setup.label("calm");
    setup.library.add_label(calm, &[c]).unwrap();

    let undo = setup.library.submit_undo(set, "Test agent").unwrap();
    let pending = setup.library.change_set(undo).unwrap().unwrap();
    assert_eq!(pending.summary.kind, ChangeKind::Labels);
    assert_eq!(pending.summary.undoes, Some(set));
    // Nothing changes before the undo is accepted.
    assert_eq!(setup.names_on(a), ["calm", "chill"]);

    let done = setup.accept(undo);
    assert_eq!(done.summary.status, ChangeStatus::Accepted);
    assert_eq!(setup.status(set), ChangeStatus::Undone);
    assert!(setup.names_on(a).is_empty());
    assert!(setup.names_on(b).is_empty());
    assert_eq!(setup.names_on(c), ["calm", "rainy"]);
    let names: Vec<String> = setup
        .library
        .labels()
        .unwrap()
        .into_iter()
        .map(|label| label.name)
        .collect();
    assert_eq!(names, ["calm", "rainy"]);
    assert!(matches!(
        setup.library.submit_undo(set, "Test agent"),
        Err(Error::NotUndoable(_))
    ));
}

#[test]
fn an_undo_leaves_a_track_changed_after_the_set_alone() {
    let mut setup = Setup::new();
    let [a, b, _] = setup.three();
    let chill = setup.library.create_label("chill", "palette:teal").unwrap();
    let set = setup.submit(&[(a, add(&["chill"])), (b, add(&["chill"]))], &[]);
    setup.accept(set);
    setup.library.remove_label(chill, &[b]).unwrap();

    let undo = setup.library.submit_undo(set, "Test agent").unwrap();
    let done = setup.accept(undo);
    assert_eq!(done.summary.status, ChangeStatus::PartlyWritten);
    assert!(matches!(
        done.files[1].result,
        Some(FileResult::Conflict(_))
    ));
    // Not every track came back, so the first set isn't marked undone.
    assert_eq!(setup.status(set), ChangeStatus::Accepted);
    assert!(setup.names_on(a).is_empty());
}

#[test]
fn a_label_set_is_checked_when_it_is_made() {
    let mut setup = Setup::new();
    let [a, b, _] = setup.three();
    let chill = setup.library.create_label("chill", "palette:teal").unwrap();
    setup.library.add_label(chill, &[a]).unwrap();
    let library = &setup.library;
    let submit = |edits: &[(TrackId, LabelEdit)], new: &[NewLabel]| {
        library.submit_label_changes("Test agent", edits, new)
    };

    assert!(matches!(
        submit(&[(b, add(&["unknown"]))], &[]),
        Err(Error::UnknownLabel(name)) if name == "unknown"
    ));
    assert!(matches!(
        submit(
            &[(b, add(&["Chill"]))],
            &[new_label("chill", "palette:green")]
        ),
        Err(Error::LabelNameTaken(_))
    ));
    assert!(matches!(
        submit(&[(b, add(&["x"]))], &[new_label("x", "pink")]),
        Err(Error::InvalidLabelColour(_))
    ));
    assert!(matches!(
        submit(&[(b, add(&["x"]))], &[new_label("", "palette:pink")]),
        Err(Error::InvalidLabelName(_))
    ));
    assert!(matches!(
        submit(
            &[(
                b,
                LabelEdit {
                    add: vec!["chill".into()],
                    remove: vec!["CHILL".into()],
                }
            )],
            &[]
        ),
        Err(Error::ContradictoryLabelEdit { .. })
    ));
    assert!(matches!(
        submit(&[(b, add(&["chill"])), (b, add(&["chill"]))], &[]),
        Err(Error::RepeatedTrack(_))
    ));
    assert!(matches!(
        submit(&[(TrackId(9999), add(&["chill"]))], &[]),
        Err(Error::UnknownTrack(_))
    ));
    // a holds chill already and b never had it: nothing would change.
    assert!(matches!(
        submit(&[(a, add(&["chill"])), (b, remove(&["chill"]))], &[]),
        Err(Error::EmptyChangeSet)
    ));

    assert!(matches!(submit(&[], &[]), Err(Error::EmptyChangeSet)));

    // No-op edits are left out, but a new label no track gets is kept.
    let set = submit(
        &[(a, add(&["chill"])), (b, add(&["chill"]))],
        &[new_label("spare", "palette:red")],
    )
    .unwrap();
    let set = setup.library.change_set(set).unwrap().unwrap();
    assert_eq!(set.files.len(), 1);
    assert_eq!(set.files[0].track, b);
    assert_eq!(set.new_labels.len(), 1);
    assert_eq!(set.new_labels[0].name, "spare");
}

#[test]
fn a_set_may_only_make_labels() {
    let mut setup = Setup::new();
    let [a, _, _] = setup.three();
    // Only new labels, and edits that change nothing: the labels are still made.
    let set = setup.submit(
        &[(a, remove(&["focus"]))],
        &[
            new_label("Focus", "palette:sky"),
            new_label("Workout", "#ff8800"),
            new_label("Sleep", "palette:lavender"),
        ],
    );
    let pending = setup.library.change_set(set).unwrap().unwrap();
    assert!(pending.files.is_empty());
    assert_eq!(pending.summary.file_count, 0);
    assert_eq!(pending.new_labels.len(), 3);
    assert!(setup.library.labels().unwrap().is_empty());

    let done = setup.accept(set);
    assert_eq!(done.summary.status, ChangeStatus::Accepted);
    assert!(
        done.new_labels
            .iter()
            .all(|label| label.created == Some(true))
    );
    // Like labels the owner makes by hand: listed, on no track, usable.
    let labels = setup.library.labels().unwrap();
    let names: Vec<&str> = labels.iter().map(|label| label.name.as_str()).collect();
    assert_eq!(names, ["Focus", "Sleep", "Workout"]);
    assert!(labels.iter().all(|label| label.track_count == 0));
    assert_eq!(labels[2].colour, "#ff8800");
    let focus = setup.label("focus");
    setup.library.add_label(focus, &[a]).unwrap();

    // The undo deletes the labels it made, except one a track holds by then.
    let undo = setup.library.submit_undo(set, "Test agent").unwrap();
    let pending = setup.library.change_set(undo).unwrap().unwrap();
    assert!(pending.files.is_empty());
    assert_eq!(pending.summary.undoes, Some(set));
    let done = setup.accept(undo);
    assert_eq!(done.summary.status, ChangeStatus::Accepted);
    assert_eq!(setup.status(set), ChangeStatus::Undone);
    let names: Vec<String> = setup
        .library
        .labels()
        .unwrap()
        .into_iter()
        .map(|label| label.name)
        .collect();
    assert_eq!(names, ["Focus"]);
    assert_eq!(setup.names_on(a), ["Focus"]);
}

#[test]
fn a_set_whose_new_labels_are_all_gone_cannot_be_undone() {
    let mut setup = Setup::new();
    setup.three();
    let set = setup.submit(&[], &[new_label("Party", "palette:red")]);
    setup.accept(set);
    let party = setup.label("party");
    setup.library.delete_label(party).unwrap();
    assert!(matches!(
        setup.library.submit_undo(set, "Test agent"),
        Err(Error::NotUndoable(_))
    ));
}

#[test]
fn a_database_from_version_11_keeps_its_change_sets() {
    let mut setup = Setup::new();
    let [a, _, _] = setup.three();
    let chill = setup.library.create_label("chill", "palette:teal").unwrap();
    let set = setup.submit(&[(a, add(&["chill"]))], &[]);
    setup.accept(set);
    let db = setup.base.join("data/library.sqlite3");
    let reopen = |setup: &mut Setup| {
        setup.library = Library::open(&db, &setup.base.join("cache")).unwrap();
    };

    // Put the database back as version 11 left it: no labels, and change sets that
    // can't be of kind 'labels'.
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "UPDATE change_sets SET kind = 'tags';
         DROP TABLE change_labels; DROP TABLE track_labels; DROP TABLE labels;
         PRAGMA user_version = 11;",
    )
    .unwrap();
    drop(conn);
    reopen(&mut setup);

    // Rebuilding change_sets kept the set and the tracks it changed.
    let kept = setup.library.change_set(set).unwrap().unwrap();
    assert_eq!(kept.summary.status, ChangeStatus::Accepted);
    assert_eq!(kept.files.len(), 1);
    assert!(setup.library.labels().unwrap().is_empty());
    assert!(setup.library.label(chill).unwrap().is_none());
    // Foreign keys are on again: deleting a set takes its files with it.
    setup.library.delete_finished_change_sets().unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    let files: i64 = conn
        .query_row("SELECT COUNT(*) FROM change_files", [], |row| row.get(0))
        .unwrap();
    assert_eq!(files, 0);
}
