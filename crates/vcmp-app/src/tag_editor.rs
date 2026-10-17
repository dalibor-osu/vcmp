//! The tag editor behind `TagEditorState`: one track from any track menu, or every
//! track of the album page's album. It shows the values the files have (empty where
//! an album's tracks differ), and writes only the fields the user changed, through
//! `Library::write_tags` on a connection of its own in the background. Afterwards
//! the library views and the player show the new tags.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use vcmp_library::{
    FieldEdit, InvalidTagEdit, Library, TagEdit, TagWriteError, TagWriteReport, Track,
};

use crate::library_service::{Delivery, LibraryPaths};
use crate::library_view::LibraryView;
use crate::{
    AppWindow, TagEditorState, TagEditorStatus, TagFailure, TagFailureReason, TagMixed, TagProblem,
    TagValues,
};

/// Longest side of the cover preview, in pixels.
const PREVIEW_PIXELS: u32 = 320;
/// Image files the cover chooser offers; the tags hold JPEG and PNG only.
const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

enum Message {
    Progress(usize),
    Finished(Result<TagWriteReport, String>),
}

/// Told when tags were written, so other parts of the window can show them.
type Written = Box<dyn Fn()>;

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

    /// Opens the editor for the library's track with this file.
    pub fn edit_track(&self, path: &Path) {
        let track = self
            .library
            .track_id_for_path(path)
            .ok()
            .flatten()
            .and_then(|id| self.library.track(id).ok().flatten());
        match track {
            Some(track) => self.open(vec![track]),
            None => log::warn!("not in the library, so not editable: {}", path.display()),
        }
    }

    /// Opens the editor for every track of the album the album page shows.
    pub fn edit_album(&self) {
        let Some(album) = self.view.shown_album() else {
            return;
        };
        match self.library.album_tracks(album.id) {
            Ok(tracks) if !tracks.is_empty() => self.open(tracks),
            Ok(_) => {}
            Err(error) => log::error!("could not read the tracks of {:?}: {error}", album.title),
        }
    }

    /// Applies what the writing thread sent until it is done, or `timeout` passes.
    /// For callers without an event loop (`Delivery::Manual`).
    pub fn wait_until_written(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.writing.get() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining) {
                Ok(message) => self.apply(message),
                Err(RecvTimeoutError::Timeout) => return false,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        true
    }

    fn open(&self, tracks: Vec<Track>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<TagEditorState>();
        let first = &tracks[0];
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
        let artist = common(&tracks, |track| track.artist.clone());
        let album = common(&tracks, |track| track.album.clone());
        let album_artist = common(&tracks, |track| track.album_artist.clone());
        let disc = common(&tracks, |track| track.disc_number);
        let year = common(&tracks, |track| track.year);
        let genre = common(&tracks, |track| track.genre.clone());
        let covers: Vec<Option<Vec<u8>>> = tracks
            .iter()
            .map(|track| self.library.embedded_cover(track.id).ok().flatten())
            .collect();
        let cover_mixed = covers.iter().any(|cover| cover != &covers[0]);

        state.set_track_count(tracks.len() as i32);
        state.set_subject(
            if single {
                first.title.clone()
            } else {
                first.album.clone().unwrap_or_default()
            }
            .into(),
        );
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
        *self.cover.borrow_mut() = FieldEdit::Keep;
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
        }
        self.tracks.borrow_mut().clear();
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
        if self.writing.get() {
            return;
        }
        let state = window.global::<TagEditorState>();
        let edit = match edit(&state.get_values(), values, self.cover.borrow().clone()) {
            Ok(edit) => edit,
            Err(problem) => {
                state.set_problem(problem);
                return;
            }
        };
        state.set_problem(TagProblem::None);
        if edit.is_empty() {
            self.close();
            return;
        }
        state.set_status(TagEditorStatus::Writing);
        state.set_done(0);
        state.set_track_count(self.tracks.borrow().len() as i32);
        self.writing.set(true);

        let ids: Vec<_> = self.tracks.borrow().iter().map(|track| track.id).collect();
        let (sender, notify, paths) =
            (self.sender.clone(), self.notify.clone(), self.paths.clone());
        thread::Builder::new()
            .name("vcmp-tags".into())
            .spawn(move || {
                let send = |message| {
                    let _ = sender.send(message);
                    notify();
                };
                let result = Library::open(&paths.database, &paths.cache)
                    .and_then(|mut library| {
                        let edits: Vec<_> = ids.iter().map(|&id| (id, &edit)).collect();
                        library.write_tags(&edits, &AtomicBool::new(false), |done, _| {
                            send(Message::Progress(done));
                        })
                    })
                    .map_err(|error| error.to_string());
                send(Message::Finished(result));
            })
            .expect("spawning the tag writing thread");
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
            Message::Finished(result) => {
                self.writing.set(false);
                let report = match result {
                    Ok(report) => report,
                    Err(error) => {
                        log::error!("could not write tags: {error}");
                        // Every file kept its tags; say so for each.
                        TagWriteReport {
                            failed: self
                                .tracks
                                .borrow()
                                .iter()
                                .map(|track| vcmp_library::FailedTagWrite {
                                    track: track.id,
                                    path: Some(track.path.clone()),
                                    error: TagWriteError::Write(error.clone()),
                                })
                                .collect(),
                            ..TagWriteReport::default()
                        }
                    }
                };
                if !report.written.is_empty() {
                    self.view.reload();
                    if let Some(written) = &*self.on_written.borrow() {
                        written();
                    }
                }
                if report.failed.is_empty() {
                    self.close();
                    return;
                }
                for failure in &report.failed {
                    log::warn!(
                        "tags not written to {}: {}",
                        failure
                            .path
                            .as_deref()
                            .map_or_else(|| "a track".into(), |path| path.display().to_string()),
                        failure.error
                    );
                }
                let rows: Vec<TagFailure> = report
                    .failed
                    .iter()
                    .map(|failure| TagFailure {
                        file: failure
                            .path
                            .as_deref()
                            .and_then(Path::file_name)
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default()
                            .into(),
                        reason: reason(&failure.error),
                    })
                    .collect();
                // Trying again writes only the files that failed.
                self.tracks
                    .borrow_mut()
                    .retain(|track| report.failed.iter().any(|f| f.track == track.id));
                state.set_track_count(self.tracks.borrow().len() as i32);
                state.set_failures(ModelRc::new(VecModel::from(rows)));
                state.set_status(TagEditorStatus::Failed);
            }
        }
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
