# MCP server

How `crates/vcmp-mcp` lets AI agents work with the library and control playback, and
how the app takes their changes. The agreed decisions are in the
[MCP spec](specs/2026-10-19-mcp-library-server.md); the change sets agents' changes
become are in [library.md](library.md#change-sets), and their review in
[ui.md](ui.md). Keep this file in sync with the crate.

## Running

- **Only while VCMP runs and the switch is on** (Settings, AI agents; setting
  `mcp.enabled`, off by default). `AgentService` (`src/agents.rs`) starts the server
  when the switch goes on or VCMP starts with it on, and drops it when it goes off.
- **Where:** MCP's Streamable HTTP at `http://127.0.0.1:<port>/mcp`, the port from
  `mcp.port` (default 8267, "VCMP" on a phone keypad). `Server::start` binds the
  port before anything else, so a taken port is reported at once: Settings says so
  and offers another port (1024 to 65535). rmcp's sessions are kept, which Claude
  Code uses.
- **Threads:** the server runs its own current-thread tokio runtime on a thread named
  `vcmp-mcp`, and reads the library on a connection of its own (WAL). Reads run in
  tokio's blocking pool behind a mutex. Dropping the `Server` stops it and joins the
  thread.

## Security

| Check | Why |
|---|---|
| Listens on 127.0.0.1 only | Nothing on the network reaches it |
| Every request needs `Authorization: Bearer <token>`, else 401 | Other programs and users on the computer can't use the library without the token. The token is 32 random bytes as hex (`mcp.token`), made when the server first starts; New token replaces it. The comparison takes the same time wherever the first difference is |
| A request with an `Origin` header gets 403 (rmcp's Origin check with an empty allow list, enforced) | Browsers send `Origin` on cross-site requests and MCP clients don't, so a web page can't drive the server through the browser |
| `Host` must be a loopback name (rmcp's default) | Against DNS rebinding |
| No write to the library from the server's thread | Tag changes, label changes, undos and playlist deletions become change sets the owner reviews; playlist edits go through the app |
| Playback only with its own switch (Settings, AI agents, "Let AI agents control playback"; setting `mcp.playback`, off by default) | An agent can't start music unasked. The player checks the setting on every request, so turning it off takes effect at once; the tools stay listed and answer with an error naming the switch. Playback acts at once, without review, and no tool closes VCMP |
| A cover an agent gives by web address is fetched only for `propose_tag_changes`: http or https, up to 5 redirects, 30 s, at most 10 MB, and it must start like a JPEG or PNG | The download asks nothing of the library; what it brings goes into files only once the owner has seen the picture in the review and accepted it. A cover from a file must be given by its absolute path, with the same size and format checks |

The token is in the library database like the other settings, readable by any program
of the same user. Such a program could edit the music files directly anyway; the
review still guards every write VCMP makes for an agent.

## Tools

Answers are JSON text; an error is a tool error with a reason the agent can act on.

| Tool | Does |
|---|---|
| `search` | Artists, albums and tracks holding every word (the library's search) |
| `list_albums`, `list_artists` | Pages of 100 (at most 500) |
| `get_album` | An album and its tracks |
| `get_tracks` | Tracks by id with the nine fields, path, availability and every tag by key |
| `list_tag_keys` | Every key with how many tracks hold it |
| `find_tracks_by_tag` | Tracks holding a key (and value), or lacking it |
| `list_labels` | The owner's labels with their colour (a palette name or `#rrggbb`) and how many available tracks hold each |
| `find_tracks_by_labels` | Available tracks holding every label named (ignoring case) |
| `list_playlists`, `get_playlist` | Playlists and their entries with positions |
| `propose_tag_changes` | Records a change set of up to 500 tracks: the nine fields by name (`remove` clears them), the cover among them as `{"file": "<absolute path>"}` or `{"url": "<http or https address>"}`, and any other tag by key (an empty list removes it). Each image is read or downloaded once, however many tracks it goes into |
| `propose_label_changes` | Records a label set: groups of tracks with labels to add and remove (a track in several groups gets them all), and new labels with a colour (a palette name or `#rrggbb`); at most 500 tracks |
| `get_change_set`, `list_change_sets` | Status, and per file the values before, asked for and after, and whether the cover is replaced or removed; for a label set, per track each label held before, asked, and held after |
| `propose_undo` | Records the undo of an accepted change of tags or labels |
| `propose_playlist_deletion` | Records a deletion for review |
| `create_playlist`, `rename_playlist`, `add_to_playlist`, `remove_from_playlist`, `move_in_playlist` | Done at once |
| `play_tracks` | Plays tracks by id (at most 500) in place of what plays: a queue context of its own, named after the agent in the queue panel ("Next from Claude"). Optional `start_track_id` and `shuffle` (true or false; left out, it stays as it is) |
| `play_album`, `play_playlist`, `play_label`, `play_artist` | Play these as their pages' play buttons do, with the same queue context and order (an artist: their albums, then their appearances elsewhere). A label by name; a playlist's start by entry position |
| `queue_tracks` | Puts tracks into Up next, `next` or at the `end` |
| `playback_control` | Play, pause, toggle, stop, next, previous, seek (`position_seconds`), and `show_window` (brings the window to the front). There is no quit |
| `edit_queue` | Skip to an entry, remove or move an Up next entry, clear Up next; positions as `get_now_playing` lists them |
| `set_playback_options` | Shuffle on or off, its kind (`fewer_repeats`, `true_random`), repeat (`off`, `all`, `one`), volume 0 to 100 |
| `list_output_devices`, `set_output_device` | The devices as Settings lists them (listed afresh, so Settings' list follows), the chosen one and the one playing instead when it is missing; choosing one by name (exactly, else ignoring case), or the system default |
| `set_sleep_timer` | Off, end of this track, or 1 to 90 minutes |
| `get_now_playing` | The track, playing, paused or stopped, position, duration, volume, mute, shuffle, repeat, output device, sleep timer, what the queue plays from, Up next and the next tracks (10 by default, at most 100) |

The play and queue tools read what to play on the server's own connection, so a
track that can't be played is left out there: one whose file is gone (the library
marks it unavailable, or it isn't on the disk any more), one the library doesn't
know, or one whose path isn't valid Unicode. The answer names each under
`left_out` with the reason, and playing starts at the next playable track when the
one to start with is left out (as the window does for playlists). When nothing can
be played, the tool answers with an error listing why.

Every track an answer lists carries its labels by name. The server's instructions,
sent when an agent connects, explain the key naming, the review, that labels are the
owner's own names kept in VCMP and not tags (look at them first when asked for music
by mood), and what each of the nine fields does in VCMP (grouping by album and album
artist, Unknown albums, MusicBrainz release IDs). The agent's name in the history and in the
queue panel is the client's title or name from its handshake. The instructions also
list the playback tools, say that playback needs its own switch, and ask the agent
to tell the owner about tracks left out.

## Changes go through the app

`Changes` is the crate's seam to the app. `AppChanges` implements it: each call is
posted to Slint's event loop with `invoke_from_event_loop`, runs on
`AgentService` there, and the server's thread waits up to 30 seconds for the answer.
So a change set shows in the sidebar the moment it is recorded, and playlist edits
show in the window at once. The crate itself never depends on Slint.

`Playback` is the seam for the player, kept apart from `Changes` because nothing in
it is a change to the library. `AppPlayback` (`src/player_service/agent_playback.rs`)
implements it the same way, posting each call to `PlayerService` on the event loop
and waiting up to 30 seconds. There it checks the playback switch, then does what
the window does: the play buttons' queue contexts, `media_control` for the
buttons (with `Control::Quit` never reachable), the bar's shuffle, repeat, volume
and sleep timer functions, and Settings' device choice, all saved in the settings as
the window saves them. So the bar, the queue panel, the OS media controls, Discord and
Last.fm follow an agent exactly as they follow the owner. `get_now_playing` reads the
service's state on the UI thread (what the bar and the queue panel show) and gets the
files back as paths; the server turns them into tracks on its own connection. Device
lists come from the server's thread, as listing devices can take a while.

## The stdio relay

`vcmp mcp` (`vcmp_mcp::relay`, started from `main.rs` before any window) is for
clients that only start local programs, such as Claude Desktop. It speaks MCP's
stdio transport: a JSON-RPC message per line in, a message per line out. Each line
is posted to the running VCMP's server, and every message of the answer, a JSON
body or the data of each event of an event stream, goes out as one line. It keeps
the session id from the handshake and sends the protocol version the handshake
agreed. Requests are relayed one at a time, in order. At the end of its input it
ends the session.

- **Where to connect** comes from the library's settings, found the way VCMP finds
  them (`VCMP_HOME`, else the platform's data directory): the port and the token.
  So a client's configuration is just the program and `mcp`. Settings shows the
  entry for Claude Desktop with the path of the VCMP that runs, and its `VCMP_HOME`
  when set.
- **When it can't get through**, it reads the settings again and tries once more:
  the owner may have changed the port or made a new token. Failing still, a request
  gets a JSON-RPC error answer (code -32000) whose message says why: VCMP isn't
  open (nothing on the port), the switch is off, no library or token yet, the token
  doesn't match (the relay reads another library than the VCMP that runs), or VCMP
  was restarted (the session is gone; the client has to connect again). A
  notification has no answer to carry the reason; the relay logs it to standard
  error.
- It never opens a window, creates no library where there is none, and writes
  nothing but its output.

## Testing

`crates/vcmp-mcp/tests/server.rs` starts the server on a free port with a stand-in
for the app (`Changes` done at once on a test connection, and a recorder for
`Playback`) and talks real HTTP to it: the token and Origin checks, the handshake,
reading tags, change sets that wait, playlist edits, and every playback tool,
including requests with missing files and unknown tracks. The same file runs the relay over in-memory input and output against
that server: a whole session, the answers when nothing listens or the switch is off,
and a new token. `crates/vcmp-app/tests/agents.rs` covers the switch, the port and
token, what the relay reads from the settings, the review in the window, the
playback switch, and `PlayerService`'s side of playback with a recorder in place of
the engine (the commands sent, the settings saved, the state read back). A real
client is in [manual-testing.md](manual-testing.md).

## Known limits

- The relay was run on Linux only. Release builds on Windows have no console
  window (`windows_subsystem = "windows"`); a client that starts `vcmp.exe mcp`
  with its standard input and output redirected still hands it those pipes, but that
  is yet to be seen working.
- Settings' command adds VCMP to Claude Code in user scope. Added in local scope
  (`claude mcp add` without `--scope`, for one folder), the Claude desktop app's
  Code tab dropped it at every session's second message: 4.29.0 on Linux, with no
  request failing and nothing in VCMP's log. Its sessions start in a spare folder
  and move to the project, so local-scope servers seem to be read from the wrong
  folder from the second message on.
- rmcp ends a session after 5 minutes without a request (its default). Claude Code
  negotiates protocol 2026-07-28, which has no sessions, so it isn't affected. A
  client on an older version gets 404 for its session and has to connect again;
  through the relay, the error says that VCMP was restarted.
- A cover by web address is downloaded on the server's thread pool while the agent
  waits; a slow site holds that request up to 30 seconds.
- A playback tool answers once the player has taken the request, before the engine
  has acted on it: `get_now_playing` right after `play_tracks` can still show the
  track before.
- Positions in `edit_queue` are those the queue panel shows: Up next and at most
  100 tracks after it. Earlier tracks (the history) aren't listed.
- A change waits for the window's event loop to take it; when that takes more than
  30 seconds, the agent is told that VCMP didn't answer in time.
