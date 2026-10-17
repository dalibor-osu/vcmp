//! Feeds the card grids (albums, artists, an artist's albums): one model row per
//! card, with pictures decoded on background threads for the cards in view and
//! released again when they scroll far away.

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

/// Which of the window's card grids a `CoverGrid` feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grid {
    /// The Albums page.
    Albums,
    /// The Artists page.
    Artists,
    /// The albums on an artist's page.
    ArtistAlbums,
}

impl Grid {
    fn model(self, window: &AppWindow) -> ModelRc<AlbumTile> {
        match self {
            Grid::Albums => window.get_albums(),
            Grid::Artists => window.get_artists(),
            Grid::ArtistAlbums => window.get_artist_albums(),
        }
    }

    fn set_model(self, window: &AppWindow, model: ModelRc<AlbumTile>) {
        match self {
            Grid::Albums => window.set_albums(model),
            Grid::Artists => window.set_artists(model),
            Grid::ArtistAlbums => window.set_artist_albums(model),
        }
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// A card and the thumbnail its picture comes from, if it has one.
pub type Card = (AlbumTile, Option<PathBuf>);

/// One card grid's model, with pictures decoded on background threads for the cards
/// in view and released again when they scroll far away. Keep it alive as long as
/// the window.
pub struct CoverGrid {
    window: slint::Weak<AppWindow>,
    model: Rc<VecModel<AlbumTile>>,
    thumbnails: RefCell<Vec<Option<PathBuf>>>,
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
    /// Index of the first card in view.
    pub first_visible: usize,
}

impl CoverGrid {
    /// Shows `cards` in `grid`. `threads` decode pictures; the big album grid wants
    /// more than the others.
    pub fn connect(window: &AppWindow, grid: Grid, cards: Vec<Card>, threads: usize) -> Rc<Self> {
        let (tiles, thumbnails) = split(cards);
        let model = Rc::new(VecModel::from(tiles));
        grid.set_model(window, ModelRc::from(model.clone()));

        let weak = window.as_weak();
        let deliver = move |index: usize, id: i32, buffer: SharedPixelBuffer<Rgba8Pixel>| {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = weak.upgrade() {
                    show_cover(&window, grid, index, id, buffer);
                }
            });
        };
        let cover_grid = Rc::new(Self {
            window: window.as_weak(),
            model,
            thumbnails: RefCell::new(thumbnails),
            state: RefCell::default(),
            loader: Loader::start(Box::new(deliver), threads),
        });
        GRIDS.with(|grids| grids.borrow_mut()[grid.slot()] = Rc::downgrade(&cover_grid));
        cover_grid
    }

    /// Shows new cards, for example after a scan. The grid keeps its scroll position;
    /// pictures are loaded again for the cards in view.
    pub fn replace(&self, cards: Vec<Card>) {
        let (tiles, thumbnails) = split(cards);
        *self.thumbnails.borrow_mut() = thumbnails;
        let (first, count) = {
            let mut state = self.state.borrow_mut();
            state.loaded.clear();
            state.queued.clear();
            (state.first_visible, state.visible_count)
        };
        self.model.set_vec(tiles);
        self.show_range(first, count);
    }

    /// Queues pictures around the cards in view, nearest first, and releases those
    /// far away so memory stays bounded however long the list is.
    pub fn show_range(&self, first: usize, count: usize) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
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

        // Visible cards first, then a screen below, then a screen above.
        let visible = first..(first + count).min(len);
        let below = visible.end..(visible.end + count).min(len);
        let above = first.saturating_sub(count)..first;
        let mut jobs = Vec::new();
        for index in visible.chain(below).chain(above.rev()) {
            if state.loaded.contains(&index) {
                continue;
            }
            let path = self.thumbnails.borrow().get(index).cloned().flatten();
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

/// Tiles and thumbnails, with each tile waiting for its picture if it has one.
fn split(cards: Vec<Card>) -> (Vec<AlbumTile>, Vec<Option<PathBuf>>) {
    cards
        .into_iter()
        .map(|(mut tile, thumbnail)| {
            tile.artwork = if thumbnail.is_some() {
                ArtworkState::Loading
            } else {
                ArtworkState::None
            };
            (tile, thumbnail)
        })
        .unzip()
}

/// Feeds the album grid: one card per album. Keep it alive as long as the window.
pub struct AlbumBrowser {
    grid: Rc<CoverGrid>,
    albums: RefCell<Vec<Album>>,
}

impl AlbumBrowser {
    /// Shows `albums` in the window's album grid.
    pub fn connect(window: &AppWindow, albums: &[Album]) -> Rc<Self> {
        let threads = thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4));
        let grid = CoverGrid::connect(window, Grid::Albums, cards(albums), threads);
        let weak = Rc::downgrade(&grid);
        window.on_album_range_changed(move |first, count| {
            if let Some(grid) = weak.upgrade() {
                grid.show_range(first.max(0) as usize, count.max(0) as usize);
            }
        });
        Rc::new(Self {
            grid,
            albums: RefCell::new(albums.to_vec()),
        })
    }

    /// The album at a grid position.
    pub fn album(&self, index: usize) -> Option<Album> {
        self.albums.borrow().get(index).cloned()
    }

    pub fn album_count(&self) -> usize {
        self.albums.borrow().len()
    }

    /// Shows a new album list, for example after a scan.
    pub fn replace(&self, albums: &[Album]) {
        *self.albums.borrow_mut() = albums.to_vec();
        self.grid.replace(cards(albums));
    }

    pub fn stats(&self) -> CoverStats {
        self.grid.stats()
    }
}

thread_local! {
    /// The grids decoded pictures are delivered to, on the UI thread.
    static GRIDS: RefCell<[std::rc::Weak<CoverGrid>; 3]> = RefCell::default();
}

/// Logical size the grids show pictures at, at most; they are decoded for this times
/// the window's scale factor.
pub const TILE_PIXELS: u32 = 240;

fn show_cover(
    window: &AppWindow,
    grid: Grid,
    index: usize,
    id: i32,
    buffer: SharedPixelBuffer<Rgba8Pixel>,
) {
    let model = grid.model(window);
    let Some(mut row) = model.row_data(index) else {
        return;
    };
    if row.id != id {
        return;
    }
    let Some(cover_grid) = GRIDS.with(|grids| grids.borrow()[grid.slot()].upgrade()) else {
        return;
    };
    if !cover_grid.state.borrow().queued.contains(&index) {
        // Scrolled away before the picture arrived.
        return;
    }
    row.cover = Image::from_rgba8(buffer);
    row.artwork = ArtworkState::Loaded;
    model.set_row_data(index, row);
    cover_grid.mark_loaded(index);
}

/// One card per album, with its cover thumbnail.
pub fn cards(albums: &[Album]) -> Vec<Card> {
    albums
        .iter()
        .map(|album| {
            let tile = AlbumTile {
                id: album.id.0 as i32,
                title: album.title.as_str().into(),
                artist: album.artist.clone().unwrap_or_default().into(),
                initial: initial(&album.title).into(),
                ..AlbumTile::default()
            };
            (
                tile,
                album.artwork.as_ref().map(|art| art.thumbnail.clone()),
            )
        })
        .collect()
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
    fn start(deliver: Deliver, threads: usize) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::default(),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            decoded: AtomicUsize::new(0),
            decode_micros: AtomicUsize::new(0),
            deliver,
        });
        for _ in 0..threads.max(1) {
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

/// The first letter or digit of a title, shown where there is no picture: "(G)I-DLE"
/// shows "G". A title without any shows its first character.
pub(crate) fn initial(title: &str) -> String {
    title
        .chars()
        .find(|c| c.is_alphanumeric())
        .or_else(|| title.chars().next())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_skip_leading_punctuation() {
        assert_eq!(initial("(G)I-DLE"), "G");
        assert_eq!(initial("765 MILLION ALLSTARS"), "7");
        assert_eq!(initial("μ's"), "Μ");
        assert_eq!(initial("??????"), "?");
        assert_eq!(initial(""), "");
    }
}
