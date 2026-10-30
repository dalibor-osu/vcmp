//! Windows' title bar and window border in the theme's colours, instead of the
//! system's (often its accent colour): the bar in the sidebar's colour (`mantle`), the
//! title in `text` (`text-muted` while the window is inactive), the border in
//! `surface`, and Windows' dark frame under the dark theme. Windows 10 takes only the
//! dark frame (`vcmp_integrations::window_frame`).
//!
//! Set once the native window exists (the winit hook in `src/shortcuts.rs`) and again
//! whenever the theme's colours change (`AppWindow.frame-colors-changed`). macOS and
//! Linux keep the system's frame.

use slint::{Color, ComponentHandle};
use vcmp_integrations::window_frame::{self, FrameColours, Rgb};

use crate::{AppWindow, Theme, ThemePalette, media_keys};

/// Sets the frame's colours again whenever the theme's change.
pub fn install(window: &AppWindow) {
    let weak = window.as_weak();
    window.on_frame_colors_changed(move || {
        if let Some(window) = weak.upgrade() {
            apply(&window);
        }
    });
}

/// Gives the native window the theme's frame colours; nothing without one, or off
/// Windows.
pub fn apply(window: &AppWindow) {
    let Some(hwnd) = media_keys::native_handle(window) else {
        return;
    };
    let theme = window.global::<Theme>();
    window_frame::set(hwnd, frame_colours(&theme.get_colors(), theme.get_dark()));
}

/// The frame's colours for a theme palette.
pub fn frame_colours(palette: &ThemePalette, dark: bool) -> FrameColours {
    FrameColours {
        dark,
        caption: rgb(palette.mantle),
        text: rgb(palette.text),
        inactive_text: rgb(palette.text_muted),
        border: rgb(palette.surface),
    }
}

fn rgb(colour: Color) -> Rgb {
    [colour.red(), colour.green(), colour.blue()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    fn the_frame_takes_the_sidebar_and_text_colours() {
        let dark = frame_colours(&theme::dark(), true);
        assert!(dark.dark);
        assert_eq!(dark.caption, [0x18, 0x18, 0x25], "Mocha's mantle");
        assert_eq!(dark.text, [0xcd, 0xd6, 0xf4]);
        assert_eq!(dark.inactive_text, [0xa6, 0xad, 0xc8]);
        assert_eq!(dark.border, [0x31, 0x32, 0x44]);

        let light = frame_colours(&theme::light(), false);
        assert!(!light.dark);
        assert_eq!(light.caption, [0xe6, 0xe9, 0xef], "Latte's mantle");
        assert_eq!(light.text, [0x4c, 0x4f, 0x69]);
    }
}
