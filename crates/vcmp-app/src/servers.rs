use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, VecModel};
use vcmp_client::{
    ClientError, Connection, ServerSource, SyncError, SyncReport, TokenStore, encode_cover,
    normalize_address, sends_token_in_clear, sync,
};
use vcmp_library::{
    FailedTagWrite, FieldEdit, Library, ServerId, TagEdit, TagWriteError, TagWriteReport, Track,
};
use vcmp_protocol::{Handshake, Role};

use crate::library_service::{Delivery, LibraryPaths};
use crate::library_view::LibraryView;
use crate::{
    AppWindow, MusicServerError as ServerError, MusicServerFolder, MusicServerItem,
    MusicServerScan, MusicServerStatus as ServerStatus, MusicServers,
};

pub const SETTING_CACHE_LIMIT_MB: &str = "servers.cache_limit_mb";

pub const DEFAULT_CACHE_LIMIT_MB: u64 = vcmp_client::DEFAULT_CACHE_LIMIT / (1024 * 1024);

const SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);

enum AdminOutcome {
    Changed,
    Token(vcmp_protocol::NewToken),
}

struct AdminView {
    folders: Vec<vcmp_protocol::Folder>,
    scan: vcmp_protocol::ScanStatus,
}

const ADMIN_POLL: Duration = Duration::from_secs(2);

enum Message {
    Admin(ServerId, Result<AdminView, String>),
    AdminDone(ServerId, Result<AdminOutcome, String>),
    Progress(ServerId, usize),
    Synced(
        ServerId,
        Result<(Handshake, SyncReport), (ServerStatus, String)>,
    ),
    Checked(Result<(String, Handshake), (ServerError, String)>),
}

#[derive(Clone)]
struct Status {
    kind: ServerStatus,
    detail: String,
    role: Role,
    can_write: bool,
    received: usize,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            kind: ServerStatus::Connecting,
            detail: String::new(),
            role: Role::User,
            can_write: false,
            received: 0,
        }
    }
}

pub struct ServerService {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    paths: LibraryPaths,
    view: Rc<LibraryView>,
    tokens: Arc<dyn TokenStore>,
    source: Option<ServerSource>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    notify: Arc<dyn Fn() + Send + Sync>,
    states: RefCell<HashMap<ServerId, Status>>,
    connections: RefCell<HashMap<ServerId, Connection>>,
    syncing: RefCell<HashSet<ServerId>>,
    pending_token: RefCell<Option<String>>,
    timer: slint::Timer,
    adding: Cell<bool>,
    managing: Cell<Option<ServerId>>,
    scanning: Cell<bool>,
    admin_timer: slint::Timer,
}

thread_local! {
    static SERVICE: RefCell<std::rc::Weak<ServerService>> = RefCell::default();
    static MARKS: RefCell<HashMap<ServerId, (String, bool)>> = RefCell::default();
}

static CONNECTIONS: LazyLock<Mutex<HashMap<ServerId, Connection>>> = LazyLock::new(Mutex::default);

pub fn connection_for(server: ServerId) -> Option<Connection> {
    CONNECTIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&server)
        .cloned()
}

fn register(server: ServerId, connection: Option<Connection>) {
    let mut connections = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    match connection {
        Some(connection) => connections.insert(server, connection),
        None => connections.remove(&server),
    };
}

pub fn mark(server: Option<ServerId>) -> (slint::SharedString, bool) {
    let Some(server) = server else {
        return (slint::SharedString::default(), false);
    };
    MARKS.with(|marks| {
        marks
            .borrow()
            .get(&server)
            .map(|(name, away)| (name.as_str().into(), *away))
            .unwrap_or_default()
    })
}

impl ServerService {
    pub fn connect(
        window: &AppWindow,
        library: Rc<Library>,
        paths: LibraryPaths,
        view: Rc<LibraryView>,
        tokens: Arc<dyn TokenStore>,
        source: Option<ServerSource>,
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
            tokens,
            source,
            sender,
            receiver,
            notify,
            states: RefCell::default(),
            connections: RefCell::default(),
            syncing: RefCell::default(),
            pending_token: RefCell::default(),
            timer: slint::Timer::default(),
            adding: Cell::new(false),
            managing: Cell::new(None),
            scanning: Cell::new(false),
            admin_timer: slint::Timer::default(),
        });
        SERVICE.with(|current| *current.borrow_mut() = Rc::downgrade(&service));

        let state = window.global::<MusicServers>();
        let weak = Rc::downgrade(&service);
        state.on_add({
            let weak = weak.clone();
            move |address, token, confirmed| {
                if let Some(service) = weak.upgrade() {
                    service.add(&address, &token, confirmed);
                }
            }
        });
        state.on_remove({
            let weak = weak.clone();
            move |id| {
                if let Some(service) = weak.upgrade() {
                    service.remove(ServerId(i64::from(id)));
                }
            }
        });
        state.on_sync({
            let weak = weak.clone();
            move |id| {
                if let Some(service) = weak.upgrade() {
                    service.start_sync(ServerId(i64::from(id)));
                }
            }
        });
        state.on_set_cache_limit({
            let weak = weak.clone();
            move |megabytes| {
                if let Some(service) = weak.upgrade() {
                    service.set_cache_limit(megabytes.max(0) as u64);
                }
            }
        });
        state.on_clear_cache({
            let weak = weak.clone();
            move || {
                if let Some(service) = weak.upgrade() {
                    service.clear_cache();
                }
            }
        });
        state.on_dismiss({
            let weak = weak.clone();
            move || {
                if let Some(service) = weak.upgrade() {
                    service.set_error(ServerError::None, "");
                    if let Some(window) = service.window.upgrade() {
                        window.global::<MusicServers>().set_insecure(false);
                    }
                }
            }
        });

        state.on_manage({
            let weak = weak.clone();
            move |id| {
                if let Some(service) = weak.upgrade() {
                    service.manage(ServerId(i64::from(id)));
                }
            }
        });
        state.on_add_server_folder({
            let weak = weak.clone();
            move |folder| {
                if let Some(service) = weak.upgrade() {
                    let folder = folder.trim().to_owned();
                    if !folder.is_empty() {
                        service.admin_job(move |connection| {
                            connection
                                .add_folder(&folder, true)
                                .map(|_| AdminOutcome::Changed)
                        });
                    }
                }
            }
        });
        state.on_remove_server_folder({
            let weak = weak.clone();
            move |id| {
                if let Some(service) = weak.upgrade() {
                    service.admin_job(move |connection| {
                        connection
                            .remove_folder(i64::from(id))
                            .map(|()| AdminOutcome::Changed)
                    });
                }
            }
        });
        state.on_rescan_server({
            let weak = weak.clone();
            move || {
                if let Some(service) = weak.upgrade() {
                    service.admin_job(|connection| {
                        connection.start_scan().map(|_| AdminOutcome::Changed)
                    });
                }
            }
        });
        state.on_replace_token({
            let weak = weak.clone();
            move |admin| {
                if let Some(service) = weak.upgrade() {
                    let role = if admin { Role::Admin } else { Role::User };
                    service.admin_job(move |connection| {
                        connection.new_token(role).map(AdminOutcome::Token)
                    });
                }
            }
        });

        let limit = service.cache_limit_mb();
        state.set_cache_limit_mb(limit as i32);
        if let Some(source) = &service.source {
            source.set_limit(limit * 1024 * 1024);
        }
        for server in service.library.servers().unwrap_or_default() {
            service.open(&server);
        }
        if service.update_marks() {
            service.view.reload();
        }
        service.show();
        service.sync_all();
        let weak = Rc::downgrade(&service);
        service
            .timer
            .start(slint::TimerMode::Repeated, SYNC_INTERVAL, move || {
                if let Some(service) = weak.upgrade() {
                    service.sync_all();
                }
            });
        service
    }

    pub fn post(job: impl FnOnce(&ServerService) + Send + 'static) {
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(service) = SERVICE.with(|current| current.borrow().upgrade()) {
                job(&service);
            }
        });
    }

    pub fn connection(&self, server: ServerId) -> Option<Connection> {
        self.connections.borrow().get(&server).cloned()
    }

    pub fn is_admin(&self, server: ServerId) -> bool {
        self.states
            .borrow()
            .get(&server)
            .is_some_and(|status| status.role == Role::Admin)
    }

    pub fn away(&self) -> HashSet<ServerId> {
        self.states
            .borrow()
            .iter()
            .filter(|(_, status)| {
                matches!(
                    status.kind,
                    ServerStatus::Away
                        | ServerStatus::Refused
                        | ServerStatus::Outdated
                        | ServerStatus::OtherServer
                )
            })
            .map(|(id, _)| *id)
            .collect()
    }

    fn open(&self, server: &vcmp_library::Server) {
        let token = match self.tokens.load(&server.server_id) {
            Ok(Some(token)) => token,
            Ok(None) => {
                self.set_status(
                    server.id,
                    ServerStatus::Refused,
                    "no token is stored for this server".into(),
                );
                return;
            }
            Err(error) => {
                self.set_status(server.id, ServerStatus::Refused, error);
                return;
            }
        };
        match Connection::new(&server.address, &token) {
            Ok(connection) => {
                if let Some(source) = &self.source {
                    source.set_connection(server.id, Some(connection.clone()));
                }
                register(server.id, Some(connection.clone()));
                self.connections.borrow_mut().insert(server.id, connection);
                self.states.borrow_mut().entry(server.id).or_default();
            }
            Err(error) => self.set_status(server.id, ServerStatus::Away, error.to_string()),
        }
    }

    fn set_status(&self, server: ServerId, kind: ServerStatus, detail: String) {
        let mut states = self.states.borrow_mut();
        let status = states.entry(server).or_default();
        status.kind = kind;
        status.detail = detail;
    }

    pub fn sync_all(&self) {
        let ids: Vec<ServerId> = self.connections.borrow().keys().copied().collect();
        for id in ids {
            self.start_sync(id);
        }
    }

    pub fn start_sync(&self, id: ServerId) {
        let Some(connection) = self.connection(id) else {
            return;
        };
        if !self.syncing.borrow_mut().insert(id) {
            return;
        }
        {
            let mut states = self.states.borrow_mut();
            let status = states.entry(id).or_default();
            status.received = 0;
            if status.kind != ServerStatus::Online {
                status.kind = ServerStatus::Connecting;
            }
        }
        self.show();
        let paths = self.paths.clone();
        let sender = self.sender.clone();
        let notify = self.notify.clone();
        let spawned = thread::Builder::new()
            .name("vcmp-server-sync".into())
            .spawn(move || {
                let result = run_sync(&paths, id, &connection, |received| {
                    let _ = sender.send(Message::Progress(id, received));
                    notify();
                });
                let _ = sender.send(Message::Synced(id, result));
                notify();
            });
        if let Err(error) = spawned {
            self.syncing.borrow_mut().remove(&id);
            self.set_status(id, ServerStatus::Away, error.to_string());
            self.show();
        }
    }

    pub fn add(&self, address: &str, token: &str, confirmed: bool) {
        let address = match normalize_address(address) {
            Ok(address) => address,
            Err(error) => {
                self.set_error(ServerError::Address, &error.to_string());
                return;
            }
        };
        if token.trim().is_empty() {
            self.set_error(ServerError::Token, "");
            return;
        }
        if sends_token_in_clear(&address) && !confirmed {
            if let Some(window) = self.window.upgrade() {
                window.global::<MusicServers>().set_insecure(true);
            }
            return;
        }
        let connection = match Connection::new(&address, token) {
            Ok(connection) => connection,
            Err(error) => {
                self.set_error(ServerError::Address, &error.to_string());
                return;
            }
        };
        self.set_error(ServerError::None, "");
        self.adding.set(true);
        *self.pending_token.borrow_mut() = Some(token.trim().to_owned());
        self.show();
        let sender = self.sender.clone();
        let notify = self.notify.clone();
        let _ = thread::Builder::new()
            .name("vcmp-server-check".into())
            .spawn(move || {
                let result = connection
                    .handshake()
                    .map(|handshake| (address, handshake))
                    .map_err(|error| (connect_error(&error), error.to_string()));
                let _ = sender.send(Message::Checked(result));
                notify();
            });
    }

    fn checked(&self, result: Result<(String, Handshake), (ServerError, String)>) {
        self.adding.set(false);
        let token = self.pending_token.borrow_mut().take().unwrap_or_default();
        let (address, handshake) = match result {
            Ok(found) => found,
            Err((kind, detail)) => {
                self.set_error(kind, &detail);
                self.show();
                return;
            }
        };
        if let Err(error) = self.tokens.save(&handshake.server_id, &token) {
            self.set_error(ServerError::Keyring, &error);
            self.show();
            return;
        }
        let server = match self
            .library
            .add_server(&handshake.server_id, &handshake.name, &address)
        {
            Ok(server) => server,
            Err(error) => {
                self.set_error(ServerError::Library, &error.to_string());
                self.show();
                return;
            }
        };
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<MusicServers>();
            state.set_address("".into());
            state.set_token("".into());
            state.set_insecure(false);
        }
        self.open(&server);
        self.start_sync(server.id);
    }

    pub fn remove(&self, id: ServerId) {
        let Ok(Some(server)) = self.library.server(id) else {
            return;
        };
        if let Err(error) = self.library.remove_server(id) {
            self.set_error(ServerError::Library, &error.to_string());
            self.show();
            return;
        }
        if let Err(error) = self.tokens.delete(&server.server_id) {
            log::warn!("the token of {} could not be deleted: {error}", server.name);
        }
        self.connections.borrow_mut().remove(&id);
        self.states.borrow_mut().remove(&id);
        register(id, None);
        if let Some(source) = &self.source {
            source.set_connection(id, None);
        }
        self.update_marks();
        self.view.reload();
        self.show();
    }

    pub fn manage(&self, id: ServerId) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<MusicServers>();
        state.set_admin_new_token("".into());
        state.set_admin_error("".into());
        if self.managing.get() == Some(id) {
            self.managing.set(None);
            state.set_managing(-1);
            return;
        }
        self.managing.set(Some(id));
        state.set_managing(id.0 as i32);
        state.set_admin_folders(ModelRc::default());
        state.set_admin_scan(MusicServerScan::default());
        self.refresh_admin(id);
    }

    fn refresh_admin(&self, id: ServerId) {
        let Some(connection) = self.connection(id) else {
            return;
        };
        self.set_admin_busy(true);
        let sender = self.sender.clone();
        let notify = self.notify.clone();
        let _ = thread::Builder::new()
            .name("vcmp-server-admin".into())
            .spawn(move || {
                let view = connection
                    .folders()
                    .and_then(|folders| {
                        connection
                            .scan_status()
                            .map(|scan| AdminView { folders, scan })
                    })
                    .map_err(|error| error.to_string());
                let _ = sender.send(Message::Admin(id, view));
                notify();
            });
    }

    fn admin_job(
        &self,
        job: impl FnOnce(&Connection) -> Result<AdminOutcome, ClientError> + Send + 'static,
    ) {
        let Some(id) = self.managing.get() else {
            return;
        };
        let Some(connection) = self.connection(id) else {
            return;
        };
        self.set_admin_busy(true);
        let sender = self.sender.clone();
        let notify = self.notify.clone();
        let _ = thread::Builder::new()
            .name("vcmp-server-admin".into())
            .spawn(move || {
                let result = job(&connection).map_err(|error| error.to_string());
                let _ = sender.send(Message::AdminDone(id, result));
                notify();
            });
    }

    fn set_admin_busy(&self, busy: bool) {
        if let Some(window) = self.window.upgrade() {
            window.global::<MusicServers>().set_admin_busy(busy);
        }
    }

    fn show_admin(&self, id: ServerId, view: Result<AdminView, String>) {
        self.set_admin_busy(false);
        if self.managing.get() != Some(id) {
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<MusicServers>();
        let view = match view {
            Ok(view) => view,
            Err(error) => {
                state.set_admin_error(error.into());
                return;
            }
        };
        let folders: Vec<MusicServerFolder> = view
            .folders
            .iter()
            .map(|folder| MusicServerFolder {
                id: folder.id as i32,
                name: folder.name.as_str().into(),
                path: folder
                    .path
                    .clone()
                    .unwrap_or_else(|| folder.name.clone())
                    .into(),
                configured: folder.configured,
                reachable: folder.reachable,
            })
            .collect();
        state.set_admin_folders(ModelRc::new(VecModel::from(folders)));
        let scan = match &view.scan.last {
            Some(last) => MusicServerScan {
                known: true,
                running: view.scan.running,
                added: last.added as i32,
                updated: last.updated as i32,
                gone: last.became_unavailable as i32,
                back: last.became_available as i32,
                skipped: last.skipped.len() as i32,
                unreachable: last.unreachable_folders.len() as i32,
                error: last.error.clone().unwrap_or_default().into(),
            },
            None => MusicServerScan {
                running: view.scan.running,
                ..MusicServerScan::default()
            },
        };
        state.set_admin_scan(scan);
        let was_scanning = self.scanning.replace(view.scan.running);
        if view.scan.running {
            let weak = SERVICE.with(|current| current.borrow().clone());
            self.admin_timer
                .start(slint::TimerMode::SingleShot, ADMIN_POLL, move || {
                    if let Some(service) = weak.upgrade()
                        && service.managing.get() == Some(id)
                    {
                        service.refresh_admin(id);
                    }
                });
        } else if was_scanning {
            self.start_sync(id);
        }
    }

    fn admin_done(&self, id: ServerId, result: Result<AdminOutcome, String>) {
        self.set_admin_busy(false);
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<MusicServers>();
        match result {
            Ok(AdminOutcome::Changed) => {
                state.set_admin_error("".into());
                state.set_admin_folder_path("".into());
                self.scanning.set(true);
            }
            Ok(AdminOutcome::Token(token)) => {
                state.set_admin_error("".into());
                state.set_admin_new_token(token.token.as_str().into());
                state.set_admin_new_token_is_admin(token.role == Role::Admin);
                if token.role == Role::Admin
                    && let Ok(Some(server)) = self.library.server(id)
                {
                    match self.tokens.save(&server.server_id, &token.token) {
                        Ok(()) => self.open(&server),
                        Err(error) => self.set_error(ServerError::Keyring, &error),
                    }
                }
            }
            Err(error) => state.set_admin_error(error.into()),
        }
        if self.managing.get() == Some(id) {
            self.refresh_admin(id);
        }
    }

    fn cache_limit_mb(&self) -> u64 {
        self.library
            .setting(SETTING_CACHE_LIMIT_MB)
            .ok()
            .flatten()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_CACHE_LIMIT_MB)
    }

    pub fn set_cache_limit(&self, megabytes: u64) {
        if let Err(error) = self
            .library
            .set_setting(SETTING_CACHE_LIMIT_MB, &megabytes.to_string())
        {
            self.set_error(ServerError::Library, &error.to_string());
        }
        if let Some(source) = &self.source {
            source.set_limit(megabytes * 1024 * 1024);
        }
        if let Some(window) = self.window.upgrade() {
            window
                .global::<MusicServers>()
                .set_cache_limit_mb(megabytes as i32);
        }
        self.show();
    }

    pub fn clear_cache(&self) {
        if let Some(source) = &self.source {
            source.clear();
        }
        self.show();
    }

    fn set_error(&self, kind: ServerError, detail: &str) {
        if let Some(window) = self.window.upgrade() {
            let state = window.global::<MusicServers>();
            state.set_error(kind);
            state.set_error_detail(detail.into());
        }
    }

    pub fn drain(&self) {
        while let Ok(message) = self.receiver.try_recv() {
            self.apply(message);
        }
    }

    pub fn wait_until_idle(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self
                .receiver
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            {
                Ok(message) => self.apply(message),
                Err(_) => return false,
            }
            if self.syncing.borrow().is_empty() && !self.adding.get() {
                self.drain();
                return true;
            }
        }
    }

    fn apply(&self, message: Message) {
        match message {
            Message::Progress(id, received) => {
                let mut states = self.states.borrow_mut();
                let status = states.entry(id).or_default();
                status.received = received;
                status.kind = ServerStatus::Syncing;
                drop(states);
                self.show();
            }
            Message::Synced(id, result) => {
                self.syncing.borrow_mut().remove(&id);
                if !self.connections.borrow().contains_key(&id) {
                    return;
                }
                let changed = {
                    let mut states = self.states.borrow_mut();
                    let status = states.entry(id).or_default();
                    match result {
                        Ok((handshake, report)) => {
                            status.kind = ServerStatus::Online;
                            status.detail.clear();
                            status.role = handshake.role;
                            status.can_write = handshake.can_write_tags;
                            if handshake.name
                                != self
                                    .library
                                    .server(id)
                                    .ok()
                                    .flatten()
                                    .map(|server| server.name)
                                    .unwrap_or_default()
                                && let Ok(Some(server)) = self.library.server(id)
                            {
                                let _ = self.library.rename_server(
                                    id,
                                    &handshake.name,
                                    &server.address,
                                );
                            }
                            report.changed > 0 || report.artwork > 0 || report.gone > 0
                        }
                        Err((kind, detail)) => {
                            status.kind = kind;
                            status.detail = detail;
                            false
                        }
                    }
                };
                if self.update_marks() || changed {
                    self.view.reload();
                }
                self.show();
            }
            Message::Checked(result) => self.checked(result),
            Message::Admin(id, view) => self.show_admin(id, view),
            Message::AdminDone(id, result) => self.admin_done(id, result),
        }
    }

    fn update_marks(&self) -> bool {
        let away = self.away();
        let marks: HashMap<ServerId, (String, bool)> = self
            .library
            .servers()
            .unwrap_or_default()
            .into_iter()
            .map(|server| {
                let gone = away.contains(&server.id);
                (server.id, (server.name, gone))
            })
            .collect();
        MARKS.with(|current| {
            let mut current = current.borrow_mut();
            let changed = *current != marks;
            *current = marks;
            changed
        })
    }

    fn show(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<MusicServers>();
        let states = self.states.borrow();
        let items: Vec<MusicServerItem> = self
            .library
            .servers()
            .unwrap_or_default()
            .into_iter()
            .map(|server| {
                let status = states.get(&server.id).cloned().unwrap_or_default();
                MusicServerItem {
                    id: server.id.0 as i32,
                    name: server.name.as_str().into(),
                    address: server.address.as_str().into(),
                    status: status.kind,
                    detail: status.detail.as_str().into(),
                    admin: status.role == Role::Admin,
                    can_write: status.can_write,
                    received: status.received as i32,
                }
            })
            .collect();
        state.set_servers(ModelRc::new(VecModel::from(items)));
        state.set_adding(self.adding.get());
        let used = self.source.as_ref().map_or(0, ServerSource::size);
        state.set_cache_used_mb((used / (1024 * 1024)) as i32);
    }
}

fn connect_error(error: &ClientError) -> ServerError {
    match error {
        ClientError::Unauthorized => ServerError::Token,
        ClientError::Protocol { .. } => ServerError::Outdated,
        ClientError::Address(_) => ServerError::Address,
        _ => ServerError::Unreachable,
    }
}

fn run_sync(
    paths: &LibraryPaths,
    id: ServerId,
    connection: &Connection,
    progress: impl FnMut(usize),
) -> Result<(Handshake, SyncReport), (ServerStatus, String)> {
    let mut progress = progress;
    let mut library = Library::open(&paths.database, &paths.cache)
        .map_err(|error| (ServerStatus::Away, error.to_string()))?;
    let server = library
        .server(id)
        .map_err(|error| (ServerStatus::Away, error.to_string()))?
        .ok_or_else(|| (ServerStatus::Away, "the server was removed".to_owned()))?;
    sync(
        &mut library,
        &server,
        connection,
        &AtomicBool::new(false),
        |step| progress(step.received),
    )
    .map_err(|error| {
        let kind = match &error {
            SyncError::Client(ClientError::Unauthorized) => ServerStatus::Refused,
            SyncError::Client(ClientError::Protocol { .. }) => ServerStatus::Outdated,
            SyncError::OtherServer => ServerStatus::OtherServer,
            _ => ServerStatus::Away,
        };
        (kind, error.to_string())
    })
}

pub fn write_tags(paths: &LibraryPaths, edits: &[(Track, TagEdit)]) -> TagWriteReport {
    let mut report = TagWriteReport::default();
    let mut by_server: HashMap<ServerId, Vec<(&Track, &TagEdit)>> = HashMap::new();
    for (track, edit) in edits {
        if let Some(remote) = &track.remote {
            by_server
                .entry(remote.server)
                .or_default()
                .push((track, edit));
        }
    }
    for (server, edits) in by_server {
        let fail_all = |report: &mut TagWriteReport, error: TagWriteError| {
            for (track, _) in &edits {
                report.failed.push(FailedTagWrite {
                    track: track.id,
                    path: None,
                    error: copy_error(&error),
                });
            }
        };
        let Some(connection) = connection_for(server) else {
            fail_all(
                &mut report,
                TagWriteError::Write("the track's server is not connected".into()),
            );
            continue;
        };
        let mut groups = Vec::new();
        let mut ids = HashMap::new();
        for (track, edit) in &edits {
            let Some(remote) = &track.remote else {
                continue;
            };
            ids.insert(remote.id, track.id);
            groups.push(vcmp_protocol::TagWriteGroup {
                tracks: vec![remote.id],
                edit: remote_edit(edit),
            });
        }
        match connection.write_tags(&vcmp_protocol::TagWriteRequest { groups }) {
            Ok(result) => {
                report
                    .written
                    .extend(result.written.iter().filter_map(|id| ids.get(id).copied()));
                for failed in result.failed {
                    if let Some(track) = ids.get(&failed.track) {
                        report.failed.push(FailedTagWrite {
                            track: *track,
                            path: None,
                            error: local_error(failed.reason, failed.detail),
                        });
                    }
                }
            }
            Err(ClientError::Refused {
                code: vcmp_protocol::ErrorCode::AdminOnly,
                ..
            }) => fail_all(&mut report, TagWriteError::ReadOnly),
            Err(error) => fail_all(&mut report, TagWriteError::Write(error.to_string())),
        }
        if let Err(error) = resync(paths, server, &connection) {
            log::warn!("the server could not be synced after its tags were written: {error}");
        }
    }
    report
}

fn resync(paths: &LibraryPaths, server: ServerId, connection: &Connection) -> Result<(), String> {
    run_sync(paths, server, connection, |_| {})
        .map(|_| ())
        .map_err(|(_, detail)| detail)
}

fn remote_edit(edit: &TagEdit) -> vcmp_protocol::TagEdit {
    fn field<T: Clone>(edit: &FieldEdit<T>) -> vcmp_protocol::FieldEdit<T> {
        match edit {
            FieldEdit::Keep => vcmp_protocol::FieldEdit::Keep,
            FieldEdit::Set(value) => vcmp_protocol::FieldEdit::Set(value.clone()),
            FieldEdit::Clear => vcmp_protocol::FieldEdit::Clear,
        }
    }
    vcmp_protocol::TagEdit {
        title: field(&edit.title),
        artist: field(&edit.artist),
        album: field(&edit.album),
        album_artist: field(&edit.album_artist),
        track_number: field(&edit.track_number),
        disc_number: field(&edit.disc_number),
        year: field(&edit.year),
        genre: field(&edit.genre),
        cover: match &edit.cover {
            FieldEdit::Keep => vcmp_protocol::FieldEdit::Keep,
            FieldEdit::Set(data) => vcmp_protocol::FieldEdit::Set(encode_cover(data)),
            FieldEdit::Clear => vcmp_protocol::FieldEdit::Clear,
        },
    }
}

fn local_error(reason: vcmp_protocol::TagWriteFailure, detail: String) -> TagWriteError {
    use vcmp_protocol::TagWriteFailure as Failure;
    match reason {
        Failure::Unavailable => TagWriteError::Unavailable,
        Failure::ReadOnly => TagWriteError::ReadOnly,
        Failure::Unreadable => TagWriteError::Unreadable(detail),
        Failure::Verify => TagWriteError::Verify(detail),
        Failure::ChangedMeanwhile => TagWriteError::ChangedMeanwhile,
        Failure::Replace => TagWriteError::Replace(detail),
        Failure::UnsupportedKey => TagWriteError::UnsupportedKey(detail),
        Failure::Conflict => TagWriteError::Conflict(detail),
        Failure::Write | Failure::Other => TagWriteError::Write(detail),
    }
}

fn copy_error(error: &TagWriteError) -> TagWriteError {
    match error {
        TagWriteError::ReadOnly => TagWriteError::ReadOnly,
        error => TagWriteError::Write(error.to_string()),
    }
}
