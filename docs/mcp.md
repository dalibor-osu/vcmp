# MCP server

How `crates/vcmp-mcp` lets AI agents work with the library, and how the app takes
their changes. The agreed decisions are in the
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
| No write to the library from the server's thread | Tag changes, undos and playlist deletions become change sets the owner reviews; playlist edits go through the app |

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
| `list_playlists`, `get_playlist` | Playlists and their entries with positions |
| `propose_tag_changes` | Records a change set of up to 500 tracks: the nine fields by name (`remove` clears them) and any other tag by key (an empty list removes it) |
| `get_change_set`, `list_change_sets` | Status, and per file the values before, asked for and after |
| `propose_undo` | Records the undo of an accepted change |
| `propose_playlist_deletion` | Records a deletion for review |
| `create_playlist`, `rename_playlist`, `add_to_playlist`, `remove_from_playlist`, `move_in_playlist` | Done at once |

The server's instructions, sent when an agent connects, explain the key naming, the
review and what each of the nine fields does in VCMP (grouping by album and album
artist, Unknown albums, MusicBrainz release IDs). The agent's name in the history is
the client's title or name from its handshake.

## Changes go through the app

`Changes` is the crate's seam to the app. `AppChanges` implements it: each call is
posted to Slint's event loop with `invoke_from_event_loop`, runs on
`AgentService` there, and the server's thread waits up to 30 seconds for the answer.
So a change set shows in the sidebar the moment it is recorded, and playlist edits
show in the window at once. The crate itself never depends on Slint.

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
for the app (`Changes` done at once on a test connection) and talks real HTTP to it:
the token and Origin checks, the handshake, reading tags, change sets that wait, and
playlist edits. The same file runs the relay over in-memory input and output against
that server: a whole session, the answers when nothing listens or the switch is off,
and a new token. `crates/vcmp-app/tests/agents.rs` covers the switch, the port and
token, what the relay reads from the settings, and the review in the window. A real
client is in [manual-testing.md](manual-testing.md).

## Known limits

- The relay was run on Linux only. Release builds on Windows have no console
  window (`windows_subsystem = "windows"`); a client that starts `vcmp.exe mcp`
  with its standard input and output redirected still hands it those pipes, but that
  is yet to be seen working.
- Agents can't change covers.
- A change waits for the window's event loop to take it; when that takes more than
  30 seconds, the agent is told that VCMP didn't answer in time.
