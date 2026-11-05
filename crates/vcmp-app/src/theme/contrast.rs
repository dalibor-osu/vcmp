//! The WCAG AA contrast check: the pairs of a palette that must stand apart, and how
//! far they do. The built-in themes' tests and the warning Settings shows for a theme
//! file both use it, so "passes" means one thing.

use slint::Color;

use crate::ThemePalette;

const TEXT: f64 = 4.5;
const CONTROL: f64 = 3.0;

/// The pairs, by name, with the contrast each needs: 4.5:1 for text, 3:1 for
/// boundaries and indicators of controls. `ThemeWords.pair-name` words them in the
/// same order.
pub const PAIRS: [(&str, f64); 14] = [
    ("text on base", TEXT),
    ("text on mantle", TEXT),
    ("text on surface", TEXT),
    ("muted text on base", TEXT),
    ("muted text on mantle", TEXT),
    ("accent text on base", TEXT),
    ("accent text on mantle", TEXT),
    // The playing track's title on a hovered or selected row.
    ("accent text on surface", TEXT),
    ("on-accent on accent", TEXT),
    ("error text on base", TEXT),
    ("outline on base", CONTROL),
    ("outline on mantle", CONTROL),
    ("accent on surface", CONTROL),
    ("muted knob on base", CONTROL),
];

fn colours(p: &ThemePalette) -> [(Color, Color); 14] {
    [
        (p.text, p.base),
        (p.text, p.mantle),
        (p.text, p.surface),
        (p.text_muted, p.base),
        (p.text_muted, p.mantle),
        (p.accent, p.base),
        (p.accent, p.mantle),
        (p.accent, p.surface),
        (p.on_accent, p.accent),
        (p.error, p.base),
        (p.outline, p.base),
        (p.outline, p.mantle),
        (p.accent, p.surface),
        (p.text_muted, p.base),
    ]
}

/// A pair below what it needs: its place in [`PAIRS`] and its contrast.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub pair: usize,
    pub ratio: f64,
    pub needed: f64,
}

/// The pairs of the palette below WCAG AA, in the order of [`PAIRS`].
pub fn failures(palette: &ThemePalette) -> Vec<Failure> {
    colours(palette)
        .into_iter()
        .zip(PAIRS)
        .enumerate()
        .filter_map(|(pair, ((fg, bg), (_, needed)))| {
            let ratio = contrast(fg, bg);
            (ratio < needed).then_some(Failure {
                pair,
                ratio,
                needed,
            })
        })
        .collect()
}

/// The contrast ratio of two colours, from 1 to 21. Alpha is not taken into account.
pub fn contrast(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn luminance(color: Color) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.red()) + 0.7152 * channel(color.green()) + 0.0722 * channel(color.blue())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::light;

    #[test]
    fn black_on_white_is_21_to_1() {
        let ratio = contrast(
            Color::from_rgb_u8(0, 0, 0),
            Color::from_rgb_u8(255, 255, 255),
        );
        assert!((ratio - 21.0).abs() < 1e-9);
        assert_eq!(contrast(light().base, light().base), 1.0);
    }

    #[test]
    fn failing_pairs_are_named_with_their_ratio() {
        let mut palette = light();
        palette.text_muted = palette.base;
        let failed = failures(&palette);
        let names: Vec<&str> = failed.iter().map(|f| PAIRS[f.pair].0).collect();
        assert_eq!(
            names,
            [
                "muted text on base",
                "muted text on mantle",
                "muted knob on base"
            ]
        );
        assert_eq!(failed[0].ratio, 1.0);
        assert_eq!(failed[0].needed, 4.5);
        assert_eq!(failed[2].needed, 3.0);
        assert!(failures(&light()).is_empty());
    }
}
