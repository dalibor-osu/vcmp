# MCP library server

Status: agreed
Date: 2026-10-19 (updated 2026-10-20)
Area: vcmp-library (schema, scan, tag writing, change log), vcmp-app (settings,
review and history UI, services), a new MCP server crate, the `vcmp` binary (relay
subcommand); all three platforms

## Context of the change

VCMP keeps its library in SQLite (`crates/vcmp-library`, see `docs/library.md`). The
library crate already offers everything an outside tool would need to look at and
change the library: `albums`, `artists`, `tracks`, `album_tracks` and the FTS5
`search` for reading; `write_tags` with a `TagEdit` for tags; and the playlist calls
(create, rename, delete, add, remove, move). Tag writes go through a verified copy
and an atomic rename, and re-read the files into the database
(`docs/library.md`, "Writing tags").

What it does not do:

- Nothing outside the VCMP process can reach it. There is no IPC, server or command
  line interface.
- The database stores only what VCMP shows: title, artist, album, album artist,
  track and disc number, year, genre, duration, and the album's MusicBrainz release
  ID. Composer, comments, the other MusicBrainz IDs, ReplayGain and every other tag
  are read during the scan and dropped.
- `TagEdit` can change only those 9 fields (the cover included).
- Writes are final: neither the tag editor nor anything else records old values.
- The UI reloads after its own actions only. On Windows a file playback holds open
  can't be replaced, which the tag editor works around by holding the write back
  (`docs/ui.md`, "Tag editor").

The owner wants AI agents (MCP clients such as Claude Code or Claude Desktop) to
help manage the library: find problems in the metadata, fix tags and build
playlists. The Model Context Protocol is how those clients take tools from an
application. tokio, which the official Rust MCP SDK (`rmcp`) runs on, is already in
`Cargo.lock` through other dependencies.

## User request

Analyse what it would take to implement MCP inside VCMP. The goal is to make the
library metadata available to AI agents so they can manage the library: clean it
up, update file tags, create playlists and the like.

The server should run inside the app, so it is available while the app is open. It
is opt-in: a switch turns it on and off, and it is off by default.

Since VCMP is meant to edit more tags in the future, agents may edit any tag, and
the server gives them more context for the tags VCMP itself can edit.

Added after the first draft: the change log and the old covers are kept for one
month. Settings gets an option to delete these old backups, so the user can free
some space. Deleting a playlist can't be undone, so it must always be confirmed by
the user after they are shown a warning.

## Decisions

### Where should the MCP server run, and does the VCMP window need to be open for agents to use it?

| Option | What it means |
|---|---|
| Stdio, separate process (Recommended) | The agent client starts `vcmp mcp` and talks over stdin/stdout, sharing the SQLite database with the app. Works with every MCP client and without the window open. The running app has to learn to notice changes it didn't make. |
| Inside the running app (HTTP) | The app serves MCP on localhost and every change goes through its own services, so the UI updates live and playback conflicts are handled. Only works while VCMP is open, and needs a port and an access token. |
| Both: stdio that relays to the app | `vcmp mcp` forwards to the running app when it's open and works on the database itself when it isn't. Best experience, but two code paths that must behave the same. |

**Answer (Other): "It should run inside the app, so while it is open and it should be
opt-in: Switch to turn it off and on, off by default"**

### Besides reading and searching the library, what should agents be able to do through MCP in this change? (multi-select)

| Option | What it means |
|---|---|
| Write file tags (Recommended) | Agents change the 9 fields the tag editor covers, through the same verified-copy write path. This is the core of 'clean up the library', and it changes your music files. |
| Manage playlists (Recommended) | Create, rename and delete playlists and add, remove and reorder entries. Changes only the database, so there's little risk. |
| Control playback and queue | Play, pause, skip, and build or replace the queue ("play my chill playlist shuffled"). Possible because the server lives in the app; it's outside the library-management goal. |
| Manage folders and scans | Add or remove source folders, toggle recursion, start a rescan. Useful after an agent moves files, but removing a source hides tracks. |

**Answer: Write file tags, Manage playlists**

### How should tag changes an agent asks for be approved before VCMP writes them to files?

| Option | What it means |
|---|---|
| Review in VCMP (Recommended) | The agent submits a change set; VCMP shows a table of old -> new values and you accept or reject it as a whole. Safest for batches of hundreds of files; the agent waits, and it's a new dialog to design. |
| Write directly | The MCP client's own tool-call prompt is the only gate, and VCMP writes as soon as it's asked. Fastest for big cleanups; a careless agent can rewrite many files before you notice. |
| Per-session choice | A setting picks 'review each change set' or 'write directly'. Flexible, but two flows to build and test. |

**Answer: Review in VCMP**

### Should VCMP keep a record of tag changes so they can be undone?

| Option | What it means |
|---|---|
| Change log with undo (Recommended) | Each change set's old values are stored in the database, and a whole set can be reverted later. Cheap for text fields; old covers cost disk space to keep. |
| Change log, no undo | A history of what changed, when and by which agent, to read but not revert automatically. Less to build; reverting is a manual job. |
| Nothing kept | Same as the tag editor today: a write is final. Simplest; you rely on your own backups. |

**Answer: Change log with undo**

### How should MCP clients connect to the server inside VCMP?

| Option | What it means |
|---|---|
| HTTP plus a stdio relay (Recommended) | The app serves MCP over HTTP on localhost with an access token; `vcmp mcp` relays stdin/stdout to it for stdio-only clients like Claude Desktop. Every client works, and there's one port to pick and keep free. |
| HTTP on localhost only | Claude Code, Cursor and the like connect directly with a token. Least to build; Claude Desktop needs a third-party bridge such as mcp-remote. |
| Local socket plus stdio relay | No TCP port: a Unix socket or Windows named pipe guarded by file permissions, and every client goes through `vcmp mcp`. No token or port clashes; HTTP-only clients can't connect. |

**Answer: HTTP plus a stdio relay**

### Which playlist changes by an agent should go through the review in VCMP?

| Option | What it means |
|---|---|
| Only deletions reviewed (Recommended) | Creating, renaming and editing apply at once; deleting a whole playlist waits for your OK. Quick for building playlists, and the one destructive step is guarded. |
| All applied directly | Playlists live only in the database, so agents change them freely. Fastest; a deleted playlist is gone unless the change log covers it. |
| Agents edit only their own | Playlists an agent created are its to change; yours are read-only to agents. Your playlists are never at risk; an agent can't tidy them. |
| Everything reviewed | Every playlist change waits for your OK like tag changes. Safest; building a playlist means a review per step. |

**Answer: Only deletions reviewed**

### Which tag fields should agents be able to change?

| Option | What it means |
|---|---|
| The tag editor's 9 fields (Recommended) | Title, artist, album, album artist, track and disc number, year, genre, cover: the write path and its checks already exist. Agents can't fix composer, comments or MusicBrainz IDs. |
| 9 fields without the cover | Text fields only; covers stay a job for you in the tag editor. Keeps image bytes out of MCP and the change log small. |
| More fields | Add composer, comment, disc and track totals and MusicBrainz IDs. Better cleanups (MBIDs fix album grouping), but the database, write path and its checks grow first. |

**Answer (Other): "Since we want to be able to edit more tags in the future, let's
let the agents edit any tags but provide them with more context for tags VCMP can
edit."**

Reading of "more context", stated before the next round and not corrected: for the 9
fields VCMP uses, the server tells agents what each one affects in VCMP (album
grouping, artist pages, sorting); other tags are written as given, without that
guidance.

### How should agents name a tag that isn't one of VCMP's 9 fields?

| Option | What it means |
|---|---|
| Generic keys plus native escape (Recommended) | Format-independent names (Composer, Comment, MusicBrainzReleaseId, ReplayGain...) that lofty maps to ID3 frames or Vorbis fields, plus raw native keys for anything else. One call works for MP3 and FLAC alike, and nothing is out of reach. |
| Generic keys only | Only lofty's roughly 100 format-independent keys. Simple and uniform; custom fields (TXXX, unusual Vorbis names) stay unreachable. |
| Native keys only | ID3 frame IDs for MP3 and WAV, Vorbis field names for FLAC and Ogg. Everything is reachable, but agents must handle each format separately and can easily get it wrong. |

**Answer: Generic keys plus native escape**

### Where do agents read the tags VCMP doesn't store from?

| Option | What it means |
|---|---|
| From the files, on request (Recommended) | The database stays as it is; a tool reads a track's full tags from its file. About 8 ms per file cold on your disk, so a whole-library sweep takes around 30 s. |
| Stored in the database at scan | Every tag goes into a key/value table, so agents can instantly find 'every track without a MusicBrainz ID'. Schema and scan grow, and the database gets bigger. |

**Answer: Stored in the database at scan**

### How do existing libraries get the full tags into the database after the update?

| Option | What it means |
|---|---|
| Full re-read on next scan (Recommended) | The schema migration marks every track for re-reading, so the next scan stores all tags: about 35 s cold on your library, once. Agents see complete data from then on. |
| Fill in as files change | Only new and changed files get full tags; the rest wait. No slow scan, but agents can't tell missing tags from tags never read. |
| Re-read when the server is first turned on | The re-read runs only for people who enable MCP. Nobody else pays for it; a second code path for an unusual scan. |

**Answer: Full re-read on next scan**

### Where do you see agent change sets after they are written, and who can undo one?

| Option | What it means |
|---|---|
| History in VCMP, agents can ask (Recommended) | A list of change sets (agent, time, tracks, status) next to the MCP switch, each with Undo. Agents can request an undo too, which goes through the same review. You stay in control and agents can fix their own mistakes. |
| History in VCMP, only you undo | Same list, but undo is only a button in VCMP. Simpler; an agent that spots its mistake has to ask you. |
| Only through MCP | No history screen; agents list and undo change sets, with undo reviewed. Least UI work; you can't browse history without an agent. |

**Answer: History in VCMP, agents can ask**

### When an agent submits a change set, how does VCMP present it for review?

| Option | What it means |
|---|---|
| Notice, review when ready (Recommended) | A badge and a non-blocking notice; the review opens when you click it. Doesn't interrupt what you're doing; the agent may wait a while. |
| Review dialog opens at once | The review dialog comes up straight away and the window is brought forward. Fastest turnaround while you work with the agent; interrupts whatever you were doing. |

**Answer: Notice, review when ready**

### What happens to change sets still waiting for review when VCMP closes?

| Option | What it means |
|---|---|
| Discarded, agent told (Recommended) | Waiting sets are dropped and reported as rejected to any agent that asks. Nothing stale is written days later; the agent has to submit again. |
| Kept for the next start | Waiting sets are stored and shown again next time. Nothing is lost; files may have changed since, so each set is re-checked against them before it can be accepted. |

**Answer: Discarded, agent told**

### Follow-up after the first draft (the owner's answers to its open questions)

No options were offered; the owner wrote: "Let's keep the change log and old covers
for 1 month somewhere. Also let's add an option to delete the old backups in the
settings to allow user to clear some space. Deleting a playlist can not be undone.
It should always be confirmed by a user after showing them a warning." The owner
also accepted the risks as listed.

### What should the Settings button for clearing old backups delete?

| Option | What it means |
|---|---|
| Everything older sets keep (Recommended) | Does now what the one-month expiry does later: written sets drop out of the history along with their old values and covers. Frees the most space; those sets can no longer be undone or looked up. |
| Only the old covers | Covers take nearly all the space; text old values and the history stay, so text changes can still be undone. Cover changes in those sets can't be. |
| Backups, keep the history | Old values and covers go; the list of what changed, when and by which agent stays, marked 'can't be undone'. History stays readable, but nothing in it can be undone. |

**Answer: Everything older sets keep**

### Should the 'can't be undone' warning also apply when you delete a playlist yourself in VCMP?

| Option | What it means |
|---|---|
| Yes, one warning for both (Recommended) | The existing Delete question gains the warning, and agent deletions show the same text. Consistent; a small change to an existing UI string. |
| Only agent deletions | Your own Delete stays as it is today; only the review of an agent's deletion warns. Smaller change; the two flows say different things. |

**Answer: Yes, one warning for both**

## High-level plan

**1. Every tag in the database.** The scan stores each file's complete text tags
next to the fields it already keeps. A schema migration marks every track for
re-reading, so the next scan fills them in. Depends on nothing; everything an agent
reads about tags comes from here.

**2. Writing any tag.** The tag write path grows from the 9 fields to any tag, named
by a format-independent key or by the format's native key, with the same verified
copy and read-back check. Depends on 1, because the read-back compares against the
stored tags.

**3. Change log and undo.** Each change set (who asked, when, which tracks, old and
new value of every tag it touches, old covers) is kept for one month, then deleted
with its covers. Undoing a set builds the reverse set. Depends on 2.

**4. Review and history in VCMP.** A notice and badge when a set arrives, a review
view with old and new values that accepts or rejects the whole set, and a history
list with Undo next to the MCP switch, and a Settings button that deletes the
kept backups (with the space they take shown). An agent's playlist deletion is
reviewed with the same "can't be undone" warning that the owner's own Delete
question gains. Accepted sets are written by the app's own
services, so playback's open file and UI refreshes are handled as for the tag
editor. Depends on 3. The design follows `docs/DESIGN.md` and `docs/ui.md`.

**5. The MCP server in the app.** A Settings switch (off by default) starts an HTTP
server on localhost that offers reading and searching the library and its full tags,
submitting tag change sets and undo requests, asking a set's status, and changing
playlists (deletions go through review). The tool descriptions explain what VCMP's 9
fields do in VCMP. Read-only tools need only phase 1 and can be built alongside
phases 2 to 4; the write tools need phase 4.

**6. The stdio relay.** `vcmp mcp` passes stdin and stdout to the running app's
server for clients that only start local commands. Depends on 5.

**7. Documentation.** `docs/library.md` (schema, scan, writing, change log), `docs/ui.md`
(switch, review, history), a new section on MCP setup for each client in `README.md`,
and `docs/manual-testing.md` (real clients, Windows and macOS relay, playback-held
files). Runs with each phase.

## Architecture decisions

**The MCP layer is its own crate, without Slint.** It depends on `vcmp-library`
and `rmcp` and reaches the app through a trait the app implements (submit a set,
delete a playlist, ask a set's status), the way `PhotoSource` and the presence sink
work today. Reason: the protocol code can be tested against a fixture library
without a window, and the core crates stay free of Slint (CLAUDE.md). It is not
put in `vcmp-integrations`, which holds clients of online services, not a server.

**Threads.** The server runs its own tokio runtime on a worker thread and opens its
own `Library` connection for reads (WAL; one `Library` per thread). Every change
goes through the app: tag change sets and playlist deletions to the review service,
the other playlist edits to the playlist service, both posted to the Slint event
loop. Reason: the UI shows each change at once, and an accepted set waits for
playback to let go of a file just like a tag editor edit. The server thread never
writes the database itself.

**Change sets are asynchronous.** Submitting a set validates it (every edit, every
track exists and is available) and returns an id with status `pending` at once; the
agent asks for the status later. Reason: the owner reviews when ready, and MCP
clients time out tool calls that wait for minutes. Statuses: pending, accepted,
partly written, rejected, discarded, undone. Sets waiting when VCMP closes are
recorded as discarded, so an agent that asks after a restart gets an answer.

**A set records what it saw.** Each entry stores the old value read from the
database when the set was submitted. On accept, a file whose tag no longer holds
that value is skipped and reported as a conflict instead of overwritten. Undo is
the reverse set, checked the same way, so a value someone changed after the agent
is never reverted blindly. Undo always goes through review, whether the owner or an
agent asks.

**Retention.** A finished set (accepted, partly written, rejected, discarded or
undone) is kept for 30 days from when it finished, with its entries and old covers,
and then deleted; the cleanup runs at startup and once a day while VCMP is open.
The Settings button deletes every finished set at once and says how much space
that frees. Pending sets are never touched by either. A set that is gone can't be
undone, and an agent asking for it gets "no longer kept" rather than "unknown".
Reason: the owner asked for a month, and a time limit keeps the covers from
growing without bound; one rule for both paths means one code path and one test.
**Playlist deletion is never undoable.** It is not in the change log. Whether the
owner deletes a playlist or accepts an agent's deletion, VCMP first shows the same
warning that the deletion can't be undone, with the focus on Cancel as today.
Reason: the owner chose a warning over an undo, and the two paths should say the
same thing.

**Storage.**

| What | Where |
|---|---|
| All text tags of a track | New table: track, key, value, position (for repeated keys). Key is lofty's generic key where one exists, otherwise the native key with its format |
| Change sets | New table: id, agent name (from the MCP client's `clientInfo`), kind (tags or playlist deletion), status, times |
| Change set entries | New table: set, track, path, key, old value, new value, result |
| Old covers | Data directory, `images/undo/<hash>.<ext>`, the original bytes (not the cache: deleting the cache must not lose undo); deleted with the last set that refers to them |
| Switch, port, token | `settings`: `mcp.enabled` (off), `mcp.port`, `mcp.token` |

The `tracks` columns stay the scan's reading of the 9 fields (multi-value artists
joined, file name as title); the tags table holds what the file says. Pictures are
not copied into the tags table. The migration resets every track's size and
modification time fingerprint so the next scan re-reads all files.

**Tag names.** Agents address a tag by lofty's generic key (`Composer`,
`MusicBrainzRecordingId`, `ReplayGainTrackGain`) or by a native key with its format
(`id3:TXXX:MY FIELD`, `vorbis:MYFIELD`). A generic key is written to whatever the
file's tag format uses; a native key for another format is refused for that file.
The 9 VCMP fields keep their own names and their description says what they change
in VCMP (album grouping key, Unknown albums, artist pages, sort order). That text
comes from one place in Rust, so it can't drift from the code.

**Security.** The server binds only to 127.0.0.1, requires a random 256-bit bearer
token and checks the `Origin` header (the MCP specification's guard against DNS
rebinding). The token can be regenerated in Settings. When the port is taken the
switch shows the error and stays off.

**Relay.** `vcmp mcp` is a subcommand of the same binary. It reads the port and token
from the database (respecting `VCMP_HOME`), so a client's configuration is just the
command. With the app closed or the switch off it answers with an MCP error that
says so, rather than failing silently.

**Invariants for tests.** No MCP tool writes a music file or deletes a playlist
without an accepted set. Every written tag has its old value in the change log
before the file is replaced. Undo of an accepted set restores the files' tags to
what the scan read before it (checked on the fixture files of every format).
Expiry and the Settings button leave no old cover behind that no set refers to, and
never touch a pending set. No path deletes a playlist without the warning having
been confirmed.

## Weaknesses and risks

| Risk | Likelihood, cost | What reduces it |
|---|---|---|
| Agents send huge sets (the whole library); a review of 3,000 rows is unreadable, so the owner accepts without reading | High; one bad accept rewrites many files | Review grouped by album and by tag with counts, an undo that is one click, and a limit on set size the server enforces |
| Writing arbitrary tags corrupts or drops data the 9-field path never touched (frame conversions, ID3v2.3 vs 2.4, multi-value fields) | Medium; changes the owner's files | Read-back check over every tag, not only the edited ones; extend `examples/edit_check` to set and restore non-VCMP tags on copies |
| Tag values are untrusted text that an agent reads; a crafted tag can try to instruct the agent | Low for a personal library, real for downloaded files | Every file write and playlist deletion needs the owner's accept; nothing in the server acts on tag content |
| The one-time full re-read after the update | Certain; about 35 s cold on the owner's library, plus the artwork step for every album, since a changed file sends its album there | Accept it as one-time, or skip the artwork step when only the tag table is being filled; measure with `examples/scan` |
| The database grows (lyrics tags hold kilobytes) | Medium; mostly harmless | Measure on the owner's library before deciding to exclude any key |
| The stdio relay on Windows: release builds use the GUI subsystem, and stdin/stdout may not work as expected | Medium; Claude Desktop on Windows can't connect | Verify early on Windows; fall back to a separate console-subsystem relay binary if needed |
| `rmcp` and the MCP specification still change | Medium; upgrades cost work | Keep the protocol inside the one crate behind the app trait |
| Another local program with the user's rights can read the token from the database | Low; it could already edit the files directly | Accepted; the review still guards every write |
| Port clashes | Low | Configurable port, clear error |

The owner accepted these risks as listed. Questions left open:

- Whether the tag editor later shows and edits the extra tags (the "more tags in the
  future" the owner mentioned); this change only stores and writes them.
- The exact tool list and the review view's design; those belong to the next phase
  and to the owner's design direction.

## Out of scope

- Playback and queue control, and folder and scan management through MCP (not
  chosen in the second round).
- MCP while VCMP is closed, and access from other machines: the server lives in the
  running app and listens on localhost only.
- Moving, renaming or deleting music files.
- Online lookups (MusicBrainz, AcoustID fingerprints) offered by VCMP to agents;
  agents use their own tools for that.
- An AI assistant or chat inside VCMP.
- Showing or editing the extra tags in the tag editor.
- Undo of a playlist deletion, and undo of a change set more than a month old.
- Translating the server's tool descriptions: they are read by agents, not shown in
  the UI. Everything the review and history views show goes through `@tr()`.
