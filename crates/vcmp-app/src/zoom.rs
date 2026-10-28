//! Zoom: the whole UI drawn larger or smaller than the operating system's scale
//! asks, for a large monitor that leaves the app small (Settings → Appearance →
//! Zoom, Ctrl+= and Ctrl+-, Ctrl+0 back to 100 %).
//!
//! Slint draws, lays out and maps the pointer by its window's scale factor, so the
//! zoom is the operating system's factor times `Theme.zoom`, handed to the window as
//! a scale factor change. The native window keeps its size in pixels; the logical
//! size the pages lay out in shrinks or grows. When the system's factor changes (the
//! window moved to another monitor), Slint takes the new one and the winit hook
//! (`src/shortcuts.rs`) applies the zoom on top again.

use std::time::Duration;

use slint::platform::WindowEvent;
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, Timer};

use crate::{AppWindow, Theme};

/// The zoom steps; 1.0 is the system's scale. The ends match `Theme.zoom-smallest`
/// and `Theme.zoom-largest`.
pub const STEPS: [f32; 8] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

/// Answers `Theme.zoom-by`, and applies the zoom the theme holds.
pub fn install(window: &AppWindow) {
    let weak = window.as_weak();
    window.global::<Theme>().on_zoom_by(move |direction| {
        let Some(window) = weak.upgrade() else { return };
        let theme = window.global::<Theme>();
        let zoom = step(theme.get_zoom(), direction);
        if zoom != theme.get_zoom() {
            theme.set_zoom(zoom);
            apply(&window);
            theme.invoke_preferences_changed();
        }
    });
}

/// The step after `zoom` towards larger (`direction` > 0) or smaller (< 0); 0 goes
/// back to 100 %. A zoom between steps (saved by hand) moves to the nearest step in
/// that direction.
pub fn step(zoom: f32, direction: i32) -> f32 {
    match direction.signum() {
        1 => STEPS
            .into_iter()
            .find(|step| *step > zoom + 0.001)
            .unwrap_or(STEPS[STEPS.len() - 1]),
        -1 => STEPS
            .into_iter()
            .rev()
            .find(|step| *step < zoom - 0.001)
            .unwrap_or(STEPS[0]),
        _ => 1.0,
    }
}

/// A saved zoom, if it is one VCMP can apply.
pub fn parse(saved: &str) -> Option<f32> {
    saved
        .parse::<f32>()
        .ok()
        .filter(|zoom| (STEPS[0]..=STEPS[STEPS.len() - 1]).contains(zoom))
}

/// Gives the window the system's scale times the theme's zoom, unless it has it.
pub fn apply(window: &AppWindow) {
    let target = system_scale(window) * window.global::<Theme>().get_zoom();
    if (window.window().scale_factor() - target).abs() > 0.001 {
        window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: target,
            });
    }
}

/// Applies the zoom once the event that brought it about is handled: Slint sets the
/// system's factor when it creates the native window and when the system's factor
/// changes, after the winit hook has seen the event.
pub fn apply_soon(window: &AppWindow) {
    let weak = window.as_weak();
    Timer::single_shot(Duration::ZERO, move || {
        if let Some(window) = weak.upgrade() {
            apply(&window);
        }
    });
}

/// The scale the system asks for: `SLINT_SCALE_FACTOR` when set (Slint then ignores
/// the system's), else the native window's. Without a native window (tests), 1.
fn system_scale(window: &AppWindow) -> f32 {
    std::env::var("SLINT_SCALE_FACTOR")
        .ok()
        .and_then(|factor| factor.parse::<f32>().ok())
        .filter(|factor| *factor > 0.0)
        .or_else(|| {
            window
                .window()
                .with_winit_window(|native| native.scale_factor() as f32)
        })
        .unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_go_up_and_down_and_stop_at_the_ends() {
        assert_eq!(step(1.0, 1), 1.1);
        assert_eq!(step(1.0, -1), 0.9);
        assert_eq!(step(2.0, 1), 2.0);
        assert_eq!(step(0.8, -1), 0.8);
        assert_eq!(step(1.75, 0), 1.0);
        assert_eq!(step(1.2, 1), 1.25, "between steps, to the next one up");
        assert_eq!(step(1.2, -1), 1.1);
    }

    #[test]
    fn only_zooms_within_the_steps_are_read_back() {
        assert_eq!(parse("1.25"), Some(1.25));
        assert_eq!(parse("3"), None);
        assert_eq!(parse("0"), None);
        assert_eq!(parse("big"), None);
    }
}
