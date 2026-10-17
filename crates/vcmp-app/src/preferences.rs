//! Appearance preferences kept in the library database's settings table.

use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_library::Library;

use crate::{AppWindow, Theme, ThemeMode};

pub const THEME: &str = "ui.theme";
pub const REDUCED_MOTION: &str = "ui.reduced_motion";

/// Applies the saved preferences, then saves them whenever the user changes them.
pub fn connect(window: &AppWindow, library: Rc<Library>) -> vcmp_library::Result<()> {
    let theme = window.global::<Theme>();
    if let Some(mode) = library.setting(THEME)?.as_deref().and_then(parse_mode) {
        theme.set_mode(mode);
    }
    if let Some(value) = library.setting(REDUCED_MOTION)? {
        theme.set_reduced_motion(value == "true");
    }

    let weak = window.as_weak();
    theme.on_preferences_changed(move || {
        let Some(window) = weak.upgrade() else { return };
        let theme = window.global::<Theme>();
        let saved = library
            .set_setting(THEME, mode_name(theme.get_mode()))
            .and_then(|()| {
                library.set_setting(
                    REDUCED_MOTION,
                    if theme.get_reduced_motion() {
                        "true"
                    } else {
                        "false"
                    },
                )
            });
        if let Err(error) = saved {
            log::warn!("could not save appearance preferences: {error}");
        }
    });
    Ok(())
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
