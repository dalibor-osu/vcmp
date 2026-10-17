//! Drives the real window through Slint's headless testing backend: pointer clicks,
//! keyboard-only use and the effect of each setting.
//!
//! The backend freezes time, so an animation that has started stays at its first
//! frame. Reduced motion is checked by seeing changes land without any time passing.

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalSize, SharedString};
use vcmp_app::{AppWindow, Page, Theme, ThemeMode, theme};

fn open_window() -> AppWindow {
    i_slint_backend_testing::init_no_event_loop();
    let window = vcmp_app::create_window().expect("window");
    window.show().expect("show");
    window
}

fn find(window: &AppWindow, role: AccessibleRole, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
}

fn press(window: &AppWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Opens a page through its sidebar entry.
fn open_page(window: &AppWindow, label: &str) {
    find(window, AccessibleRole::Tab, label).invoke_accessible_default_action();
}

fn marker_y(window: &AppWindow) -> f32 {
    ElementHandle::find_by_element_id(window, "AppWindow::marker")
        .next()
        .expect("page marker")
        .absolute_position()
        .y
}

fn page_opacity(window: &AppWindow, type_name: &str) -> f32 {
    ElementHandle::find_by_element_type_name(window, type_name)
        .next()
        .unwrap_or_else(|| panic!("{type_name} is not open"))
        .computed_opacity()
}

#[test]
fn clicking_navigation_switches_pages() {
    let window = open_window();
    assert_eq!(window.get_current_page(), Page::Albums);

    find(&window, AccessibleRole::Tab, "About").mock_single_click(PointerEventButton::Left);
    assert_eq!(window.get_current_page(), Page::About);
    find(
        &window,
        AccessibleRole::Button,
        "Made with Slint, opens slint.dev",
    );

    find(&window, AccessibleRole::Tab, "Settings").mock_single_click(PointerEventButton::Left);
    assert_eq!(window.get_current_page(), Page::Settings);
}

#[test]
fn clicking_settings_controls_changes_them() {
    let window = open_window();
    let theme = window.global::<Theme>();
    open_page(&window, "Settings");

    find(&window, AccessibleRole::RadioButton, "Dark").mock_single_click(PointerEventButton::Left);
    assert_eq!(theme.get_mode(), ThemeMode::Dark);

    find(&window, AccessibleRole::Switch, "Reduce motion")
        .mock_single_click(PointerEventButton::Left);
    assert!(theme.get_reduced_motion());
    find(&window, AccessibleRole::Switch, "Reduce motion")
        .mock_single_click(PointerEventButton::Left);
    assert!(!theme.get_reduced_motion());
}

#[test]
fn everything_works_from_the_keyboard() {
    let window = open_window();
    let theme = window.global::<Theme>();

    // Tab order follows the visual order: Albums, Settings, About, then the page.
    press(&window, Key::Tab);
    press(&window, Key::Tab);
    press(&window, Key::Tab);
    press(&window, Key::Return);
    assert_eq!(window.get_current_page(), Page::About);

    press(&window, Key::Backtab);
    press(&window, Key::Space);
    assert_eq!(window.get_current_page(), Page::Settings);

    press(&window, Key::Tab);
    press(&window, Key::Tab);
    press(&window, Key::RightArrow);
    assert_eq!(theme.get_mode(), ThemeMode::Light);
    press(&window, Key::RightArrow);
    assert_eq!(theme.get_mode(), ThemeMode::Dark);
    press(&window, Key::RightArrow);
    assert_eq!(
        theme.get_mode(),
        ThemeMode::Dark,
        "the last option stays selected"
    );
    press(&window, Key::LeftArrow);
    press(&window, Key::LeftArrow);
    assert_eq!(theme.get_mode(), ThemeMode::System);

    press(&window, Key::Tab);
    press(&window, Key::Space);
    assert!(theme.get_reduced_motion());
}

#[test]
fn theme_choice_selects_the_palette() {
    let window = open_window();
    let theme = window.global::<Theme>();
    open_page(&window, "Settings");

    find(&window, AccessibleRole::RadioButton, "Dark").invoke_accessible_default_action();
    assert!(theme.get_dark());
    assert_eq!(theme.get_colors(), theme::dark());

    find(&window, AccessibleRole::RadioButton, "Light").invoke_accessible_default_action();
    assert!(!theme.get_dark());
    assert_eq!(theme.get_colors(), theme::light());

    // The headless backend reports no system colour scheme, which counts as light.
    find(&window, AccessibleRole::RadioButton, "System").invoke_accessible_default_action();
    assert_eq!(theme.get_mode(), ThemeMode::System);
    assert_eq!(theme.get_colors(), theme::light());
}

#[test]
fn transitions_animate_by_default() {
    let window = open_window();
    let albums_marker = marker_y(&window);
    assert_eq!(
        page_opacity(&window, "AlbumsPage"),
        0.0,
        "page starts its fade-in"
    );

    open_page(&window, "About");
    assert_eq!(
        marker_y(&window),
        albums_marker,
        "marker starts sliding from Albums"
    );
    assert_eq!(
        page_opacity(&window, "AboutPage"),
        0.0,
        "page starts its fade-in"
    );
}

#[test]
fn reduced_motion_makes_transitions_instant() {
    let window = open_window();
    let albums_marker = marker_y(&window);
    open_page(&window, "Settings");
    find(&window, AccessibleRole::Switch, "Reduce motion").invoke_accessible_default_action();

    open_page(&window, "About");
    assert!(
        marker_y(&window) > albums_marker,
        "marker is already at About"
    );
    assert_eq!(
        page_opacity(&window, "AboutPage"),
        1.0,
        "page is fully shown"
    );

    open_page(&window, "Albums");
    assert_eq!(marker_y(&window), albums_marker);
    assert_eq!(page_opacity(&window, "AlbumsPage"), 1.0);
}

#[test]
fn smallest_window_keeps_everything_inside() {
    let window = open_window();
    let width = 720.0;
    window.window().set_size(LogicalSize::new(width, 480.0));

    for page in [0, 1, 2] {
        window.set_selected_index(page);
        let roles = [
            AccessibleRole::Tab,
            AccessibleRole::RadioButton,
            AccessibleRole::Switch,
            AccessibleRole::Button,
            AccessibleRole::Text,
        ];
        for role in roles {
            for element in window
                .root_element()
                .query_descendants()
                .match_accessible_role(role)
                .find_all()
            {
                let left = element.absolute_position().x;
                let right = left + element.size().width;
                let label = element.accessible_label().unwrap_or_default();
                assert!(
                    left >= 0.0 && right <= width + 0.5,
                    "page {page}: {role:?} {label:?} spans x {left}..{right} in a {width}px window",
                );
            }
        }
    }
}
