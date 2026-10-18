//! The tag editor behind `TagEditorState`: one track from any track menu, or every
//! track of the album page's album. It shows the values the files have (empty where
//! an album's tracks differ), and writes only the fields the user changed, through
//! `Library::write_tags` on a connection of its own in the background. Afterwards
//! the library views and the player show the new tags.
//!
//! On Windows the file playback has open can't be replaced, so its edit waits until
//! the engine lets go of the file (`Event::Decoding`) while the rest of the batch is
//! written at once. The editor says so and can be closed meanwhile; opened again on
//! that track, it shows the waiting tags and builds on them.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use slint::{
    ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel,
};
use vcmp_library::{
    FailedTagWrite, FieldEdit, InvalidTagEdit, Library, TagEdit, TagWriteError, TagWriteReport,
    Track, TrackId,
};

use crate::library_service::{Delivery, LibraryPaths};
use crate::library_view::LibraryView;
use crate::selection;
use crate::{
    AppWindow, SongSelection, TagEditorState, TagEditorStatus, TagFailure, TagFailureReason,
    TagMixed, TagProblem, TagValues, TrackSelection,
};

/// Longest side of the cover preview, in pixels.
const PREVIEW_PIXELS: u32 = 320;
/// Image files the cover chooser offers; the tags hold JPEG and PNG only.
const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

/// Whether a file that playback has open can't be replaced, so edits to it wait until
/// playback lets go of it. Windows refuses to replace an open file. Elsewhere the
/// replace goes through, and playback reads on from the old file until it moves on.
const OPEN_FILES_LOCKED: bool = cfg!(windows);

enum Message {
    Progress(usize),
    Finished(TagWriteReport),
    /// Edits that waited for playback, written once it let go of their files.
    Released(Vec<TrackEdit>, TagWriteReport),
}

/// One track's edit.
#[derive(Clone)]
struct TrackEdit {
    track: Track,
    edit: TagEdit,
}

/// Told when tags were written, so other parts of the window can show them.
type Written = Box<dyn Fn()>;
/// Told with the file when tags that waited for playback could not be written after
/// all, and the editor no longer shows that wait.
type NotWritten = Box<dyn Fn(&Path)>;

pub struct TagEditor {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    paths: LibraryPaths,
    view: Rc<LibraryView>,
    // The tracks the editor writes to, and the cover it writes.
    tracks: RefCell<Vec<Track>>,
    cover: RefCell<FieldEdit<Vec<u8>>>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    notify: Arc<dyn Fn() + Send + Sync>,
    writing: Cell<bool>,
    on_written: RefCell<Option<Written>>,
    // Edits the open editor builds on, by track: what waited for playback when it
    // opened, or what failed once playback let go.
    carried: RefCell<HashMap<TrackId, TagEdit>>,
    // The file playback has open, and whether such a file can't be replaced.
    playback_file: RefCell<Option<PathBuf>>,
    open_files_locked: Cell<bool>,
    // Edits waiting for playback to let go of their file, the tracks of those being
    // written now, and the track whose wait the open editor shows after a save.
    waiting: RefCell<Vec<TrackEdit>>,
    releasing: RefCell<HashSet<TrackId>>,
    shown_waiting: Cell<Option<TrackId>>,
    on_not_written: RefCell<Option<NotWritten>>,
}

thread_local! {
    /// The editor messages are delivered to, on the UI thread.
    static EDITOR: RefCell<std::rc::Weak<TagEditor>> = RefCell::default();
}

impl TagEditor {
    /// Connects `TagEditorState` to the library. Writes open their own connection
    /// from `paths`. Keep the editor alive as long as the window.
    pub fn connect(
        window: &AppWindow,
        library: Rc<Library>,
        paths: LibraryPaths,
        view: Rc<LibraryView>,
        delivery: Delivery,
    ) -> Rc<Self> {
        let (sender, receiver) = mpsc::channel();
        let notify: Arc<dyn Fn() + Send + Sync> = match delivery {
            Delivery::EventLoop => Arc::new(|| {
                let _ = slint::invoke_from_event_loop(|| {
                    if let Some(editor) = EDITOR.with(|current| current.borrow().upgrade()) {
                        editor.drain();
                    }
                });
            }),
            Delivery::Manual => Arc::new(|| {}),
        };
        let editor = Rc::new(Self {
            window: window.as_weak(),
            library,
            paths,
            view,
            tracks: RefCell::default(),
            cover: RefCell::default(),
            sender,
            receiver,
            notify,
            writing: Cell::new(false),
            on_written: RefCell::default(),
            carried: RefCell::default(),
            playback_file: RefCell::default(),
            open_files_locked: Cell::new(OPEN_FILES_LOCKED),
            waiting: RefCell::default(),
            releasing: RefCell::default(),
            shown_waiting: Cell::default(),
            on_not_written: RefCell::default(),
        });
        EDITOR.with(|current| *current.borrow_mut() = Rc::downgrade(&editor));

        let state = window.global::<TagEditorState>();
        let weak = Rc::downgrade(&editor);
        state.on_edit_track(move |path| {
            if let Some(editor) = weak.upgrade() {
                editor.edit_track(Path::new(path.as_str()));
            }
        });
        let weak = Rc::downgrade(&editor);
        window
            .global::<TrackSelection>()
            .on_edit_tags(move |rows, row| {
                if let Some(editor) = weak.upgrade() {
                    editor.edit_paths(&selection::chosen_paths(&rows, row));
                }
            });
        let weak = Rc::downgrade(&editor);
        window
            .global::<SongSelection>()
            .on_edit_tags(move |rows, row| {
                if let Some(editor) = weak.upgrade() {
                    editor.edit_paths(&selection::chosen_paths(&rows, row));
                }
            });
        let weak = Rc::downgrade(&editor);
        state.on_edit_album(move || {
            if let Some(editor) = weak.upgrade() {
                editor.edit_album();
            }
        });
        let weak = Rc::downgrade(&editor);
        state.on_choose_cover(move || {
            if let Some(editor) = weak.upgrade() {
                editor.choose_cover();
            }
        });
        let weak = Rc::downgrade(&editor);
        state.on_remove_cover(move || {
            if let Some(editor) = weak.upgrade() {
                editor.set_cover(FieldEdit::Clear);
            }
        });
        let weak = Rc::downgrade(&editor);
        state.on_save(move |values| {
            if let Some(editor) = weak.upgrade() {
                editor.save(&values);
            }
        });
        let weak = Rc::downgrade(&editor);
        state.on_cancel(move || {
            if let Some(editor) = weak.upgrade() {
                editor.close();
            }
        });
        editor
    }

    /// Calls `written` after tags were written to at least one file.
    pub fn on_written(&self, written: impl Fn() + 'static) {
        *self.on_written.borrow_mut() = Some(Box::new(written));
    }

    /// Calls `not_written` with the file when tags that waited for playback could not
    /// be written once it let go, and the editor no longer shows them.
    pub fn on_not_written(&self, not_written: impl Fn(&Path) + 'static) {
        *self.on_not_written.borrow_mut() = Some(Box::new(not_written));
    }

    /// Takes the file playback has open now (`Event::Decoding`), and writes the edits
    /// that waited for a file it let go of.
    pub fn set_playback_file(&self, path: Option<&Path>) {
        *self.playback_file.borrow_mut() = path.map(Path::to_path_buf);
        self.write_released();
    }

    /// Whether a file playback has open can't be replaced, so its edits wait: true on
    /// Windows. Tests turn it on elsewhere.
    pub fn set_open_files_locked(&self, locked: bool) {
        self.open_files_locked.set(locked);
        self.write_released();
    }

    /// Writes the edits still waiting for playback, on this thread. For when VCMP
    /// closes: once the engine has stopped, no file is held open.
    pub fn write_waiting(&self) {
        let waiting = std::mem::take(&mut *self.waiting.borrow_mut());
        if waiting.is_empty() {
            return;
        }
        let report = write_files(&self.paths, &waiting, |_| {});
        log_failures(&report);
    }

    /// Opens the editor for the library's track with this file.
    pub fn edit_track(&self, path: &Path) {
        let track = self
            .library
            .track_id_for_path(path)
            .ok()
            .flatten()
            .and_then(|id| self.library.track(id).ok().flatten());
        match track {
            Some(track) => self.open(vec![track], false),
            None => log::warn!("not in the library, so not editable: {}", path.display()),
        }
    }

    /// Opens the editor for the library's tracks with these files, in this order.
    pub fn edit_paths(&self, paths: &[String]) {
        let tracks: Vec<Track> = paths
            .iter()
            .filter_map(|path| {
                let track = self
                    .library
                    .track_id_for_path(Path::new(path))
                    .ok()
                    .flatten()
                    .and_then(|id| self.library.track(id).ok().flatten());
                if track.is_none() {
                    log::warn!("not in the library, so not editable: {path}");
                }
                track
            })
            .collect();
        if !tracks.is_empty() {
            self.open(tracks, false);
        }
    }

    /// Opens the editor for every track of the album the album page shows.
    pub fn edit_album(&self) {
        let Some(album) = self.view.shown_album() else {
            return;
        };
        match self.library.album_tracks(album.id) {
            Ok(tracks) if !tracks.is_empty() => self.open(tracks, true),
            Ok(_) => {}
            Err(error) => log::error!("could not read the tracks of {:?}: {error}", album.title),
        }
    }

    /// Applies what the writing thread sent until it is done, or `timeout` passes.
    /// For callers without an event loop (`Delivery::Manual`).
    pub fn wait_until_written(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.writing.get() || !self.releasing.borrow().is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining) {
                Ok(message) => self.apply(message),
                Err(RecvTimeoutError::Timeout) => return false,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        true
    }

    /// Opens the editor for `tracks`: the whole album page's album, or a selection.
    fn open(&self, tracks: Vec<Track>, whole_album: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<TagEditorState>();
        // Edits still waiting for playback show as written, and saving builds on them.
        let carried: HashMap<TrackId, TagEdit> = self
            .waiting
            .borrow()
            .iter()
            .filter(|waiting| tracks.iter().any(|track| track.id == waiting.track.id))
            .map(|waiting| (waiting.track.id, waiting.edit.clone()))
            .collect();
        let shown: Vec<Track> = tracks
            .iter()
            .map(|track| match carried.get(&track.id) {
                Some(edit) => edited(track, edit),
                None => track.clone(),
            })
            .collect();
        let first = &shown[0];
        let single = tracks.len() == 1;

        // The value every track shares, or None (shown empty) where they differ.
        fn common<T: PartialEq + Clone>(tracks: &[Track], get: impl Fn(&Track) -> T) -> Option<T> {
            let value = get(&tracks[0]);
            tracks
                .iter()
                .all(|track| get(track) == value)
                .then_some(value)
        }
        let text = |value: Option<Option<String>>| value.flatten().unwrap_or_default();
        let number =
            |value: Option<Option<u32>>| value.flatten().map(|n| n.to_string()).unwrap_or_default();
        let artist = common(&shown, |track| track.artist.clone());
        let album = common(&shown, |track| track.album.clone());
        let album_artist = common(&shown, |track| track.album_artist.clone());
        let disc = common(&shown, |track| track.disc_number);
        let year = common(&shown, |track| track.year);
        let genre = common(&shown, |track| track.genre.clone());
        let covers: Vec<Option<Vec<u8>>> = tracks
            .iter()
            .map(
                |track| match carried.get(&track.id).map(|edit| &edit.cover) {
                    Some(FieldEdit::Set(data)) => Some(data.clone()),
                    Some(FieldEdit::Clear) => None,
                    _ => self.library.embedded_cover(track.id).ok().flatten(),
                },
            )
            .collect();
        let cover_mixed = covers.iter().any(|cover| cover != &covers[0]);

        state.set_track_count(tracks.len() as i32);
        // Tracks of several albums have no one album to name.
        state.set_subject(
            if single {
                first.title.clone()
            } else {
                album.clone().flatten().unwrap_or_default()
            }
            .into(),
        );
        state.set_whole_album(whole_album);
        state.set_subject_artist(
            if single {
                first.artist.clone()
            } else {
                first.album_artist.clone().or(first.artist.clone())
            }
            .unwrap_or_default()
            .into(),
        );
        state.set_values(TagValues {
            title: if single {
                first.title.as_str().into()
            } else {
                Default::default()
            },
            artist: text(artist.clone()).into(),
            album: text(album.clone()).into(),
            album_artist: text(album_artist.clone()).into(),
            track: if single {
                number(Some(first.track_number))
            } else {
                String::new()
            }
            .into(),
            disc: number(disc).into(),
            year: year
                .flatten()
                .map(|year| year.to_string())
                .unwrap_or_default()
                .into(),
            genre: text(genre.clone()).into(),
        });
        state.set_mixed(TagMixed {
            artist: artist.is_none(),
            album: album.is_none(),
            album_artist: album_artist.is_none(),
            disc: disc.is_none(),
            year: year.is_none(),
            genre: genre.is_none(),
            cover: cover_mixed,
        });
        let preview = if cover_mixed {
            None
        } else {
            covers[0].as_deref().and_then(preview)
        };
        state.set_has_cover(preview.is_some());
        state.set_cover(preview.unwrap_or_default());
        state.set_status(TagEditorStatus::Editing);
        state.set_problem(TagProblem::None);
        state.set_failures(ModelRc::default());
        state.set_done(0);
        *self.tracks.borrow_mut() = tracks;
        *self.carried.borrow_mut() = carried;
        self.shown_waiting.set(None);
        *self.cover.borrow_mut() = FieldEdit::Keep;
        self.show_waiting_note(&state);
        state.set_open(true);
    }

    fn close(&self) {
        if self.writing.get() {
            return;
        }
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<TagEditorState>();
            state.set_open(false);
            state.set_cover(Image::default());
            state.set_failures(ModelRc::default());
            state.set_waiting_file(SharedString::default());
        }
        self.tracks.borrow_mut().clear();
        self.carried.borrow_mut().clear();
        self.shown_waiting.set(None);
        *self.cover.borrow_mut() = FieldEdit::Keep;
    }

    fn choose_cover(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let picked = slint::spawn_local(async move {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("JPEG or PNG", COVER_EXTENSIONS)
                .pick_file()
                .await
            else {
                return;
            };
            let data = file.read().await;
            if let Some(editor) = weak.upgrade() {
                editor.use_cover(data);
            }
        });
        if let Err(error) = picked {
            log::error!("could not open the image chooser: {error}");
        }
    }

    /// Makes `data` the cover to write, when it is a JPEG or PNG image.
    pub fn use_cover(&self, data: Vec<u8>) {
        let readable = matches!(
            image::guess_format(&data),
            Ok(image::ImageFormat::Jpeg | image::ImageFormat::Png)
        ) && preview(&data).is_some();
        if readable {
            self.set_cover(FieldEdit::Set(data));
        } else if let Some(window) = self.window.upgrade() {
            window
                .global::<TagEditorState>()
                .set_problem(TagProblem::Cover);
        }
    }

    fn set_cover(&self, cover: FieldEdit<Vec<u8>>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<TagEditorState>();
        let shown = match &cover {
            FieldEdit::Set(data) => preview(data),
            _ => None,
        };
        state.set_has_cover(shown.is_some());
        state.set_cover(shown.unwrap_or_default());
        let mut mixed = state.get_mixed();
        mixed.cover = false;
        state.set_mixed(mixed);
        if state.get_problem() == TagProblem::Cover {
            state.set_problem(TagProblem::None);
        }
        *self.cover.borrow_mut() = cover;
    }

    fn save(&self, values: &TagValues) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<TagEditorState>();
        if self.writing.get() || state.get_status() == TagEditorStatus::Waiting {
            return;
        }
        let edit = match edit(&state.get_values(), values, self.cover.borrow().clone()) {
            Ok(edit) => edit,
            Err(problem) => {
                state.set_problem(problem);
                return;
            }
        };
        state.set_problem(TagProblem::None);
        // Each track gets the edit on top of what it carried.
        let edits: Vec<TrackEdit> = {
            let carried = self.carried.borrow();
            self.tracks
                .borrow()
                .iter()
                .map(|track| TrackEdit {
                    track: track.clone(),
                    edit: match carried.get(&track.id) {
                        Some(first) => merged(first, &edit),
                        None => edit.clone(),
                    },
                })
                .filter(|change| !change.edit.is_empty())
                .collect()
        };
        if edits.is_empty() {
            self.close();
            return;
        }
        // The file playback holds open waits until it is let go; the rest is written now.
        let held = self.held_file();
        let (hold, now): (Vec<_>, Vec<_>) = edits
            .into_iter()
            .partition(|change| Some(&change.track.path) == held.as_ref());
        self.shown_waiting
            .set(hold.first().map(|change| change.track.id));
        {
            let mut waiting = self.waiting.borrow_mut();
            for change in hold {
                waiting.retain(|waiting| waiting.track.id != change.track.id);
                waiting.push(change);
            }
        }
        if now.is_empty() {
            self.show_report(&state, TagWriteReport::default());
            return;
        }
        state.set_status(TagEditorStatus::Writing);
        state.set_done(0);
        state.set_track_count(self.tracks.borrow().len() as i32);
        self.writing.set(true);

        let (sender, notify, paths) =
            (self.sender.clone(), self.notify.clone(), self.paths.clone());
        thread::Builder::new()
            .name("vcmp-tags".into())
            .spawn(move || {
                let send = |message| {
                    let _ = sender.send(message);
                    notify();
                };
                let report = write_files(&paths, &now, |done| send(Message::Progress(done)));
                send(Message::Finished(report));
            })
            .expect("spawning the tag writing thread");
    }

    /// The file playback has open, when such a file can't be replaced.
    fn held_file(&self) -> Option<PathBuf> {
        if self.open_files_locked.get() {
            self.playback_file.borrow().clone()
        } else {
            None
        }
    }

    /// Writes, in the background, the waiting edits whose file playback let go of.
    fn write_released(&self) {
        let held = self.held_file();
        let released: Vec<TrackEdit> = {
            let mut waiting = self.waiting.borrow_mut();
            let (still, released) = std::mem::take(&mut *waiting)
                .into_iter()
                .partition(|change| Some(&change.track.path) == held.as_ref());
            *waiting = still;
            released
        };
        if released.is_empty() {
            return;
        }
        self.releasing
            .borrow_mut()
            .extend(released.iter().map(|change| change.track.id));
        let (sender, notify, paths) =
            (self.sender.clone(), self.notify.clone(), self.paths.clone());
        thread::Builder::new()
            .name("vcmp-tags".into())
            .spawn(move || {
                let report = write_files(&paths, &released, |_| {});
                let _ = sender.send(Message::Released(released, report));
                notify();
            })
            .expect("spawning the tag writing thread");
    }

    /// Whether the track's edit waits for playback, or is being written after it did.
    fn waits(&self, track: TrackId) -> bool {
        self.releasing.borrow().contains(&track)
            || self
                .waiting
                .borrow()
                .iter()
                .any(|waiting| waiting.track.id == track)
    }

    /// Names the editor's track whose edit waits for playback, if one does.
    fn show_waiting_note(&self, state: &TagEditorState) {
        let file = self
            .tracks
            .borrow()
            .iter()
            .find(|track| self.waits(track.id))
            .map(|track| file_name(&track.path))
            .unwrap_or_default();
        state.set_waiting_file(file.into());
    }

    fn drain(&self) {
        while let Ok(message) = self.receiver.try_recv() {
            self.apply(message);
        }
    }

    fn apply(&self, message: Message) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<TagEditorState>();
        match message {
            Message::Progress(done) => state.set_done(done as i32),
            Message::Finished(report) => {
                self.writing.set(false);
                self.show_report(&state, report);
            }
            Message::Released(edits, report) => self.released(&state, &edits, report),
        }
    }

    /// Shows what a save did. The editor closes when every file was written; otherwise
    /// it lists the files that kept their old tags, and names the one that waits for
    /// playback.
    fn show_report(&self, state: &TagEditorState, report: TagWriteReport) {
        if !report.written.is_empty() {
            self.written();
        }
        let waiting = self.shown_waiting.get();
        if report.failed.is_empty() && waiting.is_none() {
            self.close();
            return;
        }
        log_failures(&report);
        // Trying again writes only the files that failed. The waiting one stays for the
        // note, and saving it again waits again.
        self.tracks.borrow_mut().retain(|track| {
            Some(track.id) == waiting || report.failed.iter().any(|f| f.track == track.id)
        });
        if report.failed.is_empty() {
            state.set_failures(ModelRc::default());
            state.set_status(TagEditorStatus::Waiting);
        } else {
            let rows: Vec<TagFailure> = report.failed.iter().map(failure_row).collect();
            state.set_track_count(self.tracks.borrow().len() as i32);
            state.set_failures(ModelRc::new(VecModel::from(rows)));
            state.set_status(TagEditorStatus::Failed);
        }
        self.show_waiting_note(state);
    }

    /// Takes the result of edits that waited for playback. The open editor still
    /// showing that wait closes once they are written, or lists them if they failed.
    fn released(&self, state: &TagEditorState, edits: &[TrackEdit], report: TagWriteReport) {
        {
            let mut releasing = self.releasing.borrow_mut();
            for change in edits {
                releasing.remove(&change.track.id);
            }
        }
        if !report.written.is_empty() {
            self.written();
        }
        let shown = self
            .shown_waiting
            .get()
            .filter(|_| state.get_open() && !self.writing.get());
        let mut rows: Vec<TagFailure> = state.get_failures().iter().collect();
        for failure in &report.failed {
            let Some(change) = edits.iter().find(|change| change.track.id == failure.track) else {
                continue;
            };
            // Playback opened the file again before it could be replaced: wait again.
            if matches!(failure.error, TagWriteError::Replace(_))
                && self.held_file().as_ref() == Some(&change.track.path)
            {
                self.waiting.borrow_mut().push(change.clone());
                continue;
            }
            log_failure(failure);
            if shown == Some(change.track.id) {
                // Trying again builds on the edit that failed.
                self.carried
                    .borrow_mut()
                    .insert(change.track.id, change.edit.clone());
                self.shown_waiting.set(None);
                rows.push(failure_row(failure));
            } else if let Some(not_written) = &*self.on_not_written.borrow() {
                not_written(&change.track.path);
            }
        }
        let Some(shown) = shown else {
            if state.get_open() {
                self.show_waiting_note(state);
            }
            return;
        };
        if report.written.contains(&shown) {
            self.shown_waiting.set(None);
            if state.get_status() == TagEditorStatus::Waiting {
                self.close();
                return;
            }
            self.tracks.borrow_mut().retain(|track| track.id != shown);
            state.set_track_count(self.tracks.borrow().len() as i32);
        } else if self.shown_waiting.get().is_none() {
            state.set_track_count(self.tracks.borrow().len() as i32);
            state.set_failures(ModelRc::new(VecModel::from(rows)));
            state.set_status(TagEditorStatus::Failed);
        }
        self.show_waiting_note(state);
    }

    /// Shows newly written tags in the library views and the player.
    fn written(&self) {
        self.view.reload();
        if let Some(written) = &*self.on_written.borrow() {
            written();
        }
    }
}

/// Writes the edits on a library connection of its own. When the library can't be
/// opened, every file is reported as keeping its tags.
fn write_files(
    paths: &LibraryPaths,
    edits: &[TrackEdit],
    mut progress: impl FnMut(usize),
) -> TagWriteReport {
    let result = Library::open(&paths.database, &paths.cache).and_then(|mut library| {
        let edits: Vec<_> = edits
            .iter()
            .map(|change| (change.track.id, &change.edit))
            .collect();
        library.write_tags(&edits, &AtomicBool::new(false), |done, _| progress(done))
    });
    result.unwrap_or_else(|error| {
        log::error!("could not write tags: {error}");
        TagWriteReport {
            failed: edits
                .iter()
                .map(|change| FailedTagWrite {
                    track: change.track.id,
                    path: Some(change.track.path.clone()),
                    error: TagWriteError::Write(error.to_string()),
                })
                .collect(),
            ..TagWriteReport::default()
        }
    })
}

fn log_failures(report: &TagWriteReport) {
    for failure in &report.failed {
        log_failure(failure);
    }
}

fn log_failure(failure: &FailedTagWrite) {
    log::warn!(
        "tags not written to {}: {}",
        failure
            .path
            .as_deref()
            .map_or_else(|| "a track".into(), |path| path.display().to_string()),
        failure.error
    );
}

fn failure_row(failure: &FailedTagWrite) -> TagFailure {
    TagFailure {
        file: failure
            .path
            .as_deref()
            .map(file_name)
            .unwrap_or_default()
            .into(),
        reason: reason(&failure.error),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The track as it reads once `edit` is written.
fn edited(track: &Track, edit: &TagEdit) -> Track {
    fn text(edit: &FieldEdit<String>, value: Option<&String>) -> Option<String> {
        match edit {
            FieldEdit::Keep => value.cloned(),
            FieldEdit::Clear => None,
            FieldEdit::Set(new) => Some(new.trim().to_string()).filter(|new| !new.is_empty()),
        }
    }
    fn number<T: Copy>(edit: &FieldEdit<T>, value: Option<T>) -> Option<T> {
        match edit {
            FieldEdit::Keep => value,
            FieldEdit::Clear => None,
            FieldEdit::Set(new) => Some(*new),
        }
    }
    Track {
        title: text(&edit.title, Some(&track.title)).unwrap_or_default(),
        artist: text(&edit.artist, track.artist.as_ref()),
        album: text(&edit.album, track.album.as_ref()),
        album_artist: text(&edit.album_artist, track.album_artist.as_ref()),
        track_number: number(&edit.track_number, track.track_number),
        disc_number: number(&edit.disc_number, track.disc_number),
        year: number(&edit.year, track.year),
        genre: text(&edit.genre, track.genre.as_ref()),
        ..track.clone()
    }
}

/// `first`, with the fields `then` changes changed again.
fn merged(first: &TagEdit, then: &TagEdit) -> TagEdit {
    fn pick<T: Clone>(first: &FieldEdit<T>, then: &FieldEdit<T>) -> FieldEdit<T> {
        match then {
            FieldEdit::Keep => first.clone(),
            _ => then.clone(),
        }
    }
    TagEdit {
        title: pick(&first.title, &then.title),
        artist: pick(&first.artist, &then.artist),
        album: pick(&first.album, &then.album),
        album_artist: pick(&first.album_artist, &then.album_artist),
        track_number: pick(&first.track_number, &then.track_number),
        disc_number: pick(&first.disc_number, &then.disc_number),
        year: pick(&first.year, &then.year),
        genre: pick(&first.genre, &then.genre),
        cover: pick(&first.cover, &then.cover),
    }
}

/// The edit that turns `before` (the values the editor opened with) into `after`:
/// a field left as it was is kept, an emptied one cleared, a changed one set.
fn edit(
    before: &TagValues,
    after: &TagValues,
    cover: FieldEdit<Vec<u8>>,
) -> Result<TagEdit, TagProblem> {
    fn text(before: &str, after: &str) -> FieldEdit<String> {
        if after.trim() == before.trim() {
            FieldEdit::Keep
        } else if after.trim().is_empty() {
            FieldEdit::Clear
        } else {
            FieldEdit::Set(after.trim().to_string())
        }
    }
    fn number<T: std::str::FromStr>(
        before: &str,
        after: &str,
        problem: TagProblem,
    ) -> Result<FieldEdit<T>, TagProblem> {
        match text(before, after) {
            FieldEdit::Keep => Ok(FieldEdit::Keep),
            FieldEdit::Clear => Ok(FieldEdit::Clear),
            FieldEdit::Set(value) => value.parse().map(FieldEdit::Set).map_err(|_| problem),
        }
    }
    let edit = TagEdit {
        title: text(&before.title, &after.title),
        artist: text(&before.artist, &after.artist),
        album: text(&before.album, &after.album),
        album_artist: text(&before.album_artist, &after.album_artist),
        track_number: number(&before.track, &after.track, TagProblem::Track)?,
        disc_number: number(&before.disc, &after.disc, TagProblem::Disc)?,
        year: number(&before.year, &after.year, TagProblem::Year)?,
        genre: text(&before.genre, &after.genre),
        cover,
    };
    edit.validate().map_err(|invalid| match invalid {
        InvalidTagEdit::ControlCharacter(_) => TagProblem::LineBreak,
        InvalidTagEdit::YearOutOfRange => TagProblem::Year,
        InvalidTagEdit::CoverFormat => TagProblem::Cover,
    })?;
    Ok(edit)
}

fn reason(error: &TagWriteError) -> TagFailureReason {
    match error {
        TagWriteError::Unavailable => TagFailureReason::Unavailable,
        TagWriteError::ReadOnly => TagFailureReason::ReadOnly,
        TagWriteError::Unreadable(_) => TagFailureReason::Unreadable,
        TagWriteError::Write(_) => TagFailureReason::Write,
        TagWriteError::Verify(_) => TagFailureReason::Verify,
        TagWriteError::ChangedMeanwhile => TagFailureReason::Changed,
        TagWriteError::Replace(_) => TagFailureReason::Replace,
    }
}

/// The cover as RGBA, small enough for the editor.
fn preview(data: &[u8]) -> Option<Image> {
    let image = image::load_from_memory(data).ok()?;
    let rgba = image.thumbnail(PREVIEW_PIXELS, PREVIEW_PIXELS).into_rgba8();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
    );
    Some(Image::from_rgba8(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(artist: &str, year: &str, track: &str) -> TagValues {
        TagValues {
            artist: artist.into(),
            year: year.into(),
            track: track.into(),
            ..TagValues::default()
        }
    }

    #[test]
    fn only_changed_fields_are_written() {
        let before = values("Band", "1999", "3");
        let same = edit(&before, &before, FieldEdit::Keep).unwrap();
        assert!(same.is_empty());

        let changed = edit(&before, &values(" Other ", "", "3"), FieldEdit::Keep).unwrap();
        assert_eq!(changed.artist, FieldEdit::Set("Other".into()));
        assert_eq!(changed.year, FieldEdit::Clear);
        assert_eq!(changed.track_number, FieldEdit::Keep);
    }

    #[test]
    fn a_later_edit_changes_only_its_own_fields() {
        let first = TagEdit {
            title: FieldEdit::Set("New".into()),
            year: FieldEdit::Clear,
            ..TagEdit::default()
        };
        let then = TagEdit {
            artist: FieldEdit::Set("Duo".into()),
            year: FieldEdit::Set(2001),
            ..TagEdit::default()
        };
        let both = merged(&first, &then);
        assert_eq!(both.title, FieldEdit::Set("New".into()));
        assert_eq!(both.artist, FieldEdit::Set("Duo".into()));
        assert_eq!(both.year, FieldEdit::Set(2001), "the later value wins");
        assert_eq!(both.genre, FieldEdit::Keep);
    }

    #[test]
    fn a_waiting_edit_shows_as_written() {
        let track = Track {
            id: TrackId(1),
            path: PathBuf::from("/music/1.flac"),
            available: true,
            format: None,
            title: "One".into(),
            artist: Some("Band".into()),
            artist_id: None,
            album: Some("Album".into()),
            album_id: None,
            album_artist: None,
            track_number: Some(1),
            disc_number: Some(1),
            year: Some(1999),
            genre: None,
            duration: Duration::ZERO,
        };
        let shown = edited(
            &track,
            &TagEdit {
                title: FieldEdit::Set(" First ".into()),
                artist: FieldEdit::Clear,
                year: FieldEdit::Set(2001),
                ..TagEdit::default()
            },
        );
        assert_eq!(shown.title, "First");
        assert_eq!(shown.artist, None);
        assert_eq!(shown.year, Some(2001));
        assert_eq!(shown.album.as_deref(), Some("Album"), "the rest is kept");
    }

    #[test]
    fn wrong_values_point_at_their_field() {
        let before = values("Band", "1999", "3");
        let problem = |after: TagValues| edit(&before, &after, FieldEdit::Keep).unwrap_err();
        assert_eq!(problem(values("Band", "1999", "three")), TagProblem::Track);
        assert_eq!(problem(values("Band", "12000", "3")), TagProblem::Year);
        assert_eq!(problem(values("Band", "-5", "3")), TagProblem::Year);
        assert_eq!(problem(values("A\tB", "1999", "3")), TagProblem::LineBreak);
        assert_eq!(
            edit(&before, &before, FieldEdit::Set(b"not an image".to_vec())).unwrap_err(),
            TagProblem::Cover
        );
    }
}
