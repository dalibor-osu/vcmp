//! Appearance preferences kept in the library database's settings table: the theme,
//! reduced motion, whether the queue panel is open, and which artists the Artists
//! page lists.

use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_library::Library;

use crate::{AppWindow, Theme, ThemeMode};

pub const THEME: &str = "ui.theme";
pub const REDUCED_MOTION: &str = "ui.reduced_motion";
pub const QUEUE_OPEN: &str = "ui.queue_open";
pub const ARTISTS_ALL: &str = "ui.artists_all";

/// Applies the saved preferences, then saves them whenever the user changes them.
pub fn connect(window: &AppWindow, library: Rc<Library>) -> vcmp_library::Result<()> {
    let theme = window.global::<Theme>();
    if let Some(mode) = library.setting(THEME)?.as_deref().and_then(parse_mode) {
        theme.set_mode(mode);
    }
    if let Some(value) = library.setting(REDUCED_MOTION)? {
        theme.set_reduced_motion(value == "true");
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
            .and_then(|()| library.set_setting(REDUCED_MOTION, flag(theme.get_reduced_motion())))
            .and_then(|()| library.set_setting(QUEUE_OPEN, flag(window.get_queue_open())))
            .and_then(|()| library.set_setting(ARTISTS_ALL, flag(window.get_artists_show_all())));
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
