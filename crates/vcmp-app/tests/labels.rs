//! Labels in the window: the dialog that makes and edits them, the track and album
//! menus that put them on and take them off, the chips rows show, the filters on the
//! Songs and Albums pages, the Labels section with a label's page, and Settings.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::{Song, add};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, mock_elapsed_time};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, SharedString};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{PlayerControl, PlayerService};
use vcmp_app::{
    AppWindow, Confirmation, LabelFilterPage, LabelProblem, Labels, Page, SongSelection, TrackRow,
    TrackSelection,
};
use vcmp_library::{AlbumId, LabelId, Library, TrackId};
use vcmp_playback::Command;

#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<Command>>>);

impl PlayerControl for Recorder {
    fn send(&self, command: Command) {
        self.0.borrow_mut().push(command);
    }
}

struct App {
    _dir: tempfile::TempDir,
    library: Rc<Library>,
    window: AppWindow,
    view: Rc<LibraryView>,
    _service: Rc<PlayerService>,
    sent: Recorder,
}

/// Two songs of one album, and a single on another.
fn start() -> App {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let music = dunce::canonicalize(dir.path()).unwrap().join("music");
    add(&music, "1.flac", Song::new("One", "Band", "Album"));
    add(
        &music,
        "2.flac",
        Song {
            track: 2,
            ..Song::new("Two", "Band", "Album")
        },
    );
    add(&music, "single.flac", Song::new("Single", "Solo", "Lone"));
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
    let sent = Recorder::default();
    let service = PlayerService::connect(
        &window,
        library.clone(),
        view.clone(),
        Box::new(sent.clone()),
        None,
    );
    window.show().unwrap();
    window.window().set_size(LogicalSize::new(1100.0, 2400.0));
    mock_elapsed_time(Duration::ZERO);
    App {
        _dir: dir,
        library,
        window,
        view,
        _service: service,
        sent,
    }
}

impl App {
    fn labels(&self) -> Labels<'_> {
        self.window.global::<Labels>()
    }

    fn id(&self, title: &str) -> TrackId {
        self.library
            .tracks()
            .unwrap()
            .into_iter()
            .find(|track| track.title == title)
            .unwrap_or_else(|| panic!("no track {title:?}"))
            .id
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

    /// Makes a label through the dialog, from Settings.
    fn make(&self, name: &str, palette: i32) -> LabelId {
        self.labels().invoke_edit(-1);
        assert!(self.labels().get_editor_open());
        self.labels()
            .invoke_save(name.into(), palette, Default::default());
        assert!(!self.labels().get_editor_open(), "{name:?} was saved");
        self.library.label_named(name).unwrap().unwrap().id
    }

    fn names_on(&self, title: &str) -> Vec<String> {
        self.library
            .track_labels_of(self.id(title))
            .unwrap()
            .into_iter()
            .map(|label| label.name)
            .collect()
    }

    fn open_album(&self, title: &str) -> ModelRc<TrackRow> {
        let index = self
            .window
            .get_albums()
            .iter()
            .position(|tile| tile.title == title)
            .unwrap();
        self.window.invoke_album_activated(index as i32);
        self.window.get_album_tracks()
    }

    /// Each choice of the menu prepared last: its name, whether it is checked, and
    /// whether only some tracks hold it.
    fn choices(&self) -> Vec<(String, bool, bool)> {
        self.labels()
            .get_choices()
            .iter()
            .map(|choice| (choice.name.to_string(), choice.checked, choice.partial))
            .collect()
    }

    /// Lets the window catch up: a dialog shown, and the answer to a question handed
    /// to the page that asked it.
    fn frame(&self) {
        slint::platform::update_timers_and_animations();
        let _ = ElementHandle::find_by_accessible_label(&self.window, "Cancel").count();
        slint::platform::update_timers_and_animations();
    }

    fn key(&self, key: Key) {
        let text: SharedString = key.into();
        let window = self.window.window();
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
    }

    fn find(&self, role: AccessibleRole, label: &str) -> Option<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .find(|element| element.accessible_role() == Some(role))
    }
}

fn row_labels(row: &TrackRow) -> Vec<String> {
    row.labels
        .iter()
        .map(|label| label.name.to_string())
        .collect()
}

#[test]
fn the_dialog_makes_and_edits_labels_and_says_what_is_wrong() {
    let app = start();
    let labels = app.labels();
    let chill = app.make("chill", 6);
    let entry = labels.get_all().row_data(0).unwrap();
    assert_eq!((entry.name.as_str(), entry.count), ("chill", 0));
    assert_eq!(
        app.library.label(chill).unwrap().unwrap().colour,
        "palette:teal"
    );

    // A new label starts on a palette colour no label has yet.
    labels.invoke_edit(-1);
    assert_ne!(labels.get_editor_colour(), 6);
    labels.invoke_save("  ".into(), 0, Default::default());
    assert_eq!(labels.get_editor_problem(), LabelProblem::Empty);
    labels.invoke_save("x".repeat(41).into(), 0, Default::default());
    assert_eq!(labels.get_editor_problem(), LabelProblem::TooLong);
    labels.invoke_save("CHILL".into(), 0, Default::default());
    assert_eq!(labels.get_editor_problem(), LabelProblem::Taken);
    // Past the palette, the colour is the typed hex code.
    labels.invoke_save("rainy".into(), 10, "#12345".into());
    assert_eq!(labels.get_editor_problem(), LabelProblem::Colour);
    labels.invoke_save("rainy".into(), 10, "A1B2C3".into());
    assert!(!labels.get_editor_open());
    let rainy = app.library.label_named("rainy").unwrap().unwrap();
    assert_eq!(rainy.colour, "#a1b2c3");

    // Editing shows the label as it is, and keeps its id.
    labels.invoke_edit(chill.0 as i32);
    assert_eq!(labels.get_editor_name(), "chill");
    assert_eq!(labels.get_editor_colour(), 6);
    labels.invoke_save("Chill".into(), 10, "#102030".into());
    let edited = app.library.label(chill).unwrap().unwrap();
    assert_eq!(
        (edited.name.as_str(), edited.colour.as_str()),
        ("Chill", "#102030")
    );
    labels.invoke_edit(rainy.id.0 as i32);
    assert_eq!(labels.get_editor_colour(), 10, "a custom colour");
    assert_eq!(labels.get_editor_hex(), "#a1b2c3");
    labels.invoke_close_editor();
    assert!(!labels.get_editor_open());

    labels.invoke_delete(chill.0 as i32);
    let names: Vec<String> = labels
        .get_all()
        .iter()
        .map(|l| l.name.to_string())
        .collect();
    assert_eq!(names, ["rainy"]);
}

#[test]
fn a_track_menu_puts_a_label_on_every_chosen_track_or_takes_it_off() {
    let app = start();
    let chill = app.make("chill", 6);
    let rows = app.open_album("Album");
    let selection = app.window.global::<TrackSelection>();

    // Both tracks selected: the menu acts on them.
    selection.invoke_change(rows.clone(), 1, 0, vcmp_app::SelectionChange::Range);
    selection.invoke_prepare_labels(rows.clone(), 0);
    assert_eq!(app.choices(), [("chill".to_string(), false, false)]);
    selection.invoke_toggle_label(rows.clone(), 0, chill.0 as i32);
    assert_eq!(app.names_on("One"), ["chill"]);
    assert_eq!(app.names_on("Two"), ["chill"]);
    // The rows show it at once.
    let rows = app.window.get_album_tracks();
    assert_eq!(row_labels(&rows.row_data(0).unwrap()), ["chill"]);

    // One of them alone loses it; then the two disagree and the label is on some.
    selection.invoke_toggle_label(rows.clone(), 1, chill.0 as i32);
    assert!(app.names_on("Two").is_empty());
    let rows = app.window.get_album_tracks();
    selection.invoke_change(rows.clone(), 1, 0, vcmp_app::SelectionChange::Range);
    selection.invoke_prepare_labels(rows.clone(), 1);
    assert_eq!(app.choices(), [("chill".to_string(), false, true)]);
    // Choosing it puts it on all of them; again takes it off all.
    selection.invoke_toggle_label(rows.clone(), 1, chill.0 as i32);
    assert_eq!(app.names_on("Two"), ["chill"]);
    selection.invoke_prepare_labels(rows.clone(), 1);
    assert_eq!(app.choices(), [("chill".to_string(), true, false)]);
    selection.invoke_toggle_label(rows, 1, chill.0 as i32);
    assert!(app.names_on("One").is_empty());
    assert!(app.names_on("Two").is_empty());
}

#[test]
fn a_new_label_from_a_menu_goes_on_the_menus_tracks() {
    let app = start();
    let rows = app.open_album("Album");
    app.window
        .global::<TrackSelection>()
        .invoke_new_label(rows, 1);
    let labels = app.labels();
    assert!(labels.get_editor_open());
    assert_eq!(labels.get_editor_tracks(), 1);
    labels.invoke_save("rainy".into(), 0, Default::default());
    assert_eq!(app.names_on("Two"), ["rainy"]);
    assert!(app.names_on("One").is_empty());

    // From the songs table, the queue or now playing (by file), and an album.
    app.window.invoke_songs_needed();
    let songs = app.window.get_songs();
    let single = songs.iter().position(|row| row.title == "Single").unwrap();
    app.window
        .global::<SongSelection>()
        .invoke_new_label(songs.clone(), single as i32);
    labels.invoke_save("lone".into(), 1, Default::default());
    assert_eq!(app.names_on("Single"), ["lone"]);
    let path = songs.row_data(single).unwrap().path;
    labels.invoke_prepare_path(path.clone());
    assert_eq!(
        app.choices(),
        [
            ("lone".to_string(), true, false),
            ("rainy".to_string(), false, false)
        ]
    );
    let rainy = app.library.label_named("rainy").unwrap().unwrap().id;
    labels.invoke_toggle_path(path, rainy.0 as i32);
    assert_eq!(app.names_on("Single"), ["lone", "rainy"]);
}

#[test]
fn an_albums_menu_labels_the_tracks_it_has() {
    let app = start();
    let chill = app.make("chill", 6);
    let album = app.album("Album").0 as i32;
    let labels = app.labels();
    labels.invoke_prepare_album(album);
    assert_eq!(app.choices(), [("chill".to_string(), false, false)]);
    labels.invoke_toggle_album(album, chill.0 as i32);
    assert_eq!(app.names_on("One"), ["chill"]);
    assert_eq!(app.names_on("Two"), ["chill"]);
    assert!(app.names_on("Single").is_empty());
    labels.invoke_toggle_album(album, chill.0 as i32);
    assert!(app.names_on("One").is_empty());

    labels.invoke_new_for_album(album);
    assert_eq!(labels.get_editor_tracks(), 2);
    labels.invoke_save("whole".into(), 2, Default::default());
    assert_eq!(app.names_on("Two"), ["whole"]);
}

#[test]
fn filters_keep_songs_with_every_label_and_albums_with_one_such_track() {
    let app = start();
    let chill = app.make("chill", 6);
    let rainy = app.make("rainy", 8);
    app.library
        .add_label(chill, &[app.id("One"), app.id("Single")])
        .unwrap();
    app.library.add_label(rainy, &[app.id("One")]).unwrap();
    app.view.labels().changed();
    app.window.invoke_songs_needed();
    let labels = app.labels();
    let titles = || -> Vec<String> {
        let mut titles: Vec<String> = app
            .window
            .get_songs()
            .iter()
            .map(|row| row.title.to_string())
            .collect();
        titles.sort();
        titles
    };
    let albums = || -> Vec<String> {
        app.window
            .get_albums()
            .iter()
            .map(|tile| tile.title.to_string())
            .collect()
    };
    assert_eq!(titles().len(), 3);

    labels.invoke_toggle_filter(LabelFilterPage::Songs, chill.0 as i32);
    assert_eq!(titles(), ["One", "Single"]);
    assert_eq!(labels.get_songs_filter().row_count(), 1);
    labels.invoke_toggle_filter(LabelFilterPage::Songs, rainy.0 as i32);
    assert_eq!(titles(), ["One"], "every label picked");
    // The typed filter narrows as well.
    app.window.set_songs_filter("single".into());
    app.window.invoke_songs_filter_edited("single".into());
    mock_elapsed_time(Duration::from_millis(500));
    assert!(titles().is_empty());
    app.window.set_songs_filter("".into());
    app.window.invoke_songs_filter_edited("".into());
    mock_elapsed_time(Duration::from_millis(500));
    labels.invoke_clear_filter(LabelFilterPage::Songs);
    assert_eq!(titles().len(), 3);
    assert_eq!(
        albums().len(),
        2,
        "the Songs filter leaves the albums alone"
    );

    labels.invoke_toggle_filter(LabelFilterPage::Albums, rainy.0 as i32);
    assert_eq!(albums(), ["Album"], "one track of it is enough");
    labels.invoke_toggle_filter(LabelFilterPage::Albums, chill.0 as i32);
    assert_eq!(albums(), ["Album"]);
    // A label deleted leaves the filter.
    labels.invoke_delete(rainy.0 as i32);
    assert_eq!(labels.get_albums_filter().row_count(), 1);
    let mut both = albums();
    both.sort();
    assert_eq!(both, ["Album", "Lone"]);
}

#[test]
fn a_labels_page_plays_its_tracks_as_a_context_of_its_own() {
    let app = start();
    let chill = app.make("chill", 6);
    app.library
        .add_label(chill, &[app.id("Two"), app.id("Single")])
        .unwrap();
    app.view.labels().changed();

    app.window.invoke_label_activated(0);
    assert!(app.window.get_label_open());
    assert_eq!(app.window.get_open_label_index(), 0);
    let detail = app.window.get_label_detail();
    assert_eq!((detail.name.as_str(), detail.track_count), ("chill", 2));
    let rows = app.window.get_label_tracks();
    assert_eq!(rows.row_count(), 2);
    assert_eq!(row_labels(&rows.row_data(0).unwrap()), ["chill"]);

    app.window.invoke_play_label(1, false);
    let sent = app.sent.0.borrow();
    let Some(Command::PlayContext {
        entries,
        origin,
        start,
    }) = sent.last()
    else {
        panic!("nothing played: {sent:?}");
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(
        origin.as_deref(),
        Some(format!("label:{}", chill.0).as_str())
    );
    assert_eq!(*start, Some(1));
    drop(sent);

    // A label deleted while its page is open says so.
    app.labels().invoke_delete(chill.0 as i32);
    assert!(app.window.get_label_detail().missing);
    assert_eq!(app.window.get_label_tracks().row_count(), 0);
}

#[test]
fn the_sidebar_and_settings_list_the_labels() {
    let app = start();
    app.make("chill", 6);
    app.make("rainy", 8);
    app.window.set_selected_index(4);
    assert_eq!(app.window.get_current_page(), Page::Labels);
    let entry = app
        .find(AccessibleRole::Button, "chill")
        .expect("in the sidebar");
    entry.invoke_accessible_default_action();
    assert!(app.window.get_label_open());

    app.window.set_selected_index(7);
    assert_eq!(app.window.get_current_page(), Page::Settings);
    assert!(app.find(AccessibleRole::Button, "Edit chill").is_some());
    app.find(AccessibleRole::Button, "Delete rainy")
        .unwrap()
        .invoke_accessible_default_action();
    // Deleting asks first.
    assert!(app.library.label_named("rainy").unwrap().is_some());
    app.frame();
    app.find(AccessibleRole::Button, "Delete label")
        .expect("the question")
        .invoke_accessible_default_action();
    app.frame();
    assert!(app.library.label_named("rainy").unwrap().is_none());
}

#[test]
fn the_delete_question_is_a_dialog_that_keeps_the_keyboard() {
    let app = start();
    app.make("rainy", 8);
    app.window.set_selected_index(7);
    let question = app.window.global::<Confirmation>();
    let ask = || {
        app.find(AccessibleRole::Button, "Delete rainy")
            .unwrap()
            .invoke_accessible_default_action();
        app.frame();
        assert!(question.get_open());
    };

    // Enter presses the button with the focus. It starts on Cancel, so Enter keeps
    // the label, and goes back to the Delete… button that asked: Enter there asks
    // again.
    ask();
    assert_eq!(question.get_question(), "Delete the label “rainy”?");
    let enter = || {
        app.key(Key::Return);
        app.frame();
    };
    enter();
    assert!(!question.get_open());
    assert!(app.library.label_named("rainy").unwrap().is_some());
    enter();
    assert!(question.get_open(), "Enter on Delete… asks again");

    // Tab goes round the dialog's two buttons, never to the page behind: twice
    // comes back to Cancel.
    app.key(Key::Tab);
    app.key(Key::Tab);
    enter();
    assert!(!question.get_open());
    assert!(app.library.label_named("rainy").unwrap().is_some());

    // Escape cancels, and the focus goes back to Delete… too.
    enter();
    assert!(question.get_open());
    app.key(Key::Escape);
    app.frame();
    assert!(!question.get_open());
    assert!(app.library.label_named("rainy").unwrap().is_some());
    enter();
    assert!(question.get_open(), "Enter on Delete… asks again");

    // A click on the scrim beside the card cancels too.
    let window = app.window.window();
    let corner = LogicalPosition::new(4.0, 4.0);
    window.dispatch_event(WindowEvent::PointerPressed {
        position: corner,
        button: PointerEventButton::Left,
    });
    window.dispatch_event(WindowEvent::PointerReleased {
        position: corner,
        button: PointerEventButton::Left,
    });
    app.frame();
    assert!(!question.get_open());
    assert!(app.library.label_named("rainy").unwrap().is_some());

    // Tab from Cancel goes to Delete label, and Enter there deletes.
    ask();
    app.key(Key::Tab);
    enter();
    assert!(!question.get_open());
    assert!(app.library.label_named("rainy").unwrap().is_none());
}

#[test]
fn the_labels_page_without_labels_keeps_its_title_at_the_top() {
    let app = start();
    app.window.set_selected_index(4);
    let title = ElementHandle::find_by_accessible_label(&app.window, "Labels")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Text))
        .expect("the page's title");
    assert!(
        title.absolute_position().y < 120.0,
        "the title is at {}",
        title.absolute_position().y
    );
    let button = app
        .find(AccessibleRole::Button, "New label…")
        .expect("New label");
    assert!(
        button.size().height <= 48.0,
        "the button is {} tall",
        button.size().height
    );
}

#[test]
fn the_album_pages_more_menu_labels_the_album() {
    let app = start();
    let chill = app.make("chill", 6);
    app.library.add_label(chill, &[app.id("One")]).unwrap();
    app.view.labels().changed();
    app.open_album("Album");
    let detail = app.window.get_album_detail();
    assert_eq!(i64::from(detail.id), app.album("Album").0);

    // The button fills the menu's labels for the album before it opens.
    app.labels().set_choices(ModelRc::default());
    app.find(AccessibleRole::Button, "More for Album")
        .expect("the More button")
        .invoke_accessible_default_action();
    assert_eq!(app.choices(), [("chill".to_string(), false, true)]);
    app.labels().invoke_toggle_album(detail.id, chill.0 as i32);
    assert_eq!(app.names_on("Two"), ["chill"]);
    let rows = app.window.get_album_tracks();
    assert_eq!(row_labels(&rows.row_data(1).unwrap()), ["chill"]);
}
