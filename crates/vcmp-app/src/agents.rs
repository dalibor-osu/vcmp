//! AI agents behind `AgentState`: the switch in Settings, the changes agents asked for
//! (`vcmp_library` change sets) with their review and undo, and the note in the
//! sidebar while one waits.
//!
//! Nothing an agent asks to write is written until the owner accepts it in the review
//! dialog, and nor are the labels an agent asks to put on tracks or take off. Accepting writes the files on a library connection of its own in the
//! background, like the tag editor, then the library views, the playlists and the
//! player show the result. An undo, whether the owner or an agent asks for it, is a
//! change of its own and is reviewed the same way.
//!
//! When VCMP starts, changes still waiting from the last run are discarded and
//! finished ones older than 30 days are deleted; the deletion runs daily after that.
//!
//! While the switch is on, `vcmp_mcp`'s server runs, and [`AppChanges`] brings what
//! agents ask for to this service on the UI thread.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use vcmp_library::{
    ChangeKind as Kind, ChangeSet, ChangeSetId, ChangeSetSummary, ChangeStatus as Status,
    ChangedFile, FileResult, LabelEdit, Library, NewLabel, PlaylistId, TagEdit, TrackId,
};
use vcmp_mcp::{Changes, PlaylistEdit, Server, ServerConfig};

use crate::library_service::{Delivery, LibraryPaths};
use crate::library_view::LibraryView;
use crate::{
    AgentState, AppWindow, ChangeEntry, ChangeField, ChangeGroup, ChangeKind, ChangeProblem,
    ChangeRow, ChangeStatus, ReviewStatus, ServerStatus,
};

/// Setting key: agents may connect while VCMP is open ("true" or "false"; absent:
/// false).
pub const SETTING_ENABLED: &str = "mcp.enabled";
/// Setting key: the port the server listens on (absent: `vcmp_mcp::DEFAULT_PORT`; 0
/// picks a free one, for tests).
pub const SETTING_PORT: &str = "mcp.port";
/// Setting key: the token agents must send, made when the server first starts.
pub const SETTING_TOKEN: &str = "mcp.token";

/// How long an agent's request waits for the window to take it.
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(30);

/// How often finished changes older than 30 days are looked for while VCMP runs.
const UPKEEP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Whether a file that playback has open can't be replaced: on Windows. A change that
/// would rewrite it waits until playback moves on.
const OPEN_FILES_LOCKED: bool = cfg!(windows);

enum Message {
    Progress(usize),
    Finished(Result<ChangeSet, String>),
}

/// Told when files or playlists changed, so other parts of the window show it.
type Changed = Box<dyn Fn()>;

pub struct AgentService {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    paths: LibraryPaths,
    view: Rc<LibraryView>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    notify: Arc<dyn Fn() + Send + Sync>,
    // The change the review dialog shows, and whether it is being written.
    reviewing: Cell<Option<ChangeSetId>>,
    reviewing_paths: RefCell<Vec<PathBuf>>,
    writing: Cell<bool>,
    playback_file: RefCell<Option<PathBuf>>,
    open_files_locked: Cell<bool>,
    on_written: RefCell<Option<Changed>>,
    upkeep: Timer,
    server: RefCell<Option<Server>>,
}

thread_local! {
    /// The service messages are delivered to, on the UI thread.
    static SERVICE: RefCell<std::rc::Weak<AgentService>> = RefCell::default();
}

impl AgentService {
    /// Connects `AgentState` to the library. Accepted changes are written on a
    /// connection opened from `paths`. Keep the service alive as long as the window.
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
            view,
            sender,
            receiver,
            notify,
            reviewing: Cell::default(),
            reviewing_paths: RefCell::default(),
            writing: Cell::new(false),
            playback_file: RefCell::default(),
            open_files_locked: Cell::new(OPEN_FILES_LOCKED),
            on_written: RefCell::default(),
            upkeep: Timer::default(),
            server: RefCell::default(),
        });
        SERVICE.with(|current| *current.borrow_mut() = Rc::downgrade(&service));

        // Nothing waits across a restart: the agent that asked is gone.
        if let Err(error) = service.library.discard_pending_change_sets() {
            log::warn!("could not discard the changes left waiting: {error}");
        }
        service.expire();
        let weak = Rc::downgrade(&service);
        service
            .upkeep
            .start(TimerMode::Repeated, UPKEEP_INTERVAL, move || {
                if let Some(service) = weak.upgrade() {
                    service.expire();
                    service.refresh();
                }
            });

        let state = window.global::<AgentState>();
        let enabled = service
            .library
            .setting(SETTING_ENABLED)
            .ok()
            .flatten()
            .as_deref()
            == Some("true");
        state.set_enabled(enabled);
        service.show_connection();
        if enabled {
            service.start_server();
        }
        let weak = Rc::downgrade(&service);
        state.on_set_enabled(move |on| {
            if let Some(service) = weak.upgrade() {
                service.set_enabled(on);
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_set_port(move |text| {
            if let Some(service) = weak.upgrade() {
                service.set_port(&text);
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_new_token(move || {
            if let Some(service) = weak.upgrade() {
                service.new_token();
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_review(move |id| {
            if let Some(service) = weak.upgrade() {
                service.review(ChangeSetId(i64::from(id)));
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_review_next(move || {
            if let Some(service) = weak.upgrade() {
                service.review_next();
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_undo(move |id| {
            if let Some(service) = weak.upgrade() {
                service.undo(ChangeSetId(i64::from(id)));
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_accept(move || {
            if let Some(service) = weak.upgrade() {
                service.accept();
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_reject(move || {
            if let Some(service) = weak.upgrade() {
                service.reject();
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_close_review(move || {
            if let Some(service) = weak.upgrade() {
                service.close_review();
            }
        });
        let weak = Rc::downgrade(&service);
        state.on_delete_backups(move || {
            if let Some(service) = weak.upgrade() {
                service.delete_backups();
            }
        });
        service.refresh();
        service
    }

    /// Called after an accepted change wrote files, so the player shows the new tags.
    pub fn on_written(&self, written: impl Fn() + 'static) {
        *self.on_written.borrow_mut() = Some(Box::new(written));
    }

    /// The file playback has open now, if any.
    pub fn set_playback_file(&self, path: Option<&Path>) {
        *self.playback_file.borrow_mut() = path.map(Path::to_path_buf);
        self.show_playing();
    }

    /// Whether a file playback has open can't be replaced; true on Windows. Tests
    /// set it to check either way.
    pub fn set_open_files_locked(&self, locked: bool) {
        self.open_files_locked.set(locked);
        self.show_playing();
    }

    /// Records a change of tags an agent asks for, and shows that it waits.
    pub fn submit_tags(
        &self,
        agent: &str,
        edits: &[(TrackId, TagEdit)],
    ) -> vcmp_library::Result<ChangeSetId> {
        let id = self.library.submit_tag_changes(agent, edits)?;
        self.refresh();
        Ok(id)
    }

    /// Records a change of labels an agent asks for, and shows that it waits.
    pub fn submit_labels(
        &self,
        agent: &str,
        edits: &[(TrackId, LabelEdit)],
        new_labels: &[NewLabel],
    ) -> vcmp_library::Result<ChangeSetId> {
        let id = self
            .library
            .submit_label_changes(agent, edits, new_labels)?;
        self.refresh();
        Ok(id)
    }

    /// Records a request to delete a playlist, and shows that it waits.
    pub fn submit_playlist_deletion(
        &self,
        agent: &str,
        playlist: vcmp_library::PlaylistId,
    ) -> vcmp_library::Result<ChangeSetId> {
        let id = self.library.submit_playlist_deletion(agent, playlist)?;
        self.refresh();
        Ok(id)
    }

    /// Records an undo an agent asks for, and shows that it waits.
    pub fn submit_undo(&self, agent: &str, id: ChangeSetId) -> vcmp_library::Result<ChangeSetId> {
        let undo = self.library.submit_undo(id, agent)?;
        self.refresh();
        Ok(undo)
    }

    /// Makes a playlist edit an agent asks for, and shows the playlists again.
    pub fn edit_playlist(&self, edit: PlaylistEdit) -> vcmp_library::Result<Option<PlaylistId>> {
        let library = &self.library;
        let created = match edit {
            PlaylistEdit::Create(name) => Some(library.create_playlist(&name)?),
            PlaylistEdit::Rename(id, name) => {
                library.rename_playlist(id, &name)?;
                None
            }
            PlaylistEdit::Add(id, tracks) => {
                library.add_to_playlist(id, &tracks)?;
                None
            }
            PlaylistEdit::Remove(id, positions) => {
                library.remove_from_playlist(id, &positions)?;
                None
            }
            PlaylistEdit::Move(id, from, to) => {
                library.move_in_playlist(id, from, to)?;
                None
            }
        };
        self.view.playlists().reload();
        Ok(created)
    }

    /// Whether the server is running now.
    pub fn serving(&self) -> bool {
        self.server.borrow().is_some()
    }

    /// Shows the history, the waiting count and the backups again.
    pub fn refresh(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<AgentState>();
        let sets = self.library.change_sets().unwrap_or_else(|error| {
            log::error!("could not read the changes agents asked for: {error}");
            Vec::new()
        });
        // A change whose undo waits for review gets no second one.
        let undo_waiting: HashSet<ChangeSetId> = sets
            .iter()
            .filter(|set| set.status == Status::Pending)
            .filter_map(|set| set.undoes)
            .collect();
        let now = SystemTime::now();
        let entries: Vec<ChangeEntry> = sets
            .iter()
            .map(|set| entry(set, now, &undo_waiting))
            .collect();
        state.set_pending_count(
            sets.iter()
                .filter(|set| set.status == Status::Pending)
                .count() as i32,
        );
        state.set_history(ModelRc::new(VecModel::from(entries)));
        state.set_has_finished(sets.iter().any(|set| set.status != Status::Pending));
        let size = self.library.finished_change_sets_size().unwrap_or(0);
        state.set_backups_size(if size == 0 {
            SharedString::default()
        } else {
            size_text(size).into()
        });
    }

    /// Opens the review of a change. One already decided shows what it did.
    pub fn review(&self, id: ChangeSetId) {
        if self.writing.get() {
            self.back_to_history();
            return;
        }
        let set = match self.library.change_set(id) {
            Ok(Some(set)) => set,
            Ok(None) => {
                self.refresh();
                self.back_to_history();
                return;
            }
            Err(error) => {
                log::error!("could not read change {id:?}: {error}");
                self.back_to_history();
                return;
            }
        };
        self.show_review(&set);
    }

    /// A review asked for from the list of changes didn't open: the list comes back.
    fn back_to_history(&self) {
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<AgentState>();
            if state.get_review_from_history() && !state.get_review_open() {
                state.set_review_from_history(false);
                state.set_history_open(true);
            }
        }
    }

    /// Applies messages until the change being written is done, or `timeout` passes.
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

    fn set_enabled(&self, on: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        window.global::<AgentState>().set_enabled(on);
        if let Err(error) = self
            .library
            .set_setting(SETTING_ENABLED, if on { "true" } else { "false" })
        {
            log::warn!("could not save the AI agents switch: {error}");
        }
        if on {
            self.start_server();
        } else {
            self.stop_server();
        }
    }

    fn port(&self) -> u16 {
        self.library
            .setting(SETTING_PORT)
            .ok()
            .flatten()
            .and_then(|port| port.parse().ok())
            .unwrap_or(vcmp_mcp::DEFAULT_PORT)
    }

    /// The saved token, or a new one, saved, when there is none yet.
    fn token(&self) -> String {
        if let Some(token) = self.library.setting(SETTING_TOKEN).ok().flatten() {
            return token;
        }
        let token = vcmp_mcp::new_token();
        if let Err(error) = self.library.set_setting(SETTING_TOKEN, &token) {
            log::warn!("could not save the agents' token: {error}");
        }
        token
    }

    /// Starts the server (again) on the saved port with the saved token, and shows
    /// how agents connect, or that the port is taken.
    fn start_server(&self) {
        self.stop_server();
        let config = ServerConfig {
            port: self.port(),
            token: self.token(),
            database: self.paths.database.clone(),
            cache: self.paths.cache.clone(),
        };
        match Server::start(config, Arc::new(AppChanges)) {
            Ok(server) => *self.server.borrow_mut() = Some(server),
            Err(error) => log::warn!("could not start the server for AI agents: {error}"),
        }
        self.show_connection();
    }

    fn stop_server(&self) {
        // Dropping it stops it.
        self.server.borrow_mut().take();
        self.show_connection();
    }

    fn show_connection(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<AgentState>();
        let token = self
            .library
            .setting(SETTING_TOKEN)
            .ok()
            .flatten()
            .unwrap_or_default();
        let server = self.server.borrow();
        let port = server.as_ref().map_or_else(|| self.port(), Server::port);
        state.set_port(port.to_string().into());
        state.set_token(token.as_str().into());
        state.set_status(match &*server {
            Some(_) => ServerStatus::Running,
            None if state.get_enabled() => ServerStatus::Failed,
            None => ServerStatus::Off,
        });
        state.set_address(vcmp_mcp::address(port).into());
        state.set_setup_command(vcmp_mcp::claude_code_command(port, &token).into());
        state.set_desktop_config(desktop_config().into());
    }

    /// A port the owner typed: from 1024 to 65535, the ones a program may take
    /// without special rights. Anything else leaves the port as it was.
    fn set_port(&self, text: &str) {
        match text.trim().parse::<u16>() {
            Ok(port) if port >= 1024 => {
                if let Err(error) = self.library.set_setting(SETTING_PORT, &port.to_string()) {
                    log::warn!("could not save the agents' port: {error}");
                }
                if self.enabled() {
                    self.start_server();
                } else {
                    self.show_connection();
                }
            }
            _ => self.show_connection(),
        }
    }

    /// Replaces the token, so agents with the old one can't connect any more.
    fn new_token(&self) {
        if let Err(error) = self
            .library
            .set_setting(SETTING_TOKEN, &vcmp_mcp::new_token())
        {
            log::warn!("could not save the agents' token: {error}");
        }
        if self.enabled() {
            self.start_server();
        } else {
            self.show_connection();
        }
    }

    fn enabled(&self) -> bool {
        self.window
            .upgrade()
            .is_some_and(|window| window.global::<AgentState>().get_enabled())
    }

    /// The oldest change still waiting.
    fn review_next(&self) {
        let sets = self.library.change_sets().unwrap_or_default();
        if let Some(oldest) = sets.iter().rev().find(|set| set.status == Status::Pending) {
            self.review(oldest.id);
        }
    }

    fn show_review(&self, set: &ChangeSet) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<AgentState>();
        self.reviewing.set(Some(set.summary.id));
        // A change of labels writes no file, so playback holds none open for it.
        *self.reviewing_paths.borrow_mut() = if set.summary.kind == Kind::Labels {
            Vec::new()
        } else {
            set.files.iter().map(|file| file.path.clone()).collect()
        };
        state.set_review_kind(kind(&set.summary));
        state.set_review_agent(set.summary.agent.as_str().into());
        state.set_review_files(set.files.len() as i32);
        state.set_review_playlist(match &set.summary.kind {
            Kind::PlaylistDeletion { name, .. } => name.as_str().into(),
            Kind::Tags | Kind::Labels => SharedString::default(),
        });
        // Names, joined as a list; the comma is not a word to translate.
        let new_labels: Vec<&str> = set
            .new_labels
            .iter()
            .map(|label| label.name.as_str())
            .collect();
        state.set_review_new_labels(new_labels.join(", ").into());
        state.set_review_new_label_count(new_labels.len() as i32);
        state.set_review_gone_labels(self.gone_labels(set).join(", ").into());
        let (shared, groups) = review_groups(&self.library, &set.files);
        state.set_review_shared(ModelRc::new(VecModel::from(shared)));
        state.set_review_groups(ModelRc::new(VecModel::from(groups)));
        state.set_review_failed(false);
        state.set_review_done(0);
        if set.summary.status == Status::Pending {
            state.set_review_status(ReviewStatus::Waiting);
            state.set_review_problems(ModelRc::default());
            state.set_review_written(0);
        } else {
            show_results(&state, set);
        }
        self.show_playing();
        state.set_review_open(true);
    }

    /// For the undo of a change of labels: the labels that change made which still
    /// exist, by name. The undo deletes them unless a track holds them by then.
    fn gone_labels(&self, set: &ChangeSet) -> Vec<String> {
        let (Kind::Labels, Some(undone)) = (&set.summary.kind, set.summary.undoes) else {
            return Vec::new();
        };
        let Ok(Some(undone)) = self.library.change_set(undone) else {
            return Vec::new();
        };
        undone
            .new_labels
            .into_iter()
            .filter(|label| label.created == Some(true))
            .filter_map(|label| self.library.label_named(&label.name).ok().flatten())
            .map(|label| label.name)
            .collect()
    }

    /// Names the file playback holds open when the change would rewrite it, on a
    /// system where that can't be done.
    fn show_playing(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let playing = self
            .playback_file
            .borrow()
            .as_ref()
            .filter(|_| self.open_files_locked.get())
            .filter(|file| self.reviewing_paths.borrow().contains(file))
            .map(|file| file_name(file))
            .unwrap_or_default();
        window
            .global::<AgentState>()
            .set_review_playing(playing.into());
    }

    fn accept(&self) {
        let (Some(window), Some(id)) = (self.window.upgrade(), self.reviewing.get()) else {
            return;
        };
        let state = window.global::<AgentState>();
        if self.writing.get()
            || state.get_review_status() != ReviewStatus::Waiting
            || !state.get_review_playing().is_empty()
        {
            return;
        }
        self.writing.set(true);
        state.set_review_done(0);
        state.set_review_status(ReviewStatus::Writing);
        let (sender, notify, paths) =
            (self.sender.clone(), self.notify.clone(), self.paths.clone());
        thread::Builder::new()
            .name("vcmp-changes".into())
            .spawn(move || {
                let send = |message| {
                    let _ = sender.send(message);
                    notify();
                };
                let result = Library::open(&paths.database, &paths.cache)
                    .and_then(|mut library| {
                        library.accept_change_set(id, |done, _| send(Message::Progress(done)))
                    })
                    .map_err(|error| error.to_string());
                send(Message::Finished(result));
            })
            .expect("spawning the change writing thread");
    }

    fn reject(&self) {
        let Some(id) = self.reviewing.get() else {
            return;
        };
        if self.writing.get() {
            return;
        }
        if let Err(error) = self.library.reject_change_set(id) {
            log::error!("could not reject change {id:?}: {error}");
        }
        self.close_review();
        self.refresh();
    }

    fn close_review(&self) {
        if self.writing.get() {
            return;
        }
        self.reviewing.set(None);
        self.reviewing_paths.borrow_mut().clear();
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<AgentState>();
            state.set_review_open(false);
            state.set_review_playing(SharedString::default());
            // Back to the list it was opened from.
            if state.get_review_from_history() {
                state.set_review_from_history(false);
                state.set_history_open(true);
            }
        }
    }

    /// The owner's undo: a change of its own, opened for review at once.
    fn undo(&self, id: ChangeSetId) {
        match self.library.submit_undo(id, "") {
            Ok(undo) => {
                self.refresh();
                self.review(undo);
            }
            Err(error) => {
                log::warn!("could not undo change {id:?}: {error}");
                self.refresh();
                self.back_to_history();
            }
        }
    }

    fn delete_backups(&self) {
        if let Err(error) = self.library.delete_finished_change_sets() {
            log::error!("could not delete the backups of finished changes: {error}");
        }
        self.refresh();
    }

    fn expire(&self) {
        if let Err(error) = self.library.expire_change_sets() {
            log::warn!("could not delete changes older than 30 days: {error}");
        }
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
        let state = window.global::<AgentState>();
        match message {
            Message::Progress(done) => state.set_review_done(done as i32),
            Message::Finished(result) => {
                self.writing.set(false);
                match result {
                    Ok(set) => {
                        let written = set
                            .files
                            .iter()
                            .any(|file| file.result == Some(FileResult::Written));
                        let changed = match set.summary.kind {
                            Kind::PlaylistDeletion { .. } => true,
                            Kind::Tags => written,
                            Kind::Labels => {
                                // Labels live in the database only: the lists show
                                // them, and no file was rewritten.
                                self.view.labels().changed();
                                false
                            }
                        };
                        if changed {
                            self.view.reload();
                            self.view.playlists().reload();
                            if let Some(written) = &*self.on_written.borrow() {
                                written();
                            }
                        }
                        log_problems(&set);
                        show_results(&state, &set);
                    }
                    Err(error) => {
                        log::error!("could not write the change: {error}");
                        state.set_review_failed(true);
                        state.set_review_status(ReviewStatus::Done);
                    }
                }
                self.refresh();
            }
        }
    }
}

/// Where `vcmp mcp` relays to: the port and token in the library's settings. Fails,
/// saying why, when there is no library yet or the switch is off.
pub fn relay_connection(paths: &LibraryPaths) -> Result<vcmp_mcp::relay::Connection, String> {
    if !paths.database.exists() {
        return Err(format!(
            "VCMP has no library at {} yet: start VCMP and turn on Settings, AI agents.",
            paths.database.display()
        ));
    }
    let library = Library::open(&paths.database, &paths.cache)
        .map_err(|error| format!("VCMP's library can't be read: {error}"))?;
    let setting = |key| library.setting(key).ok().flatten();
    if setting(SETTING_ENABLED).as_deref() != Some("true") {
        return Err(
            "VCMP's AI agents switch is off: turn on \"Let AI agents use your library\" in VCMP's Settings, under AI agents."
                .into(),
        );
    }
    // The server makes the token when it first starts.
    let token = setting(SETTING_TOKEN)
        .ok_or("VCMP hasn't started its server for AI agents yet: open VCMP.")?;
    let port = setting(SETTING_PORT)
        .and_then(|port| port.parse().ok())
        .unwrap_or(vcmp_mcp::DEFAULT_PORT);
    Ok(vcmp_mcp::relay::Connection { port, token })
}

/// The entry for Claude Desktop's `mcpServers`: this program with the argument `mcp`,
/// and `VCMP_HOME` when it is set, so the relay reads the library this VCMP uses.
fn desktop_config() -> String {
    let program = std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "vcmp".into());
    let mut server = serde_json::json!({ "command": program, "args": ["mcp"] });
    if let Some(home) = std::env::var_os("VCMP_HOME") {
        let home = std::fs::canonicalize(&home).unwrap_or_else(|_| PathBuf::from(home));
        server["env"] = serde_json::json!({ "VCMP_HOME": home.to_string_lossy() });
    }
    format!("\"vcmp\": {server}")
}

/// What agents ask for, brought from the server's thread to the service on the UI
/// thread, so the window shows it at once. Each call waits for the window to take it.
pub struct AppChanges;

impl AppChanges {
    fn on_ui<T: Send + 'static>(
        work: impl FnOnce(&AgentService) -> vcmp_library::Result<T> + Send + 'static,
    ) -> Result<T, String> {
        let (sender, receiver) = mpsc::channel();
        slint::invoke_from_event_loop(move || {
            let result = match SERVICE.with(|current| current.borrow().upgrade()) {
                Some(service) => work(&service).map_err(|error| error.to_string()),
                None => Err("VCMP is closing".to_owned()),
            };
            let _ = sender.send(result);
        })
        .map_err(|_| "VCMP is closing".to_owned())?;
        receiver
            .recv_timeout(BRIDGE_TIMEOUT)
            .map_err(|_| "VCMP didn't answer in time".to_owned())?
    }
}

impl Changes for AppChanges {
    fn submit_tags(
        &self,
        agent: &str,
        edits: Vec<(TrackId, TagEdit)>,
    ) -> Result<ChangeSetId, String> {
        let agent = agent.to_owned();
        Self::on_ui(move |service| service.submit_tags(&agent, &edits))
    }

    fn submit_labels(
        &self,
        agent: &str,
        edits: Vec<(TrackId, LabelEdit)>,
        new_labels: Vec<NewLabel>,
    ) -> Result<ChangeSetId, String> {
        let agent = agent.to_owned();
        Self::on_ui(move |service| service.submit_labels(&agent, &edits, &new_labels))
    }

    fn submit_playlist_deletion(
        &self,
        agent: &str,
        playlist: PlaylistId,
    ) -> Result<ChangeSetId, String> {
        let agent = agent.to_owned();
        Self::on_ui(move |service| service.submit_playlist_deletion(&agent, playlist))
    }

    fn submit_undo(&self, agent: &str, change: ChangeSetId) -> Result<ChangeSetId, String> {
        let agent = agent.to_owned();
        Self::on_ui(move |service| service.submit_undo(&agent, change))
    }

    fn edit_playlist(&self, edit: PlaylistEdit) -> Result<Option<PlaylistId>, String> {
        Self::on_ui(move |service| service.edit_playlist(edit))
    }
}

fn kind(summary: &ChangeSetSummary) -> ChangeKind {
    match summary.kind {
        Kind::PlaylistDeletion { .. } => ChangeKind::Playlist,
        Kind::Tags if summary.undoes.is_some() => ChangeKind::Undo,
        Kind::Tags => ChangeKind::Tags,
        Kind::Labels if summary.undoes.is_some() => ChangeKind::LabelUndo,
        Kind::Labels => ChangeKind::Labels,
    }
}

fn entry(
    set: &ChangeSetSummary,
    now: SystemTime,
    undo_waiting: &HashSet<ChangeSetId>,
) -> ChangeEntry {
    let minutes = now
        .duration_since(set.created)
        .unwrap_or_default()
        .as_secs()
        / 60;
    ChangeEntry {
        id: set.id.0 as i32,
        kind: kind(set),
        files: set.file_count as i32,
        playlist: match &set.kind {
            Kind::PlaylistDeletion { name, .. } => name.as_str().into(),
            Kind::Tags | Kind::Labels => SharedString::default(),
        },
        agent: set.agent.as_str().into(),
        minutes_ago: i32::try_from(minutes).unwrap_or(i32::MAX),
        status: match set.status {
            Status::Pending => ChangeStatus::Pending,
            Status::Accepted => ChangeStatus::Accepted,
            Status::PartlyWritten => ChangeStatus::PartlyWritten,
            Status::Rejected => ChangeStatus::Rejected,
            Status::Discarded => ChangeStatus::Discarded,
            Status::Undone => ChangeStatus::Undone,
        },
        can_undo: matches!(set.kind, Kind::Tags | Kind::Labels)
            && matches!(set.status, Status::Accepted | Status::PartlyWritten)
            && !undo_waiting.contains(&set.id),
    }
}

/// Where each of the nine fields' edits shows in the tag list: its field, and the key
/// its values are listed under.
const FIELDS: [(&str, ChangeField, &str); 9] = [
    ("title", ChangeField::Title, "TrackTitle"),
    ("artist", ChangeField::Artist, "TrackArtist"),
    ("album", ChangeField::Album, "AlbumTitle"),
    ("album_artist", ChangeField::AlbumArtist, "AlbumArtist"),
    ("track_number", ChangeField::Track, "TrackNumber"),
    ("disc_number", ChangeField::Disc, "DiscNumber"),
    ("year", ChangeField::Year, "RecordingDate"),
    ("genre", ChangeField::Genre, "Genre"),
    ("cover", ChangeField::Cover, ""),
];

/// One change of one file, before it becomes a row: where it sorts, and what makes
/// it the same as another file's.
#[derive(Clone, PartialEq)]
struct Change {
    order: i32,
    key: String,
    before: String,
    after: String,
    field: ChangeField,
    // The kept images of a change of cover.
    before_cover: Option<PathBuf>,
    after_cover: Option<PathBuf>,
}

/// The review's changes: those every file shares (only when there are several
/// files), then each file's own, by path. A file whose changes are all shared has no
/// group. Rows go in form order, then by key.
fn review_groups(library: &Library, files: &[ChangedFile]) -> (Vec<ChangeRow>, Vec<ChangeGroup>) {
    let changes: Vec<Vec<Change>> = files
        .iter()
        .map(|file| file_changes(library, file))
        .collect();
    let shared: Vec<Change> = match changes.split_first() {
        Some((first, rest)) if !rest.is_empty() => first
            .iter()
            .filter(|change| rest.iter().all(|other| other.contains(change)))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    let mut images: HashMap<PathBuf, slint::Image> = HashMap::new();
    let mut row = |change: &Change| {
        let mut image = |path: &Option<PathBuf>| match path {
            Some(path) => {
                let image = images
                    .entry(path.clone())
                    .or_insert_with(|| slint::Image::load_from_path(path).unwrap_or_default());
                (true, image.clone())
            }
            None => (false, slint::Image::default()),
        };
        let (has_before_cover, before_cover) = image(&change.before_cover);
        let (has_after_cover, after_cover) = image(&change.after_cover);
        ChangeRow {
            field: change.field,
            key: change.key.as_str().into(),
            before: change.before.as_str().into(),
            after: change.after.as_str().into(),
            has_before_cover,
            before_cover,
            has_after_cover,
            after_cover,
        }
    };
    let shared_rows = shared.iter().map(&mut row).collect();
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by(|a, b| files[*a].path.cmp(&files[*b].path));
    let groups = order
        .into_iter()
        .filter_map(|at| {
            let own: Vec<ChangeRow> = changes[at]
                .iter()
                .filter(|change| !shared.contains(change))
                .map(&mut row)
                .collect();
            if own.is_empty() {
                return None;
            }
            let file = &files[at];
            let title = library
                .track(file.track)
                .ok()
                .flatten()
                .map(|track| track.title)
                .unwrap_or_else(|| file_name(&file.path));
            Some(ChangeGroup {
                title: title.into(),
                file: file_name(&file.path).into(),
                rows: ModelRc::new(VecModel::from(own)),
            })
        })
        .collect();
    (shared_rows, groups)
}

/// A file's changes in form order, then by key.
fn file_changes(library: &Library, file: &ChangedFile) -> Vec<Change> {
    let before: HashMap<&str, &Vec<String>> = file
        .values
        .iter()
        .map(|value| (value.key.as_str(), &value.before))
        .collect();
    let mut changes: Vec<Change> = file
        .edits
        .iter()
        .map(|(target, values)| {
            let (field, key, order) = field_of(target);
            if field == ChangeField::Cover {
                let before_cover = file.cover_before.clone().flatten();
                let after_cover = values
                    .as_ref()
                    .and_then(|names| names.first())
                    .map(|name| library.kept_cover(name));
                return Change {
                    order,
                    key,
                    before: String::new(),
                    after: String::new(),
                    field,
                    before_cover,
                    after_cover,
                };
            }
            // A label's values are listed under its whole target, `label:` and name.
            let listed = if field == ChangeField::Label {
                target.as_str()
            } else {
                key.as_str()
            };
            Change {
                order,
                before: before
                    .get(listed)
                    .map(|values| values.join("; "))
                    .unwrap_or_default(),
                after: values
                    .as_ref()
                    .map(|values| values.join("; "))
                    .unwrap_or_default(),
                key,
                field,
                before_cover: None,
                after_cover: None,
            }
        })
        .collect();
    changes.sort_by(|a, b| a.order.cmp(&b.order).then(a.key.cmp(&b.key)));
    changes
}

/// A target's field, the key its values are listed under, and its place in form order
/// (the other tags after the nine fields, labels last). An undo names the nine fields
/// by their keys.
fn field_of(target: &str) -> (ChangeField, String, i32) {
    if let Some(name) = target.strip_prefix("label:") {
        return (ChangeField::Label, name.to_owned(), FIELDS.len() as i32 + 1);
    }
    if let Some(key) = target.strip_prefix("tag:") {
        return match FIELDS
            .iter()
            .position(|(_, _, field_key)| *field_key == key && !key.is_empty())
        {
            Some(at) => (FIELDS[at].1, key.to_owned(), at as i32),
            None => (ChangeField::Other, key.to_owned(), FIELDS.len() as i32),
        };
    }
    match FIELDS.iter().position(|(name, _, _)| *name == target) {
        Some(at) => (FIELDS[at].1, FIELDS[at].2.to_owned(), at as i32),
        None => (ChangeField::Other, target.to_owned(), FIELDS.len() as i32),
    }
}

fn show_results(state: &AgentState, set: &ChangeSet) {
    let problems: Vec<ChangeProblem> = set
        .files
        .iter()
        .filter_map(|file| match &file.result {
            Some(FileResult::Conflict(_)) => Some(ChangeProblem {
                file: file_name(&file.path).into(),
                conflict: true,
            }),
            Some(FileResult::Failed(_)) => Some(ChangeProblem {
                file: file_name(&file.path).into(),
                conflict: false,
            }),
            _ => None,
        })
        .collect();
    let written = set
        .files
        .iter()
        .filter(|file| file.result == Some(FileResult::Written))
        .count();
    state.set_review_written(written as i32);
    state.set_review_problems(ModelRc::new(VecModel::from(problems)));
    state.set_review_status(ReviewStatus::Done);
}

fn log_problems(set: &ChangeSet) {
    for file in &set.files {
        match &file.result {
            Some(FileResult::Conflict(detail)) => {
                log::info!("{}: left as it was: {detail}", file.path.display());
            }
            Some(FileResult::Failed(detail)) => {
                log::warn!("{}: not written: {detail}", file.path.display());
            }
            _ => {}
        }
    }
}

/// A size in bytes for people: "640 KB", "2.4 MB".
fn size_text(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    let bytes = bytes as f64;
    if bytes < KB * KB {
        format!("{} KB", (bytes / KB).ceil())
    } else if bytes < KB * KB * KB {
        format!("{:.1} MB", bytes / (KB * KB))
    } else {
        format!("{:.1} GB", bytes / (KB * KB * KB))
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_people_write_them() {
        assert_eq!(size_text(1), "1 KB");
        assert_eq!(size_text(640_000), "640 KB");
        assert_eq!(size_text(2_400_000), "2.4 MB");
        assert_eq!(size_text(1_100_000_000), "1.1 GB");
    }

    #[test]
    fn an_undo_names_the_fields_by_their_keys() {
        assert_eq!(
            field_of("tag:TrackTitle"),
            (ChangeField::Title, "TrackTitle".to_owned(), 0)
        );
        assert_eq!(
            field_of("year"),
            (ChangeField::Year, "RecordingDate".to_owned(), 6)
        );
        assert_eq!(
            field_of("tag:Composer"),
            (ChangeField::Other, "Composer".to_owned(), 9)
        );
    }
}
