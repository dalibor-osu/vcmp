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
use vcmp_app::player_service::{PlayerControl, PlayerService, PresenceSink};
use vcmp_app::{AppWindow, Theme, ThemeMode};
use vcmp_integrations::discord::Listening;
use vcmp_library::Library;
use vcmp_playback::{Command, Event, NowPlaying, PlaybackState, QueueEntry, TrackRef};

struct Shot {
    name: &'static str,
    mode: ThemeMode,
    // Sidebar position: 0 Albums, 1 Settings, 2 About.
    page: i32,
    // Grid position of the album to open, if any.
    album: Option<i32>,
    // Tab presses before the frame, to show the keyboard focus ring. Focus carries
    // over between shots, so the count continues from the previous shot.
    tabs: usize,
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
    }
}

const SHOTS: &[Shot] = &[
    // First, while nothing has focus: Albums, Settings, About, then the grid.
    shot("focus-grid-dark", ThemeMode::Dark, 0, None, 4),
    shot("albums-light", ThemeMode::Light, 0, None, 0),
    shot("albums-dark", ThemeMode::Dark, 0, None, 0),
    shot("album-light", ThemeMode::Light, 0, Some(ALBUM), 0),
    shot("album-dark", ThemeMode::Dark, 0, Some(ALBUM), 0),
    shot("settings-light", ThemeMode::Light, 1, None, 0),
    shot("settings-dark", ThemeMode::Dark, 1, None, 0),
    shot("about-light", ThemeMode::Light, 2, None, 0),
    shot("about-dark", ThemeMode::Dark, 2, None, 0),
    shot("focus-nav-light", ThemeMode::Light, 1, None, 1),
    shot("focus-theme-dark", ThemeMode::Dark, 1, None, 3),
    shot("focus-switch-light", ThemeMode::Light, 1, None, 1),
];

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
    match shot.album {
        Some(index) => {
            let index = std::env::var("VCMP_SHOT_ALBUM")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(index);
            window.invoke_album_activated(index);
        }
        None => window.set_album_open(false),
    }
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
            Some(Box::new(Silent)),
        );
        let album_index = std::env::var("VCMP_SHOT_ALBUM")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        if let Some(album) = view.browser().album(album_index)
            && let Some(track) = library
                .album_tracks(album.id)
                .expect("tracks")
                .into_iter()
                .next()
        {
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
