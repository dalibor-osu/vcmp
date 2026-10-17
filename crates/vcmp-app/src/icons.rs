//! Icon art for the `IconArt` global: the SVGs under `ui/icons`, with the colour the
//! UI asks for written into them. Slint then draws them as plain images, which come
//! out right on every GPU we have seen; see `ui/player-bar.slint` for the tinting
//! that did not.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{Color, ComponentHandle, Image};

use crate::{AppWindow, Glyph, IconArt};

/// The palette holds a handful of colours, so the cache stays small; the limit only
/// guards against a colour that keeps changing, such as an animated one.
const CACHE_LIMIT: usize = 64;

/// The SVG source of a glyph. Each file paints with `currentColor`.
pub fn source(glyph: Glyph) -> &'static str {
    match glyph {
        Glyph::Play => include_str!("../ui/icons/play.svg"),
        Glyph::Pause => include_str!("../ui/icons/pause.svg"),
        Glyph::Previous => include_str!("../ui/icons/previous.svg"),
        Glyph::Next => include_str!("../ui/icons/next.svg"),
        Glyph::Shuffle => include_str!("../ui/icons/shuffle.svg"),
        Glyph::Repeat => include_str!("../ui/icons/repeat.svg"),
        Glyph::RepeatOne => include_str!("../ui/icons/repeat-one.svg"),
        Glyph::Volume => include_str!("../ui/icons/volume.svg"),
        Glyph::Muted => include_str!("../ui/icons/muted.svg"),
    }
}

/// The glyph's SVG with `color` as its `currentColor`.
pub fn tinted(glyph: Glyph, color: Color) -> String {
    let mut attributes = format!(
        r##"<svg color="#{:02x}{:02x}{:02x}""##,
        color.red(),
        color.green(),
        color.blue()
    );
    if color.alpha() < u8::MAX {
        attributes.push_str(&format!(
            r#" opacity="{:.3}""#,
            color.alpha() as f32 / 255.0
        ));
    }
    source(glyph).replacen("<svg", &attributes, 1)
}

/// The glyph as an image painted in `color`. Slint renders it at the size it is shown.
pub fn paint(glyph: Glyph, color: Color) -> Image {
    Image::load_from_svg_data(tinted(glyph, color).as_bytes()).unwrap_or_default()
}

/// Answers the UI's `IconArt.paint` calls, keeping each painted glyph for reuse.
pub fn install(window: &AppWindow) {
    // A list rather than a map: Slint's generated enums are not hashable.
    let cache: Rc<RefCell<Vec<(Glyph, Color, Image)>>> = Rc::default();
    window.global::<IconArt>().on_paint(move |glyph, color| {
        let mut cache = cache.borrow_mut();
        if let Some((_, _, image)) = cache.iter().find(|(g, c, _)| *g == glyph && *c == color) {
            return image.clone();
        }
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        let image = paint(glyph, color);
        cache.push((glyph, color, image.clone()));
        image
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Glyph; 9] = [
        Glyph::Play,
        Glyph::Pause,
        Glyph::Previous,
        Glyph::Next,
        Glyph::Shuffle,
        Glyph::Repeat,
        Glyph::RepeatOne,
        Glyph::Volume,
        Glyph::Muted,
    ];

    #[test]
    fn every_glyph_paints_with_the_colour_given() {
        let pink = Color::from_rgb_u8(0xf5, 0xc2, 0xe7);
        for glyph in ALL {
            let svg = tinted(glyph, pink);
            assert!(
                svg.starts_with(r##"<svg color="#f5c2e7" "##),
                "{glyph:?}: {svg}"
            );
            assert!(
                !svg.contains("black") && svg.contains("currentColor"),
                "{glyph:?} takes all its colour from the tint"
            );
            let image = paint(glyph, pink);
            assert_eq!(
                (image.size().width, image.size().height),
                (24, 24),
                "{glyph:?} loads"
            );
        }
    }

    #[test]
    fn a_translucent_tint_keeps_its_alpha() {
        let svg = tinted(Glyph::Play, Color::from_argb_u8(0x80, 0, 0, 0));
        assert!(
            svg.starts_with(r##"<svg color="#000000" opacity="0.502" "##),
            "{svg}"
        );
    }
}
