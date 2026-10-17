//! Appearance choices survive a restart through the library database.

use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::ComponentHandle;
use vcmp_app::{AppWindow, Theme, ThemeMode, preferences};
use vcmp_library::Library;

fn click(window: &AppWindow, role: AccessibleRole, label: &str) {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
        .invoke_accessible_default_action();
}

#[test]
fn choices_are_saved_and_restored() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let open = || {
        Library::open(
            &dir.path().join("library.sqlite3"),
            &dir.path().join("cache"),
        )
        .unwrap()
    };

    let window = vcmp_app::create_window().unwrap();
    preferences::connect(&window, Rc::new(open())).unwrap();
    assert_eq!(
        window.global::<Theme>().get_mode(),
        ThemeMode::System,
        "nothing saved yet"
    );
    click(&window, AccessibleRole::RadioButton, "Dark");
    click(&window, AccessibleRole::Switch, "Reduce motion");
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
        Some("true")
    );

    let restarted = vcmp_app::create_window().unwrap();
    preferences::connect(&restarted, Rc::new(library)).unwrap();
    let theme = restarted.global::<Theme>();
    assert_eq!(theme.get_mode(), ThemeMode::Dark);
    assert!(theme.get_reduced_motion());
}

#[test]
fn unknown_saved_values_are_ignored() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.set_setting(preferences::THEME, "sepia").unwrap();

    let window = vcmp_app::create_window().unwrap();
    preferences::connect(&window, Rc::new(library)).unwrap();
    assert_eq!(window.global::<Theme>().get_mode(), ThemeMode::System);
}
