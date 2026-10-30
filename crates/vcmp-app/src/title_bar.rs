//! Windows: the window without the system's title bar, and VCMP's own minimise,
//! maximise and close buttons in its top right corner (`ui/window-frame.slint`).
//! `vcmp_integrations::window_frame` takes the title bar off and tells Windows where
//! the buttons and drag areas are; this module hands it their places, the scale and
//! the theme's colours (the dark frame, and the border in `surface`), and carries
//! the buttons' clicks back.
//!
//! The title bar comes off once the native window exists (the winit hook in
//! `src/shortcuts.rs`). Its places and colours are handed on again whenever the
//! theme, the zoom or the sidebar's drag area changes (`AppWindow.frame-changed`).
//! macOS and Linux keep the system's title bar: `WindowFrame.custom` stays false
//! and no buttons show.

use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Timer};
use vcmp_integrations::window_frame::{self, Caption, Command, Frame, MaximizeButton};

use crate::{AppWindow, Theme, WindowFrame, media_keys};

/// Answers the caption buttons and hands on changes to the frame.
pub fn install(window: &AppWindow) {
    let frame = window.global::<WindowFrame>();
    frame.on_minimize(commander(window, Command::Minimize));
    frame.on_toggle_maximized(commander(window, Command::ToggleMaximized));
    frame.on_close(commander(window, Command::Close));
    let weak = window.as_weak();
    window.on_frame_changed(move || {
        if let Some(window) = weak.upgrade() {
            apply(&window);
        }
    });
}

/// Takes the system's title bar off the native window, which has just been made.
pub fn start(window: &AppWindow) {
    let Some(hwnd) = media_keys::native_handle(window) else {
        return;
    };
    let weak = window.as_weak();
    // Windows reports the maximise button from inside its window procedure; the UI
    // hears of it once that has returned.
    let report = move |state: MaximizeButton| {
        let weak = weak.clone();
        Timer::single_shot(Duration::ZERO, move || {
            if let Some(window) = weak.upgrade() {
                let frame = window.global::<WindowFrame>();
                frame.set_maximize_hovered(state != MaximizeButton::Normal);
                frame.set_maximize_pressed(state == MaximizeButton::Pressed);
            }
        });
    };
    let custom = window_frame::install(hwnd, frame(window), report);
    window.global::<WindowFrame>().set_custom(custom);
    resized(window);
}

/// Hands the frame its colours and places again.
pub fn apply(window: &AppWindow) {
    if !window.global::<WindowFrame>().get_custom() {
        return;
    }
    if let Some(hwnd) = media_keys::native_handle(window) {
        window_frame::set(hwnd, frame(window));
    }
}

/// The window was resized, maybe maximised or restored: the maximise button shows
/// which.
pub fn resized(window: &AppWindow) {
    let maximized = window
        .window()
        .with_winit_window(|native| native.is_maximized())
        .unwrap_or(false);
    window.global::<WindowFrame>().set_maximized(maximized);
}

/// The window became active or stopped being it: the buttons' glyphs dim.
pub fn focused(window: &AppWindow, focused: bool) {
    window.global::<WindowFrame>().set_active(focused);
}

/// The frame as the window and the theme are now.
fn frame(window: &AppWindow) -> Frame {
    let theme = window.global::<Theme>();
    let border = theme.get_colors().surface;
    Frame {
        dark: theme.get_dark(),
        border: [border.red(), border.green(), border.blue()],
        caption: Caption {
            scale: window.window().scale_factor(),
            button_width: theme.get_caption_button_width(),
            button_height: theme.get_caption_button_height(),
            drag_height: theme.get_caption_drag_height(),
            sidebar_width: window.get_frame_sidebar_width(),
            sidebar_drag_height: window.get_frame_sidebar_drag_height(),
        },
    }
}

fn commander(window: &AppWindow, command: Command) -> impl Fn() + 'static {
    let weak = window.as_weak();
    move || {
        if let Some(hwnd) = weak
            .upgrade()
            .and_then(|window| media_keys::native_handle(&window))
        {
            window_frame::command(hwnd, command);
        }
    }
}
