//! Feeds the album grid: one model row per album, with covers decoded on background
//! threads for the rows in view and released again when they scroll far away.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use vcmp_library::Album;

use crate::{AlbumTile, AppWindow, ArtworkState};

/// Owns the grid's model and its cover loading. Keep it alive as long as the window.
pub struct AlbumBrowser {
    window: slint::Weak<AppWindow>,
    model: Rc<VecModel<AlbumTile>>,
    albums: RefCell<Vec<Album>>,
    state: RefCell<CoverState>,
    loader: Loader,
}

#[derive(Default)]
struct CoverState {
    loaded: HashSet<usize>,
    queued: HashSet<usize>,
    first_visible: usize,
    visible_count: usize,
}

/// Counters for benchmarks and diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub struct CoverStats {
    pub decoded: usize,
    pub decode_time: Duration,
    pub in_memory: usize,
    pub queued: usize,
    /// Index of the first album in view.
    pub first_visible: usize,
}

impl AlbumBrowser {
    /// Shows `albums` in the window's album grid.
    pub fn connect(window: &AppWindow, albums: &[Album]) -> Rc<Self> {
        let tiles: Vec<AlbumTile> = albums.iter().map(tile).collect();
        let model = Rc::new(VecModel::from(tiles));
        window.set_albums(ModelRc::from(model.clone()));

        let weak = window.as_weak();
        let deliver = move |index: usize, id: i32, buffer: SharedPixelBuffer<Rgba8Pixel>| {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = weak.upgrade() {
                    show_cover(&window, index, id, buffer);
                }
            });
        };
        let browser = Rc::new(Self {
            window: window.as_weak(),
            model,
            albums: RefCell::new(albums.to_vec()),
            state: RefCell::default(),
            loader: Loader::start(Box::new(deliver)),
        });

        let weak_window = window.as_weak();
        let weak_browser = Rc::downgrade(&browser);
        window.on_album_range_changed(move |first, count| {
            if let (Some(window), Some(browser)) = (weak_window.upgrade(), weak_browser.upgrade()) {
                browser.show_range(&window, first.max(0) as usize, count.max(0) as usize);
            }
        });
        BROWSER.with(|current| *current.borrow_mut() = Rc::downgrade(&browser));
        browser
    }

    /// The album at a grid position.
    pub fn album(&self, index: usize) -> Option<Album> {
        self.albums.borrow().get(index).cloned()
    }

    pub fn album_count(&self) -> usize {
        self.albums.borrow().len()
    }

    /// Shows a new album list, for example after a scan. The grid keeps its scroll
    /// position; covers are loaded again for the rows in view.
    pub fn replace(&self, albums: &[Album]) {
        let tiles: Vec<AlbumTile> = albums.iter().map(tile).collect();
        *self.albums.borrow_mut() = albums.to_vec();
        let (first, count) = {
            let mut state = self.state.borrow_mut();
            state.loaded.clear();
            state.queued.clear();
            (state.first_visible, state.visible_count)
        };
        self.model.set_vec(tiles);
        if let Some(window) = self.window.upgrade() {
            self.show_range(&window, first, count);
        }
    }

    /// Queues covers around the rows in view, nearest first, and releases covers far
    /// away so memory stays bounded however long the list is.
    fn show_range(&self, window: &AppWindow, first: usize, count: usize) {
        let len = self.model.row_count();
        let size_px = (TILE_PIXELS as f32 * window.window().scale_factor()).ceil() as u32;
        let keep = first.saturating_sub(3 * count)..(first + 4 * count).min(len);
        let mut state = self.state.borrow_mut();
        state.first_visible = first;
        state.visible_count = count;

        let far: Vec<usize> = state
            .loaded
            .iter()
            .copied()
            .filter(|index| !keep.contains(index))
            .collect();
        for index in far {
            state.loaded.remove(&index);
            if let Some(mut row) = self.model.row_data(index) {
                row.cover = Image::default();
                row.artwork = ArtworkState::Loading;
                self.model.set_row_data(index, row);
            }
        }

        // Visible rows first, then a screen below, then a screen above.
        let visible = first..(first + count).min(len);
        let below = visible.end..(visible.end + count).min(len);
        let above = first.saturating_sub(count)..first;
        let mut jobs = Vec::new();
        for index in visible.chain(below).chain(above.rev()) {
            if state.loaded.contains(&index) {
                continue;
            }
            let path = self
                .albums
                .borrow()
                .get(index)
                .and_then(|album| album.artwork.as_ref().map(|art| art.thumbnail.clone()));
            if let (Some(path), Some(row)) = (path, self.model.row_data(index)) {
                jobs.push(Job {
                    index,
                    id: row.id,
                    path,
                    size_px,
                });
            }
        }
        state.queued = jobs.iter().map(|job| job.index).collect();
        self.loader.replace_queue(jobs);
    }

    fn mark_loaded(&self, index: usize) {
        let mut state = self.state.borrow_mut();
        state.queued.remove(&index);
        state.loaded.insert(index);
    }

    pub fn stats(&self) -> CoverStats {
        let state = self.state.borrow();
        CoverStats {
            decoded: self.loader.shared.decoded.load(Ordering::Relaxed),
            decode_time: Duration::from_micros(
                self.loader.shared.decode_micros.load(Ordering::Relaxed) as u64,
            ),
            in_memory: state.loaded.len(),
            queued: state.queued.len(),
            first_visible: state.first_visible,
        }
    }
}

thread_local! {
    /// The browser decoded covers are delivered to, on the UI thread.
    static BROWSER: RefCell<std::rc::Weak<AlbumBrowser>> = RefCell::default();
}

/// Logical size the grid shows covers at, at most; covers are decoded for this times
/// the window's scale factor.
pub const TILE_PIXELS: u32 = 240;

fn show_cover(window: &AppWindow, index: usize, id: i32, buffer: SharedPixelBuffer<Rgba8Pixel>) {
    let model = window.get_albums();
    let Some(mut row) = model.row_data(index) else {
        return;
    };
    if row.id != id {
        return;
    }
    let Some(browser) = BROWSER.with(|current| current.borrow().upgrade()) else {
        return;
    };
    if !browser.state.borrow().queued.contains(&index) {
        // Scrolled away before the cover arrived.
        return;
    }
    row.cover = Image::from_rgba8(buffer);
    row.artwork = ArtworkState::Loaded;
    model.set_row_data(index, row);
    browser.mark_loaded(index);
}

fn tile(album: &Album) -> AlbumTile {
    AlbumTile {
        id: album.id.0 as i32,
        title: album.title.as_str().into(),
        artist: album.artist.clone().unwrap_or_default().into(),
        initial: initial(&album.title).into(),
        artwork: if album.artwork.is_some() {
            ArtworkState::Loading
        } else {
            ArtworkState::None
        },
        cover: Image::default(),
    }
}

struct Job {
    index: usize,
    id: i32,
    path: PathBuf,
    size_px: u32,
}

type Deliver = Box<dyn Fn(usize, i32, SharedPixelBuffer<Rgba8Pixel>) + Send + Sync>;

struct Shared {
    queue: Mutex<VecDeque<Job>>,
    wake: Condvar,
    stop: AtomicBool,
    decoded: AtomicUsize,
    decode_micros: AtomicUsize,
    deliver: Deliver,
}

/// Decoding threads. Dropping the loader stops them.
struct Loader {
    shared: Arc<Shared>,
}

impl Loader {
    fn start(deliver: Deliver) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::default(),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            decoded: AtomicUsize::new(0),
            decode_micros: AtomicUsize::new(0),
            deliver,
        });
        let threads = thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
        for _ in 0..threads {
            let shared = shared.clone();
            thread::Builder::new()
                .name("vcmp-covers".into())
                .spawn(move || decode_loop(&shared))
                .expect("spawning a cover thread");
        }
        Self { shared }
    }

    /// Replaces whatever is still waiting: covers for rows scrolled past are dropped.
    fn replace_queue(&self, jobs: Vec<Job>) {
        *self.shared.queue.lock().expect("cover queue lock") = jobs.into();
        self.shared.wake.notify_all();
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}

fn decode_loop(shared: &Shared) {
    loop {
        let job = {
            let mut queue = shared.queue.lock().expect("cover queue lock");
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(job) = queue.pop_front() {
                    break job;
                }
                queue = shared.wake.wait(queue).expect("cover queue lock");
            }
        };
        let started = Instant::now();
        let Ok(image) = image::open(&job.path) else {
            continue;
        };
        // RGBA, not RGB: the GPU renderer converts RGB images on every frame, which
        // doubled the grid's render time in grid_benchmark (10.8 ms vs 5.1 ms).
        let rgba = image.thumbnail(job.size_px, job.size_px).into_rgba8();
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
        );
        shared.decoded.fetch_add(1, Ordering::Relaxed);
        shared
            .decode_micros
            .fetch_add(started.elapsed().as_micros() as usize, Ordering::Relaxed);
        (shared.deliver)(job.index, job.id, buffer);
    }
}

/// The first letter of a title, shown where there is no cover.
pub(crate) fn initial(title: &str) -> String {
    title
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
}
