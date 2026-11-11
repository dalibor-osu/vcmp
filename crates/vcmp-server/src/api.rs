use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use base64::Engine;
use tokio::sync::oneshot;
use vcmp_library::{FeedCursor, FeedTrack, FieldEdit, Library, SourceDir, TagEdit, TrackId};
use vcmp_protocol::{
    AddFolder, ApiError, Cursor, ErrorCode, FEED_PAGE_DEFAULT, FEED_PAGE_MAX, FeedPage, FeedQuery,
    Folder, Handshake, NewToken, PROTOCOL_VERSION, RemoteTrack, Role, ScanStatus, TagWriteRequest,
    TagWriteResult, paths,
};

use crate::audio;
use crate::worker::{Job, State as Shared};

const BODY_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct Failure {
    status: StatusCode,
    body: ApiError,
}

impl Failure {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let status = match code {
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::AdminOnly => StatusCode::FORBIDDEN,
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::BadRequest | ErrorCode::InvalidTagEdit | ErrorCode::NotAFolder => {
                StatusCode::BAD_REQUEST
            }
            ErrorCode::ConfiguredFolder | ErrorCode::ConfiguredToken => StatusCode::CONFLICT,
            ErrorCode::RangeNotSatisfiable => StatusCode::RANGE_NOT_SATISFIABLE,
            ErrorCode::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::Internal | ErrorCode::Other => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            body: ApiError {
                code,
                message: message.into(),
            },
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        log::error!("{error}");
        Self::new(ErrorCode::Internal, "the server failed; its log says why")
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body)).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

pub(crate) fn router(state: Arc<Shared>) -> Router {
    let api = Router::new()
        .route(paths::HANDSHAKE, get(handshake))
        .route(paths::FEED, get(feed))
        .route(paths::FOLDERS, get(folders).post(add_folder))
        .route("/api/v1/folders/{id}", delete(remove_folder))
        .route(paths::SCAN, get(scan_status).post(start_scan))
        .route("/api/v1/artwork/{key}", get(artwork))
        .route("/api/v1/thumbnails/{key}", get(thumbnail))
        .route("/api/v1/tracks/{id}/audio", get(audio))
        .route("/api/v1/tracks/{id}/cover", get(cover))
        .route(paths::TAGS, post(write_tags))
        .route("/api/v1/tokens/{role}", post(new_token))
        .fallback(|| async { Failure::not_found("no such path") })
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .with_state(state);
    Router::new()
        .route(paths::HEALTH, get(|| async { "ok" }))
        .merge(api)
}

async fn require_token(
    State(state): State<Arc<Shared>>,
    mut request: Request,
    next: Next,
) -> Response {
    let role = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|token| state.tokens.role_of(token.trim()));
    let Some(role) = role else {
        return Failure::new(
            ErrorCode::Unauthorized,
            "the server needs one of its tokens: Authorization: Bearer <token>",
        )
        .into_response();
    };
    request.extensions_mut().insert(role);
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

fn admin_only(role: Role) -> Result<(), Failure> {
    match role {
        Role::Admin => Ok(()),
        Role::User => Err(Failure::new(
            ErrorCode::AdminOnly,
            "this needs the server's admin token",
        )),
    }
}

async fn read<T: Send + 'static>(
    state: &Arc<Shared>,
    read: impl FnOnce(&Library) -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || state.read(read))
        .await
        .map_err(Failure::internal)?
}

async fn ask<T>(
    state: &Shared,
    job: impl FnOnce(oneshot::Sender<Result<T, Failure>>) -> Job,
) -> Result<T, Failure> {
    let (reply, answer) = oneshot::channel();
    state
        .jobs
        .send(job(reply))
        .map_err(|_| Failure::internal("the library's writer has stopped"))?;
    answer
        .await
        .map_err(|_| Failure::internal("the library's writer stopped before it answered"))?
}

async fn handshake(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
) -> Json<Handshake> {
    Json(Handshake {
        server_id: state.server_id.clone(),
        name: state.name.clone(),
        protocol: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").into(),
        role,
        can_write_tags: state.can_write.load(std::sync::atomic::Ordering::Relaxed),
    })
}

async fn feed(
    State(state): State<Arc<Shared>>,
    query: Result<Query<FeedQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<FeedPage>, Failure> {
    let Query(query) = query.map_err(|error| Failure::bad_request(error.body_text()))?;
    let limit = query
        .limit
        .unwrap_or(FEED_PAGE_DEFAULT)
        .clamp(1, FEED_PAGE_MAX);
    let after = FeedCursor {
        revision: query.after_revision,
        track: query.after_track,
    };
    let page = read(&state, move |library| {
        let sources = library.sources().map_err(Failure::internal)?;
        let mut tracks = library
            .track_feed(after, limit + 1)
            .map_err(Failure::internal)?;
        let more = tracks.len() > limit;
        tracks.truncate(limit);
        let head = library.feed_head().map_err(Failure::internal)?;
        Ok((tracks, sources, more, head))
    })
    .await?;
    let (tracks, sources, more, head) = page;
    let next = tracks.last().map(|track| track.cursor).unwrap_or(after);
    Ok(Json(FeedPage {
        tracks: tracks
            .into_iter()
            .map(|track| remote_track(track, &sources))
            .collect(),
        next: cursor(next),
        more,
        head: cursor(head),
    }))
}

fn cursor(feed: FeedCursor) -> Cursor {
    Cursor {
        revision: feed.revision,
        track: feed.track,
    }
}

fn remote_track(track: FeedTrack, sources: &[SourceDir]) -> RemoteTrack {
    let folder = track
        .source_id
        .and_then(|id| sources.iter().find(|source| source.id == id));
    RemoteTrack {
        id: track.cursor.track,
        revision: track.cursor.revision,
        available: track.available,
        folder: folder.map(|source| source.id),
        path: relative_path(&track.path, folder.map(|source| source.path.as_path())),
        format: track.format.map(|format| format.as_str().to_owned()),
        size: track.file_size,
        modified_ms: track.modified_ms,
        title: track.title,
        artist: track.artist,
        album: track.album,
        album_artist: track.album_artist,
        track_number: track.track_number,
        disc_number: track.disc_number,
        year: track.year,
        genre: track.genre,
        duration_ms: track.duration.as_millis() as u64,
        added_ms: track.added_ms,
        artwork: track.artwork,
        mb_release_id: track.mb_release_id,
        mb_artist_id: track.mb_artist_id,
        mb_album_artist_id: track.mb_album_artist_id,
        mb_recording_id: track.mb_recording_id,
    }
}

fn relative_path(path: &Path, folder: Option<&Path>) -> String {
    let inside = folder
        .and_then(|folder| path.strip_prefix(folder).ok())
        .unwrap_or_else(|| path.file_name().map(Path::new).unwrap_or(path));
    inside
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn folder(source: &SourceDir, state: &Shared, role: Role) -> Folder {
    Folder {
        id: source.id,
        name: source
            .path
            .file_name()
            .unwrap_or(source.path.as_os_str())
            .to_string_lossy()
            .into_owned(),
        path: (role == Role::Admin).then(|| source.path.to_string_lossy().into_owned()),
        recursive: source.recursive,
        configured: state.is_configured_folder(&source.path),
        reachable: source.path.is_dir(),
    }
}

async fn folders(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
) -> Result<Json<Vec<Folder>>, Failure> {
    let sources = read(&state, |library| {
        library.sources().map_err(Failure::internal)
    })
    .await?;
    Ok(Json(
        sources
            .iter()
            .map(|source| folder(source, &state, role))
            .collect(),
    ))
}

async fn add_folder(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
    body: Result<Json<AddFolder>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Folder>), Failure> {
    admin_only(role)?;
    let Json(body) = body.map_err(|error| Failure::bad_request(error.body_text()))?;
    let path = PathBuf::from(body.path);
    let recursive = body.recursive;
    let source = ask(&state, |reply| Job::AddFolder {
        path,
        recursive,
        reply,
    })
    .await?;
    Ok((StatusCode::CREATED, Json(folder(&source, &state, role))))
}

async fn remove_folder(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
    UrlPath(id): UrlPath<i64>,
) -> Result<StatusCode, Failure> {
    admin_only(role)?;
    ask(&state, |reply| Job::RemoveFolder { id, reply }).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn scan_status(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
) -> Json<ScanStatus> {
    let mut status = state.scan.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if role == Role::User
        && let Some(last) = &mut status.last
    {
        for file in &mut last.skipped {
            file.path = file_name(&file.path);
        }
        for folder in &mut last.unreachable_folders {
            *folder = file_name(folder);
        }
    }
    Json(status)
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

async fn start_scan(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
) -> Result<(StatusCode, Json<ScanStatus>), Failure> {
    admin_only(role)?;
    state
        .jobs
        .send(Job::Scan)
        .map_err(|_| Failure::internal("the library's writer has stopped"))?;
    let mut status = state.scan.lock().unwrap_or_else(|e| e.into_inner()).clone();
    status.running = true;
    Ok((StatusCode::ACCEPTED, Json(status)))
}

async fn artwork(
    State(state): State<Arc<Shared>>,
    UrlPath(key): UrlPath<String>,
) -> Result<Response, Failure> {
    let artwork = read(&state, move |library| {
        library
            .artwork_by_key(&key)
            .ok_or_else(|| Failure::not_found("no such artwork"))
    })
    .await?;
    image(&artwork.original).await
}

async fn thumbnail(
    State(state): State<Arc<Shared>>,
    UrlPath(key): UrlPath<String>,
) -> Result<Response, Failure> {
    let artwork = read(&state, move |library| {
        library
            .artwork_by_key(&key)
            .ok_or_else(|| Failure::not_found("no such artwork"))
    })
    .await?;
    image(&artwork.thumbnail).await
}

async fn image(path: &Path) -> Result<Response, Failure> {
    let data = tokio::fs::read(path)
        .await
        .map_err(|_| Failure::not_found("no such artwork"))?;
    let kind = match path.extension().and_then(|ext| ext.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    };
    Ok(([(header::CONTENT_TYPE, kind)], data).into_response())
}

async fn audio(
    State(state): State<Arc<Shared>>,
    UrlPath(id): UrlPath<i64>,
    headers: HeaderMap,
) -> Result<Response, Failure> {
    let track = read(&state, move |library| {
        library
            .track(TrackId(id))
            .map_err(Failure::internal)?
            .filter(|track| track.available)
            .and_then(|track| Some((track.file?, track.format)))
            .ok_or_else(|| Failure::not_found("no such track, or its file is gone"))
    })
    .await?;
    audio::respond(&track.0, track.1, &headers).await
}

async fn cover(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Response, Failure> {
    admin_only(role)?;
    let data = read(&state, move |library| {
        library
            .embedded_cover(TrackId(id))
            .map_err(Failure::internal)?
            .ok_or_else(|| Failure::not_found("the track's file holds no cover"))
    })
    .await?;
    let kind = if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else {
        "image/jpeg"
    };
    Ok(([(header::CONTENT_TYPE, kind)], data).into_response())
}

async fn write_tags(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
    body: Result<Json<TagWriteRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<TagWriteResult>, Failure> {
    admin_only(role)?;
    let Json(request) = body.map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Failure::new(ErrorCode::TooLarge, error.body_text())
        } else {
            Failure::bad_request(error.body_text())
        }
    })?;
    let groups = request
        .groups
        .into_iter()
        .map(|group| {
            let tracks = group.tracks.into_iter().map(TrackId).collect();
            Ok((tracks, tag_edit(group.edit)?))
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    let result = ask(&state, |reply| Job::WriteTags { groups, reply }).await?;
    Ok(Json(result))
}

fn tag_edit(edit: vcmp_protocol::TagEdit) -> Result<TagEdit, Failure> {
    fn field<T>(edit: vcmp_protocol::FieldEdit<T>) -> FieldEdit<T> {
        match edit {
            vcmp_protocol::FieldEdit::Keep => FieldEdit::Keep,
            vcmp_protocol::FieldEdit::Set(value) => FieldEdit::Set(value),
            vcmp_protocol::FieldEdit::Clear => FieldEdit::Clear,
        }
    }
    let cover = match edit.cover {
        vcmp_protocol::FieldEdit::Keep => FieldEdit::Keep,
        vcmp_protocol::FieldEdit::Clear => FieldEdit::Clear,
        vcmp_protocol::FieldEdit::Set(text) => FieldEdit::Set(
            base64::engine::general_purpose::STANDARD
                .decode(text.trim())
                .map_err(|_| {
                    Failure::new(ErrorCode::InvalidTagEdit, "the cover is not valid base64")
                })?,
        ),
    };
    Ok(TagEdit {
        title: field(edit.title),
        artist: field(edit.artist),
        album: field(edit.album),
        album_artist: field(edit.album_artist),
        track_number: field(edit.track_number),
        disc_number: field(edit.disc_number),
        year: field(edit.year),
        genre: field(edit.genre),
        cover,
        other: Default::default(),
    })
}

async fn new_token(
    State(state): State<Arc<Shared>>,
    Extension(role): Extension<Role>,
    UrlPath(which): UrlPath<String>,
) -> Result<Json<NewToken>, Failure> {
    admin_only(role)?;
    let which = Role::parse(&which).ok_or_else(|| Failure::not_found("no such role"))?;
    if state.tokens.is_configured(which) {
        return Err(Failure::new(
            ErrorCode::ConfiguredToken,
            "the server's configuration sets this token; change it there",
        ));
    }
    let token = read(&state, move |library| {
        crate::tokens::replace(library, which).map_err(Failure::internal)
    })
    .await?;
    let reload = state.clone();
    read(&state, move |library| {
        reload.tokens.reload(library).map_err(Failure::internal)
    })
    .await?;
    log::info!(
        "the {} token was replaced from an admin's app",
        which.as_str()
    );
    Ok(Json(NewToken { role: which, token }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_their_folder_never_show_the_servers_own() {
        let folder = Path::new("/music");
        assert_eq!(
            relative_path(Path::new("/music/Artist/Album/01.flac"), Some(folder)),
            "Artist/Album/01.flac"
        );
        assert_eq!(
            relative_path(Path::new("/elsewhere/Album/01.flac"), None),
            "01.flac"
        );
        assert_eq!(
            relative_path(Path::new("/elsewhere/Album/01.flac"), Some(folder)),
            "01.flac"
        );
    }
}
