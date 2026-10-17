//! The update check: its switch survives a restart and decides whether VCMP asks
//! GitHub, and a newer release shows a note in the sidebar and on the About page.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, LogicalSize};
use vcmp_app::{AppWindow, LibraryState, ScanPhase, UpdateState, updates};
use vcmp_integrations::updates::Release;
use vcmp_library::Library;

/// The note's spoken label, for the release the tests report.
const NOTE: &str = "VCMP 9.1.0 is available, opens its page on GitHub";

/// Opens the window; each test first sets up the testing backend, once.
fn open_window() -> AppWindow {
    let window = vcmp_app::create_window().expect("window");
    window.show().expect("show");
    // Tall enough for the whole Settings page: the finder skips what is scrolled out
    // of view.
    window.window().set_size(LogicalSize::new(1100.0, 1600.0));
    window
}

fn open_library(dir: &Path) -> Rc<Library> {
    Rc::new(Library::open(&dir.join("library.sqlite3"), &dir.join("cache")).unwrap())
}

fn find(window: &AppWindow, role: AccessibleRole, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
}

fn open_page(window: &AppWindow, label: &str) {
    find(window, AccessibleRole::Tab, label).invoke_accessible_default_action();
}

fn flip_switch(window: &AppWindow) {
    open_page(window, "Settings");
    find(window, AccessibleRole::Switch, "Check for updates").invoke_accessible_default_action();
}

/// A check that only counts how often it was asked to run.
fn counting_check(count: &Rc<Cell<u32>>) -> impl FnOnce(slint::Weak<AppWindow>) + 'static {
    let count = Rc::clone(count);
    move |_| count.set(count.get() + 1)
}

/// A check that finds VCMP 9.1.0 at once.
fn report_newer(window: slint::Weak<AppWindow>) {
    updates::show(
        &window.upgrade().unwrap(),
        &Release {
            version: "9.1.0".into(),
            url: "https://github.com/dalibor-osu/vcmp/releases/tag/v9.1.0".into(),
        },
    );
}

/// How many update notes are on screen.
fn notes(window: &AppWindow) -> usize {
    ElementHandle::find_by_accessible_label(window, NOTE)
        .filter(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .count()
}

#[test]
fn the_switch_is_saved_and_decides_whether_vcmp_checks() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let window = open_window();
    let checks = Rc::new(Cell::new(0));
    updates::connect(
        &window,
        Some(open_library(dir.path())),
        counting_check(&checks),
    );
    assert!(window.global::<UpdateState>().get_check_enabled());
    assert_eq!(
        checks.get(),
        1,
        "on by default, so VCMP checks when it starts"
    );

    flip_switch(&window);
    assert!(!window.global::<UpdateState>().get_check_enabled());
    drop(window);
    let saved = open_library(dir.path())
        .setting(updates::SETTING_UPDATE_CHECK)
        .unwrap();
    assert_eq!(saved.as_deref(), Some("false"));

    // Restarted with the check off: GitHub is asked only once the switch is back on.
    let window = open_window();
    let checks = Rc::new(Cell::new(0));
    updates::connect(
        &window,
        Some(open_library(dir.path())),
        counting_check(&checks),
    );
    assert!(!window.global::<UpdateState>().get_check_enabled());
    assert_eq!(checks.get(), 0, "switched off, so nothing is asked");

    flip_switch(&window);
    assert_eq!(checks.get(), 1, "turning it on checks at once");
    flip_switch(&window);
    flip_switch(&window);
    assert_eq!(checks.get(), 1, "once per run");
    let saved = open_library(dir.path())
        .setting(updates::SETTING_UPDATE_CHECK)
        .unwrap();
    assert_eq!(saved.as_deref(), Some("true"));
}

#[test]
fn a_newer_release_shows_a_note_in_the_sidebar_and_on_about() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, report_newer);
    let state = window.global::<UpdateState>();
    assert_eq!(
        state.get_release_url(),
        "https://github.com/dalibor-osu/vcmp/releases/tag/v9.1.0"
    );
    assert_eq!(notes(&window), 1, "the sidebar's note");

    open_page(&window, "About");
    assert_eq!(notes(&window), 2, "the sidebar's and the About page's");

    // A scan takes the sidebar's foot while it runs.
    window
        .global::<LibraryState>()
        .set_phase(ScanPhase::Reading);
    assert_eq!(notes(&window), 1, "only About's while scanning");
    window.global::<LibraryState>().set_phase(ScanPhase::Idle);
    assert_eq!(notes(&window), 2);

    // Switching the check off hides what it found.
    flip_switch(&window);
    assert_eq!(notes(&window), 0);
}

#[test]
fn a_short_window_keeps_the_note_below_the_scrolling_sections() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, report_newer);
    // The smallest window: the sections scroll above the note instead of under it.
    window.window().set_size(LogicalSize::new(720.0, 480.0));
    assert_eq!(notes(&window), 1);
    let note = ElementHandle::find_by_accessible_label(&window, NOTE)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap();
    let sections = ElementHandle::find_by_element_id(&window, "AppWindow::sidebar-scroll")
        .next()
        .expect("the sidebar's scrolling part");
    let sections_bottom = sections.absolute_position().y + sections.size().height;
    assert!(
        sections_bottom <= note.absolute_position().y + 0.5,
        "the sections end at {sections_bottom}, above the note at {}",
        note.absolute_position().y
    );
}

#[test]
fn no_note_without_a_newer_release() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, |_| {});
    open_page(&window, "About");
    assert_eq!(notes(&window), 0);
    assert_eq!(window.global::<UpdateState>().get_version(), "");
}
