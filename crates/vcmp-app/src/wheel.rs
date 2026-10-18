//! Mouse-wheel scrolling, one step per frame.
//!
//! A free-spinning or high-resolution wheel sends hundreds of wheel events a second
//! (Linux reports eight per notch for such wheels). Slint handles each one on its
//! own: it finds what is under the pointer and starts a new scroll animation. A fast
//! spin then queued more work than a second holds, above all in debug builds, and
//! the window froze until the queue drained, then scrolled the whole way at once.
//!
//! The winit hook (`src/shortcuts.rs`) hands mouse-wheel events here instead. They
//! are added up, and a timer scrolls once per frame, a third of what is left each
//! time, so a flick still glides to a stop. With reduced motion the whole distance
//! goes at once. Touchpad scrolling (pixel deltas with gesture phases) still goes to
//! Slint as it comes, for its kinetic flicks.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Timer, TimerMode};

use crate::{AppWindow, Theme};

/// Logical pixels per line of a wheel's line delta, as Slint's winit backend counts.
pub const LINE: f32 = 60.0;
/// How often the gathered distance is scrolled: once a frame at 60 Hz.
const TICK: Duration = Duration::from_millis(16);
/// The share of what is left that each frame scrolls.
const SHARE: f32 = 1.0 / 3.0;
/// Below this, the rest goes in one step, so a glide ends instead of crawling.
const LAST_STEP: f32 = 2.0;

pub struct WheelScroller {
    window: slint::Weak<AppWindow>,
    /// Distance still to scroll, in logical pixels.
    pending: Cell<(f32, f32)>,
    /// Where the pointer was when the wheel turned.
    position: Cell<LogicalPosition>,
    timer: Timer,
}

impl WheelScroller {
    pub fn new(window: &AppWindow) -> Rc<Self> {
        Rc::new(Self {
            window: window.as_weak(),
            pending: Cell::new((0.0, 0.0)),
            position: Cell::new(LogicalPosition::new(0.0, 0.0)),
            timer: Timer::default(),
        })
    }

    /// The wheel turned by `delta_x`, `delta_y` logical pixels with the pointer at
    /// `position`.
    pub fn add(self: &Rc<Self>, position: LogicalPosition, delta_x: f32, delta_y: f32) {
        self.position.set(position);
        let (x, y) = self.pending.get();
        self.pending.set((x + delta_x, y + delta_y));
        let reduced = self
            .window
            .upgrade()
            .is_some_and(|window| window.global::<Theme>().get_reduced_motion());
        if reduced {
            self.scroll(|left| left);
        } else if !self.timer.running() {
            let weak = Rc::downgrade(self);
            self.timer.start(TimerMode::Repeated, TICK, move || {
                if let Some(scroller) = weak.upgrade() {
                    scroller.scroll(step);
                }
            });
        }
    }

    /// Scrolls by `step` of what is left, and stops the timer once nothing is.
    fn scroll(&self, step: impl Fn(f32) -> f32) {
        let (x, y) = self.pending.get();
        let (dx, dy) = (step(x), step(y));
        self.pending.set((x - dx, y - dy));
        if (dx != 0.0 || dy != 0.0)
            && let Some(window) = self.window.upgrade()
        {
            window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: self.position.get(),
                    delta_x: dx,
                    delta_y: dy,
                });
        }
        if self.pending.get() == (0.0, 0.0) {
            self.timer.stop();
        }
    }
}

/// What one frame scrolls of `left`: a share of it, or all of it once it is small.
fn step(left: f32) -> f32 {
    if left.abs() <= LAST_STEP {
        left
    } else {
        let part = left * SHARE;
        if part.abs() < LAST_STEP {
            LAST_STEP.copysign(left)
        } else {
            part
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flick_glides_to_a_stop_and_scrolls_all_of_it() {
        let mut left = -600.0_f32;
        let mut steps = Vec::new();
        while left != 0.0 {
            let part = step(left);
            steps.push(part);
            left -= part;
            assert!(steps.len() < 100, "it ends");
        }
        assert_eq!(steps[0], -200.0, "a third first");
        assert!(
            steps.windows(2).all(|pair| pair[1].abs() <= pair[0].abs()),
            "slowing down: {steps:?}"
        );
        let total: f32 = steps.iter().sum();
        assert!((total + 600.0).abs() < 0.01, "all of it: {total}");
        assert!(
            steps.len() <= 16,
            "within about a quarter second: {}",
            steps.len()
        );
    }

    #[test]
    fn small_rests_go_at_once() {
        assert_eq!(step(1.5), 1.5);
        assert_eq!(step(-2.0), -2.0);
        assert_eq!(step(4.0), 2.0, "never a step smaller than the last one");
        assert_eq!(step(0.0), 0.0);
    }
}
