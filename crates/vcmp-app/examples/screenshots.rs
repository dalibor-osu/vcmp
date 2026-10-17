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
use slint::{ComponentHandle, SharedString, Timer, TimerMode};
use vcmp_app::library_service::{Delivery, LibraryPaths, LibraryService};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{CoverSearch, PlayerControl, PlayerService, Presence, PresenceSink};
use vcmp_app::{AppWindow, LastfmState, LastfmStatus, PlayerState, Theme, ThemeMode};
use vcmp_integrations::discord::Listening;
use vcmp_integrations::musicbrainz::AlbumQuery;
use vcmp_library::Library;
use vcmp_playback::{Command, Event, NowPlaying, PlaybackState, QueueEntry, QueueView, TrackRef};

struct Shot {
    name: &'static str,
    mode: ThemeMode,
    // Sidebar position: 0 Search, 1 Albums, 2 Artists, 3 Songs, 4 Folders, 5 Settings,
    // 6 About.
    page: i32,
    // Grid position of the album (or, on the Artists page, the artist) to open.
    album: Option<i32>,
    // Tab presses before the frame, to show the keyboard focus ring. Focus carries
    // over between shots, so the count continues from the previous shot.
    tabs: usize,
    // The now playing view is open over the page.
    now_playing: bool,
    // The Last.fm row in Settings, as in a build with an API key, in this state.
    lastfm: Option<LastfmStatus>,
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
        ..shot(name, mode, 5, None, 0)
    }
}

const SHOTS: &[Shot] = &[
    // First, from the start: Search, Albums, Artists, Songs, Folders, Settings,
    // About, then the grid.
    shot("focus-grid-dark", ThemeMode::Dark, 1, None, 8),
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
    shot("songs-light", ThemeMode::Light, 3, None, 0),
    shot("songs-dark", ThemeMode::Dark, 3, None, 0),
    shot("folders-light", ThemeMode::Light, 4, None, 0),
    shot("folders-dark", ThemeMode::Dark, 4, None, 0),
    now_playing("now-playing-light", ThemeMode::Light),
    now_playing("now-playing-dark", ThemeMode::Dark),
    shot("settings-light", ThemeMode::Light, 5, None, 0),
    shot("settings-dark", ThemeMode::Dark, 5, None, 0),
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
    shot("about-light", ThemeMode::Light, 6, None, 0),
    shot("about-dark", ThemeMode::Dark, 6, None, 0),
    shot("focus-nav-light", ThemeMode::Light, 5, None, 1),
    shot("focus-theme-dark", ThemeMode::Dark, 5, None, 3),
    shot("focus-switch-light", ThemeMode::Light, 5, None, 1),
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
    if shot.page == 4 {
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
        }
    }
    // The album shots show the queue panel too.
    window.set_queue_open(shot.album.is_some());
    for _ in 0..shot.tabs {
        press(window, Key::Tab);
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
        (
            LibraryService::connect(&window, library, paths, view, Delivery::EventLoop),
            player,
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
