//! The server over real HTTP on a free port, with changes made straight on a library
//! connection of the test's own and a recorder standing in for the player: the token
//! and Origin checks, the handshake, reading tags, changes that wait for review, and
//! playback with some tracks left out. Files are copies in a temporary directory.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};
use serde_json::{Value, json};
use vcmp_library::{
    AlbumId, ArtistId, ChangeSetId, ChangeStatus, FeedCursor, Format, LabelEdit, Library,
    MirroredTrack, NewLabel, PlaylistId, ServerId, TagEdit, TrackId, remote_key,
    server_folder_location,
};
use vcmp_mcp::Changes;
use vcmp_mcp::{
    Context, Control, Options, OutputDevices, Play, PlayState, Playback, PlaylistEdit, Repeat,
    Server, ServerConfig, ShuffleKind, SleepTimer, Status, relay,
};

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

/// What an agent asked of the player.
#[derive(Debug, PartialEq)]
enum Asked {
    /// The tracks by id, the context, the start and shuffle.
    Play(Vec<i64>, Context, Option<usize>, Option<bool>),
    Queue(Vec<i64>, bool),
    Control(Control),
    Options(Options),
    Device(Option<String>),
    Sleep(SleepTimer),
}

/// The player's part: records what is asked and answers the status it is given.
#[derive(Default)]
struct Recorder {
    asked: Mutex<Vec<Asked>>,
    status: Mutex<Option<Status>>,
}

impl Recorder {
    fn take(&self) -> Vec<Asked> {
        std::mem::take(&mut self.asked.lock().unwrap())
    }

    fn ask(&self, asked: Asked) -> Result<(), String> {
        self.asked.lock().unwrap().push(asked);
        Ok(())
    }
}

const DEVICES: [&str; 2] = ["Speakers", "Headphones"];

impl Playback for Recorder {
    fn play(&self, play: Play) -> Result<(), String> {
        let ids = play.tracks.iter().map(|track| track.id.0).collect();
        self.ask(Asked::Play(ids, play.context, play.start, play.shuffle))
    }

    fn queue(&self, tracks: Vec<vcmp_library::Track>, next: bool) -> Result<(), String> {
        self.ask(Asked::Queue(
            tracks.iter().map(|track| track.id.0).collect(),
            next,
        ))
    }

    fn control(&self, control: Control) -> Result<(), String> {
        self.ask(Asked::Control(control))
    }

    fn set_options(&self, options: Options) -> Result<(), String> {
        self.ask(Asked::Options(options))
    }

    fn output_devices(&self) -> Result<OutputDevices, String> {
        Ok(OutputDevices {
            devices: DEVICES.map(String::from).to_vec(),
            chosen: Some("Headphones".into()),
            fallback: Some("Speakers".into()),
        })
    }

    fn set_output_device(&self, device: Option<String>) -> Result<(), String> {
        if device
            .as_deref()
            .is_some_and(|name| !DEVICES.contains(&name))
        {
            return Err("there is no output device named that".into());
        }
        self.ask(Asked::Device(device))
    }

    fn set_sleep_timer(&self, timer: SleepTimer) -> Result<(), String> {
        self.ask(Asked::Sleep(timer))
    }

    fn status(&self) -> Result<Status, String> {
        self.status
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "no status".to_owned())
    }
}

struct Setup {
    // Dropped first: the server's thread stops before the directory goes.
    server: Server,
    library: Library,
    player: Arc<Recorder>,
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
    let player = Arc::new(Recorder::default());
    let server = Server::start(
        ServerConfig {
            port: 0,
            token: TOKEN.into(),
            database,
            cache,
        },
        changes,
        player.clone(),
    )
    .unwrap();
    let http = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    Setup {
        server,
        library,
        player,
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
        assert!(instructions.contains("left_out"), "{instructions}");
        assert!(
            instructions.contains("Playlists and the queue may still hold some"),
            "{instructions}"
        );
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
        .find(|track| {
            track
                .file
                .as_ref()
                .is_some_and(|file| file.ends_with(Path::new(path_end)))
        })
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
        "play_tracks",
        "get_now_playing",
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
    assert_eq!(missing["total"], 1);
    assert_eq!(missing["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(missing["tracks"][0]["title"], "B");
    assert!(missing["tracks"][0]["path"].is_string());

    let page = session
        .call(
            "find_tracks_by_tag",
            json!({ "key": "Composer", "missing": true, "brief": true, "offset": 1 }),
        )
        .unwrap();
    assert_eq!(page["total"], 1, "the total counts past the page");
    assert_eq!(page["tracks"], json!([]));
    let brief = session
        .call(
            "find_tracks_by_tag",
            json!({ "key": "Composer", "missing": true, "brief": true }),
        )
        .unwrap();
    let b = id_of(&setup.library, "b.flac");
    let track = &brief["tracks"][0];
    assert_eq!(track["id"], b.0);
    assert_eq!(track["title"], "B");
    assert!(track.get("path").is_none(), "{track}");
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
    let before = std::fs::read(setup.library.track(a).unwrap().unwrap().file.unwrap()).unwrap();

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
        std::fs::read(setup.library.track(a).unwrap().unwrap().file.unwrap()).unwrap(),
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
fn labels_can_be_made_for_no_track() {
    let setup = start();
    let mut session = setup.session();
    // The owner accepts in the window, on a connection of its own.
    let base = dunce::canonicalize(setup._dir.path()).unwrap();
    let mut owner = Library::open(&base.join("library.sqlite3"), &base.join("cache")).unwrap();

    let empty = session
        .call("propose_label_changes", json!({}))
        .unwrap_err();
    assert!(empty.contains("labels to create"), "{empty}");

    let answer = session
        .call(
            "propose_label_changes",
            json!({
                "new_labels": [
                    { "name": "Favourite", "colour": "pink" },
                    { "name": " Chill ", "colour": "teal" },
                    { "name": "Workout", "colour": "#ff8800" }
                ]
            }),
        )
        .unwrap();
    assert_eq!(answer["status"], "pending");
    assert_eq!(
        answer["new_labels"],
        json!(["Favourite", "Chill", "Workout"])
    );
    let id = ChangeSetId(answer["change_set_id"].as_i64().unwrap());
    let status = session
        .call("get_change_set", json!({ "change_set_id": id.0 }))
        .unwrap();
    assert_eq!(status["kind"], "labels");
    assert_eq!(status["new_labels"].as_array().unwrap().len(), 3);
    assert_eq!(status["tracks"], json!([]));

    owner.accept_change_set(id, |_, _| {}).unwrap();
    let labels = session.call("list_labels", json!({})).unwrap();
    assert_eq!(
        labels,
        json!([
            { "name": "Chill", "colour": "teal", "track_count": 0 },
            { "name": "Favourite", "colour": "pink", "track_count": 0 },
            { "name": "Workout", "colour": "#ff8800", "track_count": 0 }
        ])
    );

    let undo = session
        .call("propose_undo", json!({ "change_set_id": id.0 }))
        .unwrap();
    let undo = ChangeSetId(undo["change_set_id"].as_i64().unwrap());
    owner.accept_change_set(undo, |_, _| {}).unwrap();
    let labels = session.call("list_labels", json!({})).unwrap();
    assert_eq!(labels, json!([]));
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

/// Removes `b.flac` from the disk, as if its drive were unplugged; the library still
/// counts it as available.
fn lose_b(setup: &Setup) -> TrackId {
    let b = id_of(&setup.library, "b.flac");
    std::fs::remove_file(setup.library.track(b).unwrap().unwrap().file.unwrap()).unwrap();
    b
}

#[test]
fn agents_play_tracks_and_hear_which_were_left_out_and_why() {
    let setup = start();
    let mut session = setup.session();
    let a = id_of(&setup.library, "a.flac");
    let b = lose_b(&setup);

    let answer = session
        .call("play_tracks", json!({ "track_ids": [b.0, a.0, 999] }))
        .unwrap();
    assert_eq!(answer["playing"], 1);
    assert_eq!(answer["starts_with"], Value::Null);
    let left_out = answer["left_out"].as_array().unwrap();
    assert_eq!(left_out.len(), 2);
    assert_eq!(left_out[0]["track_id"], b.0);
    assert_eq!(left_out[0]["title"], "B");
    assert!(
        left_out[0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("its file is missing"),
        "{answer}"
    );
    assert_eq!(left_out[1]["track_id"], 999);
    assert_eq!(left_out[1]["reason"], "there is no track 999");
    assert_eq!(
        setup.player.take(),
        [Asked::Play(
            vec![a.0],
            Context::Agent("test-agent".into()),
            None,
            None
        )]
    );

    // Starting at a track left out starts at the next one that plays.
    let answer = session
        .call(
            "play_tracks",
            json!({ "track_ids": [b.0, a.0], "start_track_id": b.0, "shuffle": false }),
        )
        .unwrap();
    assert_eq!(answer["starts_with"]["title"], "A");
    assert_eq!(
        setup.player.take(),
        [Asked::Play(
            vec![a.0],
            Context::Agent("test-agent".into()),
            Some(0),
            Some(false)
        )]
    );

    let none = session
        .call("play_tracks", json!({ "track_ids": [b.0, 999] }))
        .unwrap_err();
    assert!(
        none.starts_with("none of these tracks can be played"),
        "{none}"
    );
    assert!(none.contains("its file is missing"), "{none}");
    assert!(none.contains("there is no track 999"), "{none}");
    let elsewhere = session
        .call(
            "play_tracks",
            json!({ "track_ids": [a.0], "start_track_id": b.0 }),
        )
        .unwrap_err();
    assert!(elsewhere.contains("not among track_ids"), "{elsewhere}");
    assert!(setup.player.take().is_empty(), "nothing more played");

    let answer = session
        .call(
            "queue_tracks",
            json!({ "track_ids": [a.0, b.0], "at": "next" }),
        )
        .unwrap();
    assert_eq!(answer["queued"], 1);
    assert_eq!(answer["left_out"][0]["track_id"], b.0);
    session
        .call("queue_tracks", json!({ "track_ids": [a.0] }))
        .unwrap();
    assert_eq!(
        setup.player.take(),
        [
            Asked::Queue(vec![a.0], true),
            Asked::Queue(vec![a.0], false)
        ]
    );
}

#[test]
fn albums_playlists_labels_and_artists_play_as_their_pages_do() {
    let setup = start();
    let mut session = setup.session();
    let a = id_of(&setup.library, "a.flac");
    let b = id_of(&setup.library, "b.flac");
    let album = setup.library.albums().unwrap()[0].id;
    let artist = setup.library.artists().unwrap()[0].id;
    let chill = setup.library.create_label("Chill", "palette:teal").unwrap();
    setup.library.add_label(chill, &[b]).unwrap();
    let playlist = setup.library.create_playlist("Mix").unwrap();
    setup.library.add_to_playlist(playlist, &[b, a, b]).unwrap();
    lose_b(&setup);

    let answer = session
        .call(
            "play_album",
            json!({ "album_id": album.0, "start_track_id": a.0, "shuffle": true }),
        )
        .unwrap();
    assert_eq!(answer["playing"], 1);
    assert_eq!(answer["left_out"][0]["track_id"], b.0);
    session
        .call("play_artist", json!({ "artist_id": artist.0 }))
        .unwrap();
    let answer = session
        .call(
            "play_playlist",
            json!({ "playlist_id": playlist.0, "start_position": 2 }),
        )
        .unwrap_err();
    assert!(answer.contains("from the one to start with"), "{answer}");
    let answer = session
        .call(
            "play_playlist",
            json!({ "playlist_id": playlist.0, "start_position": 0 }),
        )
        .unwrap();
    assert_eq!(answer["playing"], 1);
    assert_eq!(answer["starts_with"]["id"], a.0);
    let positions: Vec<&Value> = answer["left_out"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["position"])
        .collect();
    assert_eq!(positions, [&json!(0), &json!(2)]);
    assert_eq!(
        setup.player.take(),
        [
            Asked::Play(vec![a.0], Context::Album(album), Some(0), Some(true)),
            Asked::Play(vec![a.0], Context::Artist(artist), None, None),
            Asked::Play(vec![a.0], Context::Playlist(playlist), Some(0), None),
        ]
    );

    let only_lost = session
        .call("play_label", json!({ "label": "chill" }))
        .unwrap_err();
    assert!(only_lost.contains("its file is missing"), "{only_lost}");
    let unknown = session
        .call("play_label", json!({ "label": "loud" }))
        .unwrap_err();
    assert!(unknown.contains("list_labels"), "{unknown}");
    assert_eq!(
        session
            .call("play_album", json!({ "album_id": 999 }))
            .unwrap_err(),
        "there is no album 999"
    );
    assert!(setup.player.take().is_empty());
}

#[test]
fn agents_press_the_players_buttons_and_set_its_options() {
    let setup = start();
    let mut session = setup.session();
    for (arguments, control) in [
        (json!({ "action": "toggle" }), Control::TogglePlayPause),
        (json!({ "action": "next" }), Control::Next),
        (
            json!({ "action": "seek", "position_seconds": 12.5 }),
            Control::Seek(Duration::from_millis(12_500)),
        ),
        (json!({ "action": "show_window" }), Control::Raise),
    ] {
        session.call("playback_control", arguments).unwrap();
        assert_eq!(setup.player.take(), [Asked::Control(control)]);
    }
    let seek = session
        .call("playback_control", json!({ "action": "seek" }))
        .unwrap_err();
    assert!(seek.contains("position_seconds"), "{seek}");
    let quit = session
        .call("playback_control", json!({ "action": "quit" }))
        .unwrap_err();
    assert!(quit.contains("quit"), "no tool closes VCMP: {quit}");

    session
        .call(
            "edit_queue",
            json!({ "action": "move", "position": 2, "to": 0 }),
        )
        .unwrap();
    session
        .call("edit_queue", json!({ "action": "clear_up_next" }))
        .unwrap();
    assert_eq!(
        setup.player.take(),
        [
            Asked::Control(Control::MoveInUpNext { from: 2, to: 0 }),
            Asked::Control(Control::ClearUpNext),
        ]
    );

    session
        .call(
            "set_playback_options",
            json!({ "shuffle": true, "shuffle_kind": "true_random", "repeat": "one", "volume_percent": 40 }),
        )
        .unwrap();
    assert_eq!(
        setup.player.take(),
        [Asked::Options(Options {
            shuffle: Some(true),
            shuffle_kind: Some(ShuffleKind::TrueRandom),
            repeat: Some(Repeat::One),
            volume: Some(0.4),
        })]
    );
    let loud = session
        .call("set_playback_options", json!({ "volume_percent": 150 }))
        .unwrap_err();
    assert!(loud.contains("0 to 100"), "{loud}");

    session
        .call(
            "set_sleep_timer",
            json!({ "timer": "minutes", "minutes": 30 }),
        )
        .unwrap();
    session
        .call("set_sleep_timer", json!({ "timer": "end_of_track" }))
        .unwrap();
    let long = session
        .call(
            "set_sleep_timer",
            json!({ "timer": "minutes", "minutes": 600 }),
        )
        .unwrap_err();
    assert!(long.contains("1 to 90"), "{long}");
    assert_eq!(
        setup.player.take(),
        [
            Asked::Sleep(SleepTimer::Minutes(30)),
            Asked::Sleep(SleepTimer::EndOfTrack),
        ]
    );

    let devices = session.call("list_output_devices", json!({})).unwrap();
    assert_eq!(devices["devices"], json!(DEVICES));
    assert_eq!(devices["chosen"], "Headphones");
    assert_eq!(devices["playing_through_instead"], "Speakers");
    session
        .call("set_output_device", json!({ "device": "Speakers" }))
        .unwrap();
    session.call("set_output_device", json!({})).unwrap();
    assert!(
        session
            .call("set_output_device", json!({ "device": "Radio" }))
            .is_err()
    );
    assert_eq!(
        setup.player.take(),
        [Asked::Device(Some("Speakers".into())), Asked::Device(None)]
    );
}

#[test]
fn now_playing_names_the_tracks_and_what_they_play_from() {
    let setup = start();
    let mut session = setup.session();
    let a = setup
        .library
        .track(id_of(&setup.library, "a.flac"))
        .unwrap()
        .unwrap();
    let b = setup
        .library
        .track(id_of(&setup.library, "b.flac"))
        .unwrap()
        .unwrap();
    let album = setup.library.albums().unwrap()[0].id;
    let elsewhere = PathBuf::from("/not/in/the/library.flac");
    *setup.player.status.lock().unwrap() = Some(Status {
        state: PlayState::Paused,
        current: Some(a.file.clone().unwrap()),
        position: Duration::from_millis(61_240),
        duration: Some(Duration::from_secs(200)),
        volume: 0.5,
        muted: false,
        shuffle: true,
        shuffle_kind: ShuffleKind::FewerRepeats,
        repeat: Repeat::All,
        output: OutputDevices {
            devices: Vec::new(),
            chosen: None,
            fallback: None,
        },
        sleep_timer: SleepTimer::Minutes(25),
        context: Some(Context::Album(album)),
        up_next: vec![b.file.clone().unwrap()],
        upcoming: vec![elsewhere.clone(), b.file.clone().unwrap()],
        context_len: 5,
    });

    let now = session
        .call("get_now_playing", json!({ "upcoming_limit": 1 }))
        .unwrap();
    assert_eq!(now["state"], "paused");
    assert_eq!(now["track"]["title"], "A");
    assert_eq!(now["track"]["id"], a.id.0);
    assert_eq!(now["position_seconds"], 61.2);
    assert_eq!(now["duration_seconds"], 200);
    assert_eq!(now["volume_percent"], 50);
    assert_eq!(now["shuffle"], true);
    assert_eq!(now["shuffle_kind"], "fewer_repeats");
    assert_eq!(now["repeat"], "all");
    assert_eq!(now["output_device"], Value::Null);
    assert_eq!(
        now["sleep_timer"],
        json!({ "timer": "minutes", "minutes_left": 25 })
    );
    assert_eq!(
        now["context"],
        json!({ "kind": "album", "id": album.0, "name": "Album" })
    );
    assert_eq!(now["up_next"][0]["position"], 0);
    assert_eq!(now["up_next"][0]["track"]["title"], "B");
    assert_eq!(now["upcoming"].as_array().unwrap().len(), 1);
    assert_eq!(now["upcoming"][0]["position"], 1);
    assert_eq!(
        now["upcoming"][0]["track"],
        json!({ "path": elsewhere.to_string_lossy() })
    );
    assert_eq!(now["more_upcoming"], 1);
    assert_eq!(now["repeats"], true);
}

#[test]
fn a_repeating_context_is_listed_once() {
    let setup = start();
    let mut session = setup.session();
    let a = setup
        .library
        .track(id_of(&setup.library, "a.flac"))
        .unwrap()
        .unwrap();
    let status = |repeat, shuffle_kind, upcoming| Status {
        state: PlayState::Playing,
        current: Some(a.file.clone().unwrap()),
        position: Duration::ZERO,
        duration: None,
        volume: 1.0,
        muted: false,
        shuffle: shuffle_kind == ShuffleKind::TrueRandom,
        shuffle_kind,
        repeat,
        output: OutputDevices::default(),
        sleep_timer: SleepTimer::Off,
        context: None,
        up_next: Vec::new(),
        upcoming: vec![a.file.clone().unwrap(); upcoming],
        context_len: 1,
    };
    let mut now = |status| {
        *setup.player.status.lock().unwrap() = Some(status);
        session
            .call("get_now_playing", json!({ "upcoming_limit": 100 }))
            .unwrap()
    };

    for (repeat, kind) in [
        (Repeat::All, ShuffleKind::FewerRepeats),
        (Repeat::Off, ShuffleKind::TrueRandom),
    ] {
        let one = now(status(repeat, kind, 100));
        assert_eq!(one["upcoming"].as_array().unwrap().len(), 1, "{one}");
        assert_eq!(one["upcoming"][0]["track"]["title"], "A");
        assert_eq!(one["more_upcoming"], 0);
        assert_eq!(one["repeats"], true);
    }

    let off = now(status(Repeat::Off, ShuffleKind::FewerRepeats, 0));
    assert_eq!(off["upcoming"], json!([]));
    assert_eq!(off["repeats"], false);
}

const SERVER_SECRETS: [&str; 4] = [
    "afar.flac",
    "Remote Genre",
    "remote-release",
    "vcmp-server:",
];

struct Mirror {
    server: ServerId,
    near: TrackId,
    far: TrackId,
    far_album: AlbumId,
    artist: ArtistId,
    remote_artist: ArtistId,
}

fn mirrored(remote_id: i64, title: &str, artist: &str, album: &str) -> MirroredTrack {
    MirroredTrack {
        remote_id,
        available: true,
        folder: Some(3),
        path: format!("Hidden Folder/{remote_id} afar.flac"),
        format: Some(Format::Flac),
        size: 1000,
        modified_ms: 1,
        title: title.into(),
        artist: Some(artist.into()),
        album: Some(album.into()),
        album_artist: None,
        track_number: Some(1),
        disc_number: None,
        year: Some(1901),
        genre: Some("Remote Genre".into()),
        duration: Duration::from_secs(100),
        added_ms: 1,
        artwork: None,
        mb_release_id: Some(format!("remote-release-{remote_id}")),
        mb_artist_id: None,
        mb_album_artist_id: None,
        mb_recording_id: None,
    }
}

fn mirror(setup: &mut Setup) -> Mirror {
    let server = setup
        .library
        .add_server("test-server", "Home server", "http://192.168.1.2:8270")
        .unwrap()
        .id;
    setup
        .library
        .apply_server_page(
            server,
            &[
                mirrored(1, "Song from afar", "Artist", "Far Album"),
                mirrored(2, "Only remote", "Remote Artist", "Remote Album"),
            ],
            FeedCursor {
                revision: 1,
                track: 2,
            },
        )
        .unwrap();
    setup.library.finish_server_sync(server).unwrap();
    let near = setup.library.remote_track(server, 1).unwrap().unwrap();
    let far = setup.library.remote_track(server, 2).unwrap().unwrap();
    let far_album = setup
        .library
        .albums()
        .unwrap()
        .into_iter()
        .find(|album| album.title == "Far Album")
        .unwrap();
    assert_eq!(far_album.server, Some(server));
    let artist_named = |name: &str| {
        setup
            .library
            .artists()
            .unwrap()
            .into_iter()
            .find(|artist| artist.name == name)
            .unwrap()
    };
    let artist = artist_named("Artist");
    assert_eq!(
        (artist.album_count, artist.track_count),
        (2, 3),
        "the library counts the server's album and track"
    );
    Mirror {
        server,
        near,
        far,
        far_album: far_album.id,
        artist: artist.id,
        remote_artist: artist_named("Remote Artist").id,
    }
}

fn checked(session: &mut Session<'_>, tool: &str, arguments: Value) -> Result<Value, String> {
    let answer = session.call(tool, arguments);
    let text = match &answer {
        Ok(value) => value.to_string(),
        Err(error) => error.clone(),
    };
    for secret in SERVER_SECRETS {
        assert!(
            !text.contains(secret),
            "{tool} gave away {secret:?}: {text}"
        );
    }
    answer
}

fn near_entry() -> Value {
    json!({ "server_track": true, "title": "Song from afar", "artist": "Artist" })
}

const CANT_PLAY: &str = "it is a VCMP server's track, which agents can't play";

#[test]
fn a_servers_tracks_stay_out_of_reading_and_changes() {
    let mut setup = start();
    let mirror = mirror(&mut setup);
    let a = id_of(&setup.library, "a.flac");
    let b = id_of(&setup.library, "b.flac");
    let chill = setup.library.create_label("Chill", "palette:teal").unwrap();
    setup.library.add_label(chill, &[a, mirror.near]).unwrap();
    let playlist = setup.library.create_playlist("Mix").unwrap();
    setup.library.add_to_playlist(playlist, &[a]).unwrap();
    let mut session = setup.session();
    let local_artist = json!({
        "id": mirror.artist.0, "name": "Artist", "album_count": 1, "track_count": 2
    });

    for limit in [1, 20] {
        let found = checked(
            &mut session,
            "search",
            json!({ "text": "artist", "limit": limit }),
        )
        .unwrap();
        assert_eq!(found["artists"], json!([local_artist]), "{found}");
        let albums: Vec<&Value> = found["albums"]
            .as_array()
            .unwrap()
            .iter()
            .map(|album| &album["title"])
            .collect();
        assert_eq!(albums, [&json!("Album")], "{found}");
        let tracks = found["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), limit.min(2), "{found}");
        assert!(
            tracks
                .iter()
                .all(|track| track["id"] == a.0 || track["id"] == b.0),
            "{found}"
        );
    }
    assert_eq!(
        checked(&mut session, "search", json!({ "text": "afar" })).unwrap(),
        json!({ "artists": [], "albums": [], "tracks": [] })
    );

    let albums = checked(&mut session, "list_albums", json!({})).unwrap();
    assert_eq!(albums["total"], 1);
    assert_eq!(albums["albums"][0]["title"], "Album");
    assert_eq!(
        checked(
            &mut session,
            "get_album",
            json!({ "album_id": mirror.far_album.0 })
        )
        .unwrap_err(),
        format!("there is no album {}", mirror.far_album.0)
    );
    let artists = checked(&mut session, "list_artists", json!({})).unwrap();
    assert_eq!(artists, json!({ "total": 1, "artists": [local_artist] }));

    for ids in [json!([mirror.near.0]), json!([a.0, mirror.far.0])] {
        let refused = checked(&mut session, "get_tracks", json!({ "track_ids": ids })).unwrap_err();
        assert!(
            refused.contains("is a VCMP server's track, which agents can't read"),
            "{refused}"
        );
    }

    let missing = checked(
        &mut session,
        "find_tracks_by_tag",
        json!({ "key": "Composer", "missing": true }),
    )
    .unwrap();
    assert_eq!(missing["total"], 1, "{missing}");
    assert_eq!(missing["tracks"][0]["id"], b.0);
    let brief = checked(
        &mut session,
        "find_tracks_by_tag",
        json!({ "key": "Composer", "missing": true, "brief": true, "offset": 1 }),
    )
    .unwrap();
    assert_eq!(brief, json!({ "total": 1, "tracks": [] }));
    let held = checked(
        &mut session,
        "find_tracks_by_tag",
        json!({ "key": "Composer" }),
    )
    .unwrap();
    assert_eq!(held["total"], 1);
    assert_eq!(held["tracks"][0]["id"], a.0);

    assert_eq!(
        checked(&mut session, "list_labels", json!({})).unwrap(),
        json!([{ "name": "Chill", "colour": "teal", "track_count": 1 }])
    );
    let labelled = checked(
        &mut session,
        "find_tracks_by_labels",
        json!({ "labels": ["chill"] }),
    )
    .unwrap();
    let ids: Vec<&Value> = labelled
        .as_array()
        .unwrap()
        .iter()
        .map(|track| &track["id"])
        .collect();
    assert_eq!(ids, [&json!(a.0)]);
    assert_eq!(
        checked(
            &mut session,
            "find_tracks_by_labels",
            json!({ "labels": ["chill"], "offset": 1 }),
        )
        .unwrap(),
        json!([])
    );

    for changes in [
        json!([{ "track_id": mirror.near.0, "genre": "Jazz" }]),
        json!([
            { "track_id": a.0, "genre": "Jazz" },
            { "track_id": mirror.far.0, "genre": "Jazz" }
        ]),
    ] {
        let refused = checked(
            &mut session,
            "propose_tag_changes",
            json!({ "changes": changes }),
        )
        .unwrap_err();
        assert!(
            refused.contains("is a VCMP server's track, which agents can't change"),
            "{refused}"
        );
    }
    let refused = checked(
        &mut session,
        "propose_label_changes",
        json!({ "changes": [{ "track_ids": [b.0, mirror.near.0], "add": ["chill"] }] }),
    )
    .unwrap_err();
    assert!(
        refused.contains(&format!(
            "track {} is a VCMP server's track, which agents can't label",
            mirror.near.0
        )),
        "{refused}"
    );
    assert!(
        setup.library.change_sets().unwrap().is_empty(),
        "nothing was recorded"
    );

    let refused = checked(
        &mut session,
        "add_to_playlist",
        json!({ "playlist_id": playlist.0, "track_ids": [b.0, mirror.far.0] }),
    )
    .unwrap_err();
    assert!(
        refused.contains("which agents can't add to playlists"),
        "{refused}"
    );
    assert_eq!(setup.library.playlist_entries(playlist).unwrap().len(), 1);

    let base = dunce::canonicalize(setup._dir.path()).unwrap();
    let mut owner = Library::open(&base.join("library.sqlite3"), &base.join("cache")).unwrap();
    let far_set = owner
        .submit_label_changes(
            "Owner",
            &[(
                mirror.far,
                LabelEdit {
                    add: vec!["Rainy".into()],
                    remove: Vec::new(),
                },
            )],
            &[NewLabel {
                name: "Rainy".into(),
                colour: "palette:blue".into(),
            }],
        )
        .unwrap();
    owner.accept_change_set(far_set, |_, _| {}).unwrap();
    let refused = checked(
        &mut session,
        "propose_undo",
        json!({ "change_set_id": far_set.0 }),
    )
    .unwrap_err();
    assert_eq!(
        refused,
        format!(
            "change set {} covers a VCMP server's track, which agents can't change",
            far_set.0
        )
    );
    assert_eq!(
        checked(&mut session, "list_labels", json!({})).unwrap(),
        json!([
            { "name": "Chill", "colour": "teal", "track_count": 1 },
            { "name": "Rainy", "colour": "blue", "track_count": 0 }
        ])
    );
}

#[test]
fn a_servers_tracks_keep_their_place_in_playlists_and_the_queue() {
    let mut setup = start();
    let mirror = mirror(&mut setup);
    let a = id_of(&setup.library, "a.flac");
    let b = id_of(&setup.library, "b.flac");
    let chill = setup.library.create_label("Chill", "palette:teal").unwrap();
    setup.library.add_label(chill, &[a, mirror.near]).unwrap();
    let playlist = setup.library.create_playlist("Mix").unwrap();
    setup
        .library
        .add_to_playlist(playlist, &[a, mirror.near, b])
        .unwrap();
    let mut session = setup.session();
    let entries = |session: &mut Session<'_>| {
        checked(
            session,
            "get_playlist",
            json!({ "playlist_id": playlist.0 }),
        )
        .unwrap()["entries"]
            .clone()
    };

    let listed = entries(&mut session);
    assert_eq!(listed[0]["position"], 0);
    assert_eq!(listed[0]["track"]["id"], a.0);
    assert_eq!(listed[1], json!({ "position": 1, "track": near_entry() }));
    assert_eq!(listed[2]["position"], 2);
    assert_eq!(listed[2]["track"]["id"], b.0);

    let answer = checked(
        &mut session,
        "play_playlist",
        json!({ "playlist_id": playlist.0, "start_position": 1 }),
    )
    .unwrap();
    assert_eq!(answer["playing"], 2);
    assert_eq!(answer["starts_with"]["id"], b.0);
    assert_eq!(
        answer["left_out"],
        json!([{
            "position": 1, "server_track": true, "title": "Song from afar",
            "artist": "Artist", "reason": CANT_PLAY
        }])
    );
    assert_eq!(
        setup.player.take(),
        [Asked::Play(
            vec![a.0, b.0],
            Context::Playlist(playlist),
            Some(1),
            None
        )]
    );

    checked(
        &mut session,
        "move_in_playlist",
        json!({ "playlist_id": playlist.0, "from": 1, "to": 0 }),
    )
    .unwrap();
    let listed = entries(&mut session);
    assert_eq!(listed[0], json!({ "position": 0, "track": near_entry() }));
    assert_eq!(listed[1]["track"]["id"], a.0);
    checked(
        &mut session,
        "remove_from_playlist",
        json!({ "playlist_id": playlist.0, "positions": [0] }),
    )
    .unwrap();
    let listed = entries(&mut session);
    let ids: Vec<&Value> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["track"]["id"])
        .collect();
    assert_eq!(ids, [&json!(a.0), &json!(b.0)]);

    let answer = checked(
        &mut session,
        "play_tracks",
        json!({ "track_ids": [mirror.near.0, a.0] }),
    )
    .unwrap();
    assert_eq!(answer["playing"], 1);
    assert_eq!(
        answer["left_out"],
        json!([{
            "track_id": mirror.near.0, "server_track": true, "title": "Song from afar",
            "artist": "Artist", "reason": CANT_PLAY
        }])
    );
    let none = checked(
        &mut session,
        "queue_tracks",
        json!({ "track_ids": [mirror.far.0] }),
    )
    .unwrap_err();
    assert_eq!(
        none,
        format!(
            "none of these tracks can be played: track {}: {CANT_PLAY}",
            mirror.far.0
        )
    );
    let answer = checked(&mut session, "play_label", json!({ "label": "chill" })).unwrap();
    assert_eq!(answer["playing"], 1);
    assert_eq!(
        answer["left_out"],
        json!([{
            "server_track": true, "title": "Song from afar", "artist": "Artist",
            "reason": CANT_PLAY
        }])
    );
    let answer = checked(
        &mut session,
        "play_artist",
        json!({ "artist_id": mirror.artist.0 }),
    )
    .unwrap();
    assert_eq!(answer["playing"], 2);
    assert_eq!(
        answer["left_out"],
        json!([{
            "server_track": true, "title": "Song from afar", "artist": "Artist",
            "reason": CANT_PLAY
        }])
    );
    assert_eq!(
        checked(
            &mut session,
            "play_artist",
            json!({ "artist_id": mirror.remote_artist.0 }),
        )
        .unwrap_err(),
        format!("there is no artist {}", mirror.remote_artist.0)
    );
    assert_eq!(
        checked(
            &mut session,
            "play_album",
            json!({ "album_id": mirror.far_album.0 }),
        )
        .unwrap_err(),
        format!("there is no album {}", mirror.far_album.0)
    );
    assert_eq!(
        setup.player.take(),
        [
            Asked::Play(vec![a.0], Context::Agent("test-agent".into()), None, None),
            Asked::Play(vec![a.0], Context::Label(chill), None, None),
            Asked::Play(vec![a.0, b.0], Context::Artist(mirror.artist), None, None),
        ]
    );

    let a_file = setup.library.track(a).unwrap().unwrap().file.unwrap();
    let b_file = setup.library.track(b).unwrap().unwrap().file.unwrap();
    let key = |id: TrackId| PathBuf::from(setup.library.track(id).unwrap().unwrap().key());
    let status = |context| Status {
        state: PlayState::Playing,
        current: Some(key(mirror.near)),
        position: Duration::ZERO,
        duration: None,
        volume: 1.0,
        muted: false,
        shuffle: false,
        shuffle_kind: ShuffleKind::FewerRepeats,
        repeat: Repeat::Off,
        output: OutputDevices::default(),
        sleep_timer: SleepTimer::Off,
        context: Some(context),
        up_next: vec![key(mirror.far), a_file.clone()],
        upcoming: vec![
            b_file.clone(),
            key(mirror.near),
            PathBuf::from(remote_key(mirror.server, 99)),
        ],
        context_len: 3,
    };
    *setup.player.status.lock().unwrap() = Some(status(Context::Album(mirror.far_album)));
    let now = checked(&mut session, "get_now_playing", json!({})).unwrap();
    assert_eq!(now["track"], near_entry());
    assert_eq!(
        now["up_next"][0],
        json!({
            "position": 0,
            "track": { "server_track": true, "title": "Only remote", "artist": "Remote Artist" }
        })
    );
    assert_eq!(now["up_next"][1]["position"], 1);
    assert_eq!(now["up_next"][1]["track"]["id"], a.0);
    assert_eq!(now["upcoming"][0]["position"], 2);
    assert_eq!(now["upcoming"][0]["track"]["id"], b.0);
    assert_eq!(
        now["upcoming"][1],
        json!({ "position": 3, "track": near_entry() })
    );
    assert_eq!(
        now["upcoming"][2],
        json!({
            "position": 4,
            "track": { "server_track": true, "title": null, "artist": null }
        })
    );
    assert_eq!(
        now["context"],
        json!({ "kind": "server_album", "id": null, "name": "Far Album" })
    );
    *setup.player.status.lock().unwrap() = Some(status(Context::Folder(server_folder_location(
        mirror.server,
        3,
    ))));
    let now = checked(&mut session, "get_now_playing", json!({})).unwrap();
    assert_eq!(
        now["context"],
        json!({ "kind": "server_folder", "id": null, "name": null })
    );

    for position in [0, 3] {
        let refused = checked(
            &mut session,
            "edit_queue",
            json!({ "action": "skip_to", "position": position }),
        )
        .unwrap_err();
        assert_eq!(
            refused,
            format!(
                "the entry at position {position} is a VCMP server's track, which agents can't play"
            )
        );
    }
    for arguments in [
        json!({ "action": "move", "position": 0, "to": 1 }),
        json!({ "action": "remove", "position": 0 }),
        json!({ "action": "skip_to", "position": 2 }),
    ] {
        checked(&mut session, "edit_queue", arguments).unwrap();
    }
    assert_eq!(
        setup.player.take(),
        [
            Asked::Control(Control::MoveInUpNext { from: 0, to: 1 }),
            Asked::Control(Control::RemoveFromUpNext(0)),
            Asked::Control(Control::SkipTo(2)),
        ]
    );
}
