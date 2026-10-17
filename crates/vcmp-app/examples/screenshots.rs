//! Renders the real window in every theme and page worth checking by eye, and saves
//! each frame as a PNG under `target/screenshots/`. The window appears on screen for a
//! few seconds while it runs. Window managers that tile windows ignore the requested
//! size; the smallest-size layout is covered by tests/ui_behaviour.rs instead.
//!
//!     cargo run -p vcmp-app --example screenshots

use std::cell::Cell;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString, Timer, TimerMode};
use vcmp_app::{AppWindow, Theme, ThemeMode};

struct Shot {
    name: &'static str,
    mode: ThemeMode,
    page: i32,
    // Tab presses before the frame, to show the keyboard focus ring. Focus carries
    // over between shots, so the count continues from the previous shot.
    tabs: usize,
}

const SHOTS: &[Shot] = &[
    Shot {
        name: "settings-light",
        mode: ThemeMode::Light,
        page: 0,
        tabs: 0,
    },
    Shot {
        name: "settings-dark",
        mode: ThemeMode::Dark,
        page: 0,
        tabs: 0,
    },
    Shot {
        name: "about-light",
        mode: ThemeMode::Light,
        page: 1,
        tabs: 0,
    },
    Shot {
        name: "about-dark",
        mode: ThemeMode::Dark,
        page: 1,
        tabs: 0,
    },
    Shot {
        name: "focus-nav-light",
        mode: ThemeMode::Light,
        page: 0,
        tabs: 1,
    },
    Shot {
        name: "focus-theme-dark",
        mode: ThemeMode::Dark,
        page: 0,
        tabs: 2,
    },
    Shot {
        name: "focus-switch-light",
        mode: ThemeMode::Light,
        page: 0,
        tabs: 1,
    },
];

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
