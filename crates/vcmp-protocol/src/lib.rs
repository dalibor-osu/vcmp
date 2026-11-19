use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

pub const DEFAULT_PORT: u16 = 8268;

pub const FEED_PAGE_DEFAULT: usize = 500;
pub const FEED_PAGE_MAX: usize = 2000;

pub mod paths {
    pub const HEALTH: &str = "/health";
    pub const HANDSHAKE: &str = "/api/v1/handshake";
    pub const FEED: &str = "/api/v1/feed";
    pub const FOLDERS: &str = "/api/v1/folders";
    pub const SCAN: &str = "/api/v1/scan";
    pub const TAGS: &str = "/api/v1/tags";

    pub fn folder(id: i64) -> String {
        format!("{FOLDERS}/{id}")
    }

    pub fn artwork(key: &str) -> String {
        format!("/api/v1/artwork/{key}")
    }

    pub fn thumbnail(key: &str) -> String {
        format!("/api/v1/thumbnails/{key}")
    }

    pub fn audio(track: i64) -> String {
        format!("/api/v1/tracks/{track}/audio")
    }

    pub fn cover(track: i64) -> String {
        format!("/api/v1/tracks/{track}/cover")
    }

    pub fn token(role: crate::Role) -> String {
        format!("/api/v1/tokens/{}", role.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Admin => "admin",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "user" => Some(Role::User),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    pub server_id: String,
    pub name: String,
    pub protocol: u32,
    pub version: String,
    pub role: Role,
    pub can_write_tags: bool,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Cursor {
    pub revision: i64,
    pub track: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedQuery {
    #[serde(default)]
    pub after_revision: i64,
    #[serde(default)]
    pub after_track: i64,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedPage {
    pub tracks: Vec<RemoteTrack>,
    pub next: Cursor,
    pub more: bool,
    pub head: Cursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteTrack {
    pub id: i64,
    pub revision: i64,
    pub available: bool,
    pub folder: Option<i64>,
    pub path: String,
    pub format: Option<String>,
    pub size: u64,
    pub modified_ms: i64,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub duration_ms: u64,
    pub added_ms: i64,
    pub artwork: Option<String>,
    pub mb_release_id: Option<String>,
    pub mb_artist_id: Option<String>,
    pub mb_album_artist_id: Option<String>,
    pub mb_recording_id: Option<String>,
}

impl RemoteTrack {
    pub fn audio_etag(&self) -> String {
        audio_etag(self.size, self.modified_ms)
    }
}

pub fn audio_etag(size: u64, modified_ms: i64) -> String {
    format!("\"{size}-{modified_ms}\"")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: i64,
    pub name: String,
    pub path: Option<String>,
    pub recursive: bool,
    pub configured: bool,
    pub reachable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddFolder {
    pub path: String,
    #[serde(default = "yes")]
    pub recursive: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanStatus {
    pub running: bool,
    pub last: Option<ScanSummary>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanSummary {
    pub finished_ms: i64,
    pub added: usize,
    pub updated: usize,
    pub relinked: usize,
    pub became_unavailable: usize,
    pub became_available: usize,
    pub unchanged: usize,
    pub skipped: Vec<SkippedFile>,
    pub unreachable_folders: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum FieldEdit<T> {
    #[default]
    Keep,
    Set(T),
    Clear,
}

impl<T> FieldEdit<T> {
    pub fn is_keep(&self) -> bool {
        matches!(self, FieldEdit::Keep)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagEdit {
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub title: FieldEdit<String>,
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub artist: FieldEdit<String>,
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub album: FieldEdit<String>,
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub album_artist: FieldEdit<String>,
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub track_number: FieldEdit<u32>,
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub disc_number: FieldEdit<u32>,
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub year: FieldEdit<i32>,
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub genre: FieldEdit<String>,
    #[serde(default, skip_serializing_if = "FieldEdit::is_keep")]
    pub cover: FieldEdit<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagWriteGroup {
    pub tracks: Vec<i64>,
    pub edit: TagEdit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagWriteRequest {
    pub groups: Vec<TagWriteGroup>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagWriteResult {
    pub written: Vec<i64>,
    pub failed: Vec<FailedTagWrite>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedTagWrite {
    pub track: i64,
    pub reason: TagWriteFailure,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagWriteFailure {
    Unavailable,
    ReadOnly,
    Unreadable,
    Write,
    Verify,
    ChangedMeanwhile,
    Replace,
    UnsupportedKey,
    Conflict,
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewToken {
    pub role: Role,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthorized,
    AdminOnly,
    NotFound,
    BadRequest,
    InvalidTagEdit,
    NotAFolder,
    ConfiguredFolder,
    ConfiguredToken,
    RangeNotSatisfiable,
    TooLarge,
    Internal,
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_edit_names_only_the_fields_it_changes() {
        let edit = TagEdit {
            title: FieldEdit::Set("One".into()),
            year: FieldEdit::Clear,
            ..TagEdit::default()
        };
        let json = serde_json::to_string(&edit).unwrap();
        assert_eq!(
            json,
            r#"{"title":{"op":"set","value":"One"},"year":{"op":"clear"}}"#
        );
        assert_eq!(serde_json::from_str::<TagEdit>(&json).unwrap(), edit);
        assert_eq!(
            serde_json::from_str::<TagEdit>("{}").unwrap(),
            TagEdit::default()
        );
    }

    #[test]
    fn codes_a_newer_server_adds_are_read_as_other() {
        let error: ApiError =
            serde_json::from_str(r#"{"code":"quota_exceeded","message":"x"}"#).unwrap();
        assert_eq!(error.code, ErrorCode::Other);
        let failure: TagWriteFailure = serde_json::from_str(r#""disk_full""#).unwrap();
        assert_eq!(failure, TagWriteFailure::Other);
    }

    #[test]
    fn paths_and_roles_read_back() {
        assert_eq!(paths::audio(7), "/api/v1/tracks/7/audio");
        assert_eq!(paths::token(Role::Admin), "/api/v1/tokens/admin");
        for role in [Role::User, Role::Admin] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(audio_etag(12, 34), "\"12-34\"");
    }
}
