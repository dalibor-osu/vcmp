//! An animated album cover: its frames are wanted only while its page is in sight
//! with motion allowed, and under reduced motion the page shows the still.

mod common;

use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, Rgba, RgbaImage};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};
use vcmp_app::moving_pictures::{self, Picture};
use vcmp_app::{AppWindow, ReduceMotion, Theme, library_view};
use vcmp_library::Library;

/// A window showing one album whose folder holds an animated `cover.gif`.
fn open(motion: ReduceMotion) -> (tempfile::TempDir, AppWindow, Rc<library_view::LibraryView>) {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    add(&music, "a.flac", Song::new("Spin", "Looper", "Moving"));
    std::fs::write(music.join("cover.gif"), gif()).unwrap();

    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();

    let window = vcmp_app::create_window().unwrap();
    window.global::<Theme>().set_motion_choice(motion);
    let browser = library_view::LibraryView::connect(&window, Rc::new(library)).unwrap();
    window.show().unwrap();
    (dir, window, browser)
}

fn gif() -> Vec<u8> {
    let mut data = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut data);
        encoder.set_repeat(Repeat::Infinite).unwrap();
        for colour in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            encoder
                .encode_frame(Frame::from_parts(
                    RgbaImage::from_pixel(32, 32, Rgba(colour)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
    }
    data
}

/// Opens the album, and lets the page build its picture and run its change handlers.
fn open_album(window: &AppWindow) {
    ElementHandle::find_by_accessible_label(window, "Moving, Looper")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .expect("the album's card")
        .invoke_accessible_default_action();
    settle(window);
}

fn settle(window: &AppWindow) {
    // Looking the page up creates the elements its conditions show.
    let _ = ElementHandle::find_by_accessible_label(window, "Play").count();
    mock_elapsed_time(Duration::ZERO);
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

#[test]
fn reduced_motion_keeps_the_cover_still() {
    let (_dir, window, _browser) = open(ReduceMotion::On);
    open_album(&window);

    let album = window.get_album_detail();
    assert!(album.has_cover, "the first frame shows as the cover");
    assert!(!moving_pictures::is_wanted(Picture::Album));
    assert_eq!(album.frames.row_count(), 0, "no frames are decoded");
}

#[test]
fn the_cover_moves_only_while_its_page_is_in_sight() {
    let (_dir, window, _browser) = open(ReduceMotion::Off);
    assert!(!moving_pictures::is_wanted(Picture::Album));

    open_album(&window);
    assert!(window.get_album_detail().has_cover);
    assert!(moving_pictures::is_wanted(Picture::Album));

    // Reducing motion stops it where it is.
    window.global::<Theme>().set_motion_choice(ReduceMotion::On);
    settle(&window);
    assert!(!moving_pictures::is_wanted(Picture::Album));
    window
        .global::<Theme>()
        .set_motion_choice(ReduceMotion::Off);
    settle(&window);
    assert!(moving_pictures::is_wanted(Picture::Album));

    // Back on the grid, its frames are let go of.
    press(&window, Key::Escape);
    settle(&window);
    assert!(!window.get_album_open());
    assert!(!moving_pictures::is_wanted(Picture::Album));
    assert_eq!(window.get_album_detail().frames.row_count(), 0);
}
