//! Opening an album from the grid, its page, and every way back, with a real library
//! built from the library crate's silent fixtures.

mod common;

use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};
use vcmp_app::{AppWindow, Page, TrackRow, library_view};
use vcmp_library::Library;

/// A window showing a library with a two-disc compilation and a plain album.
fn open() -> (tempfile::TempDir, AppWindow, Rc<library_view::LibraryView>) {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    std::fs::create_dir(&music).unwrap();
    let mix = |title, artist, track, disc| Song {
        title,
        artist,
        album: "Summer Mix",
        album_artist: Some("Various Artists"),
        track,
        disc,
    };
    add(&music, "a.flac", mix("Opening", "Singer", 1, 1));
    add(&music, "b.flac", mix("Middle", "Various Artists", 2, 1));
    add(&music, "c.flac", mix("Closing", "Rapper", 1, 2));
    add(
        &music,
        "d.flac",
        Song {
            title: "Solo",
            artist: "Zed",
            album: "Zed Alone",
            album_artist: None,
            track: 1,
            disc: 1,
        },
    );

    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();

    let window = vcmp_app::create_window().unwrap();
    let browser = library_view::LibraryView::connect(&window, Rc::new(library)).unwrap();
    window.show().unwrap();
    (dir, window, browser)
}

fn rows(window: &AppWindow) -> Vec<TrackRow> {
    window.get_album_tracks().iter().collect()
}

fn press(window: &AppWindow, key: Key) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn card(window: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no card labelled {label:?}"))
}

#[test]
fn an_album_page_lists_its_tracks_by_disc() {
    let (_dir, window, _browser) = open();
    card(&window, "Summer Mix, Various Artists").invoke_accessible_default_action();

    assert!(window.get_album_open());
    let album = window.get_album_detail();
    assert_eq!(
        (
            album.title.as_str(),
            album.artist.as_str(),
            album.track_count
        ),
        ("Summer Mix", "Various Artists", 3)
    );
    assert!(!album.failed);
    assert!(window.get_album_multi_disc());

    let summary: Vec<(String, String, i32, bool)> = rows(&window)
        .into_iter()
        .map(|row| {
            (
                row.title.into(),
                row.artist.into(),
                row.disc,
                row.disc_start,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("Opening".into(), "Singer".into(), 1, true),
            // The album artist itself is not repeated on the track.
            ("Middle".into(), "".into(), 1, false),
            ("Closing".into(), "Rapper".into(), 2, true),
        ]
    );
    assert!(rows(&window).iter().all(|row| row.duration == "0:01"));
}

#[test]
fn a_single_disc_album_has_no_disc_headings() {
    let (_dir, window, _browser) = open();
    card(&window, "Zed Alone, Zed").invoke_accessible_default_action();
    assert!(!window.get_album_multi_disc());
    assert_eq!(
        rows(&window)[0].artist,
        "",
        "the track artist is the album artist"
    );
}

#[test]
fn back_button_escape_and_the_sidebar_return_to_the_grid() {
    let (_dir, window, _browser) = open();
    let open_album = || card(&window, "Zed Alone, Zed").invoke_accessible_default_action();

    open_album();
    ElementHandle::find_by_accessible_label(&window, "Albums")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .expect("back button")
        .invoke_accessible_default_action();
    assert!(!window.get_album_open());

    open_album();
    press(&window, Key::Escape);
    assert!(!window.get_album_open(), "Escape goes back");

    open_album();
    ElementHandle::find_by_accessible_label(&window, "Albums")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
        .expect("sidebar entry")
        .invoke_accessible_default_action();
    assert!(
        !window.get_album_open(),
        "choosing Albums again shows the grid"
    );
    assert_eq!(window.get_current_page(), Page::Albums);
}

#[test]
fn the_grid_works_from_the_keyboard() {
    let (_dir, window, _browser) = open();
    // Search, Albums, Artists, Playlists, Songs, Folders, Settings and About in the
    // sidebar, the albums filter, then the grid.
    for _ in 0..10 {
        press(&window, Key::Tab);
    }
    press(&window, Key::RightArrow);
    press(&window, Key::Return);
    assert!(window.get_album_open());
    assert_eq!(
        window.get_album_detail().title,
        "Zed Alone",
        "the second album opened"
    );

    press(&window, Key::Escape);
    press(&window, Key::LeftArrow);
    press(&window, Key::Return);
    assert_eq!(
        window.get_album_detail().title,
        "Summer Mix",
        "focus came back to the grid"
    );
}

#[test]
fn the_album_page_fits_the_smallest_window() {
    let (_dir, window, _browser) = open();
    let width = 720.0;
    window
        .window()
        .set_size(slint::LogicalSize::new(width, 480.0));
    card(&window, "Summer Mix, Various Artists").invoke_accessible_default_action();

    let texts = ElementHandle::find_by_element_type_name(&window, "Text");
    for text in texts.filter(|text| {
        text.accessible_label()
            .is_some_and(|label| !label.is_empty())
    }) {
        let (left, right) = (
            text.absolute_position().x,
            text.absolute_position().x + text.size().width,
        );
        let label = text.accessible_label().unwrap_or_default();
        assert!(
            left >= 0.0 && right <= width + 0.5,
            "{label:?} spans x {left}..{right}"
        );
    }
}

#[test]
fn the_filter_narrows_the_albums() {
    let (_dir, window, _browser) = open();
    window.set_albums_filter("zed".into());
    window.invoke_albums_filter_edited("zed".into());
    let titles: Vec<String> = window
        .get_albums()
        .iter()
        .map(|tile| tile.title.to_string())
        .collect();
    assert_eq!(titles, ["Zed Alone"]);
    window.invoke_album_activated(0);
    assert_eq!(
        window.get_album_detail().title,
        "Zed Alone",
        "the card shown opens"
    );
    window.set_album_open(false);

    window.set_albums_filter("various mix".into());
    window.invoke_albums_filter_edited("various mix".into());
    assert_eq!(window.get_albums().row_count(), 1, "by artist and title");
}

#[test]
fn an_open_album_follows_its_tracks_to_a_new_title() {
    let (dir, window, browser) = open();
    card(&window, "Zed Alone, Zed").invoke_accessible_default_action();
    let music = dir.path().join("music");
    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();

    // The tag editor, or another program, renames the album.
    add(
        &music,
        "d.flac",
        Song {
            title: "Solo",
            artist: "Zed",
            album: "Zed Together",
            album_artist: None,
            track: 1,
            disc: 1,
        },
    );
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    browser.reload();
    let detail = window.get_album_detail();
    assert_eq!(
        detail.title, "Zed Together",
        "the page shows the renamed album"
    );
    assert!(!detail.missing);
    assert_eq!(rows(&window).len(), 1);
    assert!(window.get_album_open());

    // Tracks that are gone leave the page marked as such.
    std::fs::remove_file(music.join("d.flac")).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    browser.reload();
    assert!(window.get_album_detail().missing);
}

#[test]
fn a_section_opens_on_its_grid_not_its_last_detail() {
    let (_dir, window, _browser) = open();
    let tab = |label: &str| {
        ElementHandle::find_by_accessible_label(&window, label)
            .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
            .unwrap_or_else(|| panic!("no tab {label:?}"))
            .invoke_accessible_default_action();
    };
    card(&window, "Zed Alone, Zed").invoke_accessible_default_action();
    tab("Songs");
    tab("Albums");
    assert_eq!(window.get_current_page(), Page::Albums);
    assert!(!window.get_album_open(), "Albums shows the grid");

    tab("Artists");
    window.invoke_artist_activated(0);
    assert!(window.get_artist_open());
    tab("Search");
    tab("Artists");
    assert!(!window.get_artist_open(), "Artists shows the grid");
}

#[test]
fn the_page_marker_sits_on_its_section_in_every_group() {
    let (_dir, window, _browser) = open();
    for label in ["Search", "Albums", "Folders", "Settings", "About"] {
        let section = ElementHandle::find_by_accessible_label(&window, label)
            .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
            .unwrap_or_else(|| panic!("no tab {label:?}"));
        section.invoke_accessible_default_action();
        // The marker slides there.
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_secs(1));
        let marker = ElementHandle::find_by_element_id(&window, "AppWindow::marker")
            .next()
            .expect("the page marker");
        let centre = marker.absolute_position().y + marker.size().height / 2.0;
        let top = section.absolute_position().y;
        let bottom = top + section.size().height;
        assert!(
            (top..bottom).contains(&centre),
            "{label}: marker at {centre}, section {top}..{bottom}"
        );
    }
}

#[test]
fn the_track_list_spans_the_page() {
    let (_dir, window, _browser) = open();
    window
        .window()
        .set_size(slint::LogicalSize::new(1600.0, 900.0));
    card(&window, "Summer Mix, Various Artists").invoke_accessible_default_action();
    let row = ElementHandle::find_by_accessible_label(&window, "Opening, Singer")
        .find(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
        .expect("a track row");
    // The sidebar takes 220 px and the page's padding 40 px on each side.
    let page = 1600.0 - 220.0 - 2.0 * 40.0;
    assert!(
        (row.size().width - page).abs() < 2.0,
        "the row is {} wide, the page {page}",
        row.size().width
    );
}

#[test]
fn tracks_without_an_album_show_in_unknown_albums() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    add(&music, "a.flac", Song::new("Solo", "Zed", "Zed Alone"));
    add(&music, "b.flac", Song::new("Loose", "Zed", ""));
    add(&music, "c.flac", Song::new("Nobody", "", ""));
    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();
    let window = vcmp_app::create_window().unwrap();
    let _view = library_view::LibraryView::connect(&window, Rc::new(library)).unwrap();
    window.show().unwrap();

    // Zed's own album, then Zed's Unknown one, then the one for tracks naming neither.
    card(&window, "Zed Alone, Zed");
    card(&window, "Unknown, Zed").invoke_accessible_default_action();
    let detail = window.get_album_detail();
    assert!(detail.unknown);
    assert_eq!(detail.initial, "?", "a question mark for a cover");
    let titles: Vec<String> = rows(&window)
        .iter()
        .map(|row| row.title.to_string())
        .collect();
    assert_eq!(titles, ["Loose"]);
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Find a cover for Unknown online")
            .next()
            .is_none(),
        "no cover to find for an Unknown album"
    );
    press(&window, Key::Escape);
    card(&window, "Unknown, Unknown artist");
}
