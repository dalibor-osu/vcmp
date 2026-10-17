//! Scrolls the album grid of the real window through thousands of synthetic albums
//! and reports render times, cover loading and memory. Thumbnails are generated once
//! into `target/grid-bench/`. Linux only (memory comes from /proc/self/status).
//!
//!     cargo run --release -p vcmp-app --example grid_benchmark -- [albums] [pixels per step]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, RenderingState, Timer, TimerMode};
use vcmp_app::albums::AlbumBrowser;
use vcmp_library::{Album, AlbumId, Artwork};

fn rss_mib() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kib: f64 = status
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse()
                .ok()
        })
        .unwrap_or(0.0);
    kib / 1024.0
}

/// A 400 px JPEG with texture, so it decodes like a real cover rather than a flat fill.
fn make_thumbnail(path: &Path, seed: u32) {
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    let image = image::RgbImage::from_fn(400, 400, |x, y| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let noise = (state & 0x3f) as u8;
        image::Rgb([
            ((x + seed * 7) % 256) as u8 / 2 + noise,
            ((y + seed * 13) % 256) as u8 / 2 + noise,
            ((x + y + seed) % 256) as u8 / 2 + noise,
        ])
    });
    image.save(path).expect("write thumbnail");
}

fn synthetic_albums(count: usize) -> Vec<Album> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/grid-bench/thumbs");
    std::fs::create_dir_all(&dir).unwrap();
    let paths: Vec<PathBuf> = (0..count).map(|i| dir.join(format!("{i}.jpg"))).collect();
    let missing: Vec<(usize, &PathBuf)> = paths
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.exists())
        .collect();
    if !missing.is_empty() {
        eprintln!("generating {} thumbnails...", missing.len());
        std::thread::scope(|scope| {
            for chunk in missing.chunks(missing.len().div_ceil(8)) {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .for_each(|(i, path)| make_thumbnail(path, *i as u32))
                });
            }
        });
    }
    paths
        .into_iter()
        .enumerate()
        .map(|(i, path)| Album {
            id: AlbumId(i as i64 + 1),
            title: format!("Album number {i}"),
            artist: Some(format!("Artist {}", i / 8)),
            artist_id: None,
            year: None,
            track_count: 10,
            artwork: Some(Artwork {
                original: path.clone(),
                thumbnail: path,
            }),
        })
        .collect()
}

fn percentiles(name: &str, mut values: Vec<f64>) {
    if values.is_empty() {
        println!("{name}: no samples");
        return;
    }
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q) as usize];
    println!(
        "{name:<26} n={:<5} p50={:>6.2}  p95={:>6.2}  p99={:>6.2}  max={:>6.2} ms",
        values.len(),
        at(0.5),
        at(0.95),
        at(0.99),
        at(1.0)
    );
}

struct Frames {
    started: Option<Instant>,
    render_ms: Vec<f64>,
    finished: Vec<Instant>,
    recording: bool,
}

fn main() -> Result<(), slint::PlatformError> {
    let count: usize = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(5000);
    let step: f32 = std::env::args()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .unwrap_or(120.0);
    let albums = synthetic_albums(count);
    let rss_start = rss_mib();

    let window = vcmp_app::create_window()?;
    let browser = AlbumBrowser::connect(&window, &albums);

    let frames = Rc::new(RefCell::new(Frames {
        started: None,
        render_ms: Vec::new(),
        finished: Vec::new(),
        recording: false,
    }));
    let recorder = frames.clone();
    window
        .window()
        .set_rendering_notifier(move |state, _| {
            let mut frames = recorder.borrow_mut();
            match state {
                RenderingState::BeforeRendering => frames.started = Some(Instant::now()),
                RenderingState::AfterRendering if frames.recording => {
                    if let Some(started) = frames.started {
                        frames
                            .render_ms
                            .push(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    frames.finished.push(Instant::now());
                }
                _ => {}
            }
        })
        .expect("rendering notifier");

    let launched = Instant::now();
    let mut covers_ready: Option<Duration> = None;
    let mut rss_loaded = 0.0;
    let mut scroll_started: Option<Instant> = None;
    let mut scroll_finished: Option<Instant> = None;
    let mut decoded_before_scroll = 0;
    let weak = window.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let Some(window) = weak.upgrade() else { return };
        let stats = browser.stats();
        let elapsed = launched.elapsed();
        if scroll_started.is_none() {
            if covers_ready.is_none() && stats.queued == 0 && stats.in_memory > 0 {
                covers_ready = Some(elapsed);
            }
            if elapsed > Duration::from_millis(1500) {
                rss_loaded = rss_mib();
                decoded_before_scroll = stats.decoded;
                scroll_started = Some(Instant::now());
                frames.borrow_mut().recording = true;
            }
            return;
        }
        if scroll_finished.is_none() {
            let size = window.window().size().to_logical(window.window().scale_factor());
            let position = LogicalPosition::new(size.width * 0.6, size.height * 0.5);
            window.window().dispatch_event(WindowEvent::PointerMoved { position });
            window.window().dispatch_event(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: -step });
            let at_end = stats.first_visible + 60 >= count;
            if at_end || scroll_started.unwrap().elapsed() > Duration::from_secs(90) {
                scroll_finished = Some(Instant::now());
                frames.borrow_mut().recording = false;
            }
            return;
        }
        if scroll_finished.unwrap().elapsed() < Duration::from_secs(1) {
            return;
        }

        let frames = frames.borrow();
        let size = window.window().size();
        println!("albums {count}, window {}x{} px, scale {:.2}, step {step} px per 16 ms", size.width, size.height, window.window().scale_factor());
        println!("covers on screen after {:?} (decoded {decoded_before_scroll})", covers_ready);
        let scroll = scroll_finished.unwrap() - scroll_started.unwrap();
        println!("scrolled to album {} in {:.1} s, {} frames", stats.first_visible, scroll.as_secs_f64(), frames.render_ms.len());
        percentiles("render time", frames.render_ms.clone());
        let intervals = frames.finished.windows(2).map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0).collect::<Vec<_>>();
        let slow = intervals.iter().filter(|ms| **ms > 34.0).count();
        percentiles("frame interval", intervals);
        println!("frame intervals over 34 ms (two refreshes missed): {slow}");
        println!(
            "covers decoded {} (mean {:.2} ms), {} in memory at the end",
            stats.decoded,
            stats.decode_time.as_secs_f64() * 1000.0 / stats.decoded.max(1) as f64,
            stats.in_memory
        );
        println!("memory: {rss_start:.0} MiB before window, {rss_loaded:.0} MiB after first covers, {:.0} MiB after scrolling", rss_mib());
        let _ = slint::quit_event_loop();
    });
    window.run()
}
