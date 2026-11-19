//! Updates in the window: the switches survive a restart and decide whether and
//! what VCMP asks GitHub, a newer release shows a note in the sidebar and on the
//! About section of Settings, and the note opens the dialog that installs it.

use std::cell::Cell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, LogicalSize};
use vcmp_app::updates::StartReason;
use vcmp_app::{
    AppWindow, LibraryState, ScanPhase, UpdateBlocker, UpdateFailure, UpdatePhase, UpdateState,
    updates,
};
use vcmp_integrations::self_update::{Blocker, Failure, HandOver};
use vcmp_integrations::updates::Release;
use vcmp_library::Library;

/// The note's spoken label, for the release the tests report.
const NOTE: &str = "VCMP 9.1.0 is available, opens the update";

/// Opens the window; each test first sets up the testing backend, once.
fn open_window() -> AppWindow {
    let window = vcmp_app::create_window().expect("window");
    window.show().expect("show");
    // Tall enough for the whole Settings page: the finder skips what is scrolled out
    // of view.
    window.window().set_size(LogicalSize::new(1100.0, 4000.0));
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
    open_page(window, "General");
    find(window, AccessibleRole::Switch, "Check for updates").invoke_accessible_default_action();
}

/// A check that only counts how often it was asked to run.
fn counting_check(count: &Rc<Cell<u32>>) -> impl Fn(slint::Weak<AppWindow>) + 'static {
    let count = Rc::clone(count);
    move |_| count.set(count.get() + 1)
}

fn release_9_1() -> Release {
    Release {
        version: "9.1.0".into(),
        url: "https://github.com/dalibor-osu/vcmp/releases/tag/v9.1.0".into(),
        ..Release::default()
    }
}

/// A check that finds VCMP 9.1.0 at once.
fn report_newer(window: slint::Weak<AppWindow>) {
    updates::show(&window.upgrade().unwrap(), &release_9_1());
}

fn find_any(window: &AppWindow, role: AccessibleRole, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, label)
        .any(|element| element.accessible_role() == Some(role))
}

/// Opens the update dialog from the sidebar's note.
fn open_dialog(window: &AppWindow) {
    find(window, AccessibleRole::Button, NOTE).invoke_accessible_default_action();
    assert!(window.global::<UpdateState>().get_dialog_open());
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

    open_page(&window, "Settings");
    open_page(&window, "About");
    assert_eq!(notes(&window), 2, "the sidebar's and the About section's");

    // A scan takes the sidebar's foot while it runs.
    window
        .global::<LibraryState>()
        .set_phase(ScanPhase::Reading);
    assert_eq!(notes(&window), 1, "only the About section's while scanning");
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
    window.window().set_size(LogicalSize::new(780.0, 480.0));
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
    open_page(&window, "Settings");
    assert_eq!(notes(&window), 0);
    assert_eq!(window.global::<UpdateState>().get_version(), "");
}

#[test]
fn the_note_opens_the_update_dialog_which_installs_it() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, |_| {});
    updates::show_with(&window, &release_9_1(), Err(Blocker::NotSigned));
    let state = window.global::<UpdateState>();
    // As in a copy that can install it.
    state.set_blocker(UpdateBlocker::None);
    open_dialog(&window);
    assert!(find_any(
        &window,
        AccessibleRole::Button,
        "Install VCMP 9.1.0 and restart"
    ));
    assert!(find_any(&window, AccessibleRole::Button, "Not now"));
    assert!(find_any(
        &window,
        AccessibleRole::Button,
        "Release notes, opens the release page on GitHub"
    ));

    find(&window, AccessibleRole::Button, "Not now").invoke_accessible_default_action();
    assert!(!state.get_dialog_open());
}

#[test]
fn a_copy_that_cannot_install_links_to_the_release() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, |_| {});
    updates::show_with(&window, &release_9_1(), Err(Blocker::ReadOnly));
    let state = window.global::<UpdateState>();
    assert_eq!(state.get_blocker(), UpdateBlocker::ReadOnly);
    open_dialog(&window);
    assert!(!find_any(
        &window,
        AccessibleRole::Button,
        "Install VCMP 9.1.0 and restart"
    ));
    assert!(find_any(
        &window,
        AccessibleRole::Button,
        "Open the release page on GitHub"
    ));
}

#[test]
fn a_debug_build_never_installs() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, report_newer);
    let blocker = window.global::<UpdateState>().get_blocker();
    assert_ne!(blocker, UpdateBlocker::None, "tests run a debug build");
}

#[test]
fn an_install_shows_its_progress_and_how_it_ended() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, report_newer);
    let state = window.global::<UpdateState>();
    state.set_blocker(UpdateBlocker::None);
    open_dialog(&window);

    // While it downloads, Cancel is the way out; Escape doesn't close the dialog.
    state.set_phase(UpdatePhase::Downloading);
    assert!(find_any(
        &window,
        AccessibleRole::Button,
        "Cancel the update"
    ));
    assert!(find_any(
        &window,
        AccessibleRole::ProgressIndicator,
        "Downloading VCMP 9.1.0"
    ));
    state.invoke_close_dialog();
    assert!(state.get_dialog_open());

    updates::installed(
        &window,
        Err(Failure::Verify("SHA256SUMS.txt is not signed".into())),
    );
    assert_eq!(state.get_phase(), UpdatePhase::Failed);
    assert_eq!(state.get_failure(), UpdateFailure::Verify);
    assert_eq!(state.get_error(), "SHA256SUMS.txt is not signed");
    assert!(find_any(
        &window,
        AccessibleRole::Button,
        "Try installing again"
    ));

    updates::installed(&window, Err(Failure::Cancelled));
    assert_eq!(state.get_phase(), UpdatePhase::Offer);

    assert_eq!(updates::take_hand_over(), None);
    let hand_over = HandOver {
        program: PathBuf::from("vcmp"),
        backup: PathBuf::from("vcmp.old"),
        version: "9.1.0".into(),
    };
    updates::installed(&window, Ok(hand_over.clone()));
    assert_eq!(state.get_phase(), UpdatePhase::Restarting);
    state.invoke_close_dialog();
    assert!(state.get_dialog_open(), "nothing stops the restart");
    assert_eq!(updates::take_hand_over(), Some(hand_over));
    assert_eq!(updates::take_hand_over(), None, "handed over once");
}

#[test]
fn the_pre_release_switch_is_saved_and_asks_again() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let window = open_window();
    let checks = Rc::new(Cell::new(0));
    updates::connect(
        &window,
        Some(open_library(dir.path())),
        counting_check(&checks),
    );
    let state = window.global::<UpdateState>();
    assert!(!state.get_include_prereleases(), "off by default");
    assert_eq!(checks.get(), 1);

    open_page(&window, "Settings");
    find(&window, AccessibleRole::Switch, "Include pre-releases")
        .invoke_accessible_default_action();
    assert!(state.get_include_prereleases());
    assert_eq!(checks.get(), 2, "turning it on asks again");
    drop(window);
    let saved = open_library(dir.path())
        .setting(updates::SETTING_UPDATE_PRERELEASES)
        .unwrap();
    assert_eq!(saved.as_deref(), Some("true"));

    let window = open_window();
    updates::connect(&window, Some(open_library(dir.path())), |_| {});
    assert!(window.global::<UpdateState>().get_include_prereleases());
}

#[test]
fn a_pre_release_shows_only_while_they_are_asked_for() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, |_| {});
    let prerelease = Release {
        prerelease: true,
        ..release_9_1()
    };
    updates::show_with(&window, &prerelease, Err(Blocker::NotSigned));
    let state = window.global::<UpdateState>();
    assert_eq!(state.get_version(), "", "found before the switch went off");

    state.invoke_set_include_prereleases(true);
    updates::show_with(&window, &prerelease, Err(Blocker::NotSigned));
    assert_eq!(state.get_version(), "9.1.0");
    assert!(state.get_prerelease());
    open_dialog(&window);
    assert!(
        ElementHandle::find_by_accessible_label(&window, "A pre-release, published for testing.")
            .next()
            .is_some()
    );
}

#[test]
fn a_version_that_did_not_start_is_reported() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    updates::connect(&window, None, |_| {});
    updates::show_rolled_back(&window, "9.1.0");
    let state = window.global::<UpdateState>();
    assert!(state.get_dialog_open());
    assert_eq!(state.get_phase(), UpdatePhase::RolledBack);
    assert_eq!(
        notes(&window),
        0,
        "the failed version is not offered again from here"
    );
    assert!(find_any(
        &window,
        AccessibleRole::Button,
        "Open the release page on GitHub"
    ));
    find(&window, AccessibleRole::Button, "Close").invoke_accessible_default_action();
    assert!(!state.get_dialog_open());
    assert_eq!(state.get_phase(), UpdatePhase::Offer);
}

#[test]
fn the_arguments_say_why_vcmp_started() {
    let args = |list: &[&str]| {
        list.iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    };
    assert_eq!(StartReason::from_args(args(&[])), StartReason::Normal);
    assert_eq!(StartReason::from_args(args(&["mcp"])), StartReason::Normal);
    assert_eq!(
        StartReason::from_args(args(&["--update-started", "/x/.vcmp-update-started"])),
        StartReason::Updated(PathBuf::from("/x/.vcmp-update-started"))
    );
    assert_eq!(
        StartReason::from_args(args(&["--update-failed", "9.1.0"])),
        StartReason::UpdateFailed("9.1.0".into())
    );
    assert_eq!(
        StartReason::from_args(args(&["--update-failed"])),
        StartReason::Normal
    );
}
