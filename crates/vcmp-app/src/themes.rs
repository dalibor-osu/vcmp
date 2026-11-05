//! The chosen theme: saved in the library database's settings as `ui.theme`, shown
//! through `theme::apply`, and offered in Settings and the setup wizard through the
//! `Themes` global with the theme files of the themes folder.
//!
//! `ui.theme` holds JSON: `{"theme":"dark","accent":"pink"}`, or for a theme file
//! `{"theme":"file:dusk","kind":"dark","accent":"pink"}`. The kind of a file theme is
//! kept so the built-in theme of its kind can stand in once the file is gone; the
//! accent is kept while a file theme is chosen, for going back to a built-in one.
//! Before themes, `ui.theme` was `light`, `dark` or `system`: `light` and `dark` are
//! the built-in themes of the same palettes, and `system`, like a first start, picks
//! Light or Dark from the system's scheme once it is known, then stays.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};
use vcmp_library::Library;

use crate::theme::file::{self, Entry, Problem};
use crate::theme::{self, ACCENTS, BuiltIn, DEFAULT_ACCENT, contrast};
use crate::{
    AccentTile, AppWindow, ContrastFailure, InvalidThemeFile, Theme, ThemeFileProblem, ThemeKind,
    ThemePalette, ThemeTile, Themes as ThemesState,
};

pub const THEME: &str = "ui.theme";

const FILE_PREFIX: &str = "file:";

/// Which theme is chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum Chosen {
    BuiltIn(BuiltIn),
    /// A theme file by its name without `.toml`, and its kind when it was chosen.
    File {
        stem: String,
        kind: ThemeKind,
    },
}

/// The chosen theme and accent, as saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub theme: Chosen,
    /// The accent's place in [`ACCENTS`].
    pub accent: usize,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    theme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    accent: String,
}

/// The saved choice; `None` when the first start's pick is still to come: nothing is
/// saved, the old `system`, or a value VCMP can't read.
pub fn read_saved(saved: Option<&str>) -> Option<Choice> {
    match saved? {
        "light" => return Some(Choice::built_in(BuiltIn::Light)),
        "dark" => return Some(Choice::built_in(BuiltIn::Dark)),
        _ => {}
    }
    let saved: Saved = serde_json::from_str(saved?).ok()?;
    let accent = theme::accent_by_name(&saved.accent).unwrap_or(DEFAULT_ACCENT);
    let theme = match saved.theme.strip_prefix(FILE_PREFIX) {
        Some(stem) if !stem.is_empty() => Chosen::File {
            stem: stem.to_owned(),
            kind: match saved.kind.as_deref() {
                Some("light") => ThemeKind::Light,
                Some("dark") => ThemeKind::Dark,
                _ => return None,
            },
        },
        Some(_) => return None,
        None => Chosen::BuiltIn(BuiltIn::from_id(&saved.theme)?),
    };
    Some(Choice { theme, accent })
}

impl Choice {
    pub fn built_in(theme: BuiltIn) -> Choice {
        Choice {
            theme: Chosen::BuiltIn(theme),
            accent: DEFAULT_ACCENT,
        }
    }

    /// The first start's pick for a system that is dark or not.
    pub fn for_system(dark: bool) -> Choice {
        Choice::built_in(BuiltIn::of_kind(if dark {
            ThemeKind::Dark
        } else {
            ThemeKind::Light
        }))
    }

    pub fn to_saved(&self) -> String {
        let accent = ACCENTS[self.accent].name.to_owned();
        let saved = match &self.theme {
            Chosen::BuiltIn(theme) => Saved {
                theme: theme.id().to_owned(),
                kind: None,
                accent,
            },
            Chosen::File { stem, kind } => Saved {
                theme: format!("{FILE_PREFIX}{stem}"),
                kind: Some(kind_name(*kind).to_owned()),
                accent,
            },
        };
        serde_json::to_string(&saved).expect("a theme choice is always JSON")
    }
}

fn kind_name(kind: ThemeKind) -> &'static str {
    match kind {
        ThemeKind::Light => "light",
        _ => "dark",
    }
}

/// What the window shows for a choice.
#[derive(Debug, PartialEq)]
pub struct Shown {
    pub id: String,
    pub palette: ThemePalette,
    pub kind: ThemeKind,
    /// The built-in theme the accent picker belongs to; `None` on a file theme.
    pub built_in: Option<BuiltIn>,
    /// The file of a chosen file theme that is gone or can't be used.
    pub missing: Option<String>,
    pub failures: Vec<contrast::Failure>,
}

/// The theme to show for a choice among these theme files.
pub fn resolve(choice: &Choice, files: &[Entry]) -> Shown {
    let built_in = |theme: BuiltIn, missing: Option<String>| Shown {
        id: theme.id().to_owned(),
        palette: theme.palette(choice.accent),
        kind: theme.kind(),
        built_in: Some(theme),
        missing,
        failures: Vec::new(),
    };
    match &choice.theme {
        Chosen::BuiltIn(theme) => built_in(*theme, None),
        Chosen::File { stem, kind } => {
            let entry = files.iter().find(|entry| &entry.stem == stem);
            match entry.map(|entry| &entry.theme) {
                Some(Ok(file)) => Shown {
                    id: format!("{FILE_PREFIX}{stem}"),
                    palette: file.palette.clone(),
                    kind: file.kind,
                    built_in: None,
                    missing: None,
                    failures: contrast::failures(&file.palette),
                },
                _ => built_in(
                    BuiltIn::of_kind(*kind),
                    Some(entry.map_or_else(|| format!("{stem}.toml"), |entry| entry.file.clone())),
                ),
            }
        }
    }
}

struct State {
    choice: Choice,
    /// The first start's pick waits for the system's scheme.
    picking: bool,
    files: Vec<Entry>,
}

/// The theme choice, its saving and the themes folder, behind the `Themes` global.
pub struct Themes {
    window: Weak<AppWindow>,
    library: Option<Rc<Library>>,
    folder: Option<PathBuf>,
    state: RefCell<State>,
}

impl Themes {
    /// Shows the saved theme, or starts the first start's pick, reads the theme files
    /// in `folder`, and connects the picker. Without a library the choice lasts for
    /// this session; without a folder there are no theme files.
    pub fn connect(
        window: &AppWindow,
        library: Option<Rc<Library>>,
        folder: Option<PathBuf>,
    ) -> Rc<Themes> {
        let saved = library.as_ref().and_then(|library| {
            library.setting(THEME).unwrap_or_else(|error| {
                log::warn!("could not read the theme: {error}");
                None
            })
        });
        let choice = read_saved(saved.as_deref());
        let files = folder.as_deref().map(file::read_folder).unwrap_or_default();
        window
            .global::<ThemesState>()
            .set_folder_available(folder.is_some());
        let themes = Rc::new(Themes {
            window: window.as_weak(),
            library,
            folder,
            state: RefCell::new(State {
                picking: choice.is_none(),
                choice: choice.unwrap_or_else(|| Choice::built_in(BuiltIn::Light)),
                files,
            }),
        });
        themes.show(window);

        let state = window.global::<ThemesState>();
        let this = themes.clone();
        state.on_choose(move |id| {
            this.choose(&id);
        });
        let this = themes.clone();
        state.on_choose_accent(move |hue| {
            this.choose_accent(hue as usize);
        });
        let this = themes.clone();
        state.on_reload(move || {
            this.reload();
        });
        let this = themes.clone();
        state.on_open_folder(move || {
            this.open_folder();
        });
        let this = themes.clone();
        state.on_save_copy(move || {
            this.save_copy();
        });
        let this = themes.clone();
        window.global::<Theme>().on_system_scheme_changed(move || {
            this.system_scheme_changed();
        });
        themes.system_scheme_changed();
        themes
    }

    pub fn choice(&self) -> Choice {
        self.state.borrow().choice.clone()
    }

    /// Picks Light or Dark once the system says which it is, if nothing was chosen.
    pub fn system_scheme_changed(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let theme = window.global::<Theme>();
        if !self.state.borrow().picking || !theme.get_system_scheme_known() {
            return;
        }
        self.set_choice(Choice::for_system(theme.get_system_dark()));
    }

    pub fn choose(&self, id: &str) {
        let accent = self.state.borrow().choice.accent;
        let theme = if let Some(stem) = id.strip_prefix(FILE_PREFIX) {
            let state = self.state.borrow();
            let Some(Ok(file)) = state
                .files
                .iter()
                .find(|entry| entry.stem == stem)
                .map(|entry| &entry.theme)
            else {
                return;
            };
            Chosen::File {
                stem: stem.to_owned(),
                kind: file.kind,
            }
        } else {
            match BuiltIn::from_id(id) {
                Some(theme) => Chosen::BuiltIn(theme),
                None => return,
            }
        };
        self.set_choice(Choice { theme, accent });
    }

    pub fn choose_accent(&self, hue: usize) {
        if hue >= ACCENTS.len() {
            return;
        }
        let theme = self.state.borrow().choice.theme.clone();
        self.set_choice(Choice { theme, accent: hue });
    }

    /// Reads the theme files again, and shows the chosen one as it is now.
    pub fn reload(&self) {
        let Some(folder) = &self.folder else { return };
        self.state.borrow_mut().files = file::read_folder(folder);
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<ThemesState>();
            state.set_saved_as(SharedString::new());
            state.set_folder_error(SharedString::new());
            self.show(&window);
        }
    }

    fn open_folder(&self) {
        let Some(folder) = &self.folder else { return };
        let result = std::fs::create_dir_all(folder).and_then(|()| open_in_file_manager(folder));
        self.report(result.err().map(|error| error.to_string()));
    }

    /// Writes the built-in theme shown now, with its accent, as a new theme file.
    fn save_copy(&self) {
        let choice = self.choice();
        let (Chosen::BuiltIn(theme), Some(folder)) = (choice.theme, &self.folder) else {
            return;
        };
        let palette = theme.palette(choice.accent);
        let text = file::write(
            &format!("{} copy", theme.file_name()),
            theme.kind(),
            &palette,
        );
        match save_new(folder, &format!("{}-copy", theme.id()), &text) {
            Ok(saved) => {
                self.state.borrow_mut().files = file::read_folder(folder);
                if let Some(window) = self.window.upgrade() {
                    self.show(&window);
                    window
                        .global::<ThemesState>()
                        .set_saved_as(saved.as_str().into());
                }
                self.report(None);
            }
            Err(error) => self.report(Some(error.to_string())),
        }
    }

    fn report(&self, error: Option<String>) {
        if let Some(window) = self.window.upgrade() {
            window
                .global::<ThemesState>()
                .set_folder_error(error.unwrap_or_default().into());
        }
    }

    fn set_choice(&self, choice: Choice) {
        {
            let mut state = self.state.borrow_mut();
            state.picking = false;
            state.choice = choice.clone();
        }
        if let Some(library) = &self.library
            && let Err(error) = library.set_setting(THEME, &choice.to_saved())
        {
            log::warn!("could not save the theme: {error}");
        }
        if let Some(window) = self.window.upgrade() {
            self.show(&window);
        }
    }

    fn show(&self, window: &AppWindow) {
        let state = self.state.borrow();
        let shown = resolve(&state.choice, &state.files);
        // Until the first start's pick, the system's scheme stays readable.
        if state.picking {
            let theme = window.global::<Theme>();
            theme.set_colors(shown.palette.clone());
            theme.set_kind(shown.kind);
        } else {
            theme::apply(window, shown.palette.clone(), shown.kind);
        }

        let picker = window.global::<ThemesState>();
        let built_in: Vec<ThemeTile> = BuiltIn::ALL
            .into_iter()
            .enumerate()
            .map(|(place, theme)| {
                tile(
                    theme.id(),
                    place as i32,
                    "",
                    &theme.palette(state.choice.accent),
                )
            })
            .collect();
        let mut tiles = built_in.clone();
        let mut invalid = Vec::new();
        for entry in &state.files {
            match &entry.theme {
                Ok(file) => tiles.push(tile(
                    &format!("{FILE_PREFIX}{}", entry.stem),
                    -1,
                    &file.name,
                    &file.palette,
                )),
                Err(problem) => invalid.push(invalid_file(entry, problem)),
            }
        }
        picker.set_built_in_tiles(ModelRc::new(VecModel::from(built_in)));
        let place = tiles.iter().position(|tile| tile.id == shown.id.as_str());
        picker.set_current_index(place.map_or(-1, |place| place as i32));
        picker.set_current(shown.id.as_str().into());
        picker.set_tiles(ModelRc::new(VecModel::from(tiles)));
        picker.set_invalid(ModelRc::new(VecModel::from(invalid)));
        picker.set_file_chosen(shown.built_in.is_none());
        let kind = shown.kind;
        let accents: Vec<AccentTile> = (0..ACCENTS.len())
            .filter_map(|hue| {
                theme::accent_colour(hue, kind).map(|colour| AccentTile {
                    hue: hue as i32,
                    colour,
                })
            })
            .collect();
        let effective = if theme::accent_colour(state.choice.accent, kind).is_some() {
            state.choice.accent
        } else {
            DEFAULT_ACCENT
        };
        let place = accents
            .iter()
            .position(|accent| accent.hue as usize == effective)
            .map_or(-1, |place| place as i32);
        picker.set_accents(ModelRc::new(VecModel::from(accents)));
        picker.set_accent(place);
        picker.set_missing(shown.missing.unwrap_or_default().into());
        let failures: Vec<ContrastFailure> = shown
            .failures
            .iter()
            .map(|failure| ContrastFailure {
                pair: failure.pair as i32,
                ratio: failure.ratio as f32,
                needed: failure.needed as f32,
            })
            .collect();
        picker.set_failures(ModelRc::new(VecModel::from(failures)));
    }
}

fn tile(id: &str, built_in: i32, name: &str, palette: &ThemePalette) -> ThemeTile {
    ThemeTile {
        id: id.into(),
        built_in,
        name: name.into(),
        base: palette.base,
        mantle: palette.mantle,
        surface: palette.surface,
        text: palette.text,
        accent: palette.accent,
    }
}

fn invalid_file(entry: &Entry, problem: &Problem) -> InvalidThemeFile {
    let (problem, detail) = match problem {
        Problem::Unreadable => (ThemeFileProblem::Unreadable, String::new()),
        Problem::NotToml(message) => (ThemeFileProblem::NotToml, message.trim().to_owned()),
        Problem::NoName => (ThemeFileProblem::NoName, String::new()),
        Problem::BadKind => (ThemeFileProblem::BadKind, String::new()),
        Problem::MissingColour(name) => (ThemeFileProblem::MissingColour, (*name).to_owned()),
        Problem::BadColour(name) => (ThemeFileProblem::BadColour, (*name).to_owned()),
    };
    InvalidThemeFile {
        file: entry.file.as_str().into(),
        problem,
        detail: detail.into(),
    }
}

/// Writes `text` to `<stem>.toml` in the folder, or `<stem>-2.toml` and on where that
/// exists; returns the file name.
fn save_new(folder: &Path, stem: &str, text: &str) -> std::io::Result<String> {
    std::fs::create_dir_all(folder)?;
    for number in 1.. {
        let name = match number {
            1 => format!("{stem}.toml"),
            n => format!("{stem}-{n}.toml"),
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.join(&name))
        {
            Ok(mut file) => {
                std::io::Write::write_all(&mut file, text.as_bytes())?;
                return Ok(name);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!("a free name is found before the numbers run out")
}

/// Opens the folder in the system's file manager.
fn open_in_file_manager(folder: &Path) -> std::io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(program).arg(folder).spawn()?;
    // Explorer reports failure even when it opened the folder, so the status says
    // nothing; waiting only reaps the process.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::file::write;

    #[test]
    fn older_values_and_the_first_start() {
        assert_eq!(read_saved(None), None, "the first start picks");
        assert_eq!(read_saved(Some("system")), None, "picks once");
        assert_eq!(read_saved(Some("sepia")), None);
        assert_eq!(
            read_saved(Some("light")),
            Some(Choice::built_in(BuiltIn::Light))
        );
        assert_eq!(
            read_saved(Some("dark")),
            Some(Choice::built_in(BuiltIn::Dark))
        );
        assert_eq!(Choice::for_system(true), Choice::built_in(BuiltIn::Dark));
        assert_eq!(Choice::for_system(false), Choice::built_in(BuiltIn::Light));
    }

    #[test]
    fn choices_survive_saving() {
        for choice in [
            Choice {
                theme: Chosen::BuiltIn(BuiltIn::Midnight),
                accent: 6,
            },
            Choice {
                theme: Chosen::File {
                    stem: "my dusk".into(),
                    kind: ThemeKind::Dark,
                },
                accent: 2,
            },
        ] {
            assert_eq!(read_saved(Some(&choice.to_saved())), Some(choice));
        }
        assert_eq!(
            Choice::built_in(BuiltIn::Darker).to_saved(),
            r#"{"theme":"darker","accent":"pink"}"#
        );
        assert_eq!(
            read_saved(Some(r#"{"theme":"file:dusk","accent":"pink"}"#)),
            None,
            "a file theme without its kind"
        );
        assert_eq!(
            read_saved(Some(r#"{"theme":"dark","accent":"beige"}"#)),
            Some(Choice::built_in(BuiltIn::Dark)),
            "an unknown accent is pink"
        );
    }

    fn entry(stem: &str, text: &str) -> Entry {
        Entry {
            stem: stem.into(),
            file: format!("{stem}.toml"),
            theme: file::parse(text),
        }
    }

    #[test]
    fn a_gone_theme_file_falls_back_to_the_built_in_of_its_kind() {
        let choice = Choice {
            theme: Chosen::File {
                stem: "dusk".into(),
                kind: ThemeKind::Dark,
            },
            accent: 6,
        };
        let shown = resolve(&choice, &[]);
        assert_eq!(shown.id, "dark");
        assert_eq!(shown.palette, BuiltIn::Dark.palette(6));
        assert_eq!(shown.missing.as_deref(), Some("dusk.toml"));

        let broken = [entry("dusk", "kind = \"dark\"")];
        assert_eq!(
            resolve(&choice, &broken).missing.as_deref(),
            Some("dusk.toml")
        );

        let mut palette = theme::light();
        palette.text = palette.base;
        let files = [entry("dusk", &write("Dusk", ThemeKind::Light, &palette))];
        let shown = resolve(&choice, &files);
        assert_eq!(shown.id, "file:dusk");
        assert_eq!(shown.kind, ThemeKind::Light, "the file says light now");
        assert_eq!(shown.missing, None);
        assert_eq!(shown.built_in, None);
        assert_eq!(shown.failures.len(), 3, "text on base, mantle and surface");
    }

    #[test]
    fn copies_get_free_names() {
        let folder = tempfile::tempdir().unwrap();
        let themes = folder.path().join("themes");
        assert_eq!(
            save_new(&themes, "dark-copy", "a").unwrap(),
            "dark-copy.toml"
        );
        assert_eq!(
            save_new(&themes, "dark-copy", "b").unwrap(),
            "dark-copy-2.toml"
        );
        assert_eq!(
            std::fs::read_to_string(themes.join("dark-copy.toml")).unwrap(),
            "a"
        );
    }
}
