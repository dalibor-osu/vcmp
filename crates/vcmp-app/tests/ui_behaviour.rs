//! Drives the real window through Slint's headless testing backend: pointer clicks,
//! keyboard-only use and the effect of each setting.
//!
//! The backend freezes time, so an animation that has started stays at its first
//! frame. Reduced motion is checked by seeing changes land without any time passing.

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalSize, SharedString};
use vcmp_app::{AppWindow, Page, ReduceMotion, Theme, ThemeMode, theme};

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

/// One option of the Reduce motion choice; "System" is also a theme option.
fn motion_option(window: &AppWindow, label: &str) -> ElementHandle {
    let label = label.to_owned();
    find(window, AccessibleRole::RadioGroup, "Reduce motion")
        .query_descendants()
        .match_accessible_role(AccessibleRole::RadioButton)
        .match_predicate(move |element| element.accessible_label().as_deref() == Some(&*label))
        .find_first()
        .expect("the Reduce motion option")
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
    // Tall enough that the Appearance section clears the player bar without scrolling.
    window.window().set_size(LogicalSize::new(1100.0, 900.0));
    let theme = window.global::<Theme>();
    open_page(&window, "Settings");

    find(&window, AccessibleRole::RadioButton, "Dark").mock_single_click(PointerEventButton::Left);
    assert_eq!(theme.get_mode(), ThemeMode::Dark);

    motion_option(&window, "On").mock_single_click(PointerEventButton::Left);
    assert_eq!(theme.get_motion_choice(), ReduceMotion::On);
    assert!(theme.get_reduced_motion());
    motion_option(&window, "Off").mock_single_click(PointerEventButton::Left);
    assert_eq!(theme.get_motion_choice(), ReduceMotion::Off);
    assert!(!theme.get_reduced_motion());
    motion_option(&window, "System").mock_single_click(PointerEventButton::Left);
    assert_eq!(theme.get_motion_choice(), ReduceMotion::System);
    assert_eq!(
        theme.get_mode(),
        ThemeMode::Dark,
        "the theme's System stays off"
    );
}

#[test]
fn everything_works_from_the_keyboard() {
    let window = open_window();
    let theme = window.global::<Theme>();

    // Tab order follows the visual order: Search, Albums, Artists, Playlists, Songs,
    // Folders, Settings, About, then the page.
    for _ in 0..8 {
        press(&window, Key::Tab);
    }
    press(&window, Key::Return);
    assert_eq!(window.get_current_page(), Page::About);

    // Enter activates; Space plays and pauses everywhere but in text fields.
    press(&window, Key::Backtab);
    press(&window, Key::Return);
    assert_eq!(window.get_current_page(), Page::Settings);

    // From the Settings entry: About, Add folder, Update automatically, the shuffle
    // choice, the output device, then the theme choice. Without folders there is no
    // Rescan button, and without a player service no Discord switch.
    for _ in 0..6 {
        press(&window, Key::Tab);
    }
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
    press(&window, Key::RightArrow);
    assert_eq!(theme.get_motion_choice(), ReduceMotion::On);
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

    // The Albums page stays alive while hidden; a curtain fades it in again on return.
    open_page(&window, "Albums");
    assert_eq!(
        page_opacity(&window, "FadeCurtain"),
        1.0,
        "curtain starts opaque"
    );
}

#[test]
fn the_queue_panel_slides_in_unless_motion_is_reduced() {
    let window = open_window();
    // Tall enough for the whole Settings page, where motion is reduced below.
    window.window().set_size(LogicalSize::new(1100.0, 1600.0));
    let panel_x = |window: &AppWindow| {
        ElementHandle::find_by_element_type_name(window, "QueuePanel")
            .next()
            .expect("the queue panel is open")
            .absolute_position()
            .x
    };
    window.set_queue_open(true);
    let start = panel_x(&window);
    assert!(
        start >= 1100.0 - 1.0,
        "starts at the window's edge, not {start}"
    );
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
    assert_eq!(panel_x(&window), 1100.0 - 320.0, "then sits at the edge");

    window.set_queue_open(false);
    open_page(&window, "Settings");
    motion_option(&window, "On").invoke_accessible_default_action();
    window.set_queue_open(true);
    assert_eq!(
        panel_x(&window),
        1100.0 - 320.0,
        "at once with reduced motion"
    );
}

#[test]
fn in_a_narrow_window_the_queue_covers_the_pages_not_the_sidebar() {
    let window = open_window();
    window.window().set_size(LogicalSize::new(900.0, 700.0));
    let panel = |window: &AppWindow| {
        ElementHandle::find_by_element_type_name(window, "QueuePanel")
            .next()
            .expect("the queue panel is open")
    };
    // Only what is shown can be found.
    let page_shown = |window: &AppWindow| {
        ElementHandle::find_by_accessible_label(window, "Filter albums")
            .next()
            .is_some()
    };
    assert!(page_shown(&window));
    window.set_queue_open(true);
    // The panel is made when first looked at, and slides in from then.
    panel(&window);
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
    let (x, width) = (
        panel(&window).absolute_position().x,
        panel(&window).size().width,
    );
    assert_eq!(
        (x, width),
        (220.0, 900.0 - 220.0),
        "from the sidebar to the edge"
    );
    assert!(
        !page_shown(&window),
        "the page under it is hidden, so the keyboard can't reach it"
    );
    find(&window, AccessibleRole::Tab, "Songs");

    press(&window, Key::Escape);
    assert!(!window.get_queue_open(), "Escape closes it");
    assert!(page_shown(&window));

    // Beside the page in a wide window, where Escape is left to the page.
    window.window().set_size(LogicalSize::new(1200.0, 700.0));
    window.set_queue_open(true);
    panel(&window);
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
    assert_eq!(panel(&window).size().width, 320.0);
    assert!(page_shown(&window));
    press(&window, Key::Escape);
    assert!(window.get_queue_open());
}

#[test]
fn every_library_page_offers_to_add_a_folder_when_empty() {
    let window = open_window();
    for section in ["Albums", "Artists", "Songs", "Folders"] {
        open_page(&window, section);
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
        let buttons = ElementHandle::find_by_accessible_label(&window, "Add music folder…")
            .filter(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .count();
        assert_eq!(buttons, 1, "{section} shows the button");
    }
}

#[test]
fn reduced_motion_makes_transitions_instant() {
    let window = open_window();
    let albums_marker = marker_y(&window);
    open_page(&window, "Settings");
    motion_option(&window, "On").invoke_accessible_default_action();

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
    assert_eq!(
        page_opacity(&window, "FadeCurtain"),
        0.0,
        "curtain already gone"
    );
}

#[test]
fn reduced_motion_follows_the_system_unless_chosen() {
    let window = open_window();
    let theme = window.global::<Theme>();
    assert_eq!(theme.get_motion_choice(), ReduceMotion::System);
    assert!(
        !theme.get_reduced_motion(),
        "a system without the setting keeps motion"
    );

    for (system, choice, reduced) in [
        (true, ReduceMotion::System, true),
        (false, ReduceMotion::System, false),
        (true, ReduceMotion::Off, false),
        (false, ReduceMotion::Off, false),
        (true, ReduceMotion::On, true),
        (false, ReduceMotion::On, true),
    ] {
        theme.set_system_reduces_motion(system);
        theme.set_motion_choice(choice);
        assert_eq!(
            theme.get_reduced_motion(),
            reduced,
            "system asks to reduce: {system}, choice: {choice:?}"
        );
    }

    // Under "System", the system's answer takes effect whenever it arrives or changes.
    theme.set_system_reduces_motion(false);
    theme.set_motion_choice(ReduceMotion::System);
    let albums_marker = marker_y(&window);
    theme.set_system_reduces_motion(true);
    open_page(&window, "About");
    assert!(
        marker_y(&window) > albums_marker,
        "marker is already at About"
    );
    assert_eq!(page_opacity(&window, "AboutPage"), 1.0);

    let about_marker = marker_y(&window);
    theme.set_system_reduces_motion(false);
    open_page(&window, "Albums");
    assert_eq!(
        marker_y(&window),
        about_marker,
        "motion is back: the marker starts sliding from About"
    );
}

#[test]
fn smallest_window_keeps_everything_inside() {
    let window = open_window();
    let width = 720.0;
    window.window().set_size(LogicalSize::new(width, 480.0));
    // Settings with the Last.fm row, as in a build with an API key, in its longest
    // state: waiting, with the link back to the browser page.
    let lastfm = window.global::<vcmp_app::LastfmState>();
    lastfm.set_available(true);
    lastfm.set_status(vcmp_app::LastfmStatus::Waiting);

    // Every page, with the queue panel closed and open, and the now playing view.
    for (page, queue_open, now_playing) in [0, 1, 2, 3, 4, 5, 6, 7]
        .into_iter()
        .flat_map(|page| [(page, false, false), (page, true, false)])
        .chain([(1, false, true)])
    {
        window.set_selected_index(page);
        window.set_queue_open(queue_open);
        window
            .global::<vcmp_app::PlayerState>()
            .set_now_playing_open(now_playing);
        // The queue panel slides in once made; check it where it comes to rest.
        let _ = ElementHandle::find_by_element_type_name(&window, "QueuePanel").count();
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
        let roles = [
            AccessibleRole::Combobox,
            AccessibleRole::Tab,
            AccessibleRole::RadioButton,
            AccessibleRole::Switch,
            AccessibleRole::Button,
            AccessibleRole::Slider,
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
                    "page {page} (queue: {queue_open}, now playing: {now_playing}): {role:?} {label:?} spans x {left}..{right} in a {width}px window",
                );
            }
        }
    }
}

#[test]
fn focus_rings_show_only_after_keyboard_use() {
    let window = open_window();
    let theme = window.global::<Theme>();
    assert!(!theme.get_keyboard_navigation(), "no ring before any input");

    press(&window, Key::Tab);
    assert!(theme.get_keyboard_navigation());

    find(&window, AccessibleRole::Tab, "About").mock_single_click(PointerEventButton::Left);
    assert!(!theme.get_keyboard_navigation(), "a click hides the ring");
}

#[test]
fn a_library_that_failed_to_open_says_so() {
    let window = open_window();
    window.set_library_failed(true);
    ElementHandle::find_by_accessible_label(
        &window,
        "The music library could not be opened, so no albums can be shown.",
    )
    .next()
    .expect("error message instead of the empty state");
}

#[test]
fn a_short_window_scrolls_the_sidebar_to_the_focused_section() {
    let window = open_window();
    window.window().set_size(LogicalSize::new(720.0, 480.0));
    let sidebar = ElementHandle::find_by_element_id(&window, "AppWindow::sidebar")
        .next()
        .expect("sidebar");
    let sections = ElementHandle::find_by_element_id(&window, "AppWindow::sidebar-scroll")
        .next()
        .expect("the sidebar's scrolling part");
    let bottom = |element: &ElementHandle| element.absolute_position().y + element.size().height;
    assert!(
        bottom(&sections) <= bottom(&sidebar) + 0.5,
        "the sections stay inside the sidebar, above the player bar"
    );
    // Elements wholly out of view can't be found.
    let about = || {
        ElementHandle::find_by_accessible_label(&window, "About")
            .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
    };
    assert!(
        about().is_none_or(|about| bottom(&about) > bottom(&sections)),
        "the last section starts out of view in the smallest window"
    );
    // From the start: Search, Albums, Artists, Playlists, Songs, Folders, Settings,
    // About.
    for _ in 0..8 {
        press(&window, Key::Tab);
    }
    let about = about().expect("About in view");
    assert!(
        bottom(&about) <= bottom(&sections) + 0.5,
        "focusing About scrolls it into view"
    );
}

#[test]
fn the_sidebar_shows_scan_progress_under_its_label() {
    use vcmp_app::{LibraryState, ScanPhase};
    let window = open_window();
    let state = window.global::<LibraryState>();
    state.set_phase(ScanPhase::Reading);
    state.set_done(50);
    state.set_total(100);

    let label = ElementHandle::find_by_accessible_label(&window, "Scanning your music")
        .next()
        .expect("the scan label");
    let bar = find(&window, AccessibleRole::ProgressIndicator, "Scan progress");
    assert_eq!(bar.accessible_value().as_deref(), Some("50"));
    assert!(
        label.size().height < 40.0,
        "the label is one line, not stretched over the sidebar: {:?}",
        label.size()
    );
    let label_bottom = label.absolute_position().y + label.size().height;
    let gap = bar.absolute_position().y - label_bottom;
    assert!(
        (0.0..=16.0).contains(&gap),
        "the bar sits right under its label, not {gap} px below it"
    );
}

#[test]
fn a_fast_wheel_scrolls_once_a_frame_and_all_the_way() {
    use slint::{LogicalPosition, ModelRc, VecModel};
    use std::time::Duration;
    use vcmp_app::SongRow;
    use vcmp_app::wheel::{LINE, WheelScroller};

    let window = open_window();
    window.window().set_size(LogicalSize::new(1100.0, 800.0));
    let songs: Vec<SongRow> = (0..300)
        .map(|n| SongRow {
            title: format!("Song {n}").into(),
            ..SongRow::default()
        })
        .collect();
    window.set_songs(ModelRc::new(VecModel::from(songs)));
    open_page(&window, "Songs");
    let shown = |title: &str| {
        ElementHandle::find_by_accessible_label(&window, title)
            .any(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
    };
    assert!(shown("Song 0"));
    let row = ElementHandle::find_by_accessible_label(&window, "Song 0")
        .find(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
        .unwrap();
    let over = LogicalPosition::new(
        row.absolute_position().x + 100.0,
        row.absolute_position().y + 100.0,
    );

    // Twenty notches at once, as a free-spinning wheel sends them: 1200 px, 30 rows.
    let scroller = WheelScroller::new(&window);
    for _ in 0..20 {
        scroller.add(over, 0.0, -LINE);
    }
    assert!(shown("Song 0"), "nothing moves until the next frame");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(16));
    assert!(!shown("Song 0"), "the first frame scrolls a third");
    for _ in 0..30 {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(16));
    }
    assert!(
        shown("Song 30") && !shown("Song 29"),
        "all the way, and no further"
    );

    // With reduced motion the whole distance goes at once.
    window.global::<Theme>().set_motion_choice(ReduceMotion::On);
    scroller.add(over, 0.0, LINE * 10.0);
    assert!(shown("Song 15") && !shown("Song 14"));
}

#[test]
fn a_hovered_song_stays_highlighted_while_the_pointer_rests() {
    use slint::{LogicalPosition, ModelRc, VecModel};
    use std::time::Duration;
    use vcmp_app::SongRow;

    let window = open_window();
    window.window().set_size(LogicalSize::new(1100.0, 800.0));
    let songs: Vec<SongRow> = (0..20)
        .map(|n| SongRow {
            title: format!("Song {n}").into(),
            // A path apart from the playing track's (none), so no row is playing.
            path: format!("/music/{n}.flac").into(),
            ..SongRow::default()
        })
        .collect();
    window.set_songs(ModelRc::new(VecModel::from(songs)));
    open_page(&window, "Songs");
    let row = ElementHandle::find_by_accessible_label(&window, "Song 3")
        .find(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
        .unwrap();
    // The hover shows the play button in the row.
    let icons = || ElementHandle::find_by_element_type_name(&window, "Icon").count();
    let before = icons();
    let (position, size) = (row.absolute_position(), row.size());
    for x in [200.0, 260.0, 320.0] {
        window.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(position.x + x, position.y + size.height / 2.0),
        });
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(16));
    }
    assert_eq!(icons(), before + 1, "the hovered row shows its play button");
    for _ in 0..20 {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(50));
        assert_eq!(icons(), before + 1, "and keeps it while the pointer rests");
    }
}

#[test]
fn the_queue_and_now_playing_animate_out_too() {
    use std::time::Duration;
    use vcmp_app::PlayerState;

    let window = open_window();
    window.window().set_size(LogicalSize::new(1100.0, 800.0));
    let count = |name: &str| ElementHandle::find_by_element_type_name(&window, name).count();
    let panel_x = || {
        ElementHandle::find_by_element_type_name(&window, "QueuePanel")
            .next()
            .map(|panel| panel.absolute_position().x)
    };
    let wait = |ms: u64| i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));

    window.set_queue_open(true);
    panel_x();
    wait(1000);
    assert_eq!(panel_x(), Some(1100.0 - 320.0));
    window.set_queue_open(false);
    panel_x();
    wait(100);
    let leaving = panel_x().expect("still there while it slides out");
    assert!(leaving > 780.0 && leaving < 1100.0, "on its way: {leaving}");
    wait(1000);
    assert_eq!(count("QueuePanel"), 0, "gone once it is out");

    let player = window.global::<PlayerState>();
    player.set_now_playing_open(true);
    count("NowPlayingView");
    wait(1000);
    player.set_now_playing_open(false);
    count("NowPlayingView");
    wait(100);
    assert_eq!(count("NowPlayingView"), 1, "still there while it fades out");
    wait(1000);
    assert_eq!(count("NowPlayingView"), 0);

    // With reduced motion both leave at once.
    window.global::<Theme>().set_motion_choice(ReduceMotion::On);
    window.set_queue_open(true);
    player.set_now_playing_open(true);
    count("QueuePanel");
    window.set_queue_open(false);
    player.set_now_playing_open(false);
    wait(0);
    assert_eq!((count("QueuePanel"), count("NowPlayingView")), (0, 0));
}
