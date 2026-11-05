//! The theme picker in Settings: the choice survives a restart, older saved values
//! keep their look, theme files load from the themes folder (or say what is wrong
//! with them), a chosen file that is gone falls back, and a built-in theme can be
//! saved as a file to start from.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, Model};
use vcmp_app::theme::{self, BuiltIn, file};
use vcmp_app::themes::{self, Themes};
use vcmp_app::{AppWindow, Theme, ThemeKind, Themes as ThemesState};
use vcmp_library::Library;

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        i_slint_backend_testing::init_no_event_loop();
        Fixture::another()
    }

    /// A second library in a test that already has the testing backend.
    fn another() -> Fixture {
        Fixture {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn library(&self) -> Library {
        Library::open(
            &self.dir.path().join("library.sqlite3"),
            &self.dir.path().join("cache"),
        )
        .unwrap()
    }

    fn folder(&self) -> PathBuf {
        self.dir.path().join("themes")
    }

    fn write_theme(&self, file_name: &str, text: &str) {
        std::fs::create_dir_all(self.folder()).unwrap();
        std::fs::write(self.folder().join(file_name), text).unwrap();
    }

    /// The window on its Settings page, tall enough for the Appearance section.
    fn open(&self) -> AppWindow {
        let window = vcmp_app::create_window().unwrap();
        window
            .window()
            .set_size(slint::LogicalSize::new(1100.0, 2400.0));
        window.show().unwrap();
        Themes::connect(&window, Some(Rc::new(self.library())), Some(self.folder()));
        click(&window, AccessibleRole::Tab, "Settings");
        window
    }

    fn saved(&self) -> Option<String> {
        self.library().setting(themes::THEME).unwrap()
    }
}

fn find(window: &AppWindow, role: AccessibleRole, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
}

fn click(window: &AppWindow, role: AccessibleRole, label: &str) {
    find(window, role, label)
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
        .invoke_accessible_default_action();
}

fn shows_text(window: &AppWindow, text: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, text)
        .next()
        .is_some()
}

fn dusk(kind: ThemeKind) -> String {
    let mut palette = theme::dark();
    palette.base = slint::Color::from_rgb_u8(0x10, 0x20, 0x30);
    file::write("Dusk", kind, &palette)
}

#[test]
fn the_chosen_theme_and_accent_are_restored() {
    let fixture = Fixture::new();
    let window = fixture.open();
    click(&window, AccessibleRole::RadioButton, "Midnight");
    click(&window, AccessibleRole::RadioButton, "Teal");
    assert_eq!(
        window.global::<Theme>().get_colors(),
        BuiltIn::Midnight.palette(6)
    );
    drop(window);

    assert_eq!(
        fixture.saved().as_deref(),
        Some(r#"{"theme":"midnight","accent":"teal"}"#)
    );
    let restarted = fixture.open();
    let theme = restarted.global::<Theme>();
    assert_eq!(theme.get_colors(), BuiltIn::Midnight.palette(6));
    assert!(theme.get_dark());
    let teal = find(&restarted, AccessibleRole::RadioButton, "Teal").unwrap();
    assert_eq!(teal.accessible_checked(), Some(true));
}

#[test]
fn the_older_light_and_dark_keep_their_look() {
    i_slint_backend_testing::init_no_event_loop();
    for (saved, palette, dark) in [
        ("light", theme::light(), false),
        ("dark", theme::dark(), true),
    ] {
        let fixture = Fixture::another();
        fixture.library().set_setting(themes::THEME, saved).unwrap();
        let window = fixture.open();
        let theme = window.global::<Theme>();
        assert_eq!(theme.get_colors(), palette, "saved {saved:?}");
        assert_eq!(theme.get_dark(), dark);
    }
}

#[test]
fn system_waits_for_the_system_to_say_and_is_not_overwritten_meanwhile() {
    // The headless backend never reports a scheme: the light theme shows, and the
    // pick stays to be made.
    let fixture = Fixture::new();
    fixture
        .library()
        .set_setting(themes::THEME, "system")
        .unwrap();
    let window = fixture.open();
    assert_eq!(window.global::<Theme>().get_colors(), theme::light());
    drop(window);
    assert_eq!(fixture.saved().as_deref(), Some("system"));
}

#[test]
fn a_theme_file_is_offered_and_applies() {
    let fixture = Fixture::new();
    fixture.write_theme("dusk.toml", &dusk(ThemeKind::Dark));
    let window = fixture.open();
    click(&window, AccessibleRole::RadioButton, "Dusk");
    let theme = window.global::<Theme>();
    assert_eq!(
        theme.get_colors().base,
        slint::Color::from_rgb_u8(0x10, 0x20, 0x30)
    );
    assert!(theme.get_dark());
    let picker = window.global::<ThemesState>();
    assert!(picker.get_file_chosen());
    assert!(
        find(&window, AccessibleRole::RadioGroup, "Accent").is_none(),
        "a theme file brings its own accent"
    );
    assert!(shows_text(&window, "A theme file brings its own accent."));
    drop(window);
    assert_eq!(
        fixture.saved().as_deref(),
        Some(r#"{"theme":"file:dusk","kind":"dark","accent":"pink"}"#)
    );
}

#[test]
fn a_theme_file_that_fails_contrast_loads_with_a_warning() {
    let fixture = Fixture::new();
    let mut palette = theme::dark();
    palette.text_muted = palette.base;
    fixture.write_theme("flat.toml", &file::write("Flat", ThemeKind::Dark, &palette));
    let window = fixture.open();
    click(&window, AccessibleRole::RadioButton, "Flat");
    assert_eq!(window.global::<Theme>().get_colors(), palette);
    assert!(shows_text(
        &window,
        "Some colours of this theme are hard to read (WCAG AA):"
    ));
    assert!(shows_text(&window, "muted text on base: 1:1, needs 4.5:1"));
}

#[test]
fn broken_theme_files_are_listed_and_not_offered() {
    let fixture = Fixture::new();
    let without_outline = dusk(ThemeKind::Dark).replace("outline = ", "# outline = ");
    fixture.write_theme("dusk.toml", &without_outline);
    fixture.write_theme("blank.toml", "kind = \"dark\"");
    let window = fixture.open();
    assert!(find(&window, AccessibleRole::RadioButton, "Dusk").is_none());
    assert!(shows_text(&window, "blank.toml: has no name."));
    assert!(shows_text(
        &window,
        "dusk.toml: the colour outline is missing."
    ));
    assert_eq!(
        window.global::<ThemesState>().get_tiles().row_count(),
        BuiltIn::ALL.len()
    );
}

#[test]
fn a_gone_theme_file_falls_back_to_the_built_in_of_its_kind() {
    let fixture = Fixture::new();
    fixture.write_theme("dusk.toml", &dusk(ThemeKind::Dark));
    let window = fixture.open();
    click(&window, AccessibleRole::RadioButton, "Dusk");

    // Gone while VCMP runs: Reload notices.
    std::fs::remove_file(fixture.folder().join("dusk.toml")).unwrap();
    click(&window, AccessibleRole::Button, "Reload theme files");
    assert_eq!(window.global::<Theme>().get_colors(), theme::dark());
    let notice = "The theme file dusk.toml is gone or can't be used, so VCMP shows Dark instead.";
    assert!(shows_text(&window, notice));
    drop(window);

    // And at the next start, with the choice still saved for when the file is back.
    let restarted = fixture.open();
    assert_eq!(restarted.global::<Theme>().get_colors(), theme::dark());
    assert!(shows_text(&restarted, notice));
    assert_eq!(
        find(&restarted, AccessibleRole::RadioButton, "Dark")
            .unwrap()
            .accessible_checked(),
        Some(true)
    );
    fixture.write_theme("dusk.toml", &dusk(ThemeKind::Dark));
    click(&restarted, AccessibleRole::Button, "Reload theme files");
    assert!(restarted.global::<ThemesState>().get_file_chosen());
    assert!(!shows_text(&restarted, notice));
}

#[test]
fn a_built_in_theme_saves_as_a_file_to_start_from() {
    let fixture = Fixture::new();
    let window = fixture.open();
    click(&window, AccessibleRole::RadioButton, "Darker");
    click(&window, AccessibleRole::RadioButton, "Green");
    click(
        &window,
        AccessibleRole::Button,
        "Save the current theme as a theme file",
    );
    assert!(shows_text(
        &window,
        "Saved as darker-copy.toml in the themes folder."
    ));
    let saved = file::parse(&read(&fixture.folder().join("darker-copy.toml"))).unwrap();
    assert_eq!(saved.name, "Darker copy");
    assert_eq!(saved.palette, BuiltIn::Darker.palette(5));

    click(&window, AccessibleRole::RadioButton, "Darker copy");
    assert_eq!(
        window.global::<Theme>().get_colors(),
        BuiltIn::Darker.palette(5)
    );
    assert_eq!(
        find(
            &window,
            AccessibleRole::Button,
            "Save the current theme as a theme file"
        )
        .unwrap()
        .accessible_enabled(),
        Some(false),
        "only built-in themes are saved as files"
    );
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}
