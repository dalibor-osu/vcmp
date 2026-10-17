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

use std::cell::Cell;
use std::rc::Rc;

use slint::ComponentHandle;
use slint::winit_030::winit::event::{ElementState, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, ModifiersState, NamedKey};
use slint::winit_030::{EventResult, WinitWindowAccessor};

use crate::{AppWindow, PlayerState};

pub fn install(window: &AppWindow) {
    let weak = window.as_weak();
    let modifiers = Rc::new(Cell::new(ModifiersState::empty()));
    window.window().on_winit_window_event(move |_, event| {
        let Some(window) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        match event {
            WindowEvent::ModifiersChanged(changed) => modifiers.set(changed.state()),
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
