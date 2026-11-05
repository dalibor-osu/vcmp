//! The first-run setup wizard: when it opens at the start, what the `ui.setup_done`
//! flag records, how it is left, Run setup again in Settings, and the update check
//! that waits for it.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, SharedString};
use vcmp_app::themes::Themes;
use vcmp_app::{
    AppWindow, Confirmation, Setup, Theme, Themes as ThemesState, UpdateState, setup, updates,
};
use vcmp_library::Library;

fn open_library(dir: &Path) -> Library {
    Library::open(&dir.join("library.sqlite3"), &dir.join("cache")).unwrap()
}

fn open_window() -> AppWindow {
    let window = vcmp_app::create_window().unwrap();
    Themes::connect(&window, None, None);
    // Tall enough for the whole Settings page: the finder skips what is scrolled out
    // of view.
    window.window().set_size(LogicalSize::new(1100.0, 1600.0));
    window.show().unwrap();
    window
}

fn find(window: &AppWindow, role: AccessibleRole, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
}

fn press(window: &AppWindow, label: &str) {
    find(window, AccessibleRole::Button, label)
        .unwrap_or_else(|| panic!("no button labelled {label:?}"))
        .invoke_accessible_default_action();
    frame(window);
}

/// Lets the window catch up: a dialog shown, a step's controls made, an answer
/// handed to the dialog that asked.
fn frame(window: &AppWindow) {
    slint::platform::update_timers_and_animations();
    let _ = ElementHandle::find_by_accessible_label(window, "Next step").count();
    slint::platform::update_timers_and_animations();
}

fn key(window: &AppWindow, key: Key) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    frame(window);
}

fn heading(window: &AppWindow, text: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, text)
        .next()
        .is_some()
}

#[test]
fn a_new_library_shows_the_wizard_once() {
    let dir = tempfile::tempdir().unwrap();
    let library = open_library(dir.path());
    assert!(setup::first_start(&library, false).unwrap());
    assert_eq!(
        library.setting(setup::SETUP_DONE).unwrap().as_deref(),
        Some("true"),
        "written as it shows, so quitting halfway counts as seen"
    );
    assert!(!setup::first_start(&library, false).unwrap());
}

#[test]
fn a_library_with_music_folders_gets_the_flag_silently() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    let library = open_library(dir.path());
    library.add_source(&music, true).unwrap();
    assert!(!setup::first_start(&library, false).unwrap());
    assert_eq!(
        library.setting(setup::SETUP_DONE).unwrap().as_deref(),
        Some("true")
    );

    // Removing every folder later doesn't bring it back.
    let id = library.sources().unwrap()[0].id;
    library.remove_source(id).unwrap();
    assert!(!setup::first_start(&library, false).unwrap());
}

#[test]
fn skipping_by_the_environment_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let library = open_library(dir.path());
    assert!(!setup::first_start(&library, true).unwrap());
    assert_eq!(library.setting(setup::SETUP_DONE).unwrap(), None);
    assert!(
        setup::first_start(&library, false).unwrap(),
        "started without it, the same library still shows the wizard"
    );
}

#[test]
fn connect_opens_the_wizard_only_for_a_new_library() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let library = open_library(dir.path());
    let window = open_window();
    assert!(setup::connect(&window, &library, false));
    assert!(window.global::<Setup>().get_open());
    assert_eq!(window.global::<Setup>().get_step(), 0);
    drop(window);

    let window = open_window();
    assert!(!setup::connect(&window, &library, false));
    assert!(!window.global::<Setup>().get_open());
}

#[test]
fn next_and_back_walk_the_steps_and_done_closes_without_asking() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    let wizard = window.global::<Setup>();
    let closed = Rc::new(Cell::new(0));
    wizard.on_closed({
        let closed = closed.clone();
        move || closed.set(closed.get() + 1)
    });
    wizard.invoke_start();
    frame(&window);
    assert!(heading(&window, "Welcome to VCMP"));

    for title in ["Music folders", "Look", "Online services", "All set"] {
        press(&window, "Next step");
        assert!(heading(&window, title), "{title} follows");
    }
    press(&window, "Back to the previous step");
    assert_eq!(wizard.get_step(), 3);
    press(&window, "Next step");

    press(&window, "Close the setup");
    assert!(!wizard.get_open());
    assert!(!window.global::<Confirmation>().get_open());
    assert_eq!(closed.get(), 1);
}

#[test]
fn skip_and_escape_ask_first_and_keep_what_was_set() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    let wizard = window.global::<Setup>();
    let question = window.global::<Confirmation>();
    let closed = Rc::new(Cell::new(0));
    wizard.on_closed({
        let closed = closed.clone();
        move || closed.set(closed.get() + 1)
    });
    wizard.invoke_start();
    frame(&window);
    press(&window, "Next step");
    press(&window, "Next step");

    // A choice in the Look step applies at once.
    find(&window, AccessibleRole::RadioButton, "Dark")
        .expect("the theme's Dark")
        .invoke_accessible_default_action();
    assert!(window.global::<Theme>().get_dark());

    // Escape asks; Cancel goes back to the same step.
    key(&window, Key::Escape);
    assert!(question.get_open());
    assert_eq!(question.get_question(), "Close the setup?");
    key(&window, Key::Escape);
    assert!(!question.get_open());
    assert!(wizard.get_open());
    assert_eq!(wizard.get_step(), 2);
    assert_eq!(closed.get(), 0);

    // Skip setup asks too, and going ahead closes it with the choice kept.
    press(&window, "Skip setup");
    assert!(question.get_open());
    press(&window, "Close the setup");
    assert!(!question.get_open());
    assert!(!wizard.get_open());
    assert_eq!(closed.get(), 1);
    assert!(window.global::<Theme>().get_dark());
}

#[test]
fn the_keyboard_stays_in_the_wizard() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    let wizard = window.global::<Setup>();
    wizard.invoke_start();
    frame(&window);

    // The focus starts on Next: Enter goes on.
    key(&window, Key::Return);
    assert_eq!(wizard.get_step(), 1);

    // On the welcome, Back is off: Tab from Next reaches Skip setup and comes round
    // to Next, never to the page behind.
    press(&window, "Back to the previous step");
    key(&window, Key::Tab);
    key(&window, Key::Tab);
    key(&window, Key::Return);
    assert_eq!(wizard.get_step(), 1, "back on Next after going round");
}

#[test]
fn run_setup_again_opens_it_with_the_current_values() {
    i_slint_backend_testing::init_no_event_loop();
    let window = open_window();
    window
        .global::<ThemesState>()
        .invoke_choose("darker".into());
    find(&window, AccessibleRole::Tab, "Settings")
        .unwrap()
        .invoke_accessible_default_action();
    press(&window, "Run setup again…");
    let wizard = window.global::<Setup>();
    assert!(wizard.get_open());
    assert_eq!(wizard.get_step(), 0);

    press(&window, "Next step");
    press(&window, "Next step");
    let darker = ElementHandle::find_by_accessible_label(&window, "Darker")
        .filter(|element| element.accessible_role() == Some(AccessibleRole::RadioButton))
        .collect::<Vec<_>>();
    assert!(
        darker
            .iter()
            .any(|element| element.accessible_checked() == Some(true)),
        "the Look step shows the theme set now"
    );

    key(&window, Key::Escape);
    press(&window, "Close the setup");
    assert!(!wizard.get_open());
}

#[test]
fn the_update_check_waits_for_the_wizard() {
    i_slint_backend_testing::init_no_event_loop();
    let checks = Rc::new(Cell::new(0));
    let count = || {
        let checks = checks.clone();
        move |_| checks.set(checks.get() + 1)
    };

    // Held while the wizard is open, asked when it closes.
    let window = open_window();
    let held = updates::connect_held(&window, None, true, count());
    assert_eq!(checks.get(), 0);
    held.release(&window);
    assert_eq!(checks.get(), 1);
    held.release(&window);
    assert_eq!(checks.get(), 1, "once per run");
    drop(window);

    // Turned off in the wizard: nothing is asked when it closes, and turning it on
    // later asks then.
    checks.set(0);
    let window = open_window();
    let held = updates::connect_held(&window, None, true, count());
    window
        .global::<UpdateState>()
        .invoke_set_check_enabled(false);
    held.release(&window);
    assert_eq!(checks.get(), 0);
    window
        .global::<UpdateState>()
        .invoke_set_check_enabled(true);
    assert_eq!(checks.get(), 1);
}
