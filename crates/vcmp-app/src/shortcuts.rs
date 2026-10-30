//! Space as play/pause, everywhere but in a text field.
//!
//! Slint routes keys to the focused element and its parents only, and it drops the
//! focus when the focused element is removed or hidden (the now playing view's close
//! button when the view closes, an album page's back button when it closes). Keys
//! then reach nothing, so a Space handler in the window would stop working. This
//! hook sees the native key and mouse events before Slint does.
//!
//! A click also takes the focus off whatever has it, as people expect a click on the
//! background to do. The window's own handler (ui/app-window.slint) catches Space
//! where the hook cannot reach, such as in tests.
//!
//! Mouse-wheel turns go to [`WheelScroller`], which scrolls once per frame however
//! fast the wheel spins (see `src/wheel.rs`), and the mouse's back and forward
//! buttons to the history (`src/navigation.rs`).
//!
//! The system's scale factor changing (another monitor) and the native window being
//! made both reset Slint's scale to the system's; the zoom goes back on top
//! (`src/zoom.rs`). The native window being made also gives Windows' title bar the
//! theme's colours (`src/title_bar.rs`).
//!
//! The keyboard's media keys control the player while the window has the focus.
//! Desktops that send them to the playing app over MPRIS (GNOME, KDE, playerctl)
//! reach it through the media controls whatever has the focus; a window manager
//! that binds nothing hands them to the focused window, which is where these come in.

use std::cell::Cell;
use std::rc::Rc;

use slint::winit_030::winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, ModifiersState, NamedKey};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, LogicalPosition};

use crate::wheel::{LINE, WheelScroller};
use crate::{AppWindow, PlayState, PlayerState, title_bar, zoom};

pub fn install(window: &AppWindow) {
    let weak = window.as_weak();
    let modifiers = Rc::new(Cell::new(ModifiersState::empty()));
    let scroller = WheelScroller::new(window);
    let pointer = Cell::new(LogicalPosition::new(0.0, 0.0));
    let started = Cell::new(false);
    window.window().on_winit_window_event(move |_, event| {
        let Some(window) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        // The first event comes from a native window that Slint has just created and
        // given the system's scale. On Windows its frame takes the theme's colours.
        if !started.replace(true) {
            zoom::apply_soon(&window);
            title_bar::apply(&window);
        }
        match event {
            WindowEvent::ScaleFactorChanged { .. } => zoom::apply_soon(&window),
            WindowEvent::ModifiersChanged(changed) => modifiers.set(changed.state()),
            WindowEvent::CursorMoved { position, .. } => {
                let scale = window.window().scale_factor();
                pointer.set(LogicalPosition::new(
                    position.x as f32 / scale,
                    position.y as f32 / scale,
                ));
            }
            // A mouse wheel, in lines; a touchpad's pixel deltas go to Slint as they
            // come, for its kinetic scrolling.
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(x, y),
                ..
            } => {
                scroller.add(pointer.get(), x * LINE, y * LINE);
                return EventResult::PreventDefault;
            }
            // Space without modifiers, while no text field takes it.
            WindowEvent::KeyboardInput { event, .. }
                if event.logical_key == Key::Named(NamedKey::Space)
                    && modifiers.get().is_empty()
                    && !window.get_typing() =>
            {
                if event.state == ElementState::Pressed && !event.repeat {
                    let state = window.global::<PlayerState>();
                    if state.get_has_track() {
                        state.invoke_toggle_play();
                    }
                }
                return EventResult::PreventDefault;
            }
            WindowEvent::KeyboardInput { event, .. } if media_key(&event.logical_key) => {
                if event.state == ElementState::Pressed && !event.repeat {
                    play_media_key(&window, &event.logical_key);
                }
                return EventResult::PreventDefault;
            }
            // The mouse's back and forward buttons go through the history
            // (src/navigation.rs).
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Back,
                ..
            } => {
                window.invoke_go_back();
                return EventResult::PreventDefault;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Forward,
                ..
            } => {
                window.invoke_go_forward();
                return EventResult::PreventDefault;
            }
            // A click first takes the focus off whatever has it, so the focus
            // ring goes and Space plays and pauses again. A click on a text
            // field gives it the focus straight back, as the field takes it when
            // pressed; the app's other controls do not take focus on a click.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => window.invoke_release_focus(),
            _ => {}
        }
        EventResult::Propagate
    });
}

fn media_key(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::MediaPlayPause
                | NamedKey::MediaPlay
                | NamedKey::MediaPause
                | NamedKey::MediaStop
                | NamedKey::MediaTrackNext
                | NamedKey::MediaTrackPrevious
        )
    )
}

/// What a media key does, as the same buttons in the player bar would.
pub fn play_media_key(window: &AppWindow, key: &Key) {
    let state = window.global::<PlayerState>();
    if !state.get_has_track() {
        return;
    }
    let playing = state.get_state() == PlayState::Playing;
    match key {
        Key::Named(NamedKey::MediaPlayPause) => state.invoke_toggle_play(),
        Key::Named(NamedKey::MediaPlay) if !playing => state.invoke_toggle_play(),
        Key::Named(NamedKey::MediaPause | NamedKey::MediaStop) if playing => {
            state.invoke_toggle_play()
        }
        Key::Named(NamedKey::MediaTrackNext) => state.invoke_next(),
        Key::Named(NamedKey::MediaTrackPrevious) => state.invoke_previous(),
        _ => {}
    }
}
