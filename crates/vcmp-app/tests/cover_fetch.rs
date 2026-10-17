//! "Find cover online" on an album page: the action, the picker and what a picked
//! cover changes, with a fake standing in for MusicBrainz and the Cover Art Archive.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};
use tempfile::TempDir;
use vcmp_app::cover_fetch::{CoverFetch, CoverSource};
use vcmp_app::library_view::LibraryView;
use vcmp_app::{AppWindow, CoverSearch, CoverSearchStatus};
use vcmp_integrations::musicbrainz::{AlbumQuery, Candidate, Cover, PickerEvent};
use vcmp_library::{AlbumId, CoverArtId, Library};

/// What was asked of the cover source.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    Search(u64, AlbumQuery, Option<String>),
    Download(u64, Cover),
    Cancel,
}

#[derive(Clone, Default)]
struct FakeSource(Rc<RefCell<Vec<Asked>>>);

impl CoverSource for FakeSource {
    fn search(&self, search: u64, album: AlbumQuery, tagged_release: Option<String>) {
        self.0
            .borrow_mut()
            .push(Asked::Search(search, album, tagged_release));
    }

    fn download(&self, search: u64, cover: Cover) {
        self.0.borrow_mut().push(Asked::Download(search, cover));
    }

    fn cancel(&self) {
        self.0.borrow_mut().push(Asked::Cancel);
    }
}

impl FakeSource {
    fn take(&self) -> Vec<Asked> {
        std::mem::take(&mut self.0.borrow_mut())
    }

    /// The number of the last search started.
    fn last_search(&self) -> u64 {
        self.0
            .borrow()
            .iter()
            .rev()
            .find_map(|asked| match asked {
                Asked::Search(id, ..) => Some(*id),
                _ => None,
            })
            .expect("a search was started")
    }
}

struct App {
    _dir: TempDir,
    window: AppWindow,
    library: Rc<Library>,
    view: Rc<LibraryView>,
    fetch: Rc<CoverFetch>,
    source: FakeSource,
    changed: Rc<RefCell<Vec<AlbumId>>>,
}

/// A window with two albums, neither with a cover.
fn open() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    add(&music, "a.flac", Song::new("Solo", "Zed", "Zed Alone"));
    add(&music, "b.flac", Song::new("Opening", "Band", "Record"));
    let mut library = Library::open(
        &dir.path().join("library.sqlite3"),
        &dir.path().join("cache"),
    )
    .unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();

    let library = Rc::new(library);
    let window = vcmp_app::create_window().unwrap();
    let view = LibraryView::connect(&window, library.clone()).unwrap();
    let source = FakeSource::default();
    let fetch = CoverFetch::connect(
        &window,
        library.clone(),
        view.clone(),
        Box::new(source.clone()),
    );
    let changed = Rc::new(RefCell::new(Vec::new()));
    let record = changed.clone();
    fetch.on_cover_changed(move |album| record.borrow_mut().push(album));
    window.show().unwrap();
    App {
        _dir: dir,
        window,
        library,
        view,
        fetch,
        source,
        changed,
    }
}

impl App {
    fn state(&self) -> CoverSearch<'_> {
        self.window.global::<CoverSearch>()
    }

    /// The first element with `role` and `label`. Both pages that show albums hold
    /// an album page; the Albums page's comes first.
    fn find(&self, role: AccessibleRole, label: &str) -> Option<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(role))
    }

    fn press(&self, role: AccessibleRole, label: &str) {
        self.find(role, label)
            .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
            .invoke_accessible_default_action();
    }

    /// What a turn of the event loop does before drawing: runs pending change
    /// handlers and creates the elements that just appeared.
    fn frame(&self) {
        slint::platform::update_timers_and_animations();
        let _ = ElementHandle::find_by_accessible_label(&self.window, "Choose a cover").count();
        slint::platform::update_timers_and_animations();
    }

    fn key(&self, key: Key) {
        let text: SharedString = key.into();
        let window = self.window.window();
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
    }

    fn album(&self, title: &str) -> AlbumId {
        self.library
            .albums()
            .unwrap()
            .into_iter()
            .find(|album| album.title == title)
            .unwrap()
            .id
    }

    /// The picker's covers as (title, details).
    fn covers(&self) -> Vec<(String, String)> {
        self.state()
            .get_covers()
            .iter()
            .map(|tile| (tile.title.into(), tile.artist.into()))
            .collect()
    }

    fn shows_text(&self, text: &str) -> bool {
        ElementHandle::find_by_accessible_label(&self.window, text)
            .next()
            .is_some()
    }
}

fn png(colour: [u8; 3]) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(32, 32, image::Rgb(colour))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn candidate(cover: Cover, year: Option<i32>, country: Option<&str>) -> PickerEvent {
    PickerEvent::Found(Candidate {
        cover,
        title: "Zed Alone".into(),
        year,
        country: country.map(String::from),
        preview: png([200, 40, 160]),
    })
}

#[test]
fn a_picked_cover_becomes_the_albums_cover() {
    let app = open();
    app.press(AccessibleRole::Button, "Zed Alone, Zed");
    assert!(app.window.get_album_open());
    assert!(!app.window.get_album_detail().has_cover);
    assert_eq!(app.source.take(), [], "nothing is fetched unasked");

    app.press(AccessibleRole::Button, "Find a cover for Zed Alone online");
    assert!(app.state().get_open());
    assert_eq!(app.state().get_status(), CoverSearchStatus::Searching);
    assert_eq!(app.state().get_album_title(), "Zed Alone");
    let search = app.source.last_search();
    assert_eq!(
        app.source.take(),
        [Asked::Search(
            search,
            AlbumQuery {
                artist: "Zed".into(),
                title: "Zed Alone".into(),
                track_count: 1,
            },
            None
        )]
    );

    // Covers appear as they are found; answers to other searches are dropped.
    let release = Cover::Release("r-1".into());
    app.fetch
        .handle(search, candidate(release.clone(), Some(2001), Some("GB")));
    app.fetch.handle(
        search,
        candidate(Cover::ReleaseGroup("rg-1".into()), None, None),
    );
    app.fetch.handle(
        search + 7,
        candidate(Cover::Release("x".into()), None, None),
    );
    app.fetch.handle(search, PickerEvent::Searched(Ok(())));
    assert_eq!(app.state().get_status(), CoverSearchStatus::Found);
    assert_eq!(
        app.covers(),
        [
            ("Zed Alone".into(), "2001 · GB".into()),
            ("Zed Alone".into(), "".into())
        ]
    );

    app.press(AccessibleRole::Button, "Zed Alone, 2001 · GB");
    assert_eq!(app.source.take(), [Asked::Download(search, release)]);
    assert_eq!(app.state().get_status(), CoverSearchStatus::Downloading);
    app.press(AccessibleRole::Button, "Zed Alone");
    assert_eq!(app.source.take(), [], "one download at a time");

    app.fetch
        .handle(search, PickerEvent::Downloaded(Ok(png([30, 102, 245]))));
    assert!(!app.state().get_open(), "the picker closes");
    let album = app.album("Zed Alone");
    assert!(app.window.get_album_detail().has_cover, "the page shows it");
    assert!(
        app.view
            .browser()
            .album(1)
            .is_some_and(|card| card.title == "Zed Alone" && card.artwork.is_some()),
        "so does the grid"
    );
    assert_eq!(*app.changed.borrow(), [album], "and the player is told");
    assert_eq!(
        app.library.cover_lookup(album).unwrap().unwrap().cover,
        Some(CoverArtId::Release("r-1".into())),
        "Discord shows the same cover"
    );
    assert!(
        app.find(AccessibleRole::Button, "Find a cover for Zed Alone online")
            .is_none(),
        "an album with a cover offers no search"
    );
    assert!(app.window.get_album_detail().fetched_cover);

    // A fetched cover can be changed: searched again, or taken away.
    app.press(AccessibleRole::Button, "Change the cover of Zed Alone");
    app.changed.borrow_mut().clear();
    app.state().invoke_remove();
    assert!(!app.window.get_album_detail().has_cover);
    assert!(!app.window.get_album_detail().fetched_cover);
    assert_eq!(app.library.album(album).unwrap().unwrap().artwork, None);
    assert_eq!(
        app.library.cover_lookup(album).unwrap(),
        None,
        "the automatic lookup may run again"
    );
    assert_eq!(*app.changed.borrow(), [album], "and the player is told");
    app.frame();
    assert!(
        app.find(AccessibleRole::Button, "Find a cover for Zed Alone online")
            .is_some(),
        "the search is offered again"
    );
}

#[test]
fn the_picker_works_from_the_keyboard() {
    let app = open();
    app.press(AccessibleRole::Button, "Record, Band");
    app.frame();
    app.press(AccessibleRole::Button, "Find a cover for Record online");
    app.frame();
    let search = app.source.last_search();
    app.source.take();

    // The focus starts on Cancel, so Enter there would close the picker; it moves to
    // the first cover when that arrives, so Enter picks it.
    app.fetch
        .handle(search, candidate(Cover::Release("r-1".into()), None, None));
    app.frame();
    app.key(Key::Return);
    assert_eq!(
        app.source.take(),
        [Asked::Download(search, Cover::Release("r-1".into()))]
    );
    app.fetch
        .handle(search, PickerEvent::Downloaded(Err("offline".into())));
    assert!(app.state().get_open(), "a failed download keeps the picker");
    assert_eq!(app.state().get_status(), CoverSearchStatus::DownloadFailed);
    assert!(app.shows_text("That cover could not be downloaded. Pick it again, or another one."));

    // Escape cancels, and the focus comes back to the album page, where Escape
    // goes back to the grid.
    app.key(Key::Escape);
    assert!(!app.state().get_open());
    assert_eq!(app.source.take(), [Asked::Cancel]);
    assert!(app.window.get_album_open());
    app.frame();
    app.key(Key::Escape);
    assert!(!app.window.get_album_open());

    // Before any cover arrives, Enter is on Cancel.
    app.press(AccessibleRole::Button, "Record, Band");
    app.frame();
    app.press(AccessibleRole::Button, "Find a cover for Record online");
    app.frame();
    app.source.take();
    app.key(Key::Return);
    assert!(!app.state().get_open());
    assert_eq!(app.source.take(), [Asked::Cancel]);
}

#[test]
fn the_picker_fits_the_smallest_window() {
    let app = open();
    let width = 720.0;
    app.window
        .window()
        .set_size(slint::LogicalSize::new(width, 480.0));
    app.press(AccessibleRole::Button, "Record, Band");
    app.press(AccessibleRole::Button, "Find a cover for Record online");
    let search = app.source.last_search();
    for n in 0..5 {
        app.fetch.handle(
            search,
            candidate(Cover::Release(format!("r-{n}")), Some(2001), Some("JP")),
        );
    }
    app.fetch
        .handle(search, PickerEvent::Searched(Err("busy".into())));
    app.frame();

    let texts = ElementHandle::find_by_element_type_name(&app.window, "Text");
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
    let cards: Vec<_> =
        ElementHandle::find_by_accessible_label(&app.window, "Zed Alone, 2001 · JP").collect();
    // The second row lies below the fold, out of the query's reach.
    assert_eq!(cards.len(), 3, "three covers to a row");
    for card in cards {
        let right = card.absolute_position().x + card.size().width;
        assert!(right <= width + 0.5, "a cover reaches x {right}");
    }
}

#[test]
fn nothing_found_and_failures_are_said_plainly() {
    let app = open();
    app.press(AccessibleRole::Button, "Record, Band");
    app.press(AccessibleRole::Button, "Find a cover for Record online");
    let search = app.source.last_search();
    app.fetch.handle(search, PickerEvent::Searched(Ok(())));
    assert_eq!(app.state().get_status(), CoverSearchStatus::NothingFound);
    assert!(app.shows_text(
        "No covers turned up for this album. MusicBrainz may know it under another title or artist."
    ));
    app.press(AccessibleRole::Button, "Cancel");
    assert!(!app.state().get_open());

    app.press(AccessibleRole::Button, "Find a cover for Record online");
    let search = app.source.last_search();
    app.fetch
        .handle(search, PickerEvent::Searched(Err("offline".into())));
    assert_eq!(app.state().get_status(), CoverSearchStatus::Failed);
    assert!(app.shows_text(
        "MusicBrainz or the Cover Art Archive could not be reached. Check the internet connection and search again."
    ));

    // Searching again starts over; a search that breaks off after finding covers
    // keeps them.
    app.press(AccessibleRole::Button, "Search again");
    let again = app.source.last_search();
    assert_ne!(again, search);
    assert_eq!(app.state().get_status(), CoverSearchStatus::Searching);
    app.fetch
        .handle(again, candidate(Cover::Release("r-1".into()), None, None));
    app.fetch
        .handle(again, PickerEvent::Searched(Err("busy".into())));
    assert_eq!(app.state().get_status(), CoverSearchStatus::CutShort);
    assert_eq!(app.covers().len(), 1);
}
