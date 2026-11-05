//! Theme files: a TOML file in the themes folder with a name, a kind and every colour
//! of the palette, which VCMP reads at start and on Reload, and writes when a
//! built-in theme is saved as a file.
//!
//! ```toml
//! name = "Dusk"
//! kind = "dark"
//!
//! [colors]
//! base = "#1e1e2e"
//! scrim = "#11111bb3"
//! ```
//!
//! Colours are `#rrggbb`; the scrim may also be `#rrggbbaa`, as it is translucent.
//! Keys VCMP doesn't know are ignored, so a file written for a newer VCMP still loads.
//! A colour added to the palette later gets a default derived from the others when a
//! file lacks it; every colour of [`COLOURS`] is required.

use std::path::Path;

use slint::Color;

use crate::{ThemeKind, ThemePalette};

/// The palette's colours by their names in the UI (`ThemePalette` in
/// `ui/theme.slint`), in the order a saved file lists them.
pub const COLOURS: [&str; 10] = [
    "base",
    "mantle",
    "surface",
    "outline",
    "text",
    "text-muted",
    "accent",
    "on-accent",
    "error",
    "scrim",
];

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeFile {
    pub name: String,
    pub kind: ThemeKind,
    pub palette: ThemePalette,
}

/// Why a theme file can't be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Unreadable,
    NotToml(String),
    NoName,
    BadKind,
    MissingColour(&'static str),
    BadColour(&'static str),
}

pub fn parse(text: &str) -> Result<ThemeFile, Problem> {
    let table: toml::Table =
        toml::from_str(text).map_err(|error| Problem::NotToml(error.message().to_owned()))?;
    let name = table
        .get("name")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or(Problem::NoName)?
        .to_owned();
    let kind = match table.get("kind").and_then(toml::Value::as_str) {
        Some("light") => ThemeKind::Light,
        Some("dark") => ThemeKind::Dark,
        _ => return Err(Problem::BadKind),
    };
    let colours = table.get("colors").and_then(toml::Value::as_table);
    let colour = |name: &'static str| -> Result<Color, Problem> {
        let text = colours
            .and_then(|colours| colours.get(name))
            .ok_or(Problem::MissingColour(name))?
            .as_str()
            .ok_or(Problem::BadColour(name))?;
        parse_colour(text, name == "scrim").ok_or(Problem::BadColour(name))
    };
    let palette = ThemePalette {
        base: colour("base")?,
        mantle: colour("mantle")?,
        surface: colour("surface")?,
        outline: colour("outline")?,
        text: colour("text")?,
        text_muted: colour("text-muted")?,
        accent: colour("accent")?,
        on_accent: colour("on-accent")?,
        error: colour("error")?,
        scrim: colour("scrim")?,
    };
    Ok(ThemeFile {
        name,
        kind,
        palette,
    })
}

/// `#rrggbb`, or `#rrggbbaa` where `alpha` allows it.
fn parse_colour(text: &str, alpha: bool) -> Option<Color> {
    let digits = text.trim().strip_prefix('#')?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(Color::from_argb_u8(
            0xff,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        )),
        8 if alpha => Some(Color::from_argb_u8(
            value as u8,
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
        )),
        _ => None,
    }
}

fn hex(colour: Color) -> String {
    let rgb = format!(
        "#{:02x}{:02x}{:02x}",
        colour.red(),
        colour.green(),
        colour.blue()
    );
    match colour.alpha() {
        0xff => rgb,
        alpha => format!("{rgb}{alpha:02x}"),
    }
}

/// The theme as a file [`parse`] reads back.
pub fn write(name: &str, kind: ThemeKind, palette: &ThemePalette) -> String {
    let kind = match kind {
        ThemeKind::Light => "light",
        _ => "dark",
    };
    let values = [
        palette.base,
        palette.mantle,
        palette.surface,
        palette.outline,
        palette.text,
        palette.text_muted,
        palette.accent,
        palette.on_accent,
        palette.error,
        palette.scrim,
    ];
    let mut text = format!(
        "# A VCMP theme. Edit the colours, then press Reload in Settings.\n\
         # Colours are #rrggbb; the scrim, behind dialogs, is #rrggbbaa.\n\
         name = {}\n\
         kind = \"{kind}\"\n\n[colors]\n",
        toml::Value::String(name.to_owned())
    );
    for (key, colour) in COLOURS.iter().zip(values) {
        text.push_str(&format!("{key} = \"{}\"\n", hex(colour)));
    }
    text
}

/// A file in the themes folder: its name without `.toml`, its whole name, and the
/// theme in it or why it can't be used.
#[derive(Clone, Debug)]
pub struct Entry {
    pub stem: String,
    pub file: String,
    pub theme: Result<ThemeFile, Problem>,
}

/// The `.toml` files in the folder, by file name; none when the folder doesn't exist.
pub fn read_folder(folder: &Path) -> Vec<Entry> {
    let Ok(listing) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = listing
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
        })
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?.to_owned();
            let file = path.file_name()?.to_str()?.to_owned();
            let theme = std::fs::read_to_string(&path)
                .map_err(|_| Problem::Unreadable)
                .and_then(|text| parse(&text));
            Some(Entry { stem, file, theme })
        })
        .collect();
    entries.sort_by_key(|entry| entry.file.to_lowercase());
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{dark, light};

    #[test]
    fn a_written_theme_reads_back() {
        for (kind, palette) in [(ThemeKind::Light, light()), (ThemeKind::Dark, dark())] {
            let text = write("My \"own\" theme", kind, &palette);
            assert_eq!(
                parse(&text),
                Ok(ThemeFile {
                    name: "My \"own\" theme".into(),
                    kind,
                    palette,
                })
            );
        }
        assert!(write("Dusk", ThemeKind::Dark, &dark()).contains("scrim = \"#11111bb3\""));
    }

    #[test]
    fn a_missing_colour_makes_the_file_invalid() {
        let text = write("Dusk", ThemeKind::Dark, &dark()).replace("outline = ", "# outline = ");
        assert_eq!(parse(&text), Err(Problem::MissingColour("outline")));
    }

    #[test]
    fn keys_from_a_newer_vcmp_are_ignored() {
        let text = write("Dusk", ThemeKind::Dark, &dark())
            + "highlight = \"#ffffff\"\n\n[fonts]\nbody = \"Sora\"\n";
        assert_eq!(parse(&text).map(|theme| theme.palette), Ok(dark()));
    }

    #[test]
    fn broken_files_say_what_is_wrong() {
        let good = write("Dusk", ThemeKind::Dark, &dark());
        assert!(matches!(parse("name = "), Err(Problem::NotToml(_))));
        assert_eq!(
            parse(&good.replace("name = \"Dusk\"", "")),
            Err(Problem::NoName)
        );
        assert_eq!(
            parse(&good.replace("name = \"Dusk\"", "name = \"  \"")),
            Err(Problem::NoName)
        );
        assert_eq!(
            parse(&good.replace("kind = \"dark\"", "kind = \"sepia\"")),
            Err(Problem::BadKind)
        );
        assert_eq!(
            parse(&good.replace("base = \"#1e1e2e\"", "base = \"#1e1e2\"")),
            Err(Problem::BadColour("base"))
        );
        assert_eq!(
            parse(&good.replace("base = \"#1e1e2e\"", "base = \"#1e1e2eff\"")),
            Err(Problem::BadColour("base")),
            "only the scrim is translucent"
        );
        assert_eq!(
            parse(&good.replace("base = \"#1e1e2e\"", "base = 30")),
            Err(Problem::BadColour("base"))
        );
    }

    #[test]
    fn the_folder_lists_toml_files_by_name() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("b.toml"),
            write("B", ThemeKind::Light, &light()),
        )
        .unwrap();
        std::fs::write(folder.path().join("A.toml"), "kind = \"dark\"").unwrap();
        std::fs::write(folder.path().join("notes.txt"), "not a theme").unwrap();
        let entries = read_folder(folder.path());
        let files: Vec<&str> = entries.iter().map(|entry| entry.file.as_str()).collect();
        assert_eq!(files, ["A.toml", "b.toml"]);
        assert_eq!(entries[0].theme, Err(Problem::NoName));
        assert_eq!(entries[1].stem, "b");
        assert!(read_folder(&folder.path().join("missing")).is_empty());
    }
}
