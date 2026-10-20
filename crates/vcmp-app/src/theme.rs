//! Colour palettes for the UI's `Theme` global: Catppuccin Latte (light) and Mocha
//! (dark), with pink as the single accent and Catppuccin red for errors.
//!
//! Latte's own pink is too pale to meet contrast requirements on its light
//! backgrounds, so the light accent is a darker shade of the same hue. Latte's red
//! reaches 4.5:1 only on base, which is where errors are written. The scrim behind a
//! dialog is the palette's darkest colour, translucent.

use slint::{Color, ComponentHandle};

use crate::{AppWindow, Theme, ThemePalette};

const fn rgb(hex: u32) -> Color {
    rgba(hex, 0xff)
}

const fn rgba(hex: u32, alpha: u8) -> Color {
    Color::from_argb_u8(alpha, (hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

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

/// Hands both palettes to the UI; the UI picks one from the theme mode.
pub fn install(window: &AppWindow) {
    let theme = window.global::<Theme>();
    theme.set_light_colors(light());
    theme.set_dark_colors(dark());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.red())
            + 0.7152 * channel(color.green())
            + 0.0722 * channel(color.blue())
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// WCAG AA: 4.5:1 for text, 3:1 for boundaries and indicators of controls.
    fn assert_pairs(name: &str, p: &ThemePalette) {
        let text_pairs = [
            ("text on base", p.text, p.base),
            ("text on mantle", p.text, p.mantle),
            ("text on surface", p.text, p.surface),
            ("muted text on base", p.text_muted, p.base),
            ("muted text on mantle", p.text_muted, p.mantle),
            ("accent text on base", p.accent, p.base),
            ("accent text on mantle", p.accent, p.mantle),
            // The playing track's title on a hovered or selected row.
            ("accent text on surface", p.accent, p.surface),
            ("on-accent on accent", p.on_accent, p.accent),
            ("error text on base", p.error, p.base),
        ];
        let control_pairs = [
            ("outline on base", p.outline, p.base),
            ("outline on mantle", p.outline, p.mantle),
            ("accent on surface", p.accent, p.surface),
            ("muted knob on base", p.text_muted, p.base),
        ];
        for (pair, fg, bg) in text_pairs {
            let ratio = contrast(fg, bg);
            assert!(ratio >= 4.5, "{name}: {pair} is {ratio:.2}:1, needs 4.5:1");
        }
        for (pair, fg, bg) in control_pairs {
            let ratio = contrast(fg, bg);
            assert!(ratio >= 3.0, "{name}: {pair} is {ratio:.2}:1, needs 3:1");
        }
    }

    #[test]
    fn light_palette_meets_contrast() {
        assert_pairs("light", &light());
    }

    #[test]
    fn dark_palette_meets_contrast() {
        assert_pairs("dark", &dark());
    }
}
