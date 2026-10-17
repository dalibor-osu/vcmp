//! Last.fm behind the `LastfmState` global: connecting an account through Last.fm's
//! page in the browser, who is connected, and the plays `PlayerService` hands over.
//! Plays wait in the library database (`scrobble_queue`) until the scrobbler thread
//! has sent them; the account's session key is in the OS keyring, and only its user
//! name is in the settings.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use slint::ComponentHandle;
use vcmp_integrations::lastfm::{
    self, Client, ConnectError, Credentials, Keyring, Report, ScrobbleStore, Scrobbler, Timing,
};
use vcmp_library::{Library, QueuedScrobble};

use crate::library_service::LibraryPaths;
use crate::player_service::ScrobbleSink;
use crate::{AppWindow, LastfmProblem, LastfmState, LastfmStatus};

/// Setting key: the connected account's Last.fm user name; absent or empty for none.
pub const SETTING_USER: &str = "lastfm.user";

/// The scrobbler thread, or a recorder in tests.
pub trait LastfmWorker {
    fn resume(&self, user: &str);
    fn connect(&self);
    fn cancel(&self);
    fn disconnect(&self);
    fn now_playing(&self, track: lastfm::Track);
    /// Plays were added to the queue.
    fn flush(&self);
}

impl LastfmWorker for Scrobbler {
    fn resume(&self, user: &str) {
        Scrobbler::resume(self, user);
    }

    fn connect(&self) {
        Scrobbler::connect(self);
    }

    fn cancel(&self) {
        Scrobbler::cancel(self);
    }

    fn disconnect(&self) {
        Scrobbler::disconnect(self);
    }

    fn now_playing(&self, track: lastfm::Track) {
        Scrobbler::now_playing(self, track);
    }

    fn flush(&self) {
        Scrobbler::flush(self);
    }
}

/// Opens a page in the user's browser; a recorder in tests.
pub type OpenUrl = Box<dyn Fn(&str) -> Result<(), String>>;

pub fn open_in_browser(url: &str) -> Result<(), String> {
    webbrowser::open(url).map_err(|error| error.to_string())
}

/// The scrobbler's side of the queue: the library database, through a connection
/// of its own on the scrobbler thread.
pub struct QueueStore(pub Library);

impl ScrobbleStore for QueueStore {
    fn oldest(&mut self, limit: usize) -> Result<Vec<(i64, lastfm::Scrobble)>, String> {
        let queued = self
            .0
            .queued_scrobbles(limit)
            .map_err(|error| error.to_string())?;
        Ok(queued
            .into_iter()
            .map(|QueuedScrobble { id, scrobble }| {
                let play = lastfm::Scrobble {
                    track: lastfm::Track {
                        artist: scrobble.artist,
                        title: scrobble.title,
                        album: scrobble.album,
                        album_artist: scrobble.album_artist,
                        track_number: scrobble.track_number,
                        duration: scrobble.duration,
                    },
                    started: scrobble.played_at,
                };
                (id, play)
            })
            .collect())
    }

    fn remove(&mut self, ids: &[i64]) -> Result<(), String> {
        self.0
            .remove_scrobbles(ids)
            .map_err(|error| error.to_string())
    }

    fn clear(&mut self) -> Result<(), String> {
        self.0.clear_scrobbles().map_err(|error| error.to_string())
    }
}

thread_local! {
    /// The service scrobbler reports are delivered to, on the UI thread.
    static SERVICE: RefCell<Weak<LastfmService>> = RefCell::default();
}

/// Starts Last.fm when this build has VCMP's API key (`credentials`): the scrobbler
/// thread, with its own database connection, and the Settings row. Without a key
/// nothing starts, nothing is sent and Settings shows no Last.fm row.
pub fn start(
    window: &AppWindow,
    library: Rc<Library>,
    paths: &LibraryPaths,
    credentials: Option<Credentials>,
) -> Option<Rc<LastfmService>> {
    let credentials = credentials?;
    let queue = match Library::open(&paths.database, &paths.cache) {
        Ok(connection) => QueueStore(connection),
        Err(error) => {
            log::error!("Last.fm is off, the scrobble queue could not be opened: {error}");
            return None;
        }
    };
    let scrobbler = Scrobbler::start(
        Client::online(credentials),
        Keyring,
        queue,
        Timing::default(),
        |report| {
            LastfmService::post(report);
        },
    );
    Some(LastfmService::connect(
        window,
        library,
        Box::new(scrobbler),
        Box::new(open_in_browser),
    ))
}

pub struct LastfmService {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    worker: Box<dyn LastfmWorker>,
    open_url: OpenUrl,
    // The page where the user allows VCMP, while connecting.
    auth_url: RefCell<Option<String>>,
}

impl LastfmService {
    /// Shows the Last.fm row and resumes the saved account, if there is one.
    pub fn connect(
        window: &AppWindow,
        library: Rc<Library>,
        worker: Box<dyn LastfmWorker>,
        open_url: OpenUrl,
    ) -> Rc<Self> {
        let service = Rc::new(Self {
            window: window.as_weak(),
            library,
            worker,
            open_url,
            auth_url: RefCell::default(),
        });
        SERVICE.with(|current| *current.borrow_mut() = Rc::downgrade(&service));

        let state = window.global::<LastfmState>();
        state.set_available(true);
        if let Some(user) = service.saved_user() {
            state.set_status(LastfmStatus::Connected);
            state.set_user(user.as_str().into());
            service.worker.resume(&user);
        }
        service.show_unsent();

        let weak = Rc::downgrade(&service);
        let with = move |action: fn(&LastfmService)| {
            let weak = weak.clone();
            move || {
                if let Some(service) = weak.upgrade() {
                    action(&service);
                }
            }
        };
        state.on_connect(with(LastfmService::start_connecting));
        state.on_cancel(with(LastfmService::cancel));
        state.on_disconnect(with(LastfmService::disconnect));
        state.on_open_auth_page(with(|service| {
            let url = service.auth_url.borrow().clone();
            if let Some(url) = url {
                service.open(&url);
            }
        }));
        service
    }

    /// Runs `report` through the service on the UI thread, from any thread. False
    /// once the event loop is gone.
    pub fn post(report: Report) -> bool {
        slint::invoke_from_event_loop(move || {
            if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                service.report(report);
            }
        })
        .is_ok()
    }

    /// Takes what the scrobbler reports.
    pub fn report(&self, report: Report) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<LastfmState>();
        let waiting = state.get_status() == LastfmStatus::Waiting;
        match report {
            Report::Authorize(url) if waiting => {
                *self.auth_url.borrow_mut() = Some(url.clone());
                self.open(&url);
            }
            Report::Connected { user, saved } => {
                if !waiting {
                    // Cancelled as the user allowed VCMP: the cancel wins.
                    self.worker.disconnect();
                    return;
                }
                self.save_user(&user);
                *self.auth_url.borrow_mut() = None;
                state.set_status(LastfmStatus::Connected);
                state.set_user(user.into());
                match saved {
                    Ok(()) => set_problem(&state, LastfmProblem::None, ""),
                    Err(error) => {
                        log::warn!("the Last.fm key could not be saved in the keyring: {error}");
                        set_problem(&state, LastfmProblem::Keyring, &error);
                    }
                }
            }
            Report::ConnectFailed(error) if waiting => {
                *self.auth_url.borrow_mut() = None;
                state.set_status(LastfmStatus::Disconnected);
                match error {
                    ConnectError::TimedOut => set_problem(&state, LastfmProblem::TimedOut, ""),
                    ConnectError::Failed(error) => {
                        log::info!("connecting to Last.fm failed: {error}");
                        set_problem(&state, LastfmProblem::ConnectFailed, &error.to_string());
                    }
                }
            }
            Report::KeyMissing => {
                self.save_user("");
                state.set_status(LastfmStatus::Disconnected);
                set_problem(&state, LastfmProblem::KeyMissing, "");
            }
            // The account stays saved, so the next start tries the keyring again.
            Report::KeyUnreadable(error) => {
                log::warn!("the Last.fm key could not be read from the keyring: {error}");
                state.set_status(LastfmStatus::Disconnected);
                set_problem(&state, LastfmProblem::Keyring, &error);
            }
            Report::Rejected => {
                self.save_user("");
                state.set_status(LastfmStatus::Disconnected);
                set_problem(&state, LastfmProblem::Rejected, "");
            }
            Report::Sent(_) => {}
            Report::Authorize(_) | Report::ConnectFailed(_) => {}
        }
        self.show_unsent();
    }

    fn start_connecting(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<LastfmState>();
        *self.auth_url.borrow_mut() = None;
        state.set_status(LastfmStatus::Waiting);
        set_problem(&state, LastfmProblem::None, "");
        self.worker.connect();
    }

    fn cancel(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<LastfmState>();
        if state.get_status() == LastfmStatus::Waiting {
            *self.auth_url.borrow_mut() = None;
            state.set_status(LastfmStatus::Disconnected);
            set_problem(&state, LastfmProblem::None, "");
            self.worker.cancel();
        }
    }

    /// Forgets the account and the plays not sent yet.
    fn disconnect(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<LastfmState>();
        self.worker.disconnect();
        self.save_user("");
        state.set_status(LastfmStatus::Disconnected);
        state.set_user("".into());
        set_problem(&state, LastfmProblem::None, "");
        // The scrobbler empties the queue, after any batch it is sending.
        state.set_unsent(0);
    }

    fn open(&self, url: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<LastfmState>();
        match (self.open_url)(url) {
            Ok(()) => {
                if state.get_problem() == LastfmProblem::Browser {
                    set_problem(&state, LastfmProblem::None, "");
                }
            }
            Err(error) => {
                log::warn!("could not open {url}: {error}");
                set_problem(&state, LastfmProblem::Browser, &error);
            }
        }
    }

    fn connected(&self) -> bool {
        self.window.upgrade().is_some_and(|window| {
            window.global::<LastfmState>().get_status() == LastfmStatus::Connected
        })
    }

    fn show_unsent(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let unsent = self
            .library
            .queued_scrobble_count()
            .unwrap_or_else(|error| {
                log::warn!("could not count the scrobble queue: {error}");
                0
            });
        window.global::<LastfmState>().set_unsent(unsent as i32);
    }

    fn saved_user(&self) -> Option<String> {
        self.library
            .setting(SETTING_USER)
            .unwrap_or_else(|error| {
                log::warn!("could not read setting {SETTING_USER}: {error}");
                None
            })
            .filter(|user| !user.is_empty())
    }

    fn save_user(&self, user: &str) {
        if let Err(error) = self.library.set_setting(SETTING_USER, user) {
            log::warn!("could not save setting {SETTING_USER}: {error}");
        }
    }
}

fn set_problem(state: &LastfmState, problem: LastfmProblem, detail: &str) {
    state.set_problem(problem);
    state.set_problem_detail(detail.into());
}

impl ScrobbleSink for LastfmService {
    fn now_playing(&self, track: lastfm::Track) {
        if self.connected() {
            self.worker.now_playing(track);
        }
    }

    /// Plays are kept only while an account is connected.
    fn scrobble(&self, play: lastfm::Scrobble) {
        if !self.connected() {
            return;
        }
        let track = play.track;
        let queued = self.library.queue_scrobble(&vcmp_library::Scrobble {
            artist: track.artist,
            title: track.title,
            album: track.album,
            album_artist: track.album_artist,
            track_number: track.track_number,
            duration: track.duration,
            played_at: play.started,
        });
        match queued {
            Ok(_) => self.worker.flush(),
            Err(error) => log::warn!("could not queue a play for Last.fm: {error}"),
        }
        self.show_unsent();
    }
}
