//! Music folders and scanning behind the `LibraryState` global: adding and removing
//! folders, scans on a background thread with progress, and automatic updates (a scan
//! at startup plus a watcher on the folders).
//!
//! Scans open their own database connection, so the UI keeps reading while they
//! write. They report through a channel that the UI thread drains; batches of albums
//! appear in the grid while a long first scan is still running.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, VecModel};
use vcmp_library::{Library, LibraryWatcher, ScanPhase as Phase, ScanProgress, ScanReport};

use crate::albums::AlbumBrowser;
use crate::{AppWindow, LibraryError, LibraryState, ScanPhase, SourceFolder, UnreadableFile};

/// Where the library's database and cache live; scans open their own connection.
#[derive(Clone, Debug)]
pub struct LibraryPaths {
    pub database: PathBuf,
    pub cache: PathBuf,
}

/// How messages from background threads reach the UI thread.
pub enum Delivery {
    /// Through Slint's event loop (the running app).
    EventLoop,
    /// Only when the owner calls `wait_until_idle` (tests, which have no event loop).
    Manual,
}

/// Progress updates are sent at most this often, so a fast scan does not flood the UI.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// While tags are read, the album grid is refreshed this often.
const ALBUMS_INTERVAL: Duration = Duration::from_secs(3);
/// The watcher waits for this long without changes before asking for a scan.
const WATCH_QUIET_PERIOD: Duration = Duration::from_secs(2);

enum Message {
    Progress(ScanProgress),
    AlbumsChanged,
    Finished(Result<ScanReport, String>),
    FoldersChanged,
}

pub struct LibraryService {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    paths: LibraryPaths,
    browser: Rc<AlbumBrowser>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    notify: Arc<dyn Fn() + Send + Sync>,
    scan_cancel: RefCell<Option<Arc<AtomicBool>>>,
    rescan_requested: Cell<bool>,
    watcher: RefCell<Option<LibraryWatcher>>,
}

thread_local! {
    /// The service messages are delivered to, on the UI thread.
    static SERVICE: RefCell<std::rc::Weak<LibraryService>> = RefCell::default();
}

impl LibraryService {
    /// Connects `LibraryState` to the library, then starts automatic updates when they
    /// are on: a scan now, and a watcher on the folders.
    pub fn connect(
        window: &AppWindow,
        library: Rc<Library>,
        paths: LibraryPaths,
        browser: Rc<AlbumBrowser>,
        delivery: Delivery,
    ) -> Rc<Self> {
        let (sender, receiver) = mpsc::channel();
        let notify: Arc<dyn Fn() + Send + Sync> = match delivery {
            Delivery::EventLoop => Arc::new(|| {
                let _ = slint::invoke_from_event_loop(|| {
                    if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                        service.drain();
                    }
                });
            }),
            Delivery::Manual => Arc::new(|| {}),
        };
        let service = Rc::new(Self {
            window: window.as_weak(),
            library,
            paths,
            browser,
            sender,
            receiver,
            notify,
            scan_cancel: RefCell::default(),
            rescan_requested: Cell::new(false),
            watcher: RefCell::default(),
        });
        SERVICE.with(|current| *current.borrow_mut() = Rc::downgrade(&service));

        let state = window.global::<LibraryState>();
        let weak = Rc::downgrade(&service);
        state.on_add_folder({
            let weak = weak.clone();
            move || {
                if let Some(service) = weak.upgrade() {
                    service.pick_folder();
                }
            }
        });
        state.on_remove_folder({
            let weak = weak.clone();
            move |id| {
                if let Some(service) = weak.upgrade() {
                    service.remove_folder(i64::from(id));
                }
            }
        });
        state.on_set_recursive({
            let weak = weak.clone();
            move |id, recursive| {
                if let Some(service) = weak.upgrade() {
                    service.set_recursive(i64::from(id), recursive);
                }
            }
        });
        state.on_set_auto_update({
            let weak = weak.clone();
            move |enabled| {
                if let Some(service) = weak.upgrade() {
                    service.set_auto_update(enabled);
                }
            }
        });
        state.on_rescan(move || {
            if let Some(service) = weak.upgrade() {
                service.start_scan();
            }
        });

        service.refresh_folders();
        let auto_update = service.library.auto_update().unwrap_or(true);
        state.set_auto_update(auto_update);
        state.set_track_count(service.library.available_track_count().unwrap_or(0) as i32);
        service.restart_watcher();
        if auto_update && !service.folders().is_empty() {
            service.start_scan();
        }
        service
    }

    /// Adds a folder, then scans so its music appears.
    pub fn add_folder(&self, path: &Path) {
        match self.library.add_source(path, true) {
            Ok(_) => {
                self.clear_error();
                self.refresh_folders();
                self.restart_watcher();
                self.start_scan();
            }
            Err(error) => self.show_error(LibraryError::AddFolder, &error.to_string()),
        }
    }

    /// Removes a folder. Its tracks become unavailable, so nothing is lost if it is
    /// added again.
    pub fn remove_folder(&self, id: i64) {
        match self.library.remove_source(id) {
            Ok(()) => {
                self.clear_error();
                self.refresh_folders();
                self.refresh_albums();
                self.refresh_track_count();
                self.restart_watcher();
            }
            Err(error) => self.show_error(LibraryError::ChangeFolder, &error.to_string()),
        }
    }

    pub fn set_recursive(&self, id: i64, recursive: bool) {
        match self.library.set_source_recursive(id, recursive) {
            Ok(()) => {
                self.clear_error();
                self.refresh_folders();
                self.restart_watcher();
                self.start_scan();
            }
            Err(error) => self.show_error(LibraryError::ChangeFolder, &error.to_string()),
        }
    }

    pub fn set_auto_update(&self, enabled: bool) {
        match self.library.set_auto_update(enabled) {
            Ok(()) => {
                if let Some(window) = self.window.upgrade() {
                    window.global::<LibraryState>().set_auto_update(enabled);
                }
                self.restart_watcher();
            }
            Err(error) => self.show_error(LibraryError::Settings, &error.to_string()),
        }
    }

    /// Starts a scan, or, when one is running, asks for another once it is done.
    pub fn start_scan(&self) {
        if self.scan_cancel.borrow().is_some() {
            self.rescan_requested.set(true);
            return;
        }
        self.rescan_requested.set(false);
        let cancel = Arc::new(AtomicBool::new(false));
        *self.scan_cancel.borrow_mut() = Some(cancel.clone());
        self.show_progress(ScanProgress {
            phase: Phase::Discovering,
            done: 0,
            total: 0,
        });

        let (sender, notify, paths) =
            (self.sender.clone(), self.notify.clone(), self.paths.clone());
        thread::Builder::new()
            .name("vcmp-scan".into())
            .spawn(move || {
                let send = |message| {
                    let _ = sender.send(message);
                    notify();
                };
                let result =
                    Library::open(&paths.database, &paths.cache).and_then(|mut library| {
                        let (mut last_progress, mut last_albums) = (Instant::now(), Instant::now());
                        let mut last_phase = Phase::Discovering;
                        library.scan(&cancel, |progress| {
                            let phase_changed = progress.phase != last_phase;
                            if phase_changed || last_progress.elapsed() >= PROGRESS_INTERVAL {
                                last_progress = Instant::now();
                                send(Message::Progress(progress));
                            }
                            if (phase_changed && last_phase == Phase::Reading)
                                || (progress.phase == Phase::Reading
                                    && last_albums.elapsed() >= ALBUMS_INTERVAL)
                            {
                                last_albums = Instant::now();
                                send(Message::AlbumsChanged);
                            }
                            last_phase = progress.phase;
                        })
                    });
                send(Message::Finished(result.map_err(|error| error.to_string())));
            })
            .expect("spawning the scan thread");
    }

    /// Applies messages until no scan is running or queued, or `timeout` passes.
    /// For callers without an event loop (`Delivery::Manual`).
    pub fn wait_until_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.scan_cancel.borrow().is_some() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining) {
                Ok(message) => self.apply(message),
                Err(RecvTimeoutError::Timeout) => return false,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        true
    }

    fn drain(&self) {
        while let Ok(message) = self.receiver.try_recv() {
            self.apply(message);
        }
    }

    fn apply(&self, message: Message) {
        match message {
            Message::Progress(progress) => self.show_progress(progress),
            Message::AlbumsChanged => self.refresh_albums(),
            Message::FoldersChanged => {
                if self.library.auto_update().unwrap_or(true) {
                    self.start_scan();
                }
            }
            Message::Finished(result) => {
                *self.scan_cancel.borrow_mut() = None;
                self.finish_scan(result);
                if self.rescan_requested.get() {
                    self.start_scan();
                }
            }
        }
    }

    fn finish_scan(&self, result: Result<ScanReport, String>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<LibraryState>();
        state.set_phase(ScanPhase::Idle);
        match result {
            Ok(report) => {
                if !report.cancelled {
                    state.set_scanned(true);
                }
                let unreadable: Vec<UnreadableFile> = report
                    .skipped
                    .iter()
                    .map(|file| UnreadableFile {
                        path: file.path.to_string_lossy().as_ref().into(),
                        reason: file.reason.as_str().into(),
                    })
                    .collect();
                state.set_unreadable(ModelRc::new(VecModel::from(unreadable)));
                self.clear_error();
            }
            Err(error) => self.show_error(LibraryError::Scan, &error),
        }
        self.refresh_albums();
        self.refresh_folders();
        self.refresh_track_count();
    }

    fn pick_folder(&self) {
        let weak = SERVICE.with(|current| current.borrow().clone());
        let picked = slint::spawn_local(async move {
            let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
                return;
            };
            if let Some(service) = weak.upgrade() {
                service.add_folder(folder.path());
            }
        });
        if let Err(error) = picked {
            self.show_error(LibraryError::AddFolder, &error.to_string());
        }
    }

    fn folders(&self) -> Vec<vcmp_library::SourceDir> {
        self.library.sources().unwrap_or_else(|error| {
            log::error!("could not read the music folders: {error}");
            Vec::new()
        })
    }

    fn refresh_folders(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let folders: Vec<SourceFolder> = self
            .folders()
            .into_iter()
            .map(|source| SourceFolder {
                id: source.id as i32,
                name: source
                    .path
                    .file_name()
                    .map_or_else(
                        || source.path.to_string_lossy(),
                        |name| name.to_string_lossy(),
                    )
                    .as_ref()
                    .into(),
                path: source.path.to_string_lossy().as_ref().into(),
                recursive: source.recursive,
                reachable: source.path.is_dir(),
            })
            .collect();
        window
            .global::<LibraryState>()
            .set_folders(ModelRc::new(VecModel::from(folders)));
    }

    fn refresh_albums(&self) {
        match self.library.albums() {
            Ok(albums) => self.browser.replace(&albums),
            Err(error) => log::error!("could not read albums: {error}"),
        }
    }

    fn refresh_track_count(&self) {
        if let (Some(window), Ok(count)) =
            (self.window.upgrade(), self.library.available_track_count())
        {
            window
                .global::<LibraryState>()
                .set_track_count(count as i32);
        }
    }

    /// Watches the folders while automatic updates are on.
    fn restart_watcher(&self) {
        *self.watcher.borrow_mut() = None;
        let folders = self.folders();
        if folders.is_empty() || !self.library.auto_update().unwrap_or(true) {
            return;
        }
        let (sender, notify) = (self.sender.clone(), self.notify.clone());
        match LibraryWatcher::start(&folders, WATCH_QUIET_PERIOD, move || {
            let _ = sender.send(Message::FoldersChanged);
            notify();
        }) {
            Ok(watcher) => *self.watcher.borrow_mut() = Some(watcher),
            Err(error) => log::warn!("could not watch the music folders: {error}"),
        }
    }

    fn show_progress(&self, progress: ScanProgress) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<LibraryState>();
        state.set_phase(match progress.phase {
            Phase::Discovering => ScanPhase::Discovering,
            Phase::Reading => ScanPhase::Reading,
            Phase::Artwork => ScanPhase::Artwork,
        });
        state.set_done(progress.done as i32);
        state.set_total(progress.total as i32);
    }

    fn show_error(&self, kind: LibraryError, detail: &str) {
        log::warn!("{kind:?}: {detail}");
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<LibraryState>();
            state.set_error(kind);
            state.set_error_detail(detail.into());
        }
    }

    fn clear_error(&self) {
        if let Some(window) = self.window.upgrade() {
            window
                .global::<LibraryState>()
                .set_error(LibraryError::None);
        }
    }
}

impl Drop for LibraryService {
    fn drop(&mut self) {
        if let Some(cancel) = self.scan_cancel.borrow().as_ref() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
