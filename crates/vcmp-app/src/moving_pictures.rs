//! Animated pictures where the picture is the subject: the album, artist and playlist
//! pages and the now playing view. Everywhere else (grids, lists, the player bar)
//! shows the still. Frames are decoded on a background thread only while the picture
//! is in sight with motion allowed, kept while it stays so, and let go after.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use vcmp_library::animation;

use crate::player_service::LARGE_COVER_PIXELS;
use crate::{AppWindow, MovingPictures, PictureFrame, PictureSlot, PlayerState, Theme};

/// What a moving picture belongs to. The album pages on Albums and on Artists share
/// one album.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    Album,
    Artist,
    Playlist,
    NowPlaying,
}

impl Picture {
    fn of(slot: PictureSlot) -> Self {
        match slot {
            PictureSlot::Album | PictureSlot::ArtistAlbum => Picture::Album,
            PictureSlot::Artist => Picture::Artist,
            PictureSlot::Playlist => Picture::Playlist,
            PictureSlot::NowPlaying => Picture::NowPlaying,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// Pixels the frames are decoded at: what the picture shows at, at most.
    fn size_px(self, window: &AppWindow) -> u32 {
        match self {
            Picture::NowPlaying => LARGE_COVER_PIXELS,
            _ => {
                let logical = window.global::<Theme>().get_detail_cover_max();
                (logical * window.window().scale_factor()).ceil() as u32
            }
        }
    }

    /// Hands the frames (or none) to the picture.
    fn apply(self, window: &AppWindow, frames: ModelRc<PictureFrame>) {
        match self {
            Picture::Album => {
                let mut detail = window.get_album_detail();
                detail.frames = frames;
                window.set_album_detail(detail);
            }
            Picture::Artist => {
                let mut detail = window.get_artist_detail();
                detail.frames = frames;
                window.set_artist_detail(detail);
            }
            Picture::Playlist => {
                let mut detail = window.get_playlist_detail();
                detail.frames = frames;
                window.set_playlist_detail(detail);
            }
            Picture::NowPlaying => window
                .global::<PlayerState>()
                .set_large_cover_frames(frames),
        }
    }
}

/// Counts what each picture asked for, so frames decoded for a picture since replaced
/// or let go are dropped.
static GENERATIONS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

#[derive(Default)]
struct Shown {
    /// The file that may hold an animation.
    path: Option<PathBuf>,
    /// Its frames, once decoded.
    frames: Option<ModelRc<PictureFrame>>,
    loading: bool,
}

#[derive(Default)]
struct State {
    shown: [Shown; 4],
    /// Which slots are in sight with motion allowed, by `PictureSlot`.
    wanted: [bool; 5],
}

impl State {
    fn wanted(&self, picture: Picture) -> bool {
        slots()
            .iter()
            .zip(self.wanted)
            .any(|(&slot, wanted)| wanted && Picture::of(slot) == picture)
    }
}

fn slots() -> [PictureSlot; 5] {
    [
        PictureSlot::Album,
        PictureSlot::ArtistAlbum,
        PictureSlot::Artist,
        PictureSlot::Playlist,
        PictureSlot::NowPlaying,
    ]
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// Listens for pictures coming into sight and leaving it.
pub fn install(window: &AppWindow) {
    let weak = window.as_weak();
    window
        .global::<MovingPictures>()
        .on_wanted(move |slot, wanted| {
            if let Some(window) = weak.upgrade() {
                set_wanted(&window, slot, wanted);
            }
        });
}

/// Whether the picture's frames are wanted: it is in sight, and motion isn't reduced.
pub fn is_wanted(picture: Picture) -> bool {
    STATE.with(|state| state.borrow().wanted(picture))
}

/// The file behind a picture changed (another album opened, a new photo chosen), or
/// its page was filled again: `path` is the file that may hold an animation, `None`
/// for a picture that can't move. Call it after setting the page's details.
pub fn show(window: &AppWindow, picture: Picture, path: Option<PathBuf>) {
    let kept = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let shown = &mut state.shown[picture.index()];
        if shown.path.is_some() && shown.path == path {
            // The same file: its frames again, or still on their way.
            return shown.frames.clone();
        }
        GENERATIONS[picture.index()].fetch_add(1, Ordering::Relaxed);
        *shown = Shown {
            path,
            ..Shown::default()
        };
        None
    });
    match kept {
        Some(frames) => picture.apply(window, frames),
        None => {
            picture.apply(window, ModelRc::default());
            load_if_wanted(window, picture);
        }
    }
}

fn set_wanted(window: &AppWindow, slot: PictureSlot, wanted: bool) {
    let picture = Picture::of(slot);
    let still_wanted = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if let Some(index) = slots().iter().position(|&each| each == slot) {
            state.wanted[index] = wanted;
        }
        let still_wanted = state.wanted(picture);
        if !still_wanted {
            let shown = &mut state.shown[picture.index()];
            GENERATIONS[picture.index()].fetch_add(1, Ordering::Relaxed);
            shown.frames = None;
            shown.loading = false;
        }
        still_wanted
    });
    if still_wanted {
        load_if_wanted(window, picture);
        return;
    }
    // The call comes while the picture is set up or its bindings change, so the
    // frames are let go of afterwards, unless the picture was wanted again by then.
    let generation = GENERATIONS[picture.index()].load(Ordering::Relaxed);
    let weak = window.as_weak();
    slint::Timer::single_shot(Duration::ZERO, move || {
        if GENERATIONS[picture.index()].load(Ordering::Relaxed) == generation
            && let Some(window) = weak.upgrade()
        {
            picture.apply(&window, ModelRc::default());
        }
    });
}

/// Decodes the picture's frames in the background, when it is wanted and they aren't
/// there or on their way.
fn load_if_wanted(window: &AppWindow, picture: Picture) {
    let path = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let wanted = state.wanted(picture);
        let shown = &mut state.shown[picture.index()];
        if !wanted || shown.frames.is_some() || shown.loading {
            return None;
        }
        let path = shown.path.clone()?;
        shown.loading = true;
        Some(path)
    });
    let Some(path) = path else {
        return;
    };
    let generation = GENERATIONS[picture.index()].load(Ordering::Relaxed);
    let size_px = picture.size_px(window);
    let weak = window.as_weak();
    thread::Builder::new()
        .name("vcmp-animation".into())
        .spawn(move || {
            let current =
                move || GENERATIONS[picture.index()].load(Ordering::Relaxed) == generation;
            let frames = fs::read(&path)
                .ok()
                .filter(|_| current())
                .and_then(|data| animation::decode_frames(&data, size_px))
                .map(|frames| {
                    frames
                        .into_iter()
                        .map(|frame| {
                            let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                                frame.image.as_raw(),
                                frame.image.width(),
                                frame.image.height(),
                            );
                            (buffer, frame.delay)
                        })
                        .collect::<Vec<_>>()
                });
            let _ = slint::invoke_from_event_loop(move || {
                if !current() {
                    return;
                }
                let model = frames.map(frames_model);
                STATE.with(|state| {
                    let shown = &mut state.borrow_mut().shown[picture.index()];
                    shown.loading = false;
                    // A still picture keeps an empty model, so it isn't decoded again.
                    shown.frames = Some(model.clone().unwrap_or_default());
                });
                if let (Some(window), Some(model)) = (weak.upgrade(), model) {
                    picture.apply(&window, model);
                }
            });
        })
        .expect("spawning the animation thread");
}

fn frames_model(frames: Vec<(SharedPixelBuffer<Rgba8Pixel>, Duration)>) -> ModelRc<PictureFrame> {
    let frames: Vec<PictureFrame> = frames
        .into_iter()
        .map(|(buffer, delay)| PictureFrame {
            image: Image::from_rgba8(buffer),
            delay: delay.as_millis() as i64,
        })
        .collect();
    ModelRc::new(VecModel::from(frames))
}
