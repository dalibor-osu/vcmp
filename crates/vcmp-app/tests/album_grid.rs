//! The album grid with many albums: cards exist only for the rows in view, the range
//! follows scrolling, and cards are reachable by assistive technology.

use std::cell::Cell;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, LogicalSize};
use vcmp_app::albums::AlbumBrowser;
use vcmp_library::{Album, AlbumId};

fn albums(count: usize) -> Vec<Album> {
    (0..count)
        .map(|i| Album {
            id: AlbumId(i as i64 + 1),
            title: format!("Album {i}"),
            artist: (i % 2 == 0).then(|| format!("Artist {}", i / 10)),
            artist_id: None,
            year: None,
            track_count: 10,
            fetched_cover: false,
            musicbrainz_id: None,
            unknown: false,
            artwork: None,
        })
        .collect()
}

fn cards(window: &vcmp_app::AppWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_type_name(window, "AlbumCard")
        .filter(|card| card.accessible_role() == Some(AccessibleRole::Button))
        .collect()
}

#[test]
fn only_rows_in_view_get_cards() {
    i_slint_backend_testing::init_no_event_loop();
    let window = vcmp_app::create_window().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 720.0));
    let browser = AlbumBrowser::connect(&window, &albums(10_000));
    window.show().unwrap();

    assert_eq!(browser.stats().first_visible, 0);
    let position = LogicalPosition::new(700.0, 400.0);
    let scroll = |times: usize| {
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        for _ in 0..times {
            window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position,
                    delta_x: 0.0,
                    delta_y: -500.0,
                });
        }
    };

    scroll(1);
    let created = cards(&window).len();
    assert!(
        (5..200).contains(&created),
        "{created} cards for 10,000 albums"
    );
    scroll(20);
    assert!(
        browser.stats().first_visible > 0,
        "scrolling moved the range"
    );
    assert_eq!(cards(&window).len(), created, "cards are reused, not added");
}

#[test]
fn cards_are_labelled_and_activate() {
    i_slint_backend_testing::init_no_event_loop();
    let window = vcmp_app::create_window().unwrap();
    let _browser = AlbumBrowser::connect(&window, &albums(3));
    window.show().unwrap();
    let activated = Rc::new(Cell::new(None));
    let sink = activated.clone();
    window.on_album_activated(move |index| sink.set(Some(index)));

    // Spare cards past the end of the list are hidden and have an empty label.
    let labels: Vec<String> = cards(&window)
        .iter()
        .filter_map(|card| card.accessible_label().map(String::from))
        .filter(|label| !label.is_empty())
        .collect();
    assert_eq!(
        labels,
        ["Album 0, Artist 0", "Album 1", "Album 2, Artist 0"]
    );

    cards(&window)[1].invoke_accessible_default_action();
    assert_eq!(activated.get(), Some(1));
}
