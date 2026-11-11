pub mod config;

mod api;
mod audio;
mod tokens;
mod worker;

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use vcmp_library::Library;
use vcmp_protocol::Role;

pub use config::{Config, ConfigError};

use crate::api::Failure;
use crate::tokens::Tokens;
use crate::worker::{Job, State};

const TOKEN_REFRESH: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("the data folder {0} can't be made: {1}")]
    Data(std::path::PathBuf, std::io::Error),
    #[error("the library could not be opened: {0}")]
    Library(#[from] vcmp_library::Error),
    #[error("{0} can't be listened on: {1}")]
    Listen(SocketAddr, std::io::Error),
    #[error("the server's threads could not start: {0}")]
    Thread(std::io::Error),
}

pub struct Server {
    address: SocketAddr,
    server_id: String,
    state: Arc<State>,
    cancel: CancellationToken,
    http: Option<JoinHandle<()>>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    pub fn start(config: Config) -> Result<Self, StartError> {
        std::fs::create_dir_all(&config.data)
            .map_err(|error| StartError::Data(config.data.clone(), error))?;
        let library = Library::open(&config.database(), &config.cache())?;
        let (server_id, made) = tokens::ensure(&library)?;
        if made {
            log::info!("new tokens were made; `vcmp-server tokens` shows them");
        }
        let mut configured = Vec::new();
        for folder in &config.music {
            match library.add_source(folder, true) {
                Ok(source) => configured.push(source.path),
                Err(error) => log::warn!(
                    "the configured music folder {} can't be added: {error}",
                    folder.display()
                ),
            }
        }
        let tokens = Tokens::new(
            config.user_token.clone(),
            config.admin_token.clone(),
            &library,
        )?;

        let listener = std::net::TcpListener::bind(config.listen)
            .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
            .map_err(|error| StartError::Listen(config.listen, error))?;
        let address = listener
            .local_addr()
            .map_err(|error| StartError::Listen(config.listen, error))?;

        let (jobs, receiver) = mpsc::channel();
        let state = Arc::new(State::new(
            server_id.clone(),
            config.name.clone(),
            tokens,
            configured,
            jobs,
            config.database(),
            config.cache(),
        ));
        let worker = {
            let state = state.clone();
            let rescan = config.rescan_interval;
            thread::Builder::new()
                .name("vcmp-server-library".into())
                .spawn(move || worker::run(library, state, receiver, rescan))
                .map_err(StartError::Thread)?
        };
        let cancel = CancellationToken::new();
        let http = {
            let state = state.clone();
            let cancel = cancel.clone();
            thread::Builder::new()
                .name("vcmp-server-http".into())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            log::error!("the server's runtime could not start: {error}");
                            return;
                        }
                    };
                    if let Err(error) = runtime.block_on(serve(listener, state, cancel)) {
                        log::error!("the server stopped: {error}");
                    }
                })
                .map_err(StartError::Thread)?
        };
        Ok(Self {
            address,
            server_id,
            state,
            cancel,
            http: Some(http),
            worker: Some(worker),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.state.cancel.store(true, Ordering::Relaxed);
        let _ = self.state.jobs.send(Job::Stop);
        self.cancel.cancel();
        for thread in [self.http.take(), self.worker.take()].into_iter().flatten() {
            let _ = thread.join();
        }
    }
}

async fn serve(
    listener: std::net::TcpListener,
    state: Arc<State>,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let refresh = {
        let state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(TOKEN_REFRESH);
            loop {
                interval.tick().await;
                let state = state.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    state.read(|library| state.tokens.reload(library).map_err(Failure::internal))
                })
                .await;
            }
        })
    };
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let result = axum::serve(listener, api::router(state))
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await;
    refresh.abort();
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenInfo {
    pub role: Role,
    pub token: String,
    pub configured: bool,
}

pub fn tokens(config: &Config) -> Result<(String, Vec<TokenInfo>), StartError> {
    std::fs::create_dir_all(&config.data)
        .map_err(|error| StartError::Data(config.data.clone(), error))?;
    let library = Library::open(&config.database(), &config.cache())?;
    let (server_id, _) = tokens::ensure(&library)?;
    let mut list = Vec::new();
    for (role, configured) in [
        (Role::User, &config.user_token),
        (Role::Admin, &config.admin_token),
    ] {
        let token = match configured {
            Some(token) => token.clone(),
            None => tokens::stored(&library, role)?.unwrap_or_default(),
        };
        list.push(TokenInfo {
            role,
            token,
            configured: configured.is_some(),
        });
    }
    Ok((server_id, list))
}

#[derive(Debug, thiserror::Error)]
pub enum ReplaceError {
    #[error("the {} token is set by the configuration; change it there", .0.as_str())]
    Configured(Role),
    #[error(transparent)]
    Start(#[from] StartError),
}

pub fn replace_token(config: &Config, role: Role) -> Result<String, ReplaceError> {
    let configured = match role {
        Role::User => config.user_token.is_some(),
        Role::Admin => config.admin_token.is_some(),
    };
    if configured {
        return Err(ReplaceError::Configured(role));
    }
    std::fs::create_dir_all(&config.data)
        .map_err(|error| StartError::Data(config.data.clone(), error))?;
    let library = Library::open(&config.database(), &config.cache()).map_err(StartError::from)?;
    tokens::ensure(&library).map_err(StartError::from)?;
    Ok(tokens::replace(&library, role).map_err(StartError::from)?)
}
