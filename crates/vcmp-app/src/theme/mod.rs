//! VCMP's built-in themes and accents, the colours of labels, and handing a theme to
//! the UI's `Theme` global.
//!
//! A built-in theme is a palette and a kind; the accent is chosen apart from it, from
//! the label hues, and fills `accent` (`on-accent` stays the theme's). Light is the
//! light theme; Dark (cool blue-grey), Darker (warm, deeper) and Midnight (neutral,
//! near black) are dark. The palettes are VCMP's own pastel ones, inspired by
//! Catppuccin, with red for errors.
//!
//! A pastel accent is too pale for contrast on the light backgrounds, so each accent
//! has a light colour of the same hue, darkened by hand until it reaches 4.5:1 on the
//! light surface, and a dark colour shared by the three dark themes. Yellow has no
//! light colour: darkened that far it is brown. The scrim behind a dialog is the
//! palette's darkest colour, translucent.
//!
//! Labels take pastel colours too: as they are on the dark themes, and darkened, in
//! the same hue, on the light one until a label's dot reaches 3:1 on the surface fill
//! of a hovered row (most pastels are paler than that).

pub mod contrast;
pub mod file;

use slint::{Color, ComponentHandle};

use crate::{AppWindow, Theme, ThemeKind, ThemePalette};

const fn rgb(hex: u32) -> Color {
    rgba(hex, 0xff)
}

const fn rgba(hex: u32, alpha: u8) -> Color {
    Color::from_argb_u8(alpha, (hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The built-in themes, in the order the picker shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltIn {
    Light,
    Dark,
    Darker,
    Midnight,
}

impl BuiltIn {
    pub const ALL: [BuiltIn; 4] = [
        BuiltIn::Light,
        BuiltIn::Dark,
        BuiltIn::Darker,
        BuiltIn::Midnight,
    ];

    /// The id saved in `ui.theme` and used by the picker.
    pub fn id(self) -> &'static str {
        match self {
            BuiltIn::Light => "light",
            BuiltIn::Dark => "dark",
            BuiltIn::Darker => "darker",
            BuiltIn::Midnight => "midnight",
        }
    }

    pub fn from_id(id: &str) -> Option<BuiltIn> {
        BuiltIn::ALL.into_iter().find(|theme| theme.id() == id)
    }

    /// The name written into a theme file saved from this theme. The picker shows
    /// built-in names translated (`ThemeWords.name`); a file's name is the user's.
    pub fn file_name(self) -> &'static str {
        match self {
            BuiltIn::Light => "Light",
            BuiltIn::Dark => "Dark",
            BuiltIn::Darker => "Darker",
            BuiltIn::Midnight => "Midnight",
        }
    }

    pub fn kind(self) -> ThemeKind {
        match self {
            BuiltIn::Light => ThemeKind::Light,
            _ => ThemeKind::Dark,
        }
    }

    /// The theme shown for a theme file of this kind that is gone, and the one the
    /// first start picks for a system of this kind.
    pub fn of_kind(kind: ThemeKind) -> BuiltIn {
        match kind {
            ThemeKind::Light => BuiltIn::Light,
            _ => BuiltIn::Dark,
        }
    }

    /// The theme's palette with the accent at `hue` in [`ACCENTS`], or pink where the
    /// accent has no colour for this theme's kind.
    pub fn palette(self, hue: usize) -> ThemePalette {
        let mut palette = match self {
            BuiltIn::Light => light(),
            BuiltIn::Dark => dark(),
            BuiltIn::Darker => darker(),
            BuiltIn::Midnight => midnight(),
        };
        palette.accent = accent_colour(hue, self.kind()).unwrap_or(palette.accent);
        palette
    }
}

/// The light theme, with the pink accent; unchanged since VCMP had only a light and a
/// dark palette.
pub fn light() -> ThemePalette {
    ThemePalette {
        base: rgb(0xeff1f5),
        mantle: rgb(0xe6e9ef),
        surface: rgb(0xccd0da),
        outline: rgb(0x7c7f93),
        text: rgb(0x4c4f69),
        text_muted: rgb(0x5c5f77),
        accent: rgb(0xa1197d),
        on_accent: rgb(0xeff1f5),
        error: rgb(0xd20f39),
        scrim: rgba(0x4c4f69, 0x73),
    }
}

/// The cool dark theme, with the pink accent; unchanged since VCMP had only a light
/// and a dark palette.
pub fn dark() -> ThemePalette {
    ThemePalette {
        base: rgb(0x1e1e2e),
        mantle: rgb(0x181825),
        surface: rgb(0x313244),
        outline: rgb(0x6c7086),
        text: rgb(0xcdd6f4),
        text_muted: rgb(0xa6adc8),
        accent: rgb(0xf5c2e7),
        on_accent: rgb(0x11111b),
        error: rgb(0xf38ba8),
        scrim: rgba(0x11111b, 0xb3),
    }
}

/// Warm brown-greys, a step deeper than Dark.
fn darker() -> ThemePalette {
    ThemePalette {
        base: rgb(0x1c1917),
        mantle: rgb(0x151311),
        surface: rgb(0x2f2a26),
        outline: rgb(0x736a63),
        text: rgb(0xe8ded5),
        text_muted: rgb(0xb5aaa0),
        accent: rgb(0xf5c2e7),
        on_accent: rgb(0x0f0d0c),
        error: rgb(0xf38ba8),
        scrim: rgba(0x0f0d0c, 0xb3),
    }
}

/// Neutral greys close to black.
fn midnight() -> ThemePalette {
    ThemePalette {
        base: rgb(0x0e0e10),
        mantle: rgb(0x08080a),
        surface: rgb(0x232327),
        outline: rgb(0x65656d),
        text: rgb(0xe2e2e8),
        text_muted: rgb(0xa3a3ad),
        accent: rgb(0xf5c2e7),
        on_accent: rgb(0x08080a),
        error: rgb(0xf38ba8),
        scrim: rgba(0x08080a, 0xb3),
    }
}

/// An accent: a label hue, with its colour on the light theme (none where the hue
/// can't reach 4.5:1 and stay recognisable) and on the dark themes.
pub struct Accent {
    pub name: &'static str,
    pub light: Option<Color>,
    pub dark: Color,
}

/// The accents, in the order of the label hues (`vcmp_library::LABEL_PALETTE`). Pink,
/// the first, is the default.
pub const ACCENTS: [Accent; 10] = [
    accent("pink", Some(0xa1197d), 0xf5c2e7),
    accent("mauve", Some(0x6f13e7), 0xcba6f7),
    accent("red", Some(0xae0c2f), 0xf38ba8),
    accent("peach", Some(0x983801), 0xfab387),
    accent("yellow", None, 0xf9e2af),
    accent("green", Some(0x28641b), 0xa6e3a1),
    accent("teal", Some(0x0f6166), 0x94e2d5),
    accent("sky", Some(0x025d81), 0x89dceb),
    accent("blue", Some(0x094bcf), 0x89b4fa),
    accent("lavender", Some(0x1034fc), 0xb4befe),
];

pub const DEFAULT_ACCENT: usize = 0;

const fn accent(name: &'static str, light: Option<u32>, dark: u32) -> Accent {
    Accent {
        name,
        light: match light {
            Some(hex) => Some(rgb(hex)),
            None => None,
        },
        dark: rgb(dark),
    }
}

/// The accent at `hue` on a theme of this kind; `None` for a hue that doesn't exist
/// or has no colour for the kind.
pub fn accent_colour(hue: usize, kind: ThemeKind) -> Option<Color> {
    let accent = ACCENTS.get(hue)?;
    match kind {
        ThemeKind::Light => accent.light,
        _ => Some(accent.dark),
    }
}

pub fn accent_by_name(name: &str) -> Option<usize> {
    ACCENTS.iter().position(|accent| accent.name == name)
}

/// The colours a label can have by name (`vcmp_library::LABEL_PALETTE`, in order):
/// on the light theme and on the dark ones.
pub const LABEL_COLOURS: [(&str, Color, Color); 10] = [
    ("pink", rgb(0xd220a3), rgb(0xf5c2e7)),
    ("mauve", rgb(0x8839ef), rgb(0xcba6f7)),
    ("red", rgb(0xd20f39), rgb(0xf38ba8)),
    ("peach", rgb(0xc84a01), rgb(0xfab387)),
    ("yellow", rgb(0xa26715), rgb(0xf9e2af)),
    ("green", rgb(0x358423), rgb(0xa6e3a1)),
    ("teal", rgb(0x148187), rgb(0x94e2d5)),
    ("sky", rgb(0x037bab), rgb(0x89dceb)),
    ("blue", rgb(0x1e66f5), rgb(0x89b4fa)),
    ("lavender", rgb(0x4763fc), rgb(0xb4befe)),
];

/// A label's colour as stored (`palette:` and a name, or `#rrggbb`) on light and on
/// dark themes. A colour that can't be read is drawn in the outline's grey.
pub fn label_colours(colour: &str) -> (Color, Color) {
    if let Some(name) = colour.strip_prefix("palette:")
        && let Some((_, light, dark)) = LABEL_COLOURS.iter().find(|(known, _, _)| *known == name)
    {
        return (*light, *dark);
    }
    match parse_hex(colour) {
        Some(colour) => (colour, colour),
        None => (light().outline, dark().outline),
    }
}

/// `#rrggbb` as a colour.
pub fn parse_hex(text: &str) -> Option<Color> {
    let digits = text.trim().strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(rgb)
}

/// Shows a palette of this kind. From then on the std widgets and the title bar
/// follow the kind, no longer the system.
pub fn apply(window: &AppWindow, palette: ThemePalette, kind: ThemeKind) {
    let theme = window.global::<Theme>();
    theme.set_colors(palette);
    theme.set_kind(kind);
    theme.invoke_apply_scheme();
}

/// Shows the light theme until the saved choice, or the first start's pick, replaces
/// it (`src/themes.rs`). The system's scheme stays readable until then.
pub fn install(window: &AppWindow) {
    let theme = window.global::<Theme>();
    theme.set_colors(BuiltIn::Light.palette(DEFAULT_ACCENT));
    theme.set_kind(ThemeKind::Light);
}

#[cfg(test)]
mod tests {
    use super::contrast::{PAIRS, failures};
    use super::*;

    fn assert_passes(name: &str, palette: &ThemePalette) {
        let failed: Vec<String> = failures(palette)
            .iter()
            .map(|failure| {
                format!(
                    "{}: {:.2}:1, needs {}:1",
                    PAIRS[failure.pair].0, failure.ratio, failure.needed
                )
            })
            .collect();
        assert!(failed.is_empty(), "{name}: {}", failed.join("; "));
    }

    #[test]
    fn every_built_in_theme_meets_contrast_with_every_accent() {
        for theme in BuiltIn::ALL {
            for (hue, accent) in ACCENTS.iter().enumerate() {
                if accent_colour(hue, theme.kind()).is_none() {
                    continue;
                }
                assert_passes(
                    &format!("{} {}", theme.id(), accent.name),
                    &theme.palette(hue),
                );
            }
        }
    }

    #[test]
    fn today_s_palettes_are_kept() {
        assert_eq!(BuiltIn::Light.palette(DEFAULT_ACCENT), light());
        assert_eq!(BuiltIn::Dark.palette(DEFAULT_ACCENT), dark());
    }

    #[test]
    fn every_accent_has_a_dark_colour_and_only_yellow_lacks_a_light_one() {
        let without_light: Vec<&str> = ACCENTS
            .iter()
            .filter(|accent| accent.light.is_none())
            .map(|accent| accent.name)
            .collect();
        assert_eq!(without_light, ["yellow"]);
        assert_eq!(accent_colour(4, ThemeKind::Light), None);
        assert_eq!(
            BuiltIn::Light.palette(4).accent,
            light().accent,
            "pink instead"
        );
        assert_eq!(accent_colour(10, ThemeKind::Dark), None);
    }

    #[test]
    fn accents_follow_the_label_hues() {
        let names: Vec<&str> = ACCENTS.iter().map(|accent| accent.name).collect();
        assert_eq!(names, vcmp_library::LABEL_PALETTE);
        for (accent, (_, _, dark)) in ACCENTS.iter().zip(LABEL_COLOURS) {
            assert_eq!(
                accent.dark, dark,
                "{}: the label's dark colour",
                accent.name
            );
        }
    }

    /// A label's dot is a graphic beside its name: 3:1 on every fill a row can have,
    /// on every built-in theme.
    #[test]
    fn label_colours_stand_out_on_rows() {
        for (name, light_colour, dark_colour) in LABEL_COLOURS {
            for theme in BuiltIn::ALL {
                let palette = theme.palette(DEFAULT_ACCENT);
                let colour = match theme.kind() {
                    ThemeKind::Light => light_colour,
                    _ => dark_colour,
                };
                for (fill, background) in [
                    ("base", palette.base),
                    ("mantle", palette.mantle),
                    ("surface", palette.surface),
                ] {
                    let ratio = contrast::contrast(colour, background);
                    assert!(
                        ratio >= 3.0,
                        "{}: label {name} on {fill} is {ratio:.2}:1, needs 3:1",
                        theme.id()
                    );
                }
            }
        }
        let names: Vec<&str> = LABEL_COLOURS.iter().map(|(name, _, _)| *name).collect();
        assert_eq!(names, vcmp_library::LABEL_PALETTE);
    }

    #[test]
    fn ids_name_the_built_in_themes() {
        for theme in BuiltIn::ALL {
            assert_eq!(BuiltIn::from_id(theme.id()), Some(theme));
        }
        assert_eq!(BuiltIn::from_id("system"), None);
        assert_eq!(accent_by_name("teal"), Some(6));
        assert_eq!(accent_by_name("beige"), None);
    }
}
