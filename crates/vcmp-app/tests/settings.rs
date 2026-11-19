//! Settings in sections: the list of sections and what each shows, the dropdown that
//! takes the list's place in a narrow window, the search across sections, and the
//! links that open Settings on a section.

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, LogicalSize};
use vcmp_app::themes::Themes;
use vcmp_app::{AgentState, AppWindow, Page, SettingsNav, SettingsSection, Setup};

/// A window on Settings, tall enough for its longest section: the finder skips what
/// is scrolled out of view.
fn open_settings(width: f32) -> AppWindow {
    i_slint_backend_testing::init_no_event_loop();
    let window = vcmp_app::create_window().unwrap();
    Themes::connect(&window, None, None);
    window.window().set_size(LogicalSize::new(width, 2400.0));
    window.show().unwrap();
    find(&window, AccessibleRole::Tab, "Settings")
        .expect("the Settings entry")
        .invoke_accessible_default_action();
    frame(&window);
    window
}

fn find(window: &AppWindow, role: AccessibleRole, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
}

fn shown(window: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, label)
        .next()
        .is_some()
}

/// Lets the window catch up: change handlers run, such as the search after typing.
fn frame(window: &AppWindow) {
    slint::platform::update_timers_and_animations();
    let _ = ElementHandle::find_by_accessible_label(window, "Settings").count();
    slint::platform::update_timers_and_animations();
}

fn open_section(window: &AppWindow, label: &str) {
    find(window, AccessibleRole::Tab, label)
        .unwrap_or_else(|| panic!("no section {label:?}"))
        .invoke_accessible_default_action();
    frame(window);
}

fn search(window: &AppWindow, text: &str) {
    window.global::<SettingsNav>().set_query(text.into());
    frame(window);
}

#[test]
fn each_section_shows_its_own_rows() {
    let window = open_settings(1100.0);
    let nav = window.global::<SettingsNav>();
    assert_eq!(
        nav.get_section(),
        SettingsSection::General,
        "at the first start"
    );
    assert!(find(&window, AccessibleRole::Switch, "Check for updates").is_some());
    assert!(find(&window, AccessibleRole::RadioGroup, "Shuffle").is_none());

    open_section(&window, "Playback");
    assert_eq!(nav.get_section(), SettingsSection::Playback);
    assert!(find(&window, AccessibleRole::RadioGroup, "Shuffle").is_some());
    assert!(find(&window, AccessibleRole::Combobox, "Output device").is_some());
    assert!(find(&window, AccessibleRole::Switch, "Check for updates").is_none());

    open_section(&window, "Appearance");
    assert!(find(&window, AccessibleRole::Button, "Zoom in").is_some());
    assert!(find(&window, AccessibleRole::RadioGroup, "Shuffle").is_none());

    // The section stays while another page is shown.
    find(&window, AccessibleRole::Tab, "Albums")
        .unwrap()
        .invoke_accessible_default_action();
    find(&window, AccessibleRole::Tab, "Settings")
        .unwrap()
        .invoke_accessible_default_action();
    frame(&window);
    assert_eq!(nav.get_section(), SettingsSection::Appearance);
    assert!(find(&window, AccessibleRole::Button, "Zoom in").is_some());
}

#[test]
fn agents_rows_show_while_agents_are_on() {
    let window = open_settings(1100.0);
    open_section(&window, "AI agents");
    assert!(
        find(
            &window,
            AccessibleRole::Switch,
            "Let AI agents use your library"
        )
        .is_some()
    );
    assert!(
        find(
            &window,
            AccessibleRole::Switch,
            "Let AI agents control playback"
        )
        .is_none()
    );
    window.global::<AgentState>().set_enabled(true);
    frame(&window);
    assert!(
        find(
            &window,
            AccessibleRole::Switch,
            "Let AI agents control playback"
        )
        .is_some()
    );
}

#[test]
fn the_search_shows_what_it_finds_in_every_section() {
    let window = open_settings(1100.0);
    let nav = window.global::<SettingsNav>();

    search(&window, "zoom");
    assert_eq!(nav.get_found_count(), 1);
    assert!(
        find(&window, AccessibleRole::Button, "Zoom in").is_some(),
        "a working row"
    );
    assert!(find(&window, AccessibleRole::Switch, "Check for updates").is_none());
    assert!(
        find(&window, AccessibleRole::Text, "Appearance").is_some(),
        "under its section's name"
    );

    // Words from two sections: a keyword of Play history and the name of Last.fm,
    // which this build leaves out.
    search(&window, "last");
    assert_eq!(nav.get_found_count(), 1);
    assert!(find(&window, AccessibleRole::Button, "Clear play history…").is_some());

    // Every word counts, and a section's name finds its rows.
    search(&window, "appearance theme");
    assert_eq!(
        nav.get_found_count(),
        1,
        "the Theme row; Theme files needs a themes folder"
    );
    search(&window, "playback");
    assert!(find(&window, AccessibleRole::RadioGroup, "Shuffle").is_some());
    assert!(find(&window, AccessibleRole::Combobox, "Output device").is_some());

    search(&window, "nothing like this");
    assert_eq!(nav.get_found_count(), 0);
    assert!(shown(&window, "No settings match “nothing like this”."));

    // Choosing a section ends the search.
    open_section(&window, "Library");
    assert_eq!(nav.get_query(), "");
    assert_eq!(nav.get_section(), SettingsSection::Library);
    assert!(find(&window, AccessibleRole::Button, "Clear play history…").is_some());
}

#[test]
fn a_narrow_window_lists_the_sections_in_a_dropdown() {
    let window = open_settings(720.0);
    assert!(
        find(&window, AccessibleRole::Tab, "Playback").is_none(),
        "no list beside the section"
    );
    let sections = find(&window, AccessibleRole::Combobox, "Settings section")
        .expect("the sections' dropdown");
    assert_eq!(sections.accessible_value().as_deref(), Some("General"));
    assert!(find(&window, AccessibleRole::Switch, "Check for updates").is_some());

    window
        .global::<SettingsNav>()
        .invoke_show(SettingsSection::OnlineServices);
    frame(&window);
    assert_eq!(
        sections.accessible_value().as_deref(),
        Some("Online services")
    );
    assert!(find(&window, AccessibleRole::Switch, "Find lyrics online").is_some());
}

#[test]
fn the_wizards_link_opens_the_ai_agents_section() {
    let window = open_settings(1100.0);
    find(&window, AccessibleRole::Tab, "Albums")
        .unwrap()
        .invoke_accessible_default_action();
    window.global::<AgentState>().set_enabled(true);
    let wizard = window.global::<Setup>();
    wizard.invoke_start();
    wizard.invoke_go_to(3);
    frame(&window);

    find(
        &window,
        AccessibleRole::Button,
        "Open AI agents settings, closes the setup",
    )
    .expect("the link under the agents' switch")
    .invoke_accessible_default_action();
    frame(&window);
    assert!(!wizard.get_open());
    assert_eq!(window.get_current_page(), Page::Settings);
    assert_eq!(
        window.global::<SettingsNav>().get_section(),
        SettingsSection::Agents
    );
    assert!(find(&window, AccessibleRole::Button, "Copy the token").is_some());
}
