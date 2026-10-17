//! Last.fm scrobbling: connecting an account through Last.fm's desktop
//! authentication, "now playing" updates, and plays sent in batches.
//!
//! VCMP's API key and shared secret are compiled in from `VCMP_LASTFM_API_KEY` and
//! `VCMP_LASTFM_SHARED_SECRET` ([`Credentials::built_in`]); a build without them has
//! no Last.fm at all. A [`Scrobbler`] does the work on its own thread: the requests,
//! the session key in the OS keyring ([`Keyring`]) and the queue of plays not sent
//! yet ([`ScrobbleStore`], the library database in the app). [`Play`] decides when a
//! play counts.
//!
//! Every request is a signed POST: the parameters sorted by name, each name followed
//! by its value, then the shared secret, hashed with MD5 ([`sign`]).

mod play;
mod scrobbler;

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Deserializer};

pub use play::{MAX_WAIT, MIN_LENGTH, Play, scrobble_point};
pub use scrobbler::{
    ConnectError, Keyring, MemorySessions, Report, ScrobbleStore, Scrobbler, SessionStore, Timing,
};

use crate::musicbrainz::USER_AGENT;

pub const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
/// Where the user allows VCMP to scrobble for them.
pub const AUTH_URL: &str = "https://www.last.fm/api/auth/";
/// The most plays one `track.scrobble` request takes.
pub const MAX_BATCH: usize = 50;

/// auth.getSession: the user has not allowed the token yet.
const TOKEN_NOT_AUTHORIZED: u32 = 14;
/// auth.getSession: the token is older than an hour, or was used already.
const TOKEN_EXPIRED: u32 = 15;

/// VCMP's API account on Last.fm.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub api_key: String,
    pub secret: String,
}

impl Credentials {
    /// The key and secret this build was made with, when both were set.
    pub fn built_in() -> Option<Self> {
        Self::new(
            option_env!("VCMP_LASTFM_API_KEY"),
            option_env!("VCMP_LASTFM_SHARED_SECRET"),
        )
    }

    /// Both parts trimmed; `None` when either is missing or empty.
    pub fn new(api_key: Option<&str>, secret: Option<&str>) -> Option<Self> {
        let (api_key, secret) = (api_key?.trim(), secret?.trim());
        (!api_key.is_empty() && !secret.is_empty()).then(|| Self {
            api_key: api_key.into(),
            secret: secret.into(),
        })
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("api_key", &self.api_key)
            .finish_non_exhaustive()
    }
}

/// A connected account: the user's name and the session key that scrobbles for
/// them. The key does not expire; the user can revoke it on last.fm.
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    pub user: String,
    pub key: String,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

/// A track as Last.fm takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    /// Sent only when it differs from the artist, as Last.fm asks.
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub duration: Option<Duration>,
}

/// One play of a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scrobble {
    pub track: Track,
    /// When the play started.
    pub started: SystemTime,
}

/// How Last.fm answered a batch: plays it took and plays it filtered out (too old,
/// or a name it ignores). Either way they are done with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sent {
    pub accepted: usize,
    pub ignored: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Last.fm could not be reached, or answered with something other than an API
    /// reply (a server error, a proxy page).
    Network(String),
    /// Last.fm answered with one of its error codes.
    Api { code: u32, message: String },
}

/// What can be done about an [`Error`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remedy {
    /// Last.fm is unreachable or busy: the same request may work later.
    RetryLater,
    /// The session key no longer works (the user revoked it): connect again.
    Reconnect,
    /// Last.fm refuses VCMP's API key or signature: nothing works until a new build.
    NewKey,
    /// The request itself is wrong; sending it again fails again.
    Drop,
}

impl Error {
    pub fn remedy(&self) -> Remedy {
        match self {
            Error::Network(_) => Remedy::RetryLater,
            Error::Api { code, .. } => match code {
                // Operation failed, service offline, temporarily unavailable, rate
                // limit exceeded.
                8 | 11 | 16 | 29 => Remedy::RetryLater,
                // Authentication failed, invalid session key.
                4 | 9 => Remedy::Reconnect,
                // Invalid API key, invalid signature, suspended API key.
                10 | 13 | 26 => Remedy::NewKey,
                _ => Remedy::Drop,
            },
        }
    }

    /// From [`Client::session`]: the user has not allowed the token yet.
    pub fn is_not_allowed_yet(&self) -> bool {
        self.is_code(TOKEN_NOT_AUTHORIZED)
    }

    /// From [`Client::session`]: the token can no longer be allowed.
    pub fn is_expired_token(&self) -> bool {
        self.is_code(TOKEN_EXPIRED)
    }

    fn is_code(&self, wanted: u32) -> bool {
        matches!(self, Error::Api { code, .. } if *code == wanted)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Network(detail) => f.write_str(detail),
            Error::Api { code, message } => write!(f, "{message} (Last.fm error {code})"),
        }
    }
}

impl std::error::Error for Error {}

/// Last.fm's `api_sig`: the MD5, in hex, of every parameter except `format` and
/// `callback`, sorted by name, each name followed by its value, and then the
/// shared secret.
pub fn sign(params: &[(&str, &str)], secret: &str) -> String {
    let mut signed: Vec<&(&str, &str)> = params
        .iter()
        .filter(|(name, _)| *name != "format" && *name != "callback")
        .collect();
    signed.sort_by(|a, b| a.0.cmp(b.0));
    let mut text = String::new();
    for (name, value) in signed {
        text.push_str(name);
        text.push_str(value);
    }
    text.push_str(secret);
    format!("{:x}", md5::compute(text.as_bytes()))
}

/// The HTTP side of the client, replaced in tests.
pub trait Transport: Send + 'static {
    /// POSTs `form`, URL-encoded, to `url`, returning the status and the body.
    fn post(&mut self, url: &str, form: &[(String, String)]) -> Result<(u16, String), String>;
}

/// The real network.
pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        Self { agent }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for Http {
    fn post(&mut self, url: &str, form: &[(String, String)]) -> Result<(u16, String), String> {
        let mut response = self
            .agent
            .post(url)
            .header("Accept", "application/json")
            .send_form(form.iter().map(|(name, value)| (name, value)))
            .map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| error.to_string())?;
        Ok((status, body))
    }
}

/// Last.fm's API, signed with VCMP's credentials.
pub struct Client<T> {
    transport: T,
    credentials: Credentials,
}

impl Client<Http> {
    pub fn online(credentials: Credentials) -> Self {
        Self::new(Http::new(), credentials)
    }
}

impl<T: Transport> Client<T> {
    pub fn new(transport: T, credentials: Credentials) -> Self {
        Self {
            transport,
            credentials,
        }
    }

    /// Starts connecting an account: a token for the user to allow on last.fm,
    /// valid for an hour.
    pub fn token(&mut self) -> Result<String, Error> {
        let reply: TokenReply = self.call("auth.getToken", Vec::new(), None)?;
        Ok(reply.token)
    }

    /// The page where the user allows `token`.
    pub fn auth_url(&self, token: &str) -> String {
        url::Url::parse_with_params(
            AUTH_URL,
            [
                ("api_key", self.credentials.api_key.as_str()),
                ("token", token),
            ],
        )
        .map(String::from)
        .unwrap_or_else(|_| AUTH_URL.to_string())
    }

    /// The account behind a token the user has allowed. Until they have, Last.fm
    /// answers an error for which [`Error::is_not_allowed_yet`] holds.
    pub fn session(&mut self, token: &str) -> Result<Session, Error> {
        let reply: SessionReply = self.call(
            "auth.getSession",
            vec![("token".into(), token.into())],
            None,
        )?;
        Ok(Session {
            user: reply.session.name,
            key: reply.session.key,
        })
    }

    /// Shows `track` as playing now on the user's profile.
    pub fn now_playing(&mut self, session_key: &str, track: &Track) -> Result<(), Error> {
        let mut params = Vec::new();
        track_params(track, "", &mut params);
        let _: IgnoredAny = self.call("track.updateNowPlaying", params, Some(session_key))?;
        Ok(())
    }

    /// Sends up to [`MAX_BATCH`] plays in one request.
    pub fn scrobble(&mut self, session_key: &str, plays: &[Scrobble]) -> Result<Sent, Error> {
        debug_assert!(plays.len() <= MAX_BATCH);
        let mut params = Vec::new();
        for (index, play) in plays.iter().enumerate() {
            let suffix = format!("[{index}]");
            track_params(&play.track, &suffix, &mut params);
            let started = play
                .started
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            params.push((format!("timestamp{suffix}"), started.to_string()));
        }
        let reply: ScrobbleReply = self.call("track.scrobble", params, Some(session_key))?;
        Ok(Sent {
            accepted: reply.scrobbles.attr.accepted,
            ignored: reply.scrobbles.attr.ignored,
        })
    }

    fn call<R: DeserializeOwned>(
        &mut self,
        method: &str,
        mut params: Vec<(String, String)>,
        session_key: Option<&str>,
    ) -> Result<R, Error> {
        params.push(("method".into(), method.into()));
        params.push(("api_key".into(), self.credentials.api_key.clone()));
        if let Some(key) = session_key {
            params.push(("sk".into(), key.into()));
        }
        let pairs: Vec<(&str, &str)> = params
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let signature = sign(&pairs, &self.credentials.secret);
        params.push(("api_sig".into(), signature));
        params.push(("format".into(), "json".into()));
        let (status, body) = self
            .transport
            .post(API_URL, &params)
            .map_err(Error::Network)?;
        parse(status, &body)
    }
}

/// The parameters for one track, with `suffix` (`[0]`, `[1]`...) for batches.
fn track_params(track: &Track, suffix: &str, params: &mut Vec<(String, String)>) {
    let mut add = |name: &str, value: String| params.push((format!("{name}{suffix}"), value));
    add("artist", track.artist.clone());
    add("track", track.title.clone());
    if let Some(album) = &track.album {
        add("album", album.clone());
    }
    if let Some(album_artist) = &track.album_artist
        && *album_artist != track.artist
    {
        add("albumArtist", album_artist.clone());
    }
    if let Some(number) = track.track_number {
        add("trackNumber", number.to_string());
    }
    if let Some(duration) = track.duration.filter(|duration| duration.as_secs() > 0) {
        add("duration", duration.as_secs().to_string());
    }
}

/// An API reply: Last.fm's error body wins over the status, which is 200 for some
/// errors and 4xx for others.
fn parse<R: DeserializeOwned>(status: u16, body: &str) -> Result<R, Error> {
    if let Ok(ApiError { error, message }) = serde_json::from_str::<ApiError>(body) {
        return Err(Error::Api {
            code: error,
            message,
        });
    }
    if status != 200 {
        return Err(Error::Network(format!("Last.fm answered {status}")));
    }
    serde_json::from_str(body)
        .map_err(|error| Error::Network(format!("unexpected answer from Last.fm: {error}")))
}

#[derive(Deserialize)]
struct ApiError {
    error: u32,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct TokenReply {
    token: String,
}

#[derive(Deserialize)]
struct SessionReply {
    session: SessionBody,
}

#[derive(Deserialize)]
struct SessionBody {
    name: String,
    key: String,
}

#[derive(Deserialize)]
struct ScrobbleReply {
    scrobbles: Scrobbles,
}

#[derive(Deserialize)]
struct Scrobbles {
    #[serde(rename = "@attr")]
    attr: Counts,
}

#[derive(Deserialize)]
struct Counts {
    #[serde(deserialize_with = "count")]
    accepted: usize,
    #[serde(deserialize_with = "count")]
    ignored: usize,
}

/// A count, which Last.fm's JSON gives as a number or as a string.
fn count<'de, D: Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Count {
        Number(usize),
        Text(String),
    }
    match Count::deserialize(deserializer)? {
        Count::Number(number) => Ok(number),
        Count::Text(text) => text.trim().parse().map_err(serde::de::Error::custom),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;

    pub type Requests = Arc<Mutex<Vec<Vec<(String, String)>>>>;
    /// A scripted answer: the status and body, or a network error.
    pub type Answer = Result<(u16, String), String>;

    /// Answers requests from a script, in order, and records their forms. An empty
    /// script answers nothing more: the test fails if a request comes.
    pub struct Scripted {
        answers: Arc<Mutex<VecDeque<Answer>>>,
        requests: Requests,
    }

    impl Scripted {
        pub fn new(
            answers: impl IntoIterator<Item = Result<(u16, String), String>>,
        ) -> (Self, Requests) {
            let requests = Requests::default();
            let transport = Self {
                answers: Arc::new(Mutex::new(answers.into_iter().collect())),
                requests: Arc::clone(&requests),
            };
            (transport, requests)
        }
    }

    impl Transport for Scripted {
        fn post(&mut self, url: &str, form: &[(String, String)]) -> Result<(u16, String), String> {
            assert_eq!(url, API_URL);
            self.requests.lock().unwrap().push(form.to_vec());
            let method = value(form, "method").unwrap_or_default();
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("no answer scripted for {method}"))
        }
    }

    pub fn value<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
        form.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn ok(body: &str) -> Result<(u16, String), String> {
        Ok((200, body.to_string()))
    }

    pub fn api_error(code: u32) -> Result<(u16, String), String> {
        Ok((
            403,
            format!(r#"{{"error":{code},"message":"error {code}","links":[]}}"#),
        ))
    }

    pub fn credentials() -> Credentials {
        Credentials::new(Some("the-key"), Some("the-secret")).unwrap()
    }

    pub fn track(title: &str) -> Track {
        Track {
            artist: "Band".into(),
            title: title.into(),
            album: Some("Record".into()),
            album_artist: Some("Band".into()),
            track_number: Some(2),
            duration: Some(Duration::from_millis(201_900)),
        }
    }

    fn client(
        answers: impl IntoIterator<Item = Result<(u16, String), String>>,
    ) -> (Client<Scripted>, Requests) {
        let (transport, requests) = Scripted::new(answers);
        (Client::new(transport, credentials()), requests)
    }

    #[test]
    fn signatures_follow_lastfms_recipe() {
        // md5("api_keykeymethodauth.getSessiontokentokensecret"), worked out with
        // Python's hashlib.
        assert_eq!(
            sign(
                &[
                    ("token", "token"),
                    ("method", "auth.getSession"),
                    ("api_key", "key"),
                    ("format", "json"),
                ],
                "secret"
            ),
            "9ac306496295a8866c4a8673395540eb"
        );
        // Values are hashed as their UTF-8 bytes.
        assert_eq!(
            sign(&[("track[0]", "ラブライブ!"), ("artist[0]", "μ's")], "s"),
            format!("{:x}", md5::compute("artist[0]μ'strack[0]ラブライブ!s"))
        );
    }

    #[test]
    fn requests_are_signed_posts_in_json() {
        let (mut client, requests) = client([ok(r#"{"token":"abc"}"#)]);
        assert_eq!(client.token().unwrap(), "abc");
        let form = requests.lock().unwrap()[0].clone();
        assert_eq!(value(&form, "method"), Some("auth.getToken"));
        assert_eq!(value(&form, "api_key"), Some("the-key"));
        assert_eq!(value(&form, "format"), Some("json"));
        let unsigned: Vec<(&str, &str)> = form
            .iter()
            .filter(|(name, _)| name != "api_sig")
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            value(&form, "api_sig"),
            Some(sign(&unsigned, "the-secret").as_str())
        );
        assert!(!form.iter().any(|(_, value)| value == "the-secret"));
    }

    #[test]
    fn the_auth_page_names_the_key_and_token() {
        let (client, _) = client([]);
        assert_eq!(
            client.auth_url("t&1"),
            "https://www.last.fm/api/auth/?api_key=the-key&token=t%261"
        );
    }

    #[test]
    fn a_session_names_the_user() {
        let (mut client, requests) = client([
            api_error(14),
            ok(r#"{"session":{"subscriber":0,"name":"listener","key":"sk-1"}}"#),
        ]);
        let waiting = client.session("abc").unwrap_err();
        assert!(waiting.is_not_allowed_yet(), "{waiting:?}");
        let session = client.session("abc").unwrap();
        assert_eq!(
            session,
            Session {
                user: "listener".into(),
                key: "sk-1".into()
            }
        );
        assert!(!format!("{session:?}").contains("sk-1"));
        let form = requests.lock().unwrap()[1].clone();
        assert_eq!(value(&form, "token"), Some("abc"));
    }

    #[test]
    fn now_playing_sends_the_track() {
        let (mut client, requests) = client([ok(r##"{"nowplaying":{"track":{"#text":"A"}}}"##)]);
        client.now_playing("sk-1", &track("Opening")).unwrap();
        let form = requests.lock().unwrap()[0].clone();
        assert_eq!(value(&form, "method"), Some("track.updateNowPlaying"));
        assert_eq!(value(&form, "sk"), Some("sk-1"));
        assert_eq!(value(&form, "artist"), Some("Band"));
        assert_eq!(value(&form, "track"), Some("Opening"));
        assert_eq!(value(&form, "album"), Some("Record"));
        assert_eq!(value(&form, "trackNumber"), Some("2"));
        assert_eq!(value(&form, "duration"), Some("201"));
        // The same as the artist, so left out.
        assert_eq!(value(&form, "albumArtist"), None);
    }

    #[test]
    fn a_batch_numbers_its_plays() {
        let (mut client, requests) = client([ok(
            r#"{"scrobbles":{"scrobble":[],"@attr":{"accepted":1,"ignored":"1"}}}"#,
        )]);
        let plays = [
            Scrobble {
                track: track("Opening"),
                started: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            },
            Scrobble {
                track: Track {
                    album: None,
                    album_artist: Some("Various Artists".into()),
                    track_number: None,
                    duration: None,
                    ..track("Closing")
                },
                started: UNIX_EPOCH + Duration::from_millis(1_700_000_250_900),
            },
        ];
        let sent = client.scrobble("sk-1", &plays).unwrap();
        assert_eq!(
            sent,
            Sent {
                accepted: 1,
                ignored: 1
            }
        );
        let form = requests.lock().unwrap()[0].clone();
        assert_eq!(value(&form, "method"), Some("track.scrobble"));
        assert_eq!(value(&form, "track[0]"), Some("Opening"));
        assert_eq!(value(&form, "timestamp[0]"), Some("1700000000"));
        assert_eq!(value(&form, "track[1]"), Some("Closing"));
        assert_eq!(value(&form, "timestamp[1]"), Some("1700000250"));
        assert_eq!(value(&form, "albumArtist[1]"), Some("Various Artists"));
        assert_eq!(value(&form, "album[1]"), None);
        assert_eq!(value(&form, "duration[1]"), None);
    }

    #[test]
    fn errors_say_what_to_do() {
        let (mut client, _) = client([
            api_error(9),
            ok(r#"{"error":11,"message":"Service Offline"}"#),
            Ok((502, "<html>Bad gateway</html>".into())),
            Err("connection refused".into()),
            api_error(26),
            api_error(6),
        ]);
        let mut next = || client.now_playing("sk", &track("A")).unwrap_err();
        let session = next();
        assert_eq!(session.remedy(), Remedy::Reconnect);
        assert_eq!(session.to_string(), "error 9 (Last.fm error 9)");
        assert_eq!(next().remedy(), Remedy::RetryLater);
        assert_eq!(
            next(),
            Error::Network("Last.fm answered 502".into()),
            "an error page is not an API answer"
        );
        assert_eq!(next(), Error::Network("connection refused".into()));
        assert_eq!(next().remedy(), Remedy::NewKey);
        assert_eq!(next().remedy(), Remedy::Drop);
        let (mut client, _) = self::client([ok("{}")]);
        assert!(matches!(
            client.token().unwrap_err(),
            Error::Network(detail) if detail.starts_with("unexpected answer")
        ));
    }

    #[test]
    fn credentials_need_both_parts() {
        assert!(Credentials::new(None, None).is_none());
        assert!(Credentials::new(Some("key"), None).is_none());
        assert!(Credentials::new(Some(" "), Some("secret")).is_none());
        let credentials = Credentials::new(Some(" key\n"), Some("secret")).unwrap();
        assert_eq!(credentials.api_key, "key");
        assert!(!format!("{credentials:?}").contains("secret"));
    }
}
