//! The tag editor: one track, an album with differing values, wrong values, a file
//! that can't be written, and the cover. Files are copies in a temporary directory.

mod common;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot, mock_elapsed_time};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use vcmp_app::library_service::{Delivery, LibraryPaths};
use vcmp_app::library_view::LibraryView;
use vcmp_app::tag_editor::TagEditor;
use vcmp_app::{AppWindow, TagEditorState, TagEditorStatus, TagFailureReason, TagProblem};
use vcmp_library::{Library, Track};

struct App {
    _dir: tempfile::TempDir,
    music: PathBuf,
    library: Rc<Library>,
    window: AppWindow,
    view: Rc<LibraryView>,
    editor: Rc<TagEditor>,
}

/// An album of two songs from different years, and a single.
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(
        &music,
        "1.flac",
        Song {
            track: 1,
            ..Song::new("One", "Band", "Album")
        },
    );
    add(
        &music,
        "2.flac",
        Song {
            track: 2,
            ..Song::new("Two", "Band", "Album")
        },
    );
    add(&music, "single.flac", Song::new("Single", "Solo", "Lone"));
    let paths = LibraryPaths {
        database: dir.path().join("library.sqlite3"),
        cache: dir.path().join("cache"),
    };
    let mut library = Library::open(&paths.database, &paths.cache).unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    let library = Rc::new(library);

    let window = vcmp_app::create_window().unwrap();
    let view = LibraryView::connect(&window, library.clone()).unwrap();
    let editor = TagEditor::connect(
        &window,
        library.clone(),
        paths,
        view.clone(),
        Delivery::Manual,
    );
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 1200.0));
    mock_elapsed_time(Duration::ZERO);
    App {
        _dir: dir,
        music,
        library,
        window,
        view,
        editor,
    }
}

fn key(window: &AppWindow, key: Key) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn png() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(32, 32, image::Rgb([30, 102, 245]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

impl App {
    fn state(&self) -> TagEditorState<'_> {
        self.window.global::<TagEditorState>()
    }

    fn track(&self, title: &str) -> Track {
        self.library
            .tracks()
            .unwrap()
            .into_iter()
            .find(|track| track.title == title)
            .unwrap_or_else(|| panic!("no track {title:?}"))
    }

    fn type_into(&self, label: &str, text: &str) {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(AccessibleRole::TextInput))
            .unwrap_or_else(|| panic!("no field {label:?}"))
            .set_accessible_value(text);
    }

    fn open_album(&self) {
        let album = self.track("One").album_id.unwrap();
        self.view.open_album_by_id(album);
        // The page takes the focus from a change handler, which the event loop runs.
        mock_elapsed_time(Duration::ZERO);
    }

    fn save(&self) {
        key(&self.window, Key::Return);
        assert!(self.editor.wait_until_written(Duration::from_secs(10)));
    }
}

#[test]
fn one_track_gets_its_new_title_everywhere() {
    let app = start();
    app.open_album();
    let path = app.music.join("1.flac");
    app.state()
        .invoke_edit_track(path.to_string_lossy().as_ref().into());
    assert!(app.state().get_open());
    assert_eq!(app.state().get_track_count(), 1);
    let values = app.state().get_values();
    assert_eq!(
        (
            values.title.as_str(),
            values.artist.as_str(),
            values.track.as_str()
        ),
        ("One", "Band", "1")
    );

    app.type_into("Title", "First");
    app.save();
    assert!(!app.state().get_open(), "closes once written");
    let track = app.track("First");
    assert_eq!(track.path, path);
    assert_eq!(track.artist.as_deref(), Some("Band"), "the rest is kept");
    let rows: Vec<String> = app
        .window
        .get_album_tracks()
        .iter()
        .map(|row| row.title.to_string())
        .collect();
    assert_eq!(rows, ["First", "Two"], "the open album page shows it");
}

#[test]
fn an_album_keeps_what_differs_unless_it_is_typed() {
    let app = start();
    // Give the two songs different years first.
    app.state()
        .invoke_edit_track(app.music.join("2.flac").to_string_lossy().as_ref().into());
    app.type_into("Year", "2001");
    app.save();

    app.open_album();
    app.state().invoke_edit_album();
    assert_eq!(app.state().get_track_count(), 2);
    let (values, mixed) = (app.state().get_values(), app.state().get_mixed());
    assert_eq!((values.album.as_str(), mixed.album), ("Album", false));
    assert_eq!((values.year.as_str(), mixed.year), ("", true));
    assert!(
        ElementHandle::find_by_accessible_label(&app.window, "Title")
            .all(|element| element.accessible_role() != Some(AccessibleRole::TextInput)),
        "no title field for many tracks"
    );

    app.type_into("Album artist", "Band");
    app.save();
    for title in ["One", "Two"] {
        assert_eq!(app.track(title).album_artist.as_deref(), Some("Band"));
    }
    assert_eq!(app.track("One").year, None, "the years stay their own");
    assert_eq!(app.track("Two").year, Some(2001));
}

#[test]
fn a_wrong_value_writes_nothing_and_says_why() {
    let app = start();
    let path = app.music.join("single.flac");
    app.state()
        .invoke_edit_track(path.to_string_lossy().as_ref().into());
    app.type_into("Year", "twenty");
    key(&app.window, Key::Return);
    assert_eq!(app.state().get_problem(), TagProblem::Year);
    assert_eq!(app.state().get_status(), TagEditorStatus::Editing);
    ElementHandle::find_by_accessible_label(
        &app.window,
        "The year must be a number from 1 to 9999.",
    )
    .next()
    .expect("the reason is shown");
    assert_eq!(app.track("Single").year, None);

    key(&app.window, Key::Escape);
    assert!(!app.state().get_open(), "Escape closes without writing");
}

#[cfg(unix)]
#[test]
fn a_read_only_file_is_reported_and_the_rest_written() {
    use std::os::unix::fs::PermissionsExt;
    let app = start();
    let locked = app.music.join("2.flac");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o444)).unwrap();
    app.open_album();
    app.state().invoke_edit_album();
    app.type_into("Genre", "Ambient");
    app.save();

    assert_eq!(app.state().get_status(), TagEditorStatus::Failed);
    let failures: Vec<(String, TagFailureReason)> = app
        .state()
        .get_failures()
        .iter()
        .map(|failure| (failure.file.to_string(), failure.reason))
        .collect();
    assert_eq!(
        failures,
        [("2.flac".to_string(), TagFailureReason::ReadOnly)]
    );
    assert_eq!(app.track("One").genre.as_deref(), Some("Ambient"));
    assert_eq!(app.track("Two").genre, None);
    assert_eq!(
        app.state().get_track_count(),
        1,
        "trying again writes only that one"
    );

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    find_button(&app, "Write the tags again").invoke_accessible_default_action();
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    assert!(!app.state().get_open());
    assert_eq!(app.track("Two").genre.as_deref(), Some("Ambient"));
}

fn find_button(app: &App, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(&app.window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no button {label:?}"))
}

#[test]
fn a_chosen_cover_is_embedded_and_can_be_removed() {
    let app = start();
    let path = app.music.join("single.flac");
    let edit = |app: &App| {
        app.state()
            .invoke_edit_track(path.to_string_lossy().as_ref().into())
    };
    edit(&app);
    assert!(!app.state().get_has_cover());
    app.editor.use_cover(b"not an image".to_vec());
    assert_eq!(app.state().get_problem(), TagProblem::Cover);
    app.editor.use_cover(png());
    assert!(app.state().get_has_cover());
    assert_eq!(app.state().get_problem(), TagProblem::None);
    find_button(&app, "Save the tags").invoke_accessible_default_action();
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    let id = app.track("Single").id;
    assert!(app.library.embedded_cover(id).unwrap().is_some());

    edit(&app);
    assert!(app.state().get_has_cover(), "the file's cover is shown");
    find_button(&app, "Remove cover").invoke_accessible_default_action();
    assert!(!app.state().get_has_cover());
    find_button(&app, "Save the tags").invoke_accessible_default_action();
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    assert!(app.library.embedded_cover(id).unwrap().is_none());
}

#[test]
fn the_editor_fits_the_smallest_window() {
    let app = start();
    app.window.window().set_size(LogicalSize::new(720.0, 480.0));
    app.state()
        .invoke_edit_track(app.music.join("1.flac").to_string_lossy().as_ref().into());
    for role in [AccessibleRole::TextInput, AccessibleRole::Button] {
        for element in app
            .window
            .root_element()
            .query_descendants()
            .match_accessible_role(role)
            .find_all()
        {
            let right = element.absolute_position().x + element.size().width;
            assert!(
                right <= 720.5,
                "{:?} {:?} ends at {right}",
                role,
                element.accessible_label()
            );
        }
    }
}

/// The title the file itself holds.
fn title_in_file(path: &std::path::Path) -> Option<String> {
    use lofty::file::TaggedFileExt;
    use lofty::tag::Accessor;
    let file = lofty::read_from_path(path).unwrap();
    file.primary_tag()
        .and_then(|tag| tag.title().map(|title| title.into_owned()))
}

impl App {
    /// Playback holds `name` open, and an open file can't be replaced, as on Windows.
    fn play_locked(&self, name: &str) -> PathBuf {
        let path = self.music.join(name);
        self.editor.set_open_files_locked(true);
        self.editor.set_playback_file(Some(&path));
        path
    }

    fn edit_track(&self, path: &std::path::Path) {
        self.state()
            .invoke_edit_track(path.to_string_lossy().as_ref().into());
        assert!(self.state().get_open());
    }

    fn shows(&self, text: &str) -> bool {
        ElementHandle::find_by_accessible_label(&self.window, text)
            .next()
            .is_some()
    }

    fn failures(&self) -> Vec<(String, TagFailureReason)> {
        self.state()
            .get_failures()
            .iter()
            .map(|failure| (failure.file.to_string(), failure.reason))
            .collect()
    }
}

const WAITING_NOTE: &str = "1.flac is in use by the player, so its new tags will be written when playback stops or the next track starts.";

#[test]
fn the_playing_track_is_written_once_playback_lets_go_of_it() {
    let app = start();
    let path = app.play_locked("1.flac");
    app.edit_track(&path);
    app.type_into("Title", "First");
    app.save();

    assert_eq!(app.state().get_status(), TagEditorStatus::Waiting);
    assert_eq!(app.state().get_waiting_file(), "1.flac");
    assert!(app.shows(WAITING_NOTE));
    assert_eq!(title_in_file(&path).as_deref(), Some("One"), "not yet");
    assert!(
        !app.shows("Save the tags"),
        "nothing more to save while it waits"
    );
    find_button(&app, "Close").invoke_accessible_default_action();
    assert!(!app.state().get_open(), "the editor needn't stay open");

    // The next track starts, so playback has let go of the file.
    app.editor
        .set_playback_file(Some(&app.music.join("2.flac")));
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    assert_eq!(title_in_file(&path).as_deref(), Some("First"));
    assert_eq!(app.track("First").path, path);
}

#[test]
fn an_album_is_written_at_once_except_the_playing_track() {
    let app = start();
    let playing = app.play_locked("2.flac");
    app.open_album();
    app.state().invoke_edit_album();
    app.type_into("Genre", "Ambient");
    app.save();

    assert_eq!(app.track("One").genre.as_deref(), Some("Ambient"));
    assert_eq!(app.track("Two").genre, None);
    assert_eq!(app.state().get_status(), TagEditorStatus::Waiting);
    assert!(app.shows(
        "2.flac is in use by the player, so its new tags will be written when playback stops or the next track starts. The other tracks have their new tags."
    ));

    // Playback stops; the editor, still open, closes once the file is written.
    app.editor.set_playback_file(None);
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    assert!(!app.state().get_open());
    assert_eq!(app.track("Two").genre.as_deref(), Some("Ambient"));
    assert!(playing.exists());
}

#[test]
fn a_waiting_track_opens_with_its_new_tags_and_keeps_them() {
    let app = start();
    let path = app.play_locked("1.flac");
    app.edit_track(&path);
    app.type_into("Title", "First");
    app.save();
    key(&app.window, Key::Escape);
    assert!(!app.state().get_open());

    app.edit_track(&path);
    assert_eq!(app.state().get_status(), TagEditorStatus::Editing);
    assert_eq!(app.state().get_values().title, "First", "the waiting title");
    assert!(app.shows(WAITING_NOTE));
    app.type_into("Artist", "Duo");
    app.save();
    assert_eq!(app.state().get_status(), TagEditorStatus::Waiting);
    assert_eq!(title_in_file(&path).as_deref(), Some("One"));

    app.editor.set_playback_file(None);
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    let track = app.track("First");
    assert_eq!(
        track.artist.as_deref(),
        Some("Duo"),
        "both edits are written"
    );
}

#[test]
fn where_open_files_can_be_replaced_the_playing_track_is_written_at_once() {
    let app = start();
    let path = app.music.join("1.flac");
    app.editor.set_open_files_locked(false);
    app.editor.set_playback_file(Some(&path));
    app.edit_track(&path);
    app.type_into("Title", "First");
    app.save();
    assert!(!app.state().get_open());
    assert_eq!(title_in_file(&path).as_deref(), Some("First"));
}

#[test]
fn waiting_edits_are_written_when_vcmp_closes() {
    let app = start();
    let path = app.play_locked("1.flac");
    app.edit_track(&path);
    app.type_into("Title", "First");
    app.save();
    key(&app.window, Key::Escape);

    // main() stops the engine first, which closes the file.
    app.editor.write_waiting();
    assert_eq!(title_in_file(&path).as_deref(), Some("First"));
    assert_eq!(app.track("First").path, path);
}

#[cfg(unix)]
#[test]
fn a_waiting_edit_that_fails_is_listed_and_can_be_tried_again() {
    use std::os::unix::fs::PermissionsExt;
    let app = start();
    let path = app.play_locked("1.flac");
    app.edit_track(&path);
    app.type_into("Title", "First");
    app.save();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();

    app.editor
        .set_playback_file(Some(&app.music.join("2.flac")));
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    assert_eq!(app.state().get_status(), TagEditorStatus::Failed);
    assert_eq!(
        app.failures(),
        [("1.flac".to_string(), TagFailureReason::ReadOnly)]
    );
    assert_eq!(app.state().get_waiting_file(), "");

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    find_button(&app, "Write the tags again").invoke_accessible_default_action();
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    assert!(!app.state().get_open());
    assert_eq!(title_in_file(&path).as_deref(), Some("First"));
}

#[cfg(unix)]
#[test]
fn a_waiting_edit_that_fails_after_the_editor_closed_is_reported() {
    use std::cell::RefCell;
    use std::os::unix::fs::PermissionsExt;
    let app = start();
    let reported = Rc::new(RefCell::new(Vec::new()));
    let record = reported.clone();
    app.editor
        .on_not_written(move |path| record.borrow_mut().push(path.to_path_buf()));
    let path = app.play_locked("1.flac");
    app.edit_track(&path);
    app.type_into("Title", "First");
    app.save();
    key(&app.window, Key::Escape);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();

    app.editor.set_playback_file(None);
    assert!(app.editor.wait_until_written(Duration::from_secs(10)));
    assert_eq!(*reported.borrow(), std::slice::from_ref(&path));
    assert!(!app.state().get_open(), "the editor doesn't pop up");
    assert_eq!(title_in_file(&path).as_deref(), Some("One"));
}
