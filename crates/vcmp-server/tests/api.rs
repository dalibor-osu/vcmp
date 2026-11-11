use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, Tag, TagExt};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tempfile::TempDir;
use vcmp_protocol::{
    ApiError, Cursor, ErrorCode, FeedPage, Folder, Handshake, NewToken, PROTOCOL_VERSION,
    RemoteTrack, Role, ScanStatus, TagWriteResult, paths,
};
use vcmp_server::{Config, Server};

const FLAC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../vcmp-library/tests/fixtures/silence.flac"
);
const OPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../vcmp-library/tests/fixtures/opus-in-ogg.ogg"
);

struct Setup {
    server: Server,
    http: ureq::Agent,
    user: String,
    admin: String,
    base: PathBuf,
    _dir: TempDir,
}

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "{error}: {} {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    fn error(&self) -> ErrorCode {
        self.json::<ApiError>().code
    }
}

fn tag(path: &Path, title: &str, album: &str) {
    let file = lofty::read_from_path(path).unwrap();
    let mut tag = Tag::new(file.primary_tag_type());
    tag.set_title(title.into());
    tag.set_artist("Artist".into());
    tag.set_album(album.into());
    tag.set_track(1);
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

fn add_song(dir: &Path, name: &str, title: &str, album: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::copy(FLAC, &path).unwrap();
    tag(&path, title, album);
    path
}

fn png() -> Vec<u8> {
    let image = image::RgbImage::from_pixel(16, 16, image::Rgb([20, 120, 220]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn setup_with(configure: impl FnOnce(&mut Config)) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(dir.path()).unwrap();
    let album = base.join("music").join("Album");
    add_song(&album, "1.flac", "One", "Album");
    add_song(&album, "2.flac", "Two", "Album");
    fs::write(album.join("cover.png"), png()).unwrap();
    fs::copy(OPUS, album.join("opus.ogg")).unwrap();

    let mut config = Config::new(base.join("data"));
    config.listen = "127.0.0.1:0".parse().unwrap();
    config.music = vec![base.join("music")];
    config.rescan_interval = None;
    configure(&mut config);
    let (_, tokens) = vcmp_server::tokens(&config).unwrap();
    let user = tokens[0].token.clone();
    let admin = tokens[1].token.clone();
    let server = Server::start(config).unwrap();
    let http = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let setup = Setup {
        server,
        http,
        user,
        admin,
        base,
        _dir: dir,
    };
    setup.wait_until("the first scan", |setup| {
        let status: ScanStatus = setup.get(paths::SCAN, Some(&setup.user)).json();
        !status.running && status.last.is_some()
    });
    setup
}

fn setup() -> Setup {
    setup_with(|_| {})
}

impl Setup {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.server.address())
    }

    fn answer(mut response: ureq::http::Response<ureq::Body>) -> Answer {
        let headers = response
            .headers()
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        Answer {
            status: response.status().as_u16(),
            headers,
            body: response
                .body_mut()
                .with_config()
                .limit(64 * 1024 * 1024)
                .read_to_vec()
                .unwrap(),
        }
    }

    fn get_with(&self, path: &str, token: Option<&str>, headers: &[(&str, &str)]) -> Answer {
        let mut request = self.http.get(self.url(path));
        if let Some(token) = token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        Self::answer(request.call().unwrap())
    }

    fn get(&self, path: &str, token: Option<&str>) -> Answer {
        self.get_with(path, token, &[])
    }

    fn post(&self, path: &str, token: &str, body: Option<Value>) -> Answer {
        let request = self
            .http
            .post(self.url(path))
            .header("Authorization", &format!("Bearer {token}"));
        let response = match body {
            Some(body) => request
                .header("Content-Type", "application/json")
                .send(body.to_string()),
            None => request.send_empty(),
        };
        Self::answer(response.unwrap())
    }

    fn delete(&self, path: &str, token: &str) -> Answer {
        Self::answer(
            self.http
                .delete(self.url(path))
                .header("Authorization", &format!("Bearer {token}"))
                .call()
                .unwrap(),
        )
    }

    fn sync(&self, after: Cursor, limit: usize) -> (Vec<RemoteTrack>, Cursor) {
        let mut tracks = Vec::new();
        let mut cursor = after;
        loop {
            let page: FeedPage = self
                .get(
                    &format!(
                        "{}?after_revision={}&after_track={}&limit={limit}",
                        paths::FEED,
                        cursor.revision,
                        cursor.track
                    ),
                    Some(&self.user),
                )
                .json();
            assert!(page.tracks.len() <= limit);
            assert!(page.next <= page.head);
            cursor = page.next;
            tracks.extend(page.tracks);
            if !page.more {
                assert!(cursor == after || cursor == page.head);
                return (tracks, cursor);
            }
        }
    }

    fn wait_until(&self, what: &str, done: impl Fn(&Setup) -> bool) {
        let start = Instant::now();
        while !done(self) {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn by_title<'a>(tracks: &'a [RemoteTrack], title: &str) -> &'a RemoteTrack {
    tracks
        .iter()
        .find(|track| track.title == title)
        .unwrap_or_else(|| panic!("no {title:?} in {tracks:?}"))
}

#[test]
fn an_app_mirrors_plays_and_edits_the_library() {
    let setup = setup();

    let health = setup.get(paths::HEALTH, None);
    assert_eq!((health.status, health.body.as_slice()), (200, &b"ok"[..]));
    let refused = setup.get(paths::HANDSHAKE, None);
    assert_eq!(refused.status, 401);
    assert_eq!(refused.error(), ErrorCode::Unauthorized);
    assert_eq!(
        setup
            .get(paths::HANDSHAKE, Some("not-the-token-at-all"))
            .status,
        401
    );

    let handshake: Handshake = setup.get(paths::HANDSHAKE, Some(&setup.user)).json();
    assert_eq!(handshake.server_id, setup.server.server_id());
    assert_eq!(handshake.protocol, PROTOCOL_VERSION);
    assert_eq!(handshake.role, Role::User);
    assert_eq!(handshake.name, "VCMP server");
    assert!(handshake.can_write_tags);
    let admin: Handshake = setup.get(paths::HANDSHAKE, Some(&setup.admin)).json();
    assert_eq!(admin.role, Role::Admin);

    let (tracks, head) = setup.sync(Cursor::default(), 1);
    assert_eq!(tracks.len(), 2);
    let one = by_title(&tracks, "One").clone();
    let two = by_title(&tracks, "Two");
    assert_eq!(one.path, "Album/1.flac");
    assert_eq!(one.format.as_deref(), Some("flac"));
    assert_eq!(one.artist.as_deref(), Some("Artist"));
    assert!(one.available && one.folder.is_some());
    let key = one.artwork.clone().expect("the folder cover");
    assert_eq!(two.artwork.as_deref(), Some(key.as_str()));
    assert!(setup.sync(head, 100).0.is_empty());
    let ahead: FeedPage = setup
        .get(
            &format!("{}?after_revision={}", paths::FEED, head.revision + 100),
            Some(&setup.user),
        )
        .json();
    assert!(ahead.tracks.is_empty() && !ahead.more);
    assert!(
        ahead.head < ahead.next,
        "an app ahead of the server can tell"
    );
    assert_eq!(ahead.head, head);

    let original = setup.get(&paths::artwork(&key), Some(&setup.user));
    assert_eq!(original.status, 200);
    assert_eq!(original.body, png());
    assert_eq!(original.header("content-type"), Some("image/png"));
    let thumbnail = setup.get(&paths::thumbnail(&key), Some(&setup.user));
    assert_eq!(thumbnail.status, 200);
    assert_eq!(thumbnail.header("content-type"), Some("image/jpeg"));
    assert_eq!(
        setup
            .get(&paths::artwork("0123456789abcdef.png"), Some(&setup.user))
            .status,
        404
    );

    let file = fs::read(setup.base.join("music/Album/1.flac")).unwrap();
    let whole = setup.get(&paths::audio(one.id), Some(&setup.user));
    assert_eq!(whole.status, 200);
    assert_eq!(whole.body, file);
    assert_eq!(whole.header("etag"), Some(one.audio_etag().as_str()));
    assert_eq!(whole.header("accept-ranges"), Some("bytes"));
    assert_eq!(whole.header("content-type"), Some("audio/flac"));
    assert_eq!(whole.header("cache-control"), Some("private, no-store"));

    let start = setup.get_with(
        &paths::audio(one.id),
        Some(&setup.user),
        &[("Range", "bytes=0-99")],
    );
    assert_eq!(start.status, 206);
    assert_eq!(start.body, file[..100]);
    assert_eq!(
        start.header("content-range"),
        Some(format!("bytes 0-99/{}", file.len()).as_str())
    );
    let etag = one.audio_etag();
    let rest = setup.get_with(
        &paths::audio(one.id),
        Some(&setup.user),
        &[("Range", "bytes=100-"), ("If-Range", &etag)],
    );
    assert_eq!(rest.status, 206);
    assert_eq!([start.body, rest.body].concat(), file);
    let past = setup.get_with(
        &paths::audio(one.id),
        Some(&setup.user),
        &[("Range", &format!("bytes={}-", file.len()))],
    );
    assert_eq!(past.status, 416);
    assert_eq!(past.error(), ErrorCode::RangeNotSatisfiable);
    assert_eq!(
        setup.get(&paths::audio(999_999), Some(&setup.user)).status,
        404
    );

    let edit = json!({ "groups": [{ "tracks": [one.id], "edit": { "title": { "op": "set", "value": "Uno" } } }] });
    let forbidden = setup.post(paths::TAGS, &setup.user, Some(edit.clone()));
    assert_eq!(forbidden.status, 403);
    assert_eq!(forbidden.error(), ErrorCode::AdminOnly);
    let written: TagWriteResult = setup.post(paths::TAGS, &setup.admin, Some(edit)).json();
    assert_eq!(written.written, [one.id]);
    assert!(written.failed.is_empty());

    let (changed, head) = setup.sync(head, 100);
    assert_eq!(changed.len(), 1);
    let uno = &changed[0];
    assert_eq!((uno.id, uno.title.as_str()), (one.id, "Uno"));
    assert_ne!(uno.audio_etag(), etag);
    let stale = setup.get_with(
        &paths::audio(one.id),
        Some(&setup.user),
        &[("Range", "bytes=100-"), ("If-Range", &etag)],
    );
    assert_eq!(
        stale.status, 200,
        "a changed file is sent whole, never resumed"
    );
    assert_eq!(stale.header("etag"), Some(uno.audio_etag().as_str()));

    let invalid = setup.post(
        paths::TAGS,
        &setup.admin,
        Some(json!({ "groups": [{ "tracks": [one.id], "edit": { "year": { "op": "set", "value": 0 } } }] })),
    );
    assert_eq!(invalid.status, 400);
    assert_eq!(invalid.error(), ErrorCode::InvalidTagEdit);

    let cover = base64::engine::general_purpose::STANDARD.encode(png());
    let with_cover: TagWriteResult = setup
        .post(
            paths::TAGS,
            &setup.admin,
            Some(json!({ "groups": [{ "tracks": [one.id], "edit": { "cover": { "op": "set", "value": cover } } }] })),
        )
        .json();
    assert_eq!(with_cover.written, [one.id]);
    assert_eq!(
        setup.get(&paths::cover(one.id), Some(&setup.user)).status,
        403
    );
    let embedded = setup.get(&paths::cover(one.id), Some(&setup.admin));
    assert_eq!(embedded.status, 200);
    assert_eq!(embedded.body, png());
    assert!(!setup.sync(head, 100).0.is_empty());
}

#[test]
fn admins_manage_music_folders_and_users_only_see_them() {
    let setup = setup();
    let (_, head) = setup.sync(Cursor::default(), 100);

    let listed: Vec<Folder> = setup.get(paths::FOLDERS, Some(&setup.user)).json();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "music");
    assert_eq!(listed[0].path, None);
    assert!(listed[0].configured && listed[0].reachable);
    let configured = listed[0].id;
    let as_admin: Vec<Folder> = setup.get(paths::FOLDERS, Some(&setup.admin)).json();
    assert_eq!(
        as_admin[0].path.as_deref().map(PathBuf::from),
        Some(setup.base.join("music"))
    );

    let for_user: ScanStatus = setup.get(paths::SCAN, Some(&setup.user)).json();
    let skipped = &for_user.last.unwrap().skipped;
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].path, "opus.ogg");
    let for_admin: ScanStatus = setup.get(paths::SCAN, Some(&setup.admin)).json();
    assert_eq!(
        PathBuf::from(&for_admin.last.unwrap().skipped[0].path),
        setup.base.join("music/Album/opus.ogg")
    );

    let removed = setup.delete(&paths::folder(configured), &setup.admin);
    assert_eq!(removed.status, 409);
    assert_eq!(removed.error(), ErrorCode::ConfiguredFolder);
    assert_eq!(
        setup.delete(&paths::folder(configured), &setup.user).status,
        403
    );
    let missing = setup.post(
        paths::FOLDERS,
        &setup.admin,
        Some(json!({ "path": setup.base.join("nowhere") })),
    );
    assert_eq!(missing.status, 400);
    assert_eq!(missing.error(), ErrorCode::NotAFolder);
    assert_eq!(setup.post(paths::SCAN, &setup.user, None).status, 403);

    let more = setup.base.join("more");
    add_song(&more.join("Other"), "3.flac", "Three", "Other");
    let added = setup.post(paths::FOLDERS, &setup.admin, Some(json!({ "path": more })));
    assert_eq!(added.status, 201);
    let added: Folder = added.json();
    assert!(!added.configured);
    setup.wait_until("the new folder's track", |setup| {
        setup
            .sync(head, 100)
            .0
            .iter()
            .any(|track| track.title == "Three")
    });
    let (tracks, head) = setup.sync(head, 100);
    let three = by_title(&tracks, "Three");
    assert_eq!(three.folder, Some(added.id));
    assert_eq!(three.path, "Other/3.flac");

    assert_eq!(
        setup.delete(&paths::folder(added.id), &setup.admin).status,
        204
    );
    let (gone, _) = setup.sync(head, 100);
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].title, "Three");
    assert!(!gone[0].available);
    assert_eq!(gone[0].folder, None);

    let started = setup.post(paths::SCAN, &setup.admin, None);
    assert_eq!(started.status, 202);
    assert!(started.json::<ScanStatus>().running);
}

#[test]
fn tokens_are_replaced_from_the_app_or_the_command_unless_configured() {
    let setup = setup();
    let fresh: NewToken = setup
        .post(&paths::token(Role::User), &setup.admin, None)
        .json();
    assert_eq!(fresh.role, Role::User);
    assert_ne!(fresh.token, setup.user);
    assert_eq!(setup.get(paths::HANDSHAKE, Some(&setup.user)).status, 401);
    assert_eq!(setup.get(paths::HANDSHAKE, Some(&fresh.token)).status, 200);
    assert_eq!(
        setup
            .post(&paths::token(Role::Admin), &fresh.token, None)
            .status,
        403
    );

    let mut config = Config::new(setup.base.join("data"));
    config.listen = "127.0.0.1:0".parse().unwrap();
    let replaced = vcmp_server::replace_token(&config, Role::User).unwrap();
    setup.wait_until("the server to read the replaced token", |setup| {
        setup.get(paths::HANDSHAKE, Some(&replaced)).status == 200
    });
    assert_eq!(setup.get(paths::HANDSHAKE, Some(&fresh.token)).status, 401);
}

#[test]
fn configured_tokens_are_used_and_cannot_be_replaced() {
    let user = "configured-user-token-0123456789";
    let admin = "configured-admin-token-0123456789";
    let setup = setup_with(|config| {
        config.user_token = Some(user.into());
        config.admin_token = Some(admin.into());
        config.name = "Home".into();
    });
    assert_eq!(setup.user, user);
    let handshake: Handshake = setup.get(paths::HANDSHAKE, Some(admin)).json();
    assert_eq!(
        (handshake.role, handshake.name.as_str()),
        (Role::Admin, "Home")
    );
    let refused = setup.post(&paths::token(Role::User), admin, None);
    assert_eq!(refused.status, 409);
    assert_eq!(refused.error(), ErrorCode::ConfiguredToken);

    let mut config = Config::new(setup.base.join("data"));
    config.user_token = Some(user.into());
    assert!(matches!(
        vcmp_server::replace_token(&config, Role::User),
        Err(vcmp_server::ReplaceError::Configured(Role::User))
    ));
}
