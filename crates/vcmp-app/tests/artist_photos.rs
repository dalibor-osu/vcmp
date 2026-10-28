//! An artist's photo: "Find photo online" with a fake standing in for MusicBrainz and
//! fanart.tv, a photo from a file, removing it, and where it shows.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, SharedString};
use tempfile::TempDir;
use vcmp_app::artist_photos::{ArtistPhotos, PhotoSource};
use vcmp_app::library_view::LibraryView;
use vcmp_app::{AppWindow, ArtworkState, PhotoSearch, PhotoSearchStatus};
use vcmp_integrations::fanart::{ArtistQuery, Photo, PhotoEvent, SearchError};
use vcmp_library::Library;

/// What was asked of the photo source.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    /// With the personal key that went along.
    Search(u64, ArtistQuery, Option<String>),
    Download(u64, String),
    Cancel,
}

#[derive(Clone, Default)]
struct FakeSource(Rc<RefCell<Vec<Asked>>>);

impl PhotoSource for FakeSource {
    fn search(&self, search: u64, artist: ArtistQuery, personal_key: Option<String>) {
        self.0
            .borrow_mut()
            .push(Asked::Search(search, artist, personal_key));
    }

    fn download(&self, search: u64, url: String) {
        self.0.borrow_mut().push(Asked::Download(search, url));
    }

    fn cancel(&self) {
        self.0.borrow_mut().push(Asked::Cancel);
    }
}

impl FakeSource {
    fn take(&self) -> Vec<Asked> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

struct App {
    _dir: TempDir,
    window: AppWindow,
    library: Rc<Library>,
    view: Rc<LibraryView>,
    photos: Rc<ArtistPhotos>,
    source: FakeSource,
}

/// A window with one artist, without a photo, whose page is open.
fn open(online: bool) -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    add(&music, "a.flac", Song::new("Solo", "Band", "Record"));
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
    let photos = ArtistPhotos::connect(
        &window,
        library.clone(),
        view.clone(),
        online.then(|| Box::new(source.clone()) as Box<dyn PhotoSource>),
    );
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 900.0));
    let band = library.artists().unwrap()[0].id;
    view.open_artist_by_id(band);
    window.set_selected_index(2);
    mock_elapsed_time(Duration::ZERO);
    App {
        _dir: dir,
        window,
        library,
        view,
        photos,
        source,
    }
}

fn png(size: u32, colour: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(size, size, image::Rgb(colour));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn key(window: &AppWindow, key: Key) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

impl App {
    fn state(&self) -> PhotoSearch<'_> {
        self.window.global::<PhotoSearch>()
    }

    fn button(&self, label: &str) -> Option<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
    }

    /// Lets the window draw: shown elements are made, change handlers run.
    fn frame(&self) {
        slint::platform::update_timers_and_animations();
        let _ = ElementHandle::find_by_accessible_label(&self.window, "Choose a photo").count();
        mock_elapsed_time(Duration::ZERO);
    }

    fn band(&self) -> vcmp_library::ArtistId {
        self.library.artists().unwrap()[0].id
    }

    fn photo(name: &str, url: &str) -> PhotoEvent {
        PhotoEvent::Found(Photo {
            url: url.into(),
            artist: name.into(),
            disambiguation: Some("UK rock band".into()),
            preview: png(32, [0, 200, 0]),
        })
    }
}

const A: &str = "https://assets.fanart.tv/fanart/music/x/artistthumb/a.jpg";
const B: &str = "https://assets.fanart.tv/fanart/music/x/artistthumb/b.jpg";

#[test]
fn a_photo_found_online_shows_on_the_page_and_the_card() {
    let app = open(true);
    assert!(!app.window.get_artist_detail().has_photo);
    // Find photo online, in the More menu: Slint's menus are not in the tree the
    // tests search, so the item's action runs directly.
    app.button("More for Band").expect("the menu is offered");
    app.state().invoke_find();
    assert!(app.state().get_open());
    assert_eq!(app.state().get_status(), PhotoSearchStatus::Searching);
    app.frame();
    let search = match app.source.take().as_slice() {
        [Asked::Search(id, query, None)] => {
            assert_eq!(
                query,
                &ArtistQuery {
                    name: "Band".into(),
                    mbid: None
                }
            );
            *id
        }
        other => panic!("{other:?}"),
    };

    app.photos.handle(search, App::photo("Band", A));
    app.photos.handle(search, App::photo("Band", B));
    app.photos.handle(search, PhotoEvent::Searched(Ok(())));
    assert_eq!(app.state().get_status(), PhotoSearchStatus::Found);
    let tiles = app.state().get_photos();
    assert_eq!(tiles.row_count(), 2);
    let tile = tiles.row_data(0).unwrap();
    assert_eq!(
        (tile.title.as_str(), tile.artist.as_str(), tile.artwork),
        ("Band", "UK rock band", ArtworkState::Loaded)
    );

    // Enter picks the focused (first) photo; the picker focuses it from a change
    // handler, which the event loop runs.
    app.frame();
    key(&app.window, Key::Return);
    assert_eq!(app.state().get_status(), PhotoSearchStatus::Downloading);
    assert_eq!(app.source.take(), [Asked::Download(search, A.into())]);
    app.photos
        .handle(search, PhotoEvent::Downloaded(Ok(png(900, [200, 0, 0]))));
    assert!(!app.state().get_open(), "the picker closes");
    assert!(app.window.get_artist_detail().has_photo);
    assert!(app.window.get_artist_detail().photo_from_fanart);
    app.frame();
    app.button("Photo from fanart.tv, opens the website")
        .expect("the photo is credited to fanart.tv");
    let photo = app.library.artist_photo(app.band()).unwrap().unwrap();
    assert!(photo.starts_with(app._dir.path()));

    // The artist card shows it too.
    let card = app.window.get_artists().row_data(0).unwrap();
    assert_eq!(card.title, "Band");
    assert_ne!(card.artwork, ArtworkState::None, "the card has a picture");

    // An answer to an old search changes nothing.
    app.photos.handle(search, PhotoEvent::Searched(Ok(())));
    assert!(!app.state().get_open());
}

#[test]
fn nothing_found_failures_and_cancel_are_said() {
    let app = open(true);
    app.view.refresh_artist_pictures();
    app.state().invoke_find();
    let Asked::Search(search, ..) = app.source.take()[0].clone() else {
        panic!()
    };
    app.photos.handle(search, PhotoEvent::Searched(Ok(())));
    assert_eq!(app.state().get_status(), PhotoSearchStatus::NothingFound);

    app.state().invoke_find();
    let Asked::Search(search, ..) = app.source.take()[0].clone() else {
        panic!()
    };
    app.photos.handle(
        search,
        PhotoEvent::Searched(Err(SearchError::Failed("offline".into()))),
    );
    assert_eq!(app.state().get_status(), PhotoSearchStatus::Failed);
    app.button("Search again").expect("a retry is offered");

    key(&app.window, Key::Escape);
    assert!(!app.state().get_open());
    assert_eq!(app.source.take(), [Asked::Cancel]);
}

#[test]
fn a_photo_from_a_file_can_be_removed_again() {
    let app = open(false);
    assert!(
        !app.state().get_available(),
        "no online search without fanart.tv's key"
    );
    app.photos.use_file(b"not a picture");
    assert!(app.state().get_file_failed());
    assert!(!app.window.get_artist_detail().has_photo);

    app.photos.use_file(&png(64, [0, 0, 200]));
    assert!(!app.state().get_file_failed());
    assert!(app.window.get_artist_detail().has_photo);
    assert!(
        !app.window.get_artist_detail().photo_from_fanart,
        "a file of the owner's isn't credited"
    );
    assert!(app.window.get_artist_detail().has_picture);

    // Remove photo, in the More menu.
    app.state().invoke_remove();
    assert!(!app.window.get_artist_detail().has_photo);
    assert_eq!(app.library.artist_photo(app.band()).unwrap(), None);
}

#[test]
fn a_personal_key_is_kept_and_goes_with_each_search() {
    let app = open(true);
    assert_eq!(app.state().get_personal_key(), "");
    app.state().invoke_set_personal_key("  my key  ".into());
    assert_eq!(app.state().get_personal_key(), "my key", "trimmed");
    assert_eq!(
        app.library
            .setting(vcmp_app::artist_photos::SETTING_FANART_KEY)
            .unwrap()
            .as_deref(),
        Some("my key")
    );
    app.state().invoke_find();
    assert!(matches!(
        app.source.take().as_slice(),
        [Asked::Search(_, _, Some(key))] if key == "my key"
    ));

    // Read back when VCMP starts again.
    let restarted = vcmp_app::create_window().unwrap();
    let _photos = ArtistPhotos::connect(
        &restarted,
        app.library.clone(),
        app.view.clone(),
        Some(Box::new(FakeSource::default())),
    );
    assert_eq!(
        restarted.global::<PhotoSearch>().get_personal_key(),
        "my key"
    );

    // A refused key is said apart from an unreachable service.
    let Asked::Search(search, ..) = ({
        app.state().invoke_find();
        app.source.take()[0].clone()
    }) else {
        panic!()
    };
    app.photos
        .handle(search, PhotoEvent::Searched(Err(SearchError::Refused(401))));
    assert_eq!(app.state().get_status(), PhotoSearchStatus::Refused);
    key(&app.window, Key::Escape);
    assert_eq!(app.source.take(), [Asked::Cancel]);

    // Removing it searches with VCMP's key alone.
    app.state().invoke_set_personal_key("".into());
    app.state().invoke_find();
    assert!(matches!(
        app.source.take().as_slice(),
        [Asked::Search(_, _, None)]
    ));
}

/// Whether Settings shows the personal key field.
fn key_field_shown(app: &App) -> bool {
    app.window.set_selected_index(7);
    app.frame();
    ElementHandle::find_by_accessible_label(&app.window, "fanart.tv personal API key")
        .next()
        .is_some()
}

#[test]
fn settings_and_about_tell_where_photos_come_from() {
    let app = open(true);
    assert!(key_field_shown(&app), "Settings offers the personal key");
    app.window.set_selected_index(8);
    app.frame();
    assert!(app.button("fanart.tv, opens the website").is_some());
    assert!(app.button("Cover Art Archive, opens the website").is_some());
    assert!(app.button("MusicBrainz, opens the website").is_some());
}

#[test]
fn a_build_that_cant_search_asks_for_no_personal_key() {
    let app = open(false);
    assert!(!key_field_shown(&app));
}
