use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use serde::de::DeserializeOwned;
use vcmp_protocol::{
    AddFolder, ApiError, Cursor, ErrorCode, FeedPage, Folder, Handshake, NewToken,
    PROTOCOL_VERSION, Role, ScanStatus, TagWriteRequest, TagWriteResult, paths,
};

const ANSWER_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("{0} is not a server address")]
    Address(String),
    #[error("the server can't be reached: {0}")]
    Unreachable(String),
    #[error("the server refused the token")]
    Unauthorized,
    #[error("the server answered {status} ({code:?}): {message}")]
    Refused {
        status: u16,
        code: ErrorCode,
        message: String,
    },
    #[error("the server speaks protocol {server}, this app {app}")]
    Protocol { server: u32, app: u32 },
    #[error("the server's answer can't be read: {0}")]
    Answer(String),
    #[error("{0}")]
    Io(String),
}

impl ClientError {
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            ClientError::Refused { code, .. } => Some(*code),
            ClientError::Unauthorized => Some(ErrorCode::Unauthorized),
            _ => None,
        }
    }

    pub fn is_unreachable(&self) -> bool {
        matches!(self, ClientError::Unreachable(_))
    }
}

pub fn normalize_address(text: &str) -> Result<String, ClientError> {
    let trimmed = text.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return Err(ClientError::Address(text.into()));
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_owned()
    } else if trimmed.contains("://") {
        return Err(ClientError::Address(text.into()));
    } else if is_local_host(host_of(&format!("http://{trimmed}"))) {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    };
    if host_of(&with_scheme).is_empty() {
        return Err(ClientError::Address(text.into()));
    }
    Ok(with_scheme)
}

fn host_of(address: &str) -> &str {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or_default();
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if let Some(inner) = authority.strip_prefix('[') {
        return inner.split(']').next().unwrap_or_default();
    }
    authority.split(':').next().unwrap_or_default()
}

fn is_local_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => !host.contains('.'),
    }
}

pub fn sends_token_in_clear(address: &str) -> bool {
    address.starts_with("http://") && !is_local_host(host_of(address))
}

#[derive(Clone)]
pub struct Connection {
    address: String,
    token: String,
    agent: ureq::Agent,
    downloads: ureq::Agent,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl Connection {
    pub fn new(address: &str, token: &str) -> Result<Self, ClientError> {
        let address = normalize_address(address)?;
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(concat!("VCMP/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let downloads = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .user_agent(concat!("VCMP/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Ok(Self {
            address,
            token: token.trim().to_owned(),
            agent,
            downloads,
        })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.address)
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }

    fn answer<T: DeserializeOwned>(
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<T, ClientError> {
        let bytes = Self::bytes(response)?;
        serde_json::from_slice(&bytes).map_err(|error| ClientError::Answer(error.to_string()))
    }

    fn bytes(
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Vec<u8>, ClientError> {
        let mut response = response.map_err(|error| ClientError::Unreachable(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(ANSWER_LIMIT)
            .read_to_vec()
            .map_err(|error| ClientError::Unreachable(error.to_string()))?;
        if (200..300).contains(&status) {
            return Ok(body);
        }
        Err(refusal(status, &body))
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        Self::answer(
            self.agent
                .get(self.url(path))
                .header("Authorization", &self.bearer())
                .call(),
        )
    }

    fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ClientError> {
        let request = self
            .agent
            .post(self.url(path))
            .header("Authorization", &self.bearer());
        let response = match body {
            Some(body) => {
                let json = serde_json::to_vec(body)
                    .map_err(|error| ClientError::Answer(error.to_string()))?;
                request
                    .header("Content-Type", "application/json")
                    .send(&json[..])
            }
            None => request.send_empty(),
        };
        Self::answer(response)
    }

    pub fn handshake(&self) -> Result<Handshake, ClientError> {
        let handshake: Handshake = self.get(paths::HANDSHAKE)?;
        if handshake.protocol != PROTOCOL_VERSION {
            return Err(ClientError::Protocol {
                server: handshake.protocol,
                app: PROTOCOL_VERSION,
            });
        }
        Ok(handshake)
    }

    pub fn feed(&self, after: Cursor, limit: usize) -> Result<FeedPage, ClientError> {
        self.get(&format!(
            "{}?after_revision={}&after_track={}&limit={limit}",
            paths::FEED,
            after.revision,
            after.track
        ))
    }

    pub fn folders(&self) -> Result<Vec<Folder>, ClientError> {
        self.get(paths::FOLDERS)
    }

    pub fn artwork(&self, key: &str) -> Result<Vec<u8>, ClientError> {
        self.image(&paths::artwork(key))
    }

    pub fn thumbnail(&self, key: &str) -> Result<Vec<u8>, ClientError> {
        self.image(&paths::thumbnail(key))
    }

    fn image(&self, path: &str) -> Result<Vec<u8>, ClientError> {
        Self::bytes(
            self.agent
                .get(self.url(path))
                .header("Authorization", &self.bearer())
                .call(),
        )
    }

    pub fn download(
        &self,
        track: i64,
        resume: Option<(u64, &str)>,
        into: &Path,
    ) -> Result<Download, ClientError> {
        let mut request = self
            .downloads
            .get(self.url(&paths::audio(track)))
            .header("Authorization", &self.bearer());
        if let Some((from, etag)) = resume {
            request = request
                .header("Range", &format!("bytes={from}-"))
                .header("If-Range", etag);
        }
        let mut response = request
            .call()
            .map_err(|error| ClientError::Unreachable(error.to_string()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = response
                .body_mut()
                .with_config()
                .limit(1024 * 1024)
                .read_to_vec()
                .unwrap_or_default();
            return Err(refusal(status, &body));
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let resumed = resume.is_some() && status == 206;
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(into)
            .map_err(|error| ClientError::Io(error.to_string()))?;
        let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut written = 0u64;
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| ClientError::Unreachable(error.to_string()))?;
            if read == 0 {
                break;
            }
            out.write_all(&buffer[..read])
                .map_err(|error| ClientError::Io(error.to_string()))?;
            written += read as u64;
        }
        Ok(Download {
            etag,
            resumed,
            written,
        })
    }

    pub fn scan_status(&self) -> Result<ScanStatus, ClientError> {
        self.get(paths::SCAN)
    }

    pub fn start_scan(&self) -> Result<ScanStatus, ClientError> {
        self.post::<(), _>(paths::SCAN, None)
    }

    pub fn add_folder(&self, path: &str, recursive: bool) -> Result<Folder, ClientError> {
        self.post(
            paths::FOLDERS,
            Some(&AddFolder {
                path: path.into(),
                recursive,
            }),
        )
    }

    pub fn remove_folder(&self, id: i64) -> Result<(), ClientError> {
        Self::bytes(
            self.agent
                .delete(self.url(&paths::folder(id)))
                .header("Authorization", &self.bearer())
                .call(),
        )
        .map(|_| ())
    }

    pub fn write_tags(&self, request: &TagWriteRequest) -> Result<TagWriteResult, ClientError> {
        self.post(paths::TAGS, Some(request))
    }

    pub fn cover(&self, track: i64) -> Result<Option<Vec<u8>>, ClientError> {
        match self.image(&paths::cover(track)) {
            Ok(data) => Ok(Some(data)),
            Err(ClientError::Refused {
                code: ErrorCode::NotFound,
                ..
            }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn new_token(&self, role: Role) -> Result<NewToken, ClientError> {
        self.post::<(), _>(&paths::token(role), None)
    }
}

pub fn encode_cover(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub etag: Option<String>,
    pub resumed: bool,
    pub written: u64,
}

fn refusal(status: u16, body: &[u8]) -> ClientError {
    if status == 401 {
        return ClientError::Unauthorized;
    }
    match serde_json::from_slice::<ApiError>(body) {
        Ok(error) => ClientError::Refused {
            status,
            code: error.code,
            message: error.message,
        },
        Err(_) => ClientError::Refused {
            status,
            code: ErrorCode::Other,
            message: String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_get_a_scheme_and_lose_trailing_slashes() {
        assert_eq!(
            normalize_address(" music.example.com/ ").unwrap(),
            "https://music.example.com"
        );
        assert_eq!(
            normalize_address("192.168.1.5:8268").unwrap(),
            "http://192.168.1.5:8268"
        );
        assert_eq!(normalize_address("nas:8268").unwrap(), "http://nas:8268");
        assert_eq!(
            normalize_address("http://music.example.com:8268").unwrap(),
            "http://music.example.com:8268"
        );
        assert!(normalize_address("").is_err());
        assert!(normalize_address("ftp://x").is_err());
        assert!(normalize_address("two words").is_err());
    }

    #[test]
    fn only_plain_http_to_a_public_host_sends_the_token_in_clear() {
        assert!(sends_token_in_clear("http://music.example.com"));
        assert!(sends_token_in_clear("http://203.0.113.7:8268"));
        assert!(!sends_token_in_clear("https://music.example.com"));
        assert!(!sends_token_in_clear("http://127.0.0.1:8268"));
        assert!(!sends_token_in_clear("http://192.168.1.5:8268"));
        assert!(!sends_token_in_clear("http://10.0.0.2"));
        assert!(!sends_token_in_clear("http://nas.local:8268"));
        assert!(!sends_token_in_clear("http://nas:8268"));
        assert!(!sends_token_in_clear("http://[::1]:8268"));
        assert!(!sends_token_in_clear("http://[fd00::5]:8268"));
    }
}
