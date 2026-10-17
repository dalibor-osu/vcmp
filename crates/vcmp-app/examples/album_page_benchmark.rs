//! Opens the album with the most tracks in a real library and scrolls its page down
//! and back up, reporting how long the page took to open and the render times.
//!
//!     VCMP_HOME=target/scan-example cargo run --release -p vcmp-app --example album_page_benchmark

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, RenderingState, Timer, TimerMode};
use vcmp_app::library_view::LibraryView;
use vcmp_library::Library;

fn percentiles(name: &str, mut values: Vec<f64>) {
    if values.is_empty() {
        println!("{name}: no samples");
        return;
    }
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q) as usize];
    println!(
        "{name:<16} n={:<5} p50={:>7.2}  p95={:>7.2}  max={:>7.2} ms",
        values.len(),
        at(0.5),
        at(0.95),
        at(1.0)
    );
}

#[derive(Default)]
struct Frames {
    started: Option<Instant>,
    render_ms: Vec<f64>,
    finished: Vec<Instant>,
}

fn main() -> Result<(), slint::PlatformError> {
    let home = PathBuf::from(std::env::var_os("VCMP_HOME").expect("set VCMP_HOME to a library"));
    let library = Rc::new(
        Library::open(&home.join("library.sqlite3"), &home.join("cache")).expect("library"),
    );
    let window = vcmp_app::create_window()?;
    let view = LibraryView::connect(&window, library).expect("albums");
    let (index, album) = (0..)
        .map_while(|index| view.browser().album(index).map(|album| (index, album)))
        .max_by_key(|(_, album)| album.track_count)
        .expect("an album");
    println!("album {:?}, {} tracks", album.title, album.track_count);

    let frames = Rc::new(RefCell::new(Frames::default()));
    let recorder = frames.clone();
    window
        .window()
        .set_rendering_notifier(move |state, _| {
            let mut frames = recorder.borrow_mut();
            match state {
                RenderingState::BeforeRendering => frames.started = Some(Instant::now()),
                RenderingState::AfterRendering => {
                    if let Some(started) = frames.started {
                        let ms = started.elapsed().as_secs_f64() * 1000.0;
                        frames.render_ms.push(ms);
                    }
                    frames.finished.push(Instant::now());
                }
                _ => {}
            }
        })
        .expect("rendering notifier");

    let weak = window.as_weak();
    let timer = Timer::default();
    let mut tick = 0u32;
    let mut opened: Option<Instant> = None;
    let mut open_frames = 0;
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let Some(window) = weak.upgrade() else { return };
        tick += 1;
        match tick {
            // Let the grid settle first.
            1..=60 => {}
            61 => {
                frames.borrow_mut().render_ms.clear();
                frames.borrow_mut().finished.clear();
                opened = Some(Instant::now());
                window.invoke_album_activated(index as i32);
            }
            62..=120 => {
                if open_frames == 0 && !frames.borrow().finished.is_empty() {
                    let first = frames.borrow().finished[0];
                    println!(
                        "page open: first frame {:.1} ms after the click (render {:.1} ms)",
                        (first - opened.unwrap()).as_secs_f64() * 1000.0,
                        frames.borrow().render_ms[0]
                    );
                    open_frames = 1;
                }
            }
            121 => {
                frames.borrow_mut().render_ms.clear();
                frames.borrow_mut().finished.clear();
            }
            122..=361 => {
                let size = window
                    .window()
                    .size()
                    .to_logical(window.window().scale_factor());
                let position = LogicalPosition::new(size.width * 0.6, size.height * 0.5);
                let delta_y = if tick < 242 { -60.0 } else { 60.0 };
                window
                    .window()
                    .dispatch_event(WindowEvent::PointerMoved { position });
                window
                    .window()
                    .dispatch_event(WindowEvent::PointerScrolled {
                        position,
                        delta_x: 0.0,
                        delta_y,
                    });
            }
            362 => {
                let frames = frames.borrow();
                percentiles("render time", frames.render_ms.clone());
                let intervals = frames
                    .finished
                    .windows(2)
                    .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
                    .collect();
                percentiles("frame interval", intervals);
                let _ = slint::quit_event_loop();
            }
            _ => {}
        }
    });
    window.run()
}
