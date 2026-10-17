//! Music folders and scanning through `LibraryState`, as the Settings page and the
//! Albums page use them. Scans run on their real background thread; the test applies
//! their messages with `wait_until_idle` because the testing backend has no event loop.

mod common;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, Model};
use tempfile::TempDir;
use vcmp_app::library_service::{Delivery, LibraryPaths, LibraryService};
use vcmp_app::{AppWindow, LibraryError, LibraryState, ScanPhase, library_view};
use vcmp_library::Library;

const IDLE: Duration = Duration::from_secs(20);

struct App {
    dir: TempDir,
    window: AppWindow,
    service: Rc<LibraryService>,
}

impl App {
    fn music(&self) -> PathBuf {
        dunce::canonicalize(self.dir.path()).unwrap().join("music")
    }

    fn state(&self) -> LibraryState<'_> {
        self.window.global::<LibraryState>()
    }

    fn album_count(&self) -> usize {
        self.window.get_albums().row_count()
    }

    fn wait(&self) {
        assert!(self.service.wait_until_idle(IDLE), "scan finished in time");
        assert_eq!(self.state().get_phase(), ScanPhase::Idle);
    }
}

fn paths(dir: &Path) -> LibraryPaths {
    LibraryPaths {
        database: dir.join("library.sqlite3"),
        cache: dir.join("cache"),
    }
}

/// Starts the app on an empty library; `prepare` may set it up before the service
/// connects, as an earlier run of the app would have.
fn start(prepare: impl FnOnce(&Library, &Path)) -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(dir.path()).unwrap();
    let music = base.join("music");
    add(&music, "1.flac", Song::new("One", "Band", "First"));
    add(&music, "2.flac", Song::new("Two", "Band", "First"));
    add(
        &music.join("Extra"),
        "3.flac",
        Song::new("Three", "Band", "Second"),
    );

    let library = Rc::new(Library::open(&paths(&base).database, &paths(&base).cache).unwrap());
    prepare(&library, &music);
    let window = vcmp_app::create_window().unwrap();
    let browser = library_view::connect(&window, library.clone()).unwrap();
    let service =
        LibraryService::connect(&window, library, paths(&base), browser, Delivery::Manual);
    window.show().unwrap();
    App {
        dir,
        window,
        service,
    }
}

#[test]
fn adding_a_folder_scans_it_into_the_albums_page() {
    let app = start(|_, _| {});
    assert_eq!(app.state().get_folders().row_count(), 0);
    assert_eq!(app.album_count(), 0);

    app.service.add_folder(&app.music());
    assert!(app.state().get_scanning(), "the scan starts right away");
    app.wait();

    let folder = app.state().get_folders().row_data(0).unwrap();
    assert_eq!(
        (folder.name.as_str(), folder.recursive, folder.reachable),
        ("music", true, true)
    );
    assert!(app.state().get_scanned());
    assert_eq!(app.state().get_track_count(), 3);
    assert_eq!(app.album_count(), 2);
    assert_eq!(app.state().get_error(), LibraryError::None);
}

#[test]
fn turning_subfolders_off_rescans_without_them() {
    let app = start(|_, _| {});
    app.service.add_folder(&app.music());
    app.wait();
    let id = app.state().get_folders().row_data(0).unwrap().id;

    app.state().invoke_set_recursive(id, false);
    app.wait();
    assert!(!app.state().get_folders().row_data(0).unwrap().recursive);
    assert_eq!(app.state().get_track_count(), 2);
    assert_eq!(app.album_count(), 1);
}

#[test]
fn removing_a_folder_empties_the_library_and_offers_to_add_one() {
    let app = start(|_, _| {});
    app.service.add_folder(&app.music());
    app.wait();
    let id = app.state().get_folders().row_data(0).unwrap().id;

    app.state().invoke_remove_folder(id);
    assert_eq!(app.state().get_folders().row_count(), 0);
    assert_eq!(app.state().get_track_count(), 0);
    assert_eq!(app.album_count(), 0);

    let asked = Rc::new(Cell::new(false));
    let flag = asked.clone();
    app.state().on_add_folder(move || flag.set(true));
    ElementHandle::find_by_accessible_label(&app.window, "Add music folder…")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .expect("the empty Albums page offers to add a folder")
        .invoke_accessible_default_action();
    assert!(asked.get());
}

#[test]
fn unreadable_files_are_listed() {
    let app = start(|_, _| {});
    std::fs::write(app.music().join("broken.mp3"), b"not audio").unwrap();
    app.service.add_folder(&app.music());
    app.wait();

    let unreadable: Vec<_> = app.state().get_unreadable().iter().collect();
    assert_eq!(unreadable.len(), 1);
    assert!(unreadable[0].path.ends_with("broken.mp3"));
    assert!(!unreadable[0].reason.is_empty());
}

#[test]
fn a_missing_folder_is_reported_not_added() {
    let app = start(|_, _| {});
    app.service.add_folder(&app.music().join("nowhere"));
    assert_eq!(app.state().get_error(), LibraryError::AddFolder);
    assert!(!app.state().get_error_detail().is_empty());
    assert!(!app.state().get_scanning());
    assert_eq!(app.state().get_folders().row_count(), 0);
}

#[test]
fn an_unreachable_folder_is_marked_and_keeps_its_place() {
    let app = start(|_, _| {});
    app.service.add_folder(&app.music());
    app.wait();

    std::fs::rename(app.music(), app.music().with_file_name("unplugged")).unwrap();
    app.state().invoke_rescan();
    app.wait();
    let folder = app.state().get_folders().row_data(0).unwrap();
    assert!(!folder.reachable);
    assert_eq!(app.album_count(), 0, "its albums are unavailable");
}

#[test]
fn startup_scans_when_updating_automatically() {
    let app = start(|library, music| {
        library.add_source(music, true).unwrap();
    });
    assert!(app.state().get_scanning(), "auto-update scans at startup");
    app.wait();
    assert_eq!(app.state().get_track_count(), 3);
}

#[test]
fn startup_does_not_scan_without_automatic_updates() {
    let app = start(|library, music| {
        library.add_source(music, true).unwrap();
        library.set_auto_update(false).unwrap();
    });
    assert!(!app.state().get_auto_update());
    assert!(!app.state().get_scanning());
}

#[test]
fn the_auto_update_choice_is_saved() {
    let app = start(|_, _| {});
    assert!(app.state().get_auto_update(), "on by default");
    app.state().invoke_set_auto_update(false);
    assert!(!app.state().get_auto_update());

    let base = dunce::canonicalize(app.dir.path()).unwrap();
    let reopened = Library::open(&paths(&base).database, &paths(&base).cache).unwrap();
    assert!(!reopened.auto_update().unwrap());
}
