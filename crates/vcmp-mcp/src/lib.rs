//! VCMP's MCP server: AI agents (Claude and other MCP clients) read the library and
//! ask for changes while VCMP is open. Must not depend on the UI crate.
//!
//! The server speaks MCP's Streamable HTTP on 127.0.0.1 only, and every request must
//! carry the token as `Authorization: Bearer <token>`. A request with a browser's
//! `Origin` header is refused, so a web page can't reach it through the user's
//! browser. It runs its own tokio runtime on a thread of its own, and reads the
//! library on a connection of its own.
//!
//! The server never writes to the library itself. Changes go through [`Changes`],
//! which the app implements: tag changes, undos and playlist deletions become change
//! sets that wait for the owner's review, and playlist edits show in the window at
//! once. See `docs/specs/2026-10-19-mcp-library-server.md`.

pub mod relay;
mod tools;

use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;
use vcmp_library::{ChangeSetId, Library, PlaylistId, TagEdit, TrackId};

pub use tools::MAX_FILES;

/// The port agents connect to unless the owner picks another: "VCMP" typed on a phone
/// keypad.
pub const DEFAULT_PORT: u16 = 8267;

/// The path the server answers on.
pub const PATH: &str = "/mcp";

/// Changes agents ask for, made by the app on its own thread so the window shows
/// them. Each call returns once the app has recorded or made the change; the error is
/// a reason to tell the agent.
pub trait Changes: Send + Sync + 'static {
    /// Records a change of tags for the owner's review.
    fn submit_tags(
        &self,
        agent: &str,
        edits: Vec<(TrackId, TagEdit)>,
    ) -> Result<ChangeSetId, String>;
    /// Records a request to delete a playlist, for the owner's review.
    fn submit_playlist_deletion(
        &self,
        agent: &str,
        playlist: PlaylistId,
    ) -> Result<ChangeSetId, String>;
    /// Records an undo of an accepted change, for the owner's review.
    fn submit_undo(&self, agent: &str, change: ChangeSetId) -> Result<ChangeSetId, String>;
    /// Makes a playlist edit at once. Returns the new playlist's id for `Create`.
    fn edit_playlist(&self, edit: PlaylistEdit) -> Result<Option<PlaylistId>, String>;
}

/// A playlist edit agents make without review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistEdit {
    Create(String),
    Rename(PlaylistId, String),
    /// Appends the tracks.
    Add(PlaylistId, Vec<TrackId>),
    /// Removes the entries at these positions.
    Remove(PlaylistId, Vec<u32>),
    /// Moves the entry at the first position to the second.
    Move(PlaylistId, u32, u32),
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// 0 picks a free port (tests).
    pub port: u16,
    pub token: String,
    pub database: PathBuf,
    pub cache: PathBuf,
}

/// Why the server didn't start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The port is taken, or can't be used.
    #[error("the port can't be used: {0}")]
    Port(std::io::Error),
    #[error("the library could not be opened: {0}")]
    Library(vcmp_library::Error),
    #[error("the server's thread could not start: {0}")]
    Thread(std::io::Error),
}

/// A running server. Dropping it stops the server and waits for its thread.
pub struct Server {
    port: u16,
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Binds the port at once, so a taken port is reported here, then serves on a
    /// thread of its own until dropped.
    pub fn start(config: ServerConfig, changes: Arc<dyn Changes>) -> Result<Self, StartError> {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, config.port)).map_err(StartError::Port)?;
        listener.set_nonblocking(true).map_err(StartError::Port)?;
        let port = listener.local_addr().map_err(StartError::Port)?.port();
        let library =
            Library::open(&config.database, &config.cache).map_err(StartError::Library)?;
        let cancel = CancellationToken::new();
        let token: Arc<str> = config.token.into();
        let stop = cancel.clone();
        let thread = thread::Builder::new()
            .name("vcmp-mcp".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log::error!("could not start the MCP server's runtime: {error}");
                        return;
                    }
                };
                if let Err(error) = runtime.block_on(serve(listener, token, library, changes, stop))
                {
                    log::error!("the MCP server stopped: {error}");
                }
            })
            .map_err(StartError::Thread)?;
        Ok(Self {
            port,
            cancel,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The address agents connect to.
    pub fn address(&self) -> String {
        address(self.port)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The address of a server on `port`.
pub fn address(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PATH}")
}

/// A new random token: 32 bytes as hex.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the system's random source");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The command that adds the server to Claude Code, in user scope: the library is the
/// same in every folder, and the Claude desktop app's Code tab drops a local-scope
/// server at a session's second message (4.29.0, see docs/mcp.md).
pub fn claude_code_command(port: u16, token: &str) -> String {
    format!(
        "claude mcp add --scope user --transport http vcmp {} --header \"Authorization: Bearer {token}\"",
        address(port)
    )
}

async fn serve(
    listener: TcpListener,
    token: Arc<str>,
    library: Library,
    changes: Arc<dyn Changes>,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let server = tools::VcmpServer::new(Arc::new(Mutex::new(library)), changes);
    // Hosts other than this computer's loopback names are refused by default, against
    // DNS rebinding. An empty list of origins, enforced, refuses every request that
    // carries an Origin: browsers send one, MCP clients don't.
    let mut config = StreamableHttpServerConfig::default().enforce_origin_validation();
    config.cancellation_token = cancel.child_token();
    let service: StreamableHttpService<tools::VcmpServer, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(server.clone()), Default::default(), config);
    let router = axum::Router::new()
        .nest_service(PATH, service)
        .layer(middleware::from_fn_with_state(token, require_token));
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { cancel.cancelled_owned().await })
        .await
}

/// Lets through only requests with the token.
async fn require_token(State(token): State<Arc<str>>, request: Request, next: Next) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match given {
        Some(given) if same(given.as_bytes(), token.as_bytes()) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "VCMP needs its token: Authorization: Bearer <token>, from VCMP's Settings.",
        )
            .into_response(),
    }
}

/// Compares in time that doesn't depend on where the first difference is.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}
