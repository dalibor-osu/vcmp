//! Renders the real window in every theme and page worth checking by eye, and saves
//! each frame as a PNG under `target/screenshots/`. The window appears on screen for a
//! few seconds while it runs. Window managers that tile windows ignore the requested
//! size; the smallest-size layout is covered by tests/ui_behaviour.rs instead.
//!
//!     cargo run -p vcmp-app --example screenshots
//!
//! With `VCMP_HOME` pointing at a library (for example `target/scan-example`), the
//! shots show that library and its music folders; point it at an empty directory to
//! see the empty states. `VCMP_SHOT_ONLY=review` takes only the shots whose names hold
//! that text.

use std::cell::Cell;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::platform::{Key, WindowEvent};
use slint::{
    ComponentHandle, Image, LogicalPosition, LogicalSize, Model, ModelRc, Rgba8Pixel,
    SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel,
};
use vcmp_app::library_service::{Delivery, LibraryPaths, LibraryService};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{CoverSearch, PlayerControl, PlayerService, Presence, PresenceSink};
use vcmp_app::tag_editor::TagEditor;
use vcmp_app::updates;
use vcmp_app::{
    AgentState, AlbumTile, AppWindow, ArtworkState, ChangeEntry, ChangeField, ChangeGroup,
    ChangeKind, ChangeProblem, ChangeRow, ChangeStatus, Confirmation, CoverSearch as CoverPicker,
    CoverSearchStatus, LastfmState, LastfmStatus, PhotoSearch, PhotoSearchStatus, PlayerState,
    ReduceMotion, ReviewStatus, SelectionChange, ServerStatus, SleepTimer, TagChange,
    TagEditorState, TagEditorStatus, TagField, Theme, ThemeMode, TrackSelection, UpdateState,
};
use vcmp_integrations::discord::Listening;
use vcmp_integrations::musicbrainz::AlbumQuery;
use vcmp_integrations::updates::Release;
use vcmp_library::Library;
use vcmp_playback::{Command, Event, NowPlaying, PlaybackState, QueueEntry, QueueView, TrackRef};

struct Shot {
    name: &'static str,
    mode: ThemeMode,
    // Sidebar position: 0 Search, 1 Albums, 2 Artists, 3 Playlists, 4 Songs, 5 Folders,
    // 6 Settings, 7 About.
    page: i32,
    // Grid position of the album (or, on the Artists and Playlists pages, the artist
    // or playlist) to open.
    album: Option<i32>,
    // Tab presses before the frame, to show the keyboard focus ring. Focus carries
    // over between shots, so the count continues from the previous shot.
    tabs: usize,
    // The now playing view is open over the page.
    now_playing: bool,
    // The Last.fm row in Settings, as in a build with an API key, in this state.
    lastfm: Option<LastfmStatus>,
    // A newer VCMP is reported, so its note shows.
    update: bool,
    // The cover picker is open, with stand-in covers.
    cover_picker: bool,
    // The tag editor is open for the album (a library is needed).
    tags: bool,
    // The editor saved, except for the first track, which the player has open.
    tags_waiting: bool,
    // The editor lists the changes of a save to every track of the album.
    tags_confirm: bool,
    // The artist photo picker is open, with stand-in photos.
    photo_picker: bool,
    // Two of the album's tracks are selected.
    selection: bool,
    // AI agents: Settings with the server on and a few changes, or a change in review.
    agents: Option<Agents>,
    // The sleep timer counts down in the player bar.
    sleep: bool,
    // The confirmation dialog asks whether to delete the agents' backups, over Settings.
    confirm: bool,
    // The pointer rests at the queue panel's right edge, where a scroll bar would show.
    queue_edge: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Agents {
    Settings,
    // Settings scrolled down to the changes and the backups.
    History,
    Review,
    ReviewDone,
    PlaylistDeletion,
}

const fn shot(
    name: &'static str,
    mode: ThemeMode,
    page: i32,
    album: Option<i32>,
    tabs: usize,
) -> Shot {
    Shot {
        name,
        mode,
        page,
        album,
        tabs,
        now_playing: false,
        lastfm: None,
        update: false,
        cover_picker: false,
        tags: false,
        tags_waiting: false,
        tags_confirm: false,
        photo_picker: false,
        selection: false,
        agents: None,
        sleep: false,
        confirm: false,
        queue_edge: false,
    }
}

/// Settings opens on its top; the AI agents section shows in the taller window these
/// shots ask for.
const fn agents(name: &'static str, mode: ThemeMode, agents: Agents) -> Shot {
    Shot {
        agents: Some(agents),
        ..shot(
            name,
            mode,
            if matches!(agents, Agents::Settings | Agents::History) {
                6
            } else {
                1
            },
            None,
            0,
        )
    }
}

const fn sleep(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        sleep: true,
        ..shot(name, mode, 1, None, 0)
    }
}

const fn selection(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        selection: true,
        ..shot(name, mode, 1, Some(ALBUM), 0)
    }
}

const fn photo_picker(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        photo_picker: true,
        ..shot(name, mode, 2, None, 0)
    }
}

const fn tags(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        tags: true,
        ..shot(name, mode, 1, Some(ALBUM), 0)
    }
}

const fn tags_waiting(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        tags_waiting: true,
        ..tags(name, mode)
    }
}

const fn tags_confirm(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        tags_confirm: true,
        ..tags(name, mode)
    }
}

const fn now_playing(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        now_playing: true,
        ..shot(name, mode, 1, None, 0)
    }
}

const fn lastfm(name: &'static str, mode: ThemeMode, status: LastfmStatus) -> Shot {
    Shot {
        lastfm: Some(status),
        ..shot(name, mode, 7, None, 0)
    }
}

/// The About page, with the note that a newer VCMP is out there and in the sidebar.
const fn update(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        update: true,
        ..shot(name, mode, 8, None, 0)
    }
}

const fn confirm(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        confirm: true,
        ..shot(name, mode, 7, None, 0)
    }
}

/// An album with the queue panel, the pointer at the panel's right edge.
const fn queue_edge(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        queue_edge: true,
        ..shot(name, mode, 1, Some(ALBUM), 0)
    }
}

const fn cover_picker(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        cover_picker: true,
        ..shot(name, mode, 1, None, 0)
    }
}

const SHOTS: &[Shot] = &[
    // First, from the start: Search, Albums, Artists, Playlists, Songs, Folders,
    // Settings, About, then the grid.
    shot("focus-grid-dark", ThemeMode::Dark, 1, None, 9),
    shot("albums-light", ThemeMode::Light, 1, None, 0),
    shot("albums-dark", ThemeMode::Dark, 1, None, 0),
    shot("album-light", ThemeMode::Light, 1, Some(ALBUM), 0),
    shot("album-dark", ThemeMode::Dark, 1, Some(ALBUM), 0),
    shot("artists-light", ThemeMode::Light, 2, None, 0),
    shot("artists-dark", ThemeMode::Dark, 2, None, 0),
    shot("artist-light", ThemeMode::Light, 2, Some(ARTIST), 0),
    shot("artist-dark", ThemeMode::Dark, 2, Some(ARTIST), 0),
    shot("search-light", ThemeMode::Light, 0, None, 0),
    shot("search-dark", ThemeMode::Dark, 0, None, 0),
    shot("playlists-light", ThemeMode::Light, 3, None, 0),
    shot("playlists-dark", ThemeMode::Dark, 3, None, 0),
    shot("playlist-light", ThemeMode::Light, 3, Some(0), 0),
    shot("playlist-dark", ThemeMode::Dark, 3, Some(0), 0),
    shot("labels-light", ThemeMode::Light, 4, None, 0),
    shot("labels-dark", ThemeMode::Dark, 4, None, 0),
    shot("songs-light", ThemeMode::Light, 5, None, 0),
    shot("songs-dark", ThemeMode::Dark, 5, None, 0),
    shot("folders-light", ThemeMode::Light, 6, None, 0),
    shot("folders-dark", ThemeMode::Dark, 6, None, 0),
    queue_edge("queue-edge-light", ThemeMode::Light),
    queue_edge("queue-edge-dark", ThemeMode::Dark),
    now_playing("now-playing-light", ThemeMode::Light),
    now_playing("now-playing-dark", ThemeMode::Dark),
    sleep("sleep-timer-light", ThemeMode::Light),
    sleep("sleep-timer-dark", ThemeMode::Dark),
    cover_picker("cover-picker-light", ThemeMode::Light),
    cover_picker("cover-picker-dark", ThemeMode::Dark),
    photo_picker("photo-picker-light", ThemeMode::Light),
    photo_picker("photo-picker-dark", ThemeMode::Dark),
    selection("selection-light", ThemeMode::Light),
    selection("selection-dark", ThemeMode::Dark),
    tags("tags-light", ThemeMode::Light),
    tags("tags-dark", ThemeMode::Dark),
    tags_waiting("tags-waiting-light", ThemeMode::Light),
    tags_waiting("tags-waiting-dark", ThemeMode::Dark),
    tags_confirm("tags-confirm-light", ThemeMode::Light),
    tags_confirm("tags-confirm-dark", ThemeMode::Dark),
    shot("settings-light", ThemeMode::Light, 7, None, 0),
    shot("settings-dark", ThemeMode::Dark, 7, None, 0),
    lastfm("lastfm-light", ThemeMode::Light, LastfmStatus::Connected),
    lastfm("lastfm-dark", ThemeMode::Dark, LastfmStatus::Connected),
    lastfm(
        "lastfm-waiting-light",
        ThemeMode::Light,
        LastfmStatus::Waiting,
    ),
    lastfm(
        "lastfm-waiting-dark",
        ThemeMode::Dark,
        LastfmStatus::Waiting,
    ),
    shot("about-light", ThemeMode::Light, 8, None, 0),
    shot("about-dark", ThemeMode::Dark, 8, None, 0),
    update("update-light", ThemeMode::Light),
    update("update-dark", ThemeMode::Dark),
    shot("focus-nav-light", ThemeMode::Light, 7, None, 1),
    shot("focus-theme-dark", ThemeMode::Dark, 7, None, 3),
    shot("focus-switch-light", ThemeMode::Light, 7, None, 1),
    agents("agents-light", ThemeMode::Light, Agents::Settings),
    agents("agents-dark", ThemeMode::Dark, Agents::Settings),
    agents("agents-history-light", ThemeMode::Light, Agents::History),
    agents("agents-history-dark", ThemeMode::Dark, Agents::History),
    agents("review-light", ThemeMode::Light, Agents::Review),
    agents("review-dark", ThemeMode::Dark, Agents::Review),
    agents("review-done-light", ThemeMode::Light, Agents::ReviewDone),
    agents(
        "review-playlist-dark",
        ThemeMode::Dark,
        Agents::PlaylistDeletion,
    ),
    confirm("confirm-dialog-light", ThemeMode::Light),
    confirm("confirm-dialog-dark", ThemeMode::Dark),
];

/// Grid position of the artist the artist shots open (set `VCMP_SHOT_ARTIST` to pick
/// another).
const ARTIST: i32 = 0;

/// Grid position of the album the album shots open (set `VCMP_SHOT_ALBUM` to pick
/// another).
const ALBUM: i32 = 0;

/// Takes playback commands and plays nothing.
struct Silent;

impl PlayerControl for Silent {
    fn send(&self, _: Command) {}
}

/// Shows nothing in Discord, so the shots never change the user's status.
impl PresenceSink for Silent {
    fn show(&self, _: Listening) {}
    fn clear(&self) {}
}

/// Looks nothing up, so the shots send nothing to MusicBrainz.
impl CoverSearch for Silent {
    fn find(&self, _: vcmp_library::AlbumId, _: AlbumQuery) {}
}

fn press(window: &AppWindow, key: Key) {
    let text: SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn prepare(window: &AppWindow, shot: &Shot) {
    window.global::<Confirmation>().invoke_answer(false);
    let theme = window.global::<Theme>();
    theme.set_mode(shot.mode);
    window.set_selected_index(shot.page);
    let player = window.global::<PlayerState>();
    player.set_now_playing_open(shot.now_playing);
    // Set directly, so no timer runs while the shots are taken.
    player.set_sleep_timer(if shot.sleep {
        SleepTimer::Timed
    } else {
        SleepTimer::Off
    });
    player.set_sleep_minutes(if shot.sleep { 29 } else { 0 });
    // Set directly: the shots connect no account and send nothing to Last.fm.
    let lastfm = window.global::<LastfmState>();
    lastfm.set_available(shot.lastfm.is_some());
    lastfm.set_status(shot.lastfm.unwrap_or(LastfmStatus::Disconnected));
    lastfm.set_user("listener".into());
    lastfm.set_unsent(3);
    // As in release builds, which have VCMP's fanart.tv key; the shots search nothing.
    window.global::<PhotoSearch>().set_available(true);
    if shot.now_playing {
        player.invoke_load_large_cover();
    }
    if shot.update {
        updates::show(
            window,
            &Release {
                version: "1.2.0".into(),
                url: "https://github.com/dalibor-osu/vcmp/releases".into(),
            },
        );
    } else {
        window.global::<UpdateState>().set_version("".into());
    }
    if shot.cover_picker {
        show_cover_picker(window);
    }
    window.global::<CoverPicker>().set_open(shot.cover_picker);
    if shot.photo_picker {
        show_photo_picker(window);
    }
    window.global::<PhotoSearch>().set_open(shot.photo_picker);
    if shot.page == 6 {
        // Inside the first music folder, where there is more than one row.
        window.invoke_folders_needed();
        window.invoke_folder_go_to(-1);
        window.invoke_open_folder(0);
    }
    if shot.page == 0 {
        // Set `VCMP_SHOT_SEARCH` to search for something else.
        let text = std::env::var("VCMP_SHOT_SEARCH").unwrap_or_else(|_| "ライブ".into());
        window.set_search_text(text.clone().into());
        window.invoke_search_edited(text.into());
    }
    match (shot.page, shot.album) {
        (2, Some(index)) => {
            let index = std::env::var("VCMP_SHOT_ARTIST")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(index);
            window.invoke_artist_activated(index);
        }
        (3, Some(index)) => window.invoke_playlist_activated(index),
        (_, Some(index)) => {
            let index = std::env::var("VCMP_SHOT_ALBUM")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(index);
            window.invoke_album_activated(index);
        }
        (_, None) => {
            window.set_album_open(false);
            window.set_artist_open(false);
            window.set_playlist_open(false);
        }
    }
    let tracks = window.get_album_tracks();
    let rows = window.global::<TrackSelection>();
    rows.invoke_change(tracks.clone(), 0, 0, SelectionChange::None);
    if shot.selection {
        // A block of two and one on its own.
        rows.invoke_change(tracks.clone(), 0, 0, SelectionChange::Toggle);
        rows.invoke_change(tracks.clone(), 1, 1, SelectionChange::Toggle);
        rows.invoke_change(tracks, 3, 3, SelectionChange::Toggle);
    }
    let editor = window.global::<TagEditorState>();
    if shot.tags {
        editor.invoke_edit_album();
        if shot.tags_waiting {
            let first = window.get_album_tracks().row_data(0).map(|row| row.path);
            let name = first
                .as_deref()
                .and_then(|path| Path::new(path).file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            editor.set_status(TagEditorStatus::Waiting);
            editor.set_waiting_file(name.into());
        }
        if shot.tags_confirm {
            // Set directly rather than saved, so the shots never write to the files.
            let change = |field, value: &str, removed| TagChange {
                field,
                value: value.into(),
                removed,
            };
            let mut changes = vec![
                change(TagField::AlbumArtist, "", true),
                change(TagField::Year, "1999", false),
                change(TagField::Genre, "Ambient", false),
            ];
            if editor.get_has_cover() {
                changes.push(change(TagField::Cover, "", false));
            }
            editor.set_confirm_files(editor.get_track_count());
            editor.set_changes(ModelRc::new(VecModel::from(changes)));
            editor.set_confirming(true);
        }
    } else {
        editor.invoke_cancel();
    }
    // The album shots show the queue panel too.
    window.set_queue_open(shot.album.is_some() && !shot.tags);
    show_agents(window, shot.agents);
    if shot.confirm {
        // The words Settings uses for the backups; Cancel answers before the next shot.
        window.global::<Confirmation>().invoke_ask(
            "Delete the backups of every finished change?".into(),
            "None of them can be undone afterwards. Changes waiting for review stay.".into(),
            "Delete backups".into(),
            "Delete backups".into(),
            "Keep the backups".into(),
        );
    }
    for _ in 0..shot.tabs {
        press(window, Key::Tab);
    }
}

/// Sets `AgentState` directly: the shots run no server and write nothing.
fn show_agents(window: &AppWindow, agents: Option<Agents>) {
    let state = window.global::<AgentState>();
    let size = match agents {
        Some(Agents::Settings) => LogicalSize::new(1100.0, 2600.0),
        _ => LogicalSize::new(1100.0, 720.0),
    };
    window.window().set_size(size);
    let Some(agents) = agents else {
        state.set_enabled(false);
        state.set_pending_count(0);
        state.set_history(ModelRc::default());
        state.set_history_open(false);
        state.set_review_open(false);
        return;
    };
    state.set_history_open(false);
    state.set_enabled(true);
    state.set_status(ServerStatus::Running);
    state.set_port("8267".into());
    state.set_address("http://127.0.0.1:8267/mcp".into());
    let token = "3f9c2a7e5b1d48f0a6c3e9b27d5f1a8c4e0b6d2f9a7c3e1b5d8f2a6c0e4b7d93";
    state.set_token(token.into());
    state.set_setup_command(vcmp_mcp::claude_code_command(8267, token).into());
    state
        .set_desktop_config("\"vcmp\": {\"args\":[\"mcp\"],\"command\":\"/opt/vcmp/vcmp\"}".into());
    state.set_backups_size("2.4 MB".into());
    state.set_has_finished(true);
    let entry = |id, kind, files, status, minutes_ago, can_undo| ChangeEntry {
        id,
        kind,
        files,
        playlist: if kind == ChangeKind::Playlist {
            "Road trip".into()
        } else {
            SharedString::default()
        },
        agent: "Claude Code".into(),
        minutes_ago,
        status,
        can_undo,
    };
    state.set_history(ModelRc::new(VecModel::from(vec![
        entry(4, ChangeKind::Tags, 12, ChangeStatus::Pending, 2, false),
        entry(
            3,
            ChangeKind::Playlist,
            0,
            ChangeStatus::Rejected,
            95,
            false,
        ),
        entry(
            2,
            ChangeKind::Tags,
            3,
            ChangeStatus::PartlyWritten,
            1600,
            true,
        ),
        entry(1, ChangeKind::Tags, 40, ChangeStatus::Undone, 5000, false),
    ])));
    state.set_pending_count(1);

    let row = |field, key: &str, before: &str, after: &str| ChangeRow {
        field,
        key: key.into(),
        before: before.into(),
        after: after.into(),
        has_before_cover: false,
        before_cover: Image::default(),
        has_after_cover: false,
        after_cover: Image::default(),
    };
    let cover = {
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(64, 64);
        pixels.make_mut_slice().fill(Rgba8Pixel {
            r: 30,
            g: 102,
            b: 245,
            a: 255,
        });
        Image::from_rgba8(pixels)
    };
    let group = |title: &str, file: &str, rows: Vec<ChangeRow>| ChangeGroup {
        title: title.into(),
        file: file.into(),
        rows: ModelRc::new(VecModel::from(rows)),
    };
    state.set_review_agent("Claude Code".into());
    state.set_review_playing(SharedString::default());
    state.set_review_failed(false);
    match agents {
        Agents::Settings => state.set_review_open(false),
        Agents::History => {
            state.set_review_open(false);
            state.set_history_open(true);
        }
        Agents::Review => {
            state.set_review_kind(ChangeKind::Tags);
            state.set_review_files(12);
            state.set_review_shared(ModelRc::new(VecModel::from(vec![
                row(ChangeField::AlbumArtist, "AlbumArtist", "", "Pink Floyd"),
                row(ChangeField::Year, "RecordingDate", "1979-11-30", "1979"),
                row(
                    ChangeField::Genre,
                    "Genre",
                    "Rock; Progressive",
                    "Progressive rock",
                ),
                ChangeRow {
                    has_after_cover: true,
                    after_cover: cover,
                    ..row(ChangeField::Cover, "", "", "")
                },
            ])));
            state.set_review_groups(ModelRc::new(VecModel::from(vec![
                group(
                    "In the Flesh?",
                    "01 - In the Flesh.flac",
                    vec![
                        row(
                            ChangeField::Title,
                            "TrackTitle",
                            "In the Flesh",
                            "In the Flesh?",
                        ),
                        row(ChangeField::Other, "Composer", "", "Roger Waters"),
                    ],
                ),
                group(
                    "The Thin Ice",
                    "02 - The Thin Ice.flac",
                    vec![
                        row(ChangeField::Other, "Composer", "", "Roger Waters"),
                        row(
                            ChangeField::Other,
                            "id3:COMM:iTunNORM",
                            " 00000A5C 00000B21",
                            "",
                        ),
                    ],
                ),
            ])));
            state.set_review_status(ReviewStatus::Waiting);
            state.set_review_open(true);
        }
        Agents::ReviewDone => {
            state.set_review_kind(ChangeKind::Tags);
            state.set_review_files(12);
            state.set_review_status(ReviewStatus::Done);
            state.set_review_written(10);
            state.set_review_problems(ModelRc::new(VecModel::from(vec![
                ChangeProblem {
                    file: "04 - Mother.flac".into(),
                    conflict: true,
                },
                ChangeProblem {
                    file: "07 - Young Lust.flac".into(),
                    conflict: false,
                },
            ])));
            state.set_review_open(true);
        }
        Agents::PlaylistDeletion => {
            state.set_review_kind(ChangeKind::Playlist);
            state.set_review_playlist("Road trip".into());
            state.set_review_files(0);
            state.set_review_shared(ModelRc::default());
            state.set_review_groups(ModelRc::default());
            state.set_review_status(ReviewStatus::Waiting);
            state.set_review_open(true);
        }
    }
}

/// Fills the cover picker with plain squares for covers, so the shots download
/// nothing.
fn show_cover_picker(window: &AppWindow) {
    let tile = |details: &str, [r, g, b]: [u8; 3]| {
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(240, 240);
        pixels.make_mut_slice().fill(Rgba8Pixel { r, g, b, a: 255 });
        AlbumTile {
            title: "Album title".into(),
            artist: details.into(),
            initial: "A".into(),
            artwork: ArtworkState::Loaded,
            cover: Image::from_rgba8(pixels),
            ..AlbumTile::default()
        }
    };
    let picker = window.global::<CoverPicker>();
    picker.set_album_title("Album title".into());
    picker.set_album_artist("Artist".into());
    picker.set_status(CoverSearchStatus::Found);
    picker.set_covers(ModelRc::new(VecModel::from(vec![
        tile("2019 · JP", [137, 180, 250]),
        tile("2020", [166, 227, 161]),
        tile("2019 · GB", [250, 179, 135]),
        tile("", [203, 166, 247]),
    ])));
}

/// Fills the photo picker with plain squares for photos, so the shots download
/// nothing.
fn show_photo_picker(window: &AppWindow) {
    let tile = |details: &str, [r, g, b]: [u8; 3]| {
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(240, 240);
        pixels.make_mut_slice().fill(Rgba8Pixel { r, g, b, a: 255 });
        AlbumTile {
            title: "Artist".into(),
            artist: details.into(),
            initial: "A".into(),
            artwork: ArtworkState::Loaded,
            cover: Image::from_rgba8(pixels),
            ..AlbumTile::default()
        }
    };
    let picker = window.global::<PhotoSearch>();
    picker.set_artist_name("Artist".into());
    picker.set_status(PhotoSearchStatus::Found);
    picker.set_photos(ModelRc::new(VecModel::from(vec![
        tile("UK rock band", [137, 180, 250]),
        tile("UK rock band", [166, 227, 161]),
        tile("Japanese singer", [250, 179, 135]),
    ])));
}

/// Puts the pointer over the first card found, so the search shots show its hover
/// fill. The results arrive after the pause before searching, so this runs just
/// before the frame is taken.
fn hover_first_card(window: &AppWindow) {
    let card = ElementHandle::find_by_element_id(window, "SearchPage::scroll")
        .next()
        .and_then(|results| {
            results
                .query_descendants()
                .match_accessible_role(AccessibleRole::List)
                .find_first()
        })
        .and_then(|grid| {
            grid.query_descendants()
                .match_accessible_role(AccessibleRole::Button)
                .find_first()
        });
    if let Some(card) = card {
        let (position, size) = (card.absolute_position(), card.size());
        window.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(
                position.x + size.width / 2.0,
                position.y + size.height / 2.0,
            ),
        });
    }
}

fn hover_queue_edge(window: &AppWindow) {
    if let Some(panel) = ElementHandle::find_by_element_type_name(window, "QueuePanel").next() {
        let (position, size) = (panel.absolute_position(), panel.size());
        window.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(
                position.x + size.width - 4.0,
                position.y + size.height / 2.0,
            ),
        });
    }
}

fn save(window: &AppWindow, path: &Path) {
    let buffer = window.window().take_snapshot().expect("snapshot");
    let file = BufWriter::new(File::create(path).expect("create png"));
    let mut encoder = png::Encoder::new(file, buffer.width(), buffer.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .expect("png header")
        .write_image_data(buffer.as_bytes())
        .expect("png data");
}

fn main() -> Result<(), slint::PlatformError> {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&out_dir).expect("output directory");

    let window = vcmp_app::create_window()?;
    // The same wiring as the app: the albums, plus music folders and scanning (an
    // automatic-update library rescans at startup, as the app would). Playback goes
    // to a silent stand-in, and the bar shows the shot album's first track as playing.
    let _library = std::env::var_os("VCMP_HOME").map(|home| {
        let home = PathBuf::from(home);
        let paths = LibraryPaths {
            database: home.join("library.sqlite3"),
            cache: home.join("cache"),
        };
        let library = Rc::new(Library::open(&paths.database, &paths.cache).expect("library"));
        let view = LibraryView::connect(&window, library.clone()).expect("albums");
        let player = PlayerService::connect(
            &window,
            library.clone(),
            view.clone(),
            Box::new(Silent),
            Some(Presence {
                sink: Box::new(Silent),
                covers: Box::new(Silent),
            }),
        );
        let album_index = std::env::var("VCMP_SHOT_ALBUM")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        // Made-up devices, so the shots never list the real ones.
        player.set_device_lister(|| {
            Ok(vec![
                "Built-in Audio Analog Stereo".into(),
                "USB DAC".into(),
            ])
        });
        if let Some(album) = view.browser().album(album_index)
            && let tracks = library.album_tracks(album.id).expect("tracks")
            && let Some(track) = tracks.first()
        {
            let entry = |track: &vcmp_library::Track| {
                QueueEntry::new(TrackRef::local(track.path.to_string_lossy().as_ref()))
            };
            player.handle_event(Event::QueueChanged(QueueView {
                // A few in Next in queue, so the panel holds more than it shows.
                up_next: tracks.iter().rev().take(4).map(entry).collect(),
                upcoming: tracks.iter().skip(1).map(entry).collect(),
                origin: Some(format!("album:{}", album.id.0)),
                ..QueueView::default()
            }));
            player.handle_event(Event::TrackChanged(Some(NowPlaying {
                entry: QueueEntry::new(TrackRef::local(track.path.to_string_lossy().as_ref())),
                duration: Some(track.duration),
            })));
            player.handle_event(Event::StateChanged(PlaybackState::Playing));
            player.handle_event(Event::Position(track.duration / 3));
        }
        let editor = TagEditor::connect(
            &window,
            library.clone(),
            paths.clone(),
            view.clone(),
            Delivery::EventLoop,
        );
        (
            LibraryService::connect(&window, library, paths, view, Delivery::EventLoop),
            player,
            editor,
        )
    });
    // Frames are taken without waiting for transitions to finish.
    window.global::<Theme>().set_motion_choice(ReduceMotion::On);

    let only = std::env::var("VCMP_SHOT_ONLY").unwrap_or_default();
    let shots: Vec<&Shot> = SHOTS
        .iter()
        .filter(|shot| shot.name.contains(only.as_str()))
        .collect();
    let step = Rc::new(Cell::new(0usize));
    let timer = Timer::default();
    let weak = window.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(400), move || {
        let Some(window) = weak.upgrade() else { return };
        let index = step.get();
        step.set(index + 1);
        // Even steps set the scene, odd steps capture it after it has been drawn.
        let shot_index = index / 2;
        match shots.get(shot_index).copied() {
            Some(shot) if index.is_multiple_of(2) => prepare(&window, shot),
            Some(shot) => {
                if shot.page == 0 {
                    hover_first_card(&window);
                }
                if shot.queue_edge {
                    hover_queue_edge(&window);
                }
                let path = out_dir.join(format!("{}.png", shot.name));
                save(&window, &path);
                println!("{}", path.display());
                window.window().dispatch_event(WindowEvent::PointerExited);
            }
            None => {
                let _ = slint::quit_event_loop();
            }
        }
    });
    window.run()
}
