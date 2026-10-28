//! Appearance preferences kept in the library database's settings table: the theme,
//! reduced motion, the zoom, whether the queue panel is open, which artists the Artists page
//! lists, and the column widths of the track lists and the songs table. Also hands
//! the operating system's reduce-motion setting to the theme.

use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_integrations::reduce_motion;
use vcmp_library::Library;

use crate::{AppWindow, ColumnWidths, ReduceMotion, Theme, ThemeMode, zoom};

pub const THEME: &str = "ui.theme";
/// "system", "on" or "off"; see `parse_motion` for the older "true" and "false".
pub const REDUCED_MOTION: &str = "ui.reduced_motion";
/// The zoom as a factor of the system's scale, such as "1.25" (`src/zoom.rs`).
pub const ZOOM: &str = "ui.zoom";
pub const QUEUE_OPEN: &str = "ui.queue_open";
pub const ARTISTS_ALL: &str = "ui.artists_all";
/// The track lists' column shares: title, then the column beside it.
pub const TRACK_COLUMNS: &str = "ui.track_columns";
/// The songs table's column shares: title, artist, album.
pub const SONG_COLUMNS: &str = "ui.song_columns";

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
    if let Some(saved) = library.setting(ZOOM)?.as_deref().and_then(zoom::parse) {
        theme.set_zoom(saved);
        zoom::apply(window);
    }
    if let Some(value) = library.setting(QUEUE_OPEN)? {
        window.set_queue_open(value == "true");
    }
    if library.setting(ARTISTS_ALL)?.as_deref() == Some("true") {
        window.set_artists_show_all(true);
        window.invoke_artists_filter_changed();
    }
    let columns = window.global::<ColumnWidths>();
    if let Some([title, detail]) = library.setting(TRACK_COLUMNS)?.as_deref().and_then(shares) {
        columns.set_track_title(title);
        columns.set_track_detail(detail);
    }
    if let Some([title, artist, album]) = library.setting(SONG_COLUMNS)?.as_deref().and_then(shares)
    {
        columns.set_song_title(title);
        columns.set_song_artist(artist);
        columns.set_song_album(album);
    }
    let weak = window.as_weak();
    let saving = library.clone();
    columns.on_save(move || {
        let Some(window) = weak.upgrade() else { return };
        let columns = window.global::<ColumnWidths>();
        let tracks = format!(
            "{} {}",
            columns.get_track_title(),
            columns.get_track_detail()
        );
        let songs = format!(
            "{} {} {}",
            columns.get_song_title(),
            columns.get_song_artist(),
            columns.get_song_album()
        );
        let saved = saving
            .set_setting(TRACK_COLUMNS, &tracks)
            .and_then(|()| saving.set_setting(SONG_COLUMNS, &songs));
        if let Err(error) = saved {
            log::warn!("could not save the column widths: {error}");
        }
    });

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
            .and_then(|()| library.set_setting(ZOOM, &format!("{}", theme.get_zoom())))
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

/// Column shares saved as numbers separated by spaces; `None` when there aren't `N`
/// of them or one isn't a positive number.
fn shares<const N: usize>(saved: &str) -> Option<[f32; N]> {
    let values: Vec<f32> = saved
        .split_whitespace()
        .map(|value| {
            value
                .parse()
                .ok()
                .filter(|v: &f32| v.is_finite() && *v > 0.0)
        })
        .collect::<Option<_>>()?;
    values.try_into().ok()
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
    fn column_shares_are_read_back_or_ignored() {
        assert_eq!(shares::<2>("0.7 0.3"), Some([0.7, 0.3]));
        assert_eq!(shares::<3>("0.5 0.25 0.25"), Some([0.5, 0.25, 0.25]));
        assert_eq!(shares::<2>("0.7"), None, "too few");
        assert_eq!(shares::<2>("0.5 0.25 0.25"), None, "too many");
        assert_eq!(shares::<2>("0.7 -1"), None);
        assert_eq!(shares::<2>("0.7 NaN"), None);
        assert_eq!(shares::<2>("wide narrow"), None);
    }

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
