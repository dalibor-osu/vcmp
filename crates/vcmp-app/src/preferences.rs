//! Appearance preferences kept in the library database's settings table: reduced
//! motion, the zoom, whether the queue panel is open, which artists the Artists page
//! lists, the column widths of the track lists and the songs table, which of the
//! songs table's columns show, and which library sections the sidebar shows. Also
//! hands the operating system's reduce-motion setting to the theme. The theme itself
//! is saved by `src/themes.rs`.

use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_integrations::reduce_motion;
use vcmp_library::Library;

use crate::{
    AppWindow, ColumnWidths, Page, ReduceMotion, SidebarSections, SongColumn, SongColumns, Theme,
    zoom,
};

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
/// The songs table's columns that show, by name, separated by spaces (see
/// `HIDEABLE`); absent: all but plays, last played and added.
pub const SONG_COLUMNS_SHOWN: &str = "ui.song_columns_shown";

/// The library sections the sidebar hides, by name, separated by spaces (see
/// `SECTIONS`); absent: none. A section not named, such as one added later, shows.
pub const SIDEBAR_HIDDEN: &str = "ui.sidebar_hidden";

/// The library sections the sidebar can hide, by their saved names, in its order.
const SECTIONS: [(&str, Page); 6] = [
    ("albums", Page::Albums),
    ("artists", Page::Artists),
    ("playlists", Page::Playlists),
    ("labels", Page::Labels),
    ("songs", Page::Songs),
    ("folders", Page::Folders),
];

/// The songs table's columns that can be hidden, by their saved names, in the order
/// of `shown_columns`.
const HIDEABLE: [&str; 8] = [
    "artist",
    "album",
    "labels",
    "year",
    "duration",
    "plays",
    "last_played",
    "added",
];

/// Applies the saved preferences, then saves them whenever the user changes them.
pub fn connect(window: &AppWindow, library: Rc<Library>) -> vcmp_library::Result<()> {
    let theme = window.global::<Theme>();
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

    let song_columns = window.global::<SongColumns>();
    if let Some(saved) = library.setting(SONG_COLUMNS_SHOWN)? {
        show_columns(&song_columns, parse_shown(&saved));
    }
    let weak = window.as_weak();
    let saving = library.clone();
    song_columns.on_toggled(move || {
        let Some(window) = weak.upgrade() else { return };
        let columns = window.global::<SongColumns>();
        let shown = format_shown(shown_columns(&columns));
        if let Err(error) = saving.set_setting(SONG_COLUMNS_SHOWN, &shown) {
            log::warn!("could not save the songs table's columns: {error}");
        }
        if !column_shown(&columns, window.get_songs_sort_column()) {
            window.invoke_sort_songs(SongColumn::Title);
        }
    });

    let weak = window.as_weak();
    theme.on_preferences_changed(move || {
        let Some(window) = weak.upgrade() else { return };
        let theme = window.global::<Theme>();
        let flag = |on: bool| if on { "true" } else { "false" };
        let saved = library
            .set_setting(REDUCED_MOTION, motion_name(theme.get_motion_choice()))
            .and_then(|()| library.set_setting(ZOOM, &format!("{}", theme.get_zoom())))
            .and_then(|()| library.set_setting(QUEUE_OPEN, flag(window.get_queue_open())))
            .and_then(|()| library.set_setting(ARTISTS_ALL, flag(window.get_artists_show_all())));
        if let Err(error) = saved {
            log::warn!("could not save appearance preferences: {error}");
        }
    });
    Ok(())
}

/// Applies the sidebar's saved sections and puts the window on the first section the
/// sidebar shows, then saves the sections whenever a switch in Settings changes them.
/// Call it before anything records where the window is, such as the back and forward
/// history.
pub fn connect_sidebar(window: &AppWindow, library: Rc<Library>) -> vcmp_library::Result<()> {
    let sections = window.global::<SidebarSections>();
    if let Some(saved) = library.setting(SIDEBAR_HIDDEN)? {
        show_sections(&sections, parse_shown_sections(&saved));
    }
    window.set_section(start_page(shown_sections(&sections)));
    let weak = window.as_weak();
    sections.on_toggled(move || {
        let Some(window) = weak.upgrade() else { return };
        let hidden = format_hidden_sections(shown_sections(&window.global::<SidebarSections>()));
        if let Err(error) = library.set_setting(SIDEBAR_HIDDEN, &hidden) {
            log::warn!("could not save the sidebar's sections: {error}");
        }
    });
    Ok(())
}

/// The section the window opens on: the first library section shown, Search when
/// the sidebar shows none.
pub fn start_page(shown: [bool; 6]) -> Page {
    SECTIONS
        .iter()
        .zip(shown)
        .find(|(_, shown)| *shown)
        .map_or(Page::Search, |((_, page), _)| *page)
}

fn shown_sections(sections: &SidebarSections) -> [bool; 6] {
    [
        sections.get_albums(),
        sections.get_artists(),
        sections.get_playlists(),
        sections.get_labels(),
        sections.get_songs(),
        sections.get_folders(),
    ]
}

fn show_sections(sections: &SidebarSections, shown: [bool; 6]) {
    let [albums, artists, playlists, labels, songs, folders] = shown;
    sections.set_albums(albums);
    sections.set_artists(artists);
    sections.set_playlists(playlists);
    sections.set_labels(labels);
    sections.set_songs(songs);
    sections.set_folders(folders);
}

/// The sections shown, from the saved names of the hidden ones; a name not known is
/// left out.
fn parse_shown_sections(saved: &str) -> [bool; 6] {
    SECTIONS.map(|(name, _)| !saved.split_whitespace().any(|word| word == name))
}

fn format_hidden_sections(shown: [bool; 6]) -> String {
    SECTIONS
        .iter()
        .zip(shown)
        .filter(|(_, shown)| !*shown)
        .map(|((name, _), _)| *name)
        .collect::<Vec<_>>()
        .join(" ")
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

fn shown_columns(columns: &SongColumns) -> [bool; 8] {
    [
        columns.get_artist(),
        columns.get_album(),
        columns.get_labels(),
        columns.get_year(),
        columns.get_duration(),
        columns.get_plays(),
        columns.get_last_played(),
        columns.get_added(),
    ]
}

fn column_shown(columns: &SongColumns, column: SongColumn) -> bool {
    match column {
        SongColumn::Title => true,
        SongColumn::Artist => columns.get_artist(),
        SongColumn::Album => columns.get_album(),
        SongColumn::Year => columns.get_year(),
        SongColumn::Duration => columns.get_duration(),
        SongColumn::Plays => columns.get_plays(),
        SongColumn::LastPlayed => columns.get_last_played(),
        SongColumn::Added => columns.get_added(),
    }
}

fn show_columns(columns: &SongColumns, shown: [bool; 8]) {
    let [
        artist,
        album,
        labels,
        year,
        duration,
        plays,
        last_played,
        added,
    ] = shown;
    columns.set_artist(artist);
    columns.set_album(album);
    columns.set_labels(labels);
    columns.set_year(year);
    columns.set_duration(duration);
    columns.set_plays(plays);
    columns.set_last_played(last_played);
    columns.set_added(added);
}

/// The saved names of the shown columns; a name not known is left out.
fn parse_shown(saved: &str) -> [bool; 8] {
    HIDEABLE.map(|name| saved.split_whitespace().any(|word| word == name))
}

fn format_shown(shown: [bool; 8]) -> String {
    HIDEABLE
        .iter()
        .zip(shown)
        .filter(|(_, shown)| *shown)
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(" ")
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
    fn shown_song_columns_are_read_back() {
        let shown = [true, false, true, true, false, true, false, true];
        assert_eq!(
            format_shown(shown),
            "artist labels year plays added",
            "by name"
        );
        assert_eq!(parse_shown(&format_shown(shown)), shown);
        assert_eq!(parse_shown(""), [false; 8], "only the title");
        assert_eq!(
            parse_shown("album someday"),
            [false, true, false, false, false, false, false, false]
        );
    }

    #[test]
    fn hidden_sidebar_sections_are_read_back() {
        let shown = [false, true, true, false, true, false];
        assert_eq!(format_hidden_sections(shown), "albums labels folders");
        assert_eq!(parse_shown_sections(&format_hidden_sections(shown)), shown);
        assert_eq!(parse_shown_sections(""), [true; 6], "nothing hidden");
        assert_eq!(
            parse_shown_sections("songs someday"),
            [true, true, true, true, false, true],
            "an unknown name hides nothing"
        );
    }

    #[test]
    fn the_window_opens_on_the_first_section_shown() {
        assert_eq!(start_page([true; 6]), Page::Albums);
        assert_eq!(
            start_page([false, false, true, true, true, true]),
            Page::Playlists
        );
        assert_eq!(start_page([false; 6]), Page::Search);
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
