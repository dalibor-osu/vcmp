//! Appearance choices survive a restart through the library database.

use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::ComponentHandle;
use vcmp_app::{AppWindow, ReduceMotion, Theme, ThemeMode, preferences};
use vcmp_library::Library;

fn click(window: &AppWindow, role: AccessibleRole, label: &str) {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
        .invoke_accessible_default_action();
}

fn open_library(dir: &tempfile::TempDir) -> Library {
    Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap()
}

/// The choice under Reduce motion; "System" is also a theme option.
fn click_motion(window: &AppWindow, label: &str) {
    let label = label.to_owned();
    ElementHandle::find_by_accessible_label(window, "Reduce motion")
        .find(|element| element.accessible_role() == Some(AccessibleRole::RadioGroup))
        .expect("the Reduce motion choice")
        .query_descendants()
        .match_accessible_role(AccessibleRole::RadioButton)
        .match_predicate(move |element| element.accessible_label().as_deref() == Some(&*label))
        .find_first()
        .expect("the Reduce motion option")
        .invoke_accessible_default_action();
}

#[test]
fn choices_are_saved_and_restored() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let open = || open_library(&dir);

    let window = vcmp_app::create_window().unwrap();
    // Shown, so the whole Settings page is laid out and its controls exist.
    window.show().unwrap();
    preferences::connect(&window, Rc::new(open())).unwrap();
    assert_eq!(
        window.global::<Theme>().get_mode(),
        ThemeMode::System,
        "nothing saved yet"
    );
    click(&window, AccessibleRole::Tab, "Settings");
    click(&window, AccessibleRole::RadioButton, "Dark");
    click_motion(&window, "On");
    drop(window);

    let library = open();
    assert_eq!(
        library.setting(preferences::THEME).unwrap().as_deref(),
        Some("dark")
    );
    assert_eq!(
        library
            .setting(preferences::REDUCED_MOTION)
            .unwrap()
            .as_deref(),
        Some("on")
    );

    let restarted = vcmp_app::create_window().unwrap();
    preferences::connect(&restarted, Rc::new(library)).unwrap();
    let theme = restarted.global::<Theme>();
    assert_eq!(theme.get_mode(), ThemeMode::Dark);
    assert_eq!(theme.get_motion_choice(), ReduceMotion::On);
    assert!(theme.get_reduced_motion());
}

#[test]
fn the_system_setting_is_not_saved_as_a_choice() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let window = vcmp_app::create_window().unwrap();
    window.show().unwrap();
    preferences::connect(&window, Rc::new(open_library(&dir))).unwrap();
    window.global::<Theme>().set_system_reduces_motion(true);
    click(&window, AccessibleRole::Tab, "Settings");
    click(&window, AccessibleRole::RadioButton, "Dark");
    drop(window);

    assert_eq!(
        open_library(&dir)
            .setting(preferences::REDUCED_MOTION)
            .unwrap()
            .as_deref(),
        Some("system")
    );
}

#[test]
fn the_older_reduce_motion_switch_is_read() {
    i_slint_backend_testing::init_no_event_loop();
    for (saved, choice) in [("true", ReduceMotion::On), ("false", ReduceMotion::System)] {
        let dir = tempfile::tempdir().unwrap();
        let library = open_library(&dir);
        library
            .set_setting(preferences::REDUCED_MOTION, saved)
            .unwrap();
        let window = vcmp_app::create_window().unwrap();
        preferences::connect(&window, Rc::new(library)).unwrap();
        assert_eq!(
            window.global::<Theme>().get_motion_choice(),
            choice,
            "saved {saved:?}"
        );
    }
}

#[test]
fn unknown_saved_values_are_ignored() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let library = open_library(&dir);
    library.set_setting(preferences::THEME, "sepia").unwrap();
    library
        .set_setting(preferences::REDUCED_MOTION, "sometimes")
        .unwrap();

    let window = vcmp_app::create_window().unwrap();
    preferences::connect(&window, Rc::new(library)).unwrap();
    let theme = window.global::<Theme>();
    assert_eq!(theme.get_mode(), ThemeMode::System);
    assert_eq!(theme.get_motion_choice(), ReduceMotion::System);
}
