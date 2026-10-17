//! Renders the real window in every theme and page worth checking by eye, and saves
//! each frame as a PNG under `target/screenshots/`. The window appears on screen for a
//! few seconds while it runs. Window managers that tile windows ignore the requested
//! size; the smallest-size layout is covered by tests/ui_behaviour.rs instead.
//!
//!     cargo run -p vcmp-app --example screenshots
//!
//! With `VCMP_HOME` pointing at a library (for example `target/scan-example`), the
//! shots show that library and its music folders; point it at an empty directory to
//! see the empty states.

use std::cell::Cell;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use slint::platform::{Key, WindowEvent};
use slint::{
    ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer,
    TimerMode, VecModel,
};
use vcmp_app::library_service::{Delivery, LibraryPaths, LibraryService};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{CoverSearch, PlayerControl, PlayerService, Presence, PresenceSink};
use vcmp_app::tag_editor::TagEditor;
use vcmp_app::updates;
use vcmp_app::{
    AlbumTile, AppWindow, ArtworkState, CoverSearch as CoverPicker, CoverSearchStatus, LastfmState,
    LastfmStatus, PlayerState, TagEditorState, TagEditorStatus, Theme, ThemeMode, UpdateState,
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

const fn now_playing(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        now_playing: true,
        ..shot(name, mode, 1, None, 0)
    }
}

const fn lastfm(name: &'static str, mode: ThemeMode, status: LastfmStatus) -> Shot {
    Shot {
        lastfm: Some(status),
        ..shot(name, mode, 6, None, 0)
    }
}

/// The About page, with the note that a newer VCMP is out there and in the sidebar.
const fn update(name: &'static str, mode: ThemeMode) -> Shot {
    Shot {
        update: true,
        ..shot(name, mode, 7, None, 0)
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
    shot("songs-light", ThemeMode::Light, 4, None, 0),
    shot("songs-dark", ThemeMode::Dark, 4, None, 0),
    shot("folders-light", ThemeMode::Light, 5, None, 0),
    shot("folders-dark", ThemeMode::Dark, 5, None, 0),
    now_playing("now-playing-light", ThemeMode::Light),
    now_playing("now-playing-dark", ThemeMode::Dark),
    cover_picker("cover-picker-light", ThemeMode::Light),
    cover_picker("cover-picker-dark", ThemeMode::Dark),
    tags("tags-light", ThemeMode::Light),
    tags("tags-dark", ThemeMode::Dark),
    tags_waiting("tags-waiting-light", ThemeMode::Light),
    tags_waiting("tags-waiting-dark", ThemeMode::Dark),
    shot("settings-light", ThemeMode::Light, 6, None, 0),
    shot("settings-dark", ThemeMode::Dark, 6, None, 0),
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
    shot("about-light", ThemeMode::Light, 7, None, 0),
    shot("about-dark", ThemeMode::Dark, 7, None, 0),
    update("update-light", ThemeMode::Light),
    update("update-dark", ThemeMode::Dark),
    shot("focus-nav-light", ThemeMode::Light, 6, None, 1),
    shot("focus-theme-dark", ThemeMode::Dark, 6, None, 3),
    shot("focus-switch-light", ThemeMode::Light, 6, None, 1),
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
    let theme = window.global::<Theme>();
    theme.set_mode(shot.mode);
    window.set_selected_index(shot.page);
    let player = window.global::<PlayerState>();
    player.set_now_playing_open(shot.now_playing);
    // Set directly: the shots connect no account and send nothing to Last.fm.
    let lastfm = window.global::<LastfmState>();
    lastfm.set_available(shot.lastfm.is_some());
    lastfm.set_status(shot.lastfm.unwrap_or(LastfmStatus::Disconnected));
    lastfm.set_user("listener".into());
    lastfm.set_unsent(3);
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
    if shot.page == 5 {
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
    } else {
        editor.invoke_cancel();
    }
    // The album shots show the queue panel too.
    window.set_queue_open(shot.album.is_some() && !shot.tags);
    for _ in 0..shot.tabs {
        press(window, Key::Tab);
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
                up_next: tracks.iter().rev().take(1).map(entry).collect(),
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
    window.global::<Theme>().set_reduced_motion(true);

    let step = Rc::new(Cell::new(0usize));
    let timer = Timer::default();
    let weak = window.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(400), move || {
        let Some(window) = weak.upgrade() else { return };
        let index = step.get();
        step.set(index + 1);
        // Even steps set the scene, odd steps capture it after it has been drawn.
        let shot_index = index / 2;
        match SHOTS.get(shot_index) {
            Some(shot) if index.is_multiple_of(2) => prepare(&window, shot),
            Some(shot) => {
                let path = out_dir.join(format!("{}.png", shot.name));
                save(&window, &path);
                println!("{}", path.display());
            }
            None => {
                let _ = slint::quit_event_loop();
            }
        }
    });
    window.run()
}
