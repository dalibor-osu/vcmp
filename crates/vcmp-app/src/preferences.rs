//! Appearance preferences kept in the library database's settings table: the theme,
//! reduced motion, whether the queue panel is open, and which artists the Artists
//! page lists. Also hands the operating system's reduce-motion setting to the theme.

use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_integrations::reduce_motion;
use vcmp_library::Library;

use crate::{AppWindow, ReduceMotion, Theme, ThemeMode};

pub const THEME: &str = "ui.theme";
/// "system", "on" or "off"; see `parse_motion` for the older "true" and "false".
pub const REDUCED_MOTION: &str = "ui.reduced_motion";
pub const QUEUE_OPEN: &str = "ui.queue_open";
pub const ARTISTS_ALL: &str = "ui.artists_all";

/// Applies the saved preferences, then saves them whenever the user changes them.
pub fn connect(window: &AppWindow, library: Rc<Library>) -> vcmp_library::Result<()> {
    let theme = window.global::<Theme>();
    if let Some(mode) = library.setting(THEME)?.as_deref().and_then(parse_mode) {
        theme.set_mode(mode);
    }
    if let Some(choice) = library
        .setting(REDUCED_MOTION)?
        .as_deref()
        .and_then(parse_motion)
    {
        theme.set_motion_choice(choice);
    }
    if let Some(value) = library.setting(QUEUE_OPEN)? {
        window.set_queue_open(value == "true");
    }
    if library.setting(ARTISTS_ALL)?.as_deref() == Some("true") {
        window.set_artists_show_all(true);
        window.invoke_artists_filter_changed();
    }

    let weak = window.as_weak();
    theme.on_preferences_changed(move || {
        let Some(window) = weak.upgrade() else { return };
        let theme = window.global::<Theme>();
        let flag = |on: bool| if on { "true" } else { "false" };
        let saved = library
            .set_setting(THEME, mode_name(theme.get_mode()))
            .and_then(|()| {
                library.set_setting(REDUCED_MOTION, motion_name(theme.get_motion_choice()))
            })
            .and_then(|()| library.set_setting(QUEUE_OPEN, flag(window.get_queue_open())))
            .and_then(|()| library.set_setting(ARTISTS_ALL, flag(window.get_artists_show_all())));
        if let Err(error) = saved {
            log::warn!("could not save appearance preferences: {error}");
        }
    });
    Ok(())
}

/// Hands the operating system's reduce-motion setting to the theme once it is known,
/// and again whenever it changes. Until it is known, and where the system has no such
/// setting, the system counts as not asking.
pub fn follow_system_motion(window: &AppWindow) {
    let weak = window.as_weak();
    reduce_motion::follow(move |reduce| {
        let _ = weak.upgrade_in_event_loop(move |window| {
            window.global::<Theme>().set_system_reduces_motion(reduce);
        });
    });
}

fn mode_name(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::System => "system",
        ThemeMode::Light => "light",
        ThemeMode::Dark => "dark",
    }
}

fn parse_mode(name: &str) -> Option<ThemeMode> {
    match name {
        "system" => Some(ThemeMode::System),
        "light" => Some(ThemeMode::Light),
        "dark" => Some(ThemeMode::Dark),
        _ => None,
    }
}

fn motion_name(choice: ReduceMotion) -> &'static str {
    match choice {
        ReduceMotion::System => "system",
        ReduceMotion::On => "on",
        ReduceMotion::Off => "off",
    }
}

/// Before VCMP followed the operating system the setting was "true" or "false".
/// "false" was saved along with every other appearance change, so it doesn't show
/// that the user chose motion: it now follows the system.
fn parse_motion(name: &str) -> Option<ReduceMotion> {
    match name {
        "system" | "false" => Some(ReduceMotion::System),
        "on" | "true" => Some(ReduceMotion::On),
        "off" => Some(ReduceMotion::Off),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_choices_survive_saving() {
        for choice in [ReduceMotion::System, ReduceMotion::On, ReduceMotion::Off] {
            assert_eq!(parse_motion(motion_name(choice)), Some(choice));
        }
    }

    #[test]
    fn older_saved_switch_values_are_read() {
        assert_eq!(parse_motion("true"), Some(ReduceMotion::On));
        assert_eq!(
            parse_motion("false"),
            Some(ReduceMotion::System),
            "the old default follows the system now"
        );
        assert_eq!(parse_motion("sometimes"), None);
    }
}
