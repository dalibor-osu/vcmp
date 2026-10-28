//! The server over real HTTP on a free port, with changes made straight on a library
//! connection of the test's own: the token and Origin checks, the handshake, reading
//! tags, and changes that wait for review. Files are copies in a temporary directory.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};
use serde_json::{Value, json};
use vcmp_library::{
    ChangeSetId, ChangeStatus, LabelEdit, Library, NewLabel, PlaylistId, TagEdit, TrackId,
};
use vcmp_mcp::{Changes, PlaylistEdit, Server, ServerConfig, relay};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../vcmp-library/tests/fixtures/silence.flac"
);
const TOKEN: &str = "test-token";

/// The app's part, done at once on a connection of the test's own.
struct Direct(Mutex<Library>);

impl Changes for Direct {
    fn submit_tags(
        &self,
        agent: &str,
        edits: Vec<(TrackId, TagEdit)>,
    ) -> Result<ChangeSetId, String> {
        let library = self.0.lock().unwrap();
        library
            .submit_tag_changes(agent, &edits)
            .map_err(|error| error.to_string())
    }

    fn submit_labels(
        &self,
        agent: &str,
        edits: Vec<(TrackId, LabelEdit)>,
        new_labels: Vec<NewLabel>,
    ) -> Result<ChangeSetId, String> {
        let library = self.0.lock().unwrap();
        library
            .submit_label_changes(agent, &edits, &new_labels)
            .map_err(|error| error.to_string())
    }

    fn submit_playlist_deletion(
        &self,
        agent: &str,
        playlist: PlaylistId,
    ) -> Result<ChangeSetId, String> {
        let library = self.0.lock().unwrap();
        library
            .submit_playlist_deletion(agent, playlist)
            .map_err(|error| error.to_string())
    }

    fn submit_undo(&self, agent: &str, change: ChangeSetId) -> Result<ChangeSetId, String> {
        let library = self.0.lock().unwrap();
        library
            .submit_undo(change, agent)
            .map_err(|error| error.to_string())
    }

    fn edit_playlist(&self, edit: PlaylistEdit) -> Result<Option<PlaylistId>, String> {
        let library = self.0.lock().unwrap();
        let result = match edit {
            PlaylistEdit::Create(name) => library.create_playlist(&name).map(Some),
            PlaylistEdit::Rename(id, name) => library.rename_playlist(id, &name).map(|()| None),
            PlaylistEdit::Add(id, tracks) => library.add_to_playlist(id, &tracks).map(|()| None),
            PlaylistEdit::Remove(id, positions) => {
                library.remove_from_playlist(id, &positions).map(|()| None)
            }
            PlaylistEdit::Move(id, from, to) => {
                library.move_in_playlist(id, from, to).map(|()| None)
            }
        };
        result.map_err(|error| error.to_string())
    }
}

struct Setup {
    // Dropped first: the server's thread stops before the directory goes.
    server: Server,
    library: Library,
    http: ureq::Agent,
    _dir: tempfile::TempDir,
}

/// Two tagged files, one with a composer, scanned.
fn start() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(dir.path()).unwrap();
    let music = base.join("music");
    std::fs::create_dir_all(&music).unwrap();
    tagged(&music.join("a.flac"), "A", Some("Writer"));
    tagged(&music.join("b.flac"), "B", None);
    let database = base.join("library.sqlite3");
    let cache = base.join("cache");
    let mut library = Library::open(&database, &cache).unwrap();
    library.add_source(&music, true).unwrap();
    library.scan(&AtomicBool::new(false), |_| {}).unwrap();

    let changes = Arc::new(Direct(Mutex::new(
        Library::open(&database, &cache).unwrap(),
    )));
    let server = Server::start(
        ServerConfig {
            port: 0,
            token: TOKEN.into(),
            database,
            cache,
        },
        changes,
    )
    .unwrap();
    let http = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    Setup {
        server,
        library,
        http,
        _dir: dir,
    }
}

fn tagged(path: &PathBuf, title: &str, composer: Option<&str>) {
    std::fs::copy(FIXTURE, path).unwrap();
    let file = lofty::read_from_path(path).unwrap();
    let mut tag = Tag::new(file.primary_tag_type());
    tag.set_title(title.into());
    tag.set_artist("Artist".into());
    tag.set_album("Album".into());
    if let Some(composer) = composer {
        tag.insert_text(ItemKey::Composer, composer.into());
    }
    tag.save_to_path(path, WriteOptions::default()).unwrap();
}

/// A JSON-RPC answer, from a JSON body or an event stream's data lines.
fn answer(body: &str) -> Value {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return value;
    }
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .find(|value| value.get("id").is_some())
        .unwrap_or_else(|| panic!("no answer in {body:?}"))
}

/// An MCP session: the handshake done, ready for tool calls.
struct Session<'a> {
    setup: &'a Setup,
    id: String,
    next: u64,
}

impl Setup {
    fn post(
        &self,
        body: &Value,
        session: Option<&str>,
        token: Option<&str>,
        origin: Option<&str>,
    ) -> (u16, String, Option<String>) {
        let mut request = self
            .http
            .post(self.server.address())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(token) = token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        let mut response = request.send(body.to_string()).unwrap();
        let status = response.status().as_u16();
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let text = response.body_mut().read_to_string().unwrap_or_default();
        (status, text, session)
    }

    fn session(&self) -> Session<'_> {
        let (status, body, session) = self.post(
            &json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": { "name": "test-agent", "version": "1.0" }
                }
            }),
            None,
            Some(TOKEN),
            None,
        );
        assert_eq!(status, 200, "{body}");
        let id = session.expect("a session id");
        let instructions = answer(&body)["result"]["instructions"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(instructions.contains("album_artist"), "{instructions}");
        assert_eq!(answer(&body)["result"]["serverInfo"]["name"], "vcmp");
        self.post(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            Some(&id),
            Some(TOKEN),
            None,
        );
        Session {
            setup: self,
            id,
            next: 1,
        }
    }
}

impl Session<'_> {
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let (status, body, _) = self.setup.post(
            &json!({ "jsonrpc": "2.0", "id": self.next, "method": method, "params": params }),
            Some(&self.id),
            Some(TOKEN),
            None,
        );
        assert_eq!(status, 200, "{body}");
        answer(&body)
    }

    /// Calls a tool; its JSON answer, or the error text.
    fn call(&mut self, tool: &str, arguments: Value) -> Result<Value, String> {
        let result = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        )["result"]
            .clone();
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        if result["isError"] == json!(true) {
            return Err(text.to_owned());
        }
        Ok(serde_json::from_str(text).unwrap_or(Value::String(text.to_owned())))
    }
}

fn id_of(library: &Library, path_end: &str) -> TrackId {
    library
        .tracks()
        .unwrap()
        .into_iter()
        .find(|track| track.path.ends_with(Path::new(path_end)))
        .unwrap()
        .id
}

#[test]
fn only_requests_with_the_token_and_without_a_browser_origin_get_in() {
    let setup = start();
    let initialize = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1.0" }
        }
    });
    let (status, _, _) = setup.post(&initialize, None, None, None);
    assert_eq!(status, 401);
    let (status, _, _) = setup.post(&initialize, None, Some("wrong"), None);
    assert_eq!(status, 401);
    let (status, _, _) = setup.post(&initialize, None, Some(TOKEN), Some("https://example.com"));
    assert_eq!(status, 403);
    let (status, _, _) = setup.post(&initialize, None, Some(TOKEN), None);
    assert_eq!(status, 200);
}

#[test]
fn agents_read_every_tag_and_find_tracks_by_tag() {
    let setup = start();
    let mut session = setup.session();
    let tools = session.request("tools/list", json!({}))["result"]["tools"].clone();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    for expected in [
        "search",
        "get_tracks",
        "propose_tag_changes",
        "propose_playlist_deletion",
    ] {
        assert!(names.contains(&expected), "{names:?}");
    }

    let a = id_of(&setup.library, "a.flac");
    let tracks = session
        .call("get_tracks", json!({ "track_ids": [a.0] }))
        .unwrap();
    assert_eq!(tracks[0]["title"], "A");
    assert_eq!(tracks[0]["tags"]["Composer"], json!(["Writer"]));

    let missing = session
        .call(
            "find_tracks_by_tag",
            json!({ "key": "Composer", "missing": true }),
        )
        .unwrap();
    assert_eq!(missing.as_array().unwrap().len(), 1);
    assert_eq!(missing[0]["title"], "B");
    assert_eq!(
        session
            .call("get_tracks", json!({ "track_ids": [999] }))
            .unwrap_err(),
        "there is no track 999"
    );
}

#[test]
fn tag_changes_wait_for_review_under_the_agents_name() {
    let setup = start();
    let mut session = setup.session();
    let a = id_of(&setup.library, "a.flac");
    let before = std::fs::read(setup.library.track(a).unwrap().unwrap().path).unwrap();

    let answer = session
        .call(
            "propose_tag_changes",
            json!({ "changes": [{
                "track_id": a.0,
                "album_artist": "Artist",
                "remove": ["genre"],
                "tags": { "Composer": ["Someone"], "Label": [] }
            }] }),
        )
        .unwrap();
    assert_eq!(answer["status"], "pending");
    let id = ChangeSetId(answer["change_set_id"].as_i64().unwrap());
    let set = setup.library.change_set(id).unwrap().unwrap();
    assert_eq!(set.summary.status, ChangeStatus::Pending);
    assert_eq!(set.summary.agent, "test-agent");
    assert_eq!(
        std::fs::read(setup.library.track(a).unwrap().unwrap().path).unwrap(),
        before,
        "nothing is written"
    );
    let status = session
        .call("get_change_set", json!({ "change_set_id": id.0 }))
        .unwrap();
    assert_eq!(status["status"], "pending");

    let refused = session
        .call(
            "propose_tag_changes",
            json!({ "changes": [{ "track_id": a.0, "tags": { "TrackTitle": ["X"] } }] }),
        )
        .unwrap_err();
    assert!(refused.contains("TrackTitle"), "{refused}");
    let both = session
        .call(
            "propose_tag_changes",
            json!({ "changes": [{ "track_id": a.0, "genre": "Jazz", "remove": ["genre"] }] }),
        )
        .unwrap_err();
    assert!(both.contains("both set and removed"), "{both}");
}

/// A 1 x 1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn a_cover_from_a_file_waits_for_review_like_any_tag() {
    let setup = start();
    let mut session = setup.session();
    let (a, b) = (
        id_of(&setup.library, "a.flac"),
        id_of(&setup.library, "b.flac"),
    );
    let image = setup._dir.path().join("cover.png");
    std::fs::write(&image, PNG).unwrap();
    let cover = json!({ "file": image.to_str().unwrap() });

    let answer = session
        .call(
            "propose_tag_changes",
            json!({ "changes": [
                { "track_id": a.0, "cover": cover },
                { "track_id": b.0, "cover": cover },
            ] }),
        )
        .unwrap();
    let id = ChangeSetId(answer["change_set_id"].as_i64().unwrap());
    let set = setup.library.change_set(id).unwrap().unwrap();
    assert_eq!(set.files.len(), 2);
    for file in &set.files {
        assert!(
            matches!(&file.edits[..], [(target, Some(_))] if target == "cover"),
            "{:?}",
            file.edits
        );
        assert_eq!(file.cover_before, Some(None), "the files had no cover");
    }
    let status = session
        .call("get_change_set", json!({ "change_set_id": id.0 }))
        .unwrap();
    assert_eq!(status["files"][0]["cover"], "replaced");

    let text = setup._dir.path().join("notes.txt");
    std::fs::write(&text, "not a picture").unwrap();
    let refused = session
        .call(
            "propose_tag_changes",
            json!({ "changes": [{ "track_id": a.0, "cover": { "file": text.to_str().unwrap() } }] }),
        )
        .unwrap_err();
    assert!(refused.contains("not a JPEG or PNG"), "{refused}");
    let relative = session
        .call(
            "propose_tag_changes",
            json!({ "changes": [{ "track_id": a.0, "cover": { "file": "cover.png" } }] }),
        )
        .unwrap_err();
    assert!(relative.contains("absolute path"), "{relative}");
    let page = session
        .call(
            "propose_tag_changes",
            json!({ "changes": [{ "track_id": a.0, "cover": { "url": "file:///etc/passwd" } }] }),
        )
        .unwrap_err();
    assert!(page.contains("only http and https"), "{page}");

    let removed = session
        .call(
            "propose_tag_changes",
            json!({ "changes": [{ "track_id": a.0, "remove": ["cover"] }] }),
        )
        .unwrap();
    let set = setup
        .library
        .change_set(ChangeSetId(removed["change_set_id"].as_i64().unwrap()))
        .unwrap()
        .unwrap();
    assert!(matches!(&set.files[0].edits[..], [(target, None)] if target == "cover"));
}

#[test]
fn agents_find_tracks_by_label_and_label_changes_wait_for_review() {
    let setup = start();
    let mut session = setup.session();
    let a = id_of(&setup.library, "a.flac");
    let b = id_of(&setup.library, "b.flac");
    let chill = setup.library.create_label("Chill", "palette:teal").unwrap();
    setup.library.add_label(chill, &[a]).unwrap();

    let labels = session.call("list_labels", json!({})).unwrap();
    assert_eq!(
        labels,
        json!([{ "name": "Chill", "colour": "teal", "track_count": 1 }])
    );
    let found = session
        .call("find_tracks_by_labels", json!({ "labels": ["chill"] }))
        .unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);
    assert_eq!(found[0]["id"], a.0);
    assert_eq!(found[0]["labels"], json!(["Chill"]));
    let unknown = session
        .call("find_tracks_by_labels", json!({ "labels": ["loud"] }))
        .unwrap_err();
    assert!(unknown.contains("loud"), "{unknown}");
    let tracks = session
        .call(
            "get_tracks",
            json!({ "track_ids": [b.0], "include_tags": false }),
        )
        .unwrap();
    assert_eq!(tracks[0]["labels"], json!([]));

    let answer = session
        .call(
            "propose_label_changes",
            json!({
                "changes": [
                    { "track_ids": [a.0, b.0], "add": ["rainy"] },
                    { "track_ids": [b.0], "add": ["chill"] }
                ],
                "new_labels": [{ "name": "Rainy", "colour": "Blue" }]
            }),
        )
        .unwrap();
    assert_eq!(answer["status"], "pending");
    let id = ChangeSetId(answer["change_set_id"].as_i64().unwrap());
    let set = setup.library.change_set(id).unwrap().unwrap();
    assert_eq!(set.summary.agent, "test-agent");
    assert_eq!(set.files.len(), 2);
    assert_eq!(set.new_labels[0].colour, "palette:blue");
    assert!(setup.library.label_named("rainy").unwrap().is_none());
    let status = session
        .call("get_change_set", json!({ "change_set_id": id.0 }))
        .unwrap();
    assert_eq!(status["kind"], "labels");
    assert_eq!(status["new_labels"][0]["name"], "Rainy");
    let b_labels = &status["tracks"][1]["labels"];
    assert_eq!(b_labels[0]["label"], "Chill");
    assert_eq!(b_labels[0]["held_before"], false);
    assert_eq!(b_labels[0]["asked"], true);

    let colour = session
        .call(
            "propose_label_changes",
            json!({
                "changes": [{ "track_ids": [a.0], "add": ["x"] }],
                "new_labels": [{ "name": "x", "colour": "beige" }]
            }),
        )
        .unwrap_err();
    assert!(colour.contains("teal"), "{colour}");
    let nothing = session
        .call(
            "propose_label_changes",
            json!({ "changes": [{ "track_ids": [a.0], "add": ["chill"] }] }),
        )
        .unwrap_err();
    assert!(nothing.contains("at least one change"), "{nothing}");
}

#[test]
fn playlists_change_at_once_but_deleting_waits() {
    let setup = start();
    let mut session = setup.session();
    let a = id_of(&setup.library, "a.flac");
    let b = id_of(&setup.library, "b.flac");

    let created = session
        .call("create_playlist", json!({ "name": "Mix" }))
        .unwrap();
    let id = created["playlist_id"].as_i64().unwrap();
    session
        .call(
            "add_to_playlist",
            json!({ "playlist_id": id, "track_ids": [a.0, b.0] }),
        )
        .unwrap();
    session
        .call(
            "move_in_playlist",
            json!({ "playlist_id": id, "from": 1, "to": 0 }),
        )
        .unwrap();
    session
        .call(
            "rename_playlist",
            json!({ "playlist_id": id, "name": "Better mix" }),
        )
        .unwrap();
    let playlist = session
        .call("get_playlist", json!({ "playlist_id": id }))
        .unwrap();
    assert_eq!(playlist["playlist"]["name"], "Better mix");
    assert_eq!(playlist["entries"][0]["track"]["title"], "B");

    let asked = session
        .call("propose_playlist_deletion", json!({ "playlist_id": id }))
        .unwrap();
    assert_eq!(asked["status"], "pending");
    assert!(setup.library.playlist(PlaylistId(id)).unwrap().is_some());
}

/// Runs the relay over `lines`, with `settings`, and returns what it wrote, line by
/// line.
fn relay(settings: &relay::Settings<'_>, lines: &[Value]) -> Vec<Value> {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let mut output = Vec::new();
    relay::run(settings, input.as_bytes(), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn handshake_and_call(tool: &str, arguments: Value) -> Vec<Value> {
    vec![
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "relayed-agent", "version": "1.0" }
            }
        }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        }),
    ]
}

#[test]
fn the_relay_carries_a_session_over_standard_input_and_output() {
    let setup = start();
    let port = setup.server.port();
    let settings = || {
        Ok(relay::Connection {
            port,
            token: TOKEN.into(),
        })
    };
    let a = id_of(&setup.library, "a.flac");
    let answers = relay(
        &settings,
        &handshake_and_call("get_tracks", json!({ "track_ids": [a.0] })),
    );
    let ids: Vec<&Value> = answers.iter().map(|answer| &answer["id"]).collect();
    assert_eq!(ids, [&json!(1), &json!(2), &json!(3)], "{answers:?}");
    assert_eq!(answers[0]["result"]["serverInfo"]["name"], "vcmp");
    assert!(answers[1]["result"]["tools"].as_array().unwrap().len() > 10);
    let text = answers[2]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"Writer\""), "{text}");

    // A change through the relay is recorded under the relayed client's name.
    let answers = relay(
        &settings,
        &handshake_and_call(
            "propose_tag_changes",
            json!({ "changes": [{ "track_id": a.0, "genre": "Relayed" }] }),
        ),
    );
    let text = answers[2]["result"]["content"][0]["text"].as_str().unwrap();
    let id: Value = serde_json::from_str(text).unwrap();
    let set = setup
        .library
        .change_set(ChangeSetId(id["change_set_id"].as_i64().unwrap()))
        .unwrap()
        .unwrap();
    assert_eq!(set.summary.agent, "relayed-agent");
}

#[test]
fn the_relay_says_why_it_reaches_no_vcmp() {
    // A port nothing listens on: VCMP is closed.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let closed = || {
        Ok(relay::Connection {
            port,
            token: TOKEN.into(),
        })
    };
    let answers = relay(&closed, &handshake_and_call("list_playlists", json!({})));
    // The notification has no answer; each request gets the reason.
    assert_eq!(answers.len(), 3, "{answers:?}");
    let message = answers[0]["error"]["message"].as_str().unwrap();
    assert!(message.contains("isn't open"), "{message}");
    assert_eq!(answers[0]["id"], 1);

    let off = || Err("VCMP's AI agents switch is off.".to_owned());
    let answers = relay(&off, &handshake_and_call("list_playlists", json!({})));
    assert_eq!(
        answers[0]["error"]["message"],
        "VCMP's AI agents switch is off."
    );
}

#[test]
fn the_relay_reads_the_settings_again_for_a_new_token() {
    let setup = start();
    let port = setup.server.port();
    // The token the relay read first is old by the time it sends.
    let reads = std::cell::Cell::new(0);
    let settings = || {
        reads.set(reads.get() + 1);
        Ok(relay::Connection {
            port,
            token: if reads.get() == 1 { "old" } else { TOKEN }.into(),
        })
    };
    let answers = relay(&settings, &handshake_and_call("list_playlists", json!({})));
    assert!(
        answers.iter().all(|answer| answer.get("error").is_none()),
        "{answers:?}"
    );
    assert_eq!(reads.get(), 2);
}
