# Project review: code, UI, UX and features

Reviewed `main` at `67a154f` (version 26.11.0) on 2026-11-10, on Windows 11. The
review changes no code. It follows the review of 2026-11-04, which looked at
duplication only; this one looks at correctness, security, the UI, usability and
what the product is missing.

## Scope and method

- **Run on Windows 11** (Rust 1.99.1): `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace`, with the MSVC toolchain. All three pass; 717 tests,
  none failed or ignored. So the suite passes locally on Windows while CI is off.
- **Seen on screen:** `--example screenshots` against a made-up library of 67 tagged
  tracks in 12 albums (`target/review-music`, built with ffmpeg: long titles,
  Czech, Icelandic and Japanese names, an album without a cover, an untagged file,
  the same tracks in two formats). Eleven of its 89 pictures were looked at: Albums,
  an album with the queue, Songs, Artists, Search, Settings (light, and the whole
  page in dark), the tag editor, the Library check, an agent's change in review and
  now playing with lyrics. Playlists, Labels, Folders, the setup wizard, About and
  a narrow window were not looked at. The app itself was not operated by hand:
  nothing was clicked, played or heard.
- **Read closely:** every `.slint` file's main parts, `vcmp-mcp`, most of
  `vcmp-integrations`, and the parts of `vcmp-library`, `vcmp-playback` and
  `vcmp-app/src` named in their sections below. The docs' "Known limits" were read
  first, and what they already list is not repeated as a finding.
- **Who read what:** five reviewers read the areas in parallel (UI; MCP and
  integrations; library; playback; app glue), each once. The cited code was then
  opened a second time for U1, U2, U4, the queue's click, M1, M2, M5, the relay's
  migration, L1, the symlink skip, B1, B2, A1 and the second-instance discard. Every
  other "verified" below rests on one reviewer's reading.
- **Not covered:** macOS, Linux, real audio devices, real online services (Last.fm,
  fanart.tv, LRCLIB, Discord), performance with a large library, and a screen
  reader.

Each finding says how sure it is: **verified** means the code path was read end to
end, **plausible** means it needs a run to confirm. Nothing below was reproduced in
the running app unless it says so.

## What is good

The project is in better shape than most at this size (about 80,000 lines, 250
commits in under four weeks).

- **The rules are enforced, not hoped for.** Colours and durations only in
  `theme.slint`, every string in `@tr()`, AA contrast per theme, no allocation in
  the audio callback: each has a test.
- **The architecture holds.** The core crates don't know Slint, the playback engine
  runs against a test device, the library runs against temporary copies, and the
  online services sit behind traits with scripted answers. 717 tests run in under a
  minute once built.
- **The user's files are treated with care.** Tags are written into a copy, read
  back and only then swapped in; agents' changes wait for review, refuse a file
  that changed meanwhile and can be undone for 30 days.
- **The documentation is unusually honest.** Known limits are written down, and
  `manual-testing.md` marks what was "never seen working". That made this review
  much faster.
- **The look is consistent.** In the screenshots every page uses the same spacing,
  type sizes and controls, long and non-Latin names elide cleanly, and empty states
  say what to do next.

## Summary

The findings that matter most, across all areas. Details follow in the sections.

| # | Finding | Area | Severity | Sure? |
|---|---|---|---|---|
| U1 | A mouse click leaves the keyboard dead until Tab, also inside dialogs | UI | High | Verified in code |
| M1 | An agent's cover "by file" reads any path with no real size limit | MCP | Medium | Verified |
| M2 | An agent's cover "by URL" fetches any host, private ones included, before review | MCP | Medium | Verified |
| M3 | Covers are copied per track and prepared on the UI thread | MCP | Medium | Verified / plausible |
| U2 | Window shortcuts act behind open dialogs | UI | Medium | Verified |
| U4 | The first failed "Add folder" shows no error | UI | Medium | Verified |
| L1 | A database error during a scan hangs the scan thread until restart | Library | Medium | Hang verified, trigger plausible |
| L2 | An agent's change interrupted while it is written can't be undone | Library | Medium | Verified by reading |
| B1 | An output stream that dies of any other error is never reopened | Playback | Medium | VCMP's side verified |
| B2 | Repeat switched off after the context wrapped keeps playing up to 100 tracks | Playback | Medium | Verified by trace |
| B3 | A track that opens but holds no audio makes repeat spin | Playback | Medium | Verified by trace |
| A1 | Opening the tag editor reads every chosen file's cover on the UI thread | App | Medium | Path verified, cost unmeasured |
| A4 | A panic on a worker or the engine thread leaves the window stuck, silently | App, playback | Medium | Plausible |
| A7 | A second VCMP on the same library throws away the first one's waiting agent changes | App | Low | Verified |
| P1 | CI is off, and the Windows build that ships is one nothing tests | Process | Medium | Verified |

No finding puts the user's music at risk in normal use. The one High is a usability
defect; the Mediums are a hang, lost undo, a player that goes quiet and an agent
that can be talked into reading or fetching what it shouldn't.

## UI defects

Paths are under `crates/vcmp-app/`.

### U1. A mouse click leaves the keyboard dead until Tab (High, verified in code)

`src/shortcuts.rs:118-121` calls `release-focus` on every mouse press, and
`ui/app-window.slint:279-282` moves the focus to the window's root scope. No control
takes the focus back on a click (`focus-on-click: false` everywhere, for example
`ui/album-page.slint:675`), and the root scope handles Escape only for the covering
queue (`ui/app-window.slint:398`).

- **In a track list:** click a track to select it, then press Delete, Ctrl+A,
  Shift+Down, Enter or Escape. Nothing happens until Tab is pressed. `docs/ui.md`,
  "Selecting tracks", describes mouse and keyboard as two ways into one selection;
  after a click the keyboard half is unreachable.
- **In a dialog:** open the tag editor, click the card's background or "Choose
  image…", then press Escape. The dialog's Escape handler sits on a `FocusScope`
  inside `Modal` (`ui/components.slint:1172-1181`), which no longer has the focus.
  Tab then starts from the sidebar behind the scrim, outside the focus trap.

The tests miss it because they don't go through the winit hook. The two that call
`invoke_release_focus()` themselves (`tests/player.rs:1596, 1615`,
`tests/search.rs:147`) then press only Space or Ctrl+F, which the window's own scope
handles; none presses a key meant for a list or a dialog after it. This was read in
the code and not reproduced in the running app.

**Simpler form:** after a click, give the focus to the nearest list or dialog
instead of the root (a `Modal` could take it back in `release-focus` while one is
open; a track list could call `list.focus()` in `clicked`), with
`Theme.keyboard-navigation = false` keeping the ring hidden as now.

### U2. Window shortcuts act behind open dialogs (Medium, verified)

`ui/app-window.slint:390-415` runs Ctrl+F, Alt+Left and Alt+Right in
`capture-key-pressed` without asking whether a dialog is open, and
`src/shortcuts.rs:98-112` does the same for the mouse's back and forward buttons.
Ctrl+F in the tag editor switches the page behind it to Search and puts the focus in
its field under the scrim; the back button navigates behind the setup wizard.

### U3. The now playing view doesn't keep the focus (Medium, verified in markup)

Pages are hidden from Tab only under the covering queue
(`ui/app-window.slint:646-648`). Under the now playing view the sidebar and the page
stay tab stops: Shift+Tab from the title gives an invisible focus, and Enter there
switches the hidden page.

### U4. The first failed "Add folder" shows no error (Medium, verified)

The error text is inside `if LibraryState.folders.length > 0`
(`ui/library-settings.slint:138`, the text at 165). With no folder yet, which is the
setup wizard and every empty library, a folder that can't be added produces nothing.
The "Add music folder…" buttons on the empty pages never show it either.

### U5. Fixed widths starve the stretching part near the minimum width (Medium, arithmetic verified, look plausible)

- **Songs table** (`ui/songs-page.slint:86-97, 192-196`): once a label exists, 396 px
  of the 420 px left at a 720 px window is fixed, so Title, Artist and Album get
  about 8 px each. With Plays, Last played and Added on at the default width, about
  30 px each.
- **Library check tabs** (`ui/library-check.slint:336-338`): at least 528 px wide
  against 420 px available, and 480 px at the default window with the queue open.
- **Accent row** (`ui/themes.slint:455-471`): ten swatches leave nothing for the
  title and description at 720 px.
- **Label filter chips** (`ui/labels.slint:287-296`) can't wrap.

The 720 px test (`tests/ui_behaviour.rs:413`) checks that elements stay inside the
window, not that text keeps room, and never opens the Library check.

### U6. Lists that create every row (Medium, plausible)

`TrackList` (`ui/album-page.slint:722`), the queue (`ui/queue-panel.slint:449, 495`)
and the agents' review (`ui/agents.slint:856`) create a row per entry, and nothing in
Rust caps playlists, Favourites, label pages or folders. `docs/ui.md` lists the limit
only for the Library check. Favourites after importing thousands of Last.fm loves, a
5,000-entry M3U, or Ctrl+A in Songs and "Add to queue" would each rebuild thousands
of rows; the documented 33 ms for 116 tracks suggests over a second each time.

### U7. The player bar's error can't be read (Medium, layout verified)

`ui/player-bar.slint:440-447` shows "A track could not be played and was skipped:
<file> (<error>)" as one elided 12 px line in a column 120 to 280 px wide. Only its
first words fit, and the file's name appears nowhere else.

### U8. Small things (Low, verified)

- The cover error says "isn't a JPEG or PNG image" (`ui/tag-editor.slint:531`), but
  GIF and WebP are accepted (`src/tag_editor.rs:43`).
- The tag editor for an album whose cover is a folder image shows "No cover"
  (seen in `tags-dark.png`), while the album page behind it shows the cover. True of
  the files, but it reads as if the cover were missing.

## Accessibility

1. **A switched-on icon button differs by tint only** (Medium, verified):
   `ui/components.slint:617` draws the glyph in the accent when active. Shuffle,
   repeat, lyrics, now playing, queue and the sleep timer all rely on it
   (`ui/player-bar.slint:482-642`). On Dark, pink against the text colour is about
   1.05:1 and lavender about 1.2:1, so "on" is a change of hue alone; in
   `album-dark.png` the open queue's button is hard to tell from its neighbours.
   `docs/ui.md` says state is never shown by colour alone, and the contrast tests
   don't pair the accent with the text colour. A dot under the glyph or a filled
   background would do.
2. **Few shortcuts, and none listed** (Medium, verified): next, previous, seek,
   volume, like, queue and now playing have no key unless their control has the
   focus (`ui/app-window.slint:388-443`). Nothing in the UI lists the shortcuts
   that exist; Alt+Left/Right, Ctrl+F and the zoom keys are in the README or a
   Settings description only.
3. **Lists keep the current row as private state** (Medium, plausible): the
   accessibility focus never moves with the arrow keys; rows only flip
   `accessible-item-selected` (`ui/album-page.slint:241, 674`,
   `ui/album-grid.slint:50, 267`, `ui/queue-panel.slint:41`). Whether NVDA or Orca
   announce the movement is untested.
4. **Icon-only buttons have no tooltip** (Low, verified): `IconButton`
   (`ui/components.slint:577-651`) has none; only `TrackMark` does. The moon, the
   speech bubble and the expand glyph have to be clicked to be understood.
5. **Small targets** (Low, verified): sliders are 20 px high
   (`ui/player-bar.slint:202`) and column edges 16 px wide (`ui/columns.slint:74`),
   against the documented 40 px.
6. **An open dropdown takes no keys** (Low, plausible): Enter opens the list, whose
   options have no key handling (`ui/components.slint:778-841`).
7. **`AlbumGrid` has no accessible label** (Low, verified):
   `ui/album-grid.slint:208`; `CardGrid` has one at 378.
8. **No scroll bars at all** is a documented limit (`docs/ui.md`, "Known limits").
   It deserves more weight than a limit: Settings is about 2,500 px tall
   (`appearance-dark.png`) and nothing shows that it continues, where one is, or
   lets a mouse without a wheel move through it.

## MCP server and relay

Paths are under `crates/`. The request gate itself is sound: loopback bind, a
256-bit token from `getrandom`, the Origin and Host checks and a 4 MiB body cap
(`vcmp-mcp/src/lib.rs:126-131, 215-249`). The findings are about what a tool does
once a request is in. The threat is not a hostile owner but an agent that follows an
instruction hidden in a tag it read (the spec's own prompt-injection risk).

### M1. A cover "by file" reads any path with no real size limit (Medium, verified)

`read_cover` (`vcmp-mcp/src/tools.rs:916-934`) checks `metadata().len()` against
10 MB and then calls `fs::read`, which reads to the end whatever the length said.
Nothing requires a regular file.

- A device or pipe reports length 0 and never ends (`/dev/zero` on Linux, a named
  pipe on Windows): VCMP runs out of memory, or a pool thread blocks for good.
- On Windows a UNC path (`\\host\share\c.jpg`) is absolute, so VCMP opens an SMB
  connection to a host the agent chose, with Windows' automatic sign-in (plausible;
  confirm with a capture).
- The different error texts tell the agent whether any path exists.

**Simpler form:** require `is_file()`, read through `File::take(MAX + 1)`, refuse
UNC and device paths.

### M2. A cover "by URL" fetches any host before the owner sees anything (Medium, verified; documented, but riskier than described)

`download_cover` (`vcmp-mcp/src/tools.rs:936-952`) accepts any http or https
address, follows five redirects and returns ureq's error text. `docs/mcp.md` says
the download "asks nothing of the library", which is true, but the request itself is
the effect: a tag that says "set the cover from http://192.168.1.1/…?x=…" turns an
agent with no network access of its own (Claude Desktop with only this server) into
one that can send GET requests into the home network and learn from the errors which
ports answer. The spec's mitigation for injected instructions assumes nothing acts
before review; this does.

**Simpler form:** resolve the host and refuse loopback, private and link-local
addresses on every redirect, or download only when the owner opens the review.

### M3. Covers are copied per track and prepared on the UI thread (Medium, clone verified, timing plausible)

`covers.get(&source).cloned()` (`vcmp-mcp/src/tools.rs:1010`) puts a full copy of
the image into every track's edit: one 10 MB cover on 500 tracks is about 5 GB.
`submit_tags` then runs on the UI thread (`vcmp-app/src/agents.rs:838-853`,
`vcmp-library/src/changes.rs:218-229, 640-654`) and reads, hashes and writes a cover
per track. After 30 s the agent is told VCMP didn't answer, but the posted work
still runs, so the set is recorded anyway and a retry makes a second one.

**Simpler form:** share a cover as `Arc<[u8]>`, cap the number of distinct covers
and their total size, prepare the set off the UI thread.

### M4. Stopping the server waits on the UI thread (Medium, verified by reading)

`Server::drop` joins the server thread (`vcmp-mcp/src/lib.rs:171-178`), whose
runtime waits for every running blocking task. Turning the switch off, pressing New
token or changing the port while a cover downloads freezes the window until the
downloads end (up to 30 s each).

### M5. The relay drops an answer silently and re-sends on any error (Medium, drop verified, duplicate plausible)

In `vcmp-mcp/src/relay.rs`, a body that fails to read becomes `""` with status 200
(`read_to_string().unwrap_or_default()`, line 173-178), so a request with an id gets
no line back and Claude Desktop waits for good, where `docs/mcp.md` promises an
error. Every `send` error maps to `Unreachable` (line 162) and `post` then sends the
body again (139-146): after a timeout that can run `add_to_playlist` twice.

### M6. Smaller items

- **The relay opens the library read-write and migrates it** (Low-Medium, verified):
  `relay_connection` calls `Library::open` (`vcmp-app/src/agents.rs:801`), which sets
  WAL and runs the migrations, although `docs/mcp.md` says the relay writes nothing.
  A newer `vcmp mcp` would migrate the schema under an older running VCMP.
- **The token goes to whoever holds the port** (Low-Medium, verified): while VCMP is
  closed and the switch stays on, any local process can listen on 127.0.0.1:8267 and
  receives the token in the relay's first request. It follows from the agreed HTTP
  transport; it belongs in the known limits at least.
- **`propose_label_changes` allocates before it checks its limits** (Low, verified):
  `vcmp-mcp/src/tools.rs:663-685` clones the label lists per id, repeats included,
  and applies the 500 cap afterwards.
- **Playlist tools report success for no-ops** (Low, verified): renaming an unknown
  playlist, or moving and removing out-of-range positions, answers "renamed",
  "moved", "removed" (`vcmp-mcp/src/tools.rs:809-876`). Removing every position
  empties a playlist at once, which equals the deletion that waits for review.
- **Tests:** nothing covers Host rejection, URL covers (size, redirects, private
  addresses), special files, a relay stream cut mid-answer, or stopping the server
  with a request in flight.

## Integrations

No findings in the update check, LRCLIB, MusicBrainz, fanart.tv, the media controls,
the reduce-motion code or the `unsafe` blocks of `window_frame.rs`. Secrets are kept
out of `Debug` output and logs, every client has a timeout and a body cap, and
MusicBrainz pacing is shared between the workers.

- **The Discord handshake can block for good, and `Drop` joins** (Low, plausible):
  `vcmp-integrations/src/discord.rs:114-132` connects through a `recv()` with no
  read timeout, and dropping the presence joins that thread from the UI thread. A
  hung Discord would hang "Show on Discord" off, or quitting.
- **Last.fm can lose queued plays** (Low, verified by reading): every API error code
  the code doesn't list maps to `Remedy::Drop`
  (`vcmp-integrations/src/lastfm.rs:150-159`), so an unexpected request-level code
  discards the queue one play at a time; and a Cancel that races a successful
  authorisation sends `Disconnect`, which clears the scrobble and love queues
  (`lastfm/scrobbler.rs:352-364, 499-506`, `vcmp-app/src/lastfm_service.rs:305-309`).

## Library

Read closely: `edit.rs`, `changes.rs`, `schema.rs`, `lib.rs`, `scan.rs`, `tags.rs`,
`query.rs`, `m3u.rs`, `artwork.rs`, `watch.rs`, `playlists.rs`; the rest in part.
Paths are under `crates/vcmp-library/src/`.

Found sound: the copy, check and replace path of the tag writer (`edit.rs:298-378`);
search input (`query.rs:267-320` quotes every word, so FTS operators can't be
injected); the migrations (`schema.rs:429-447`); and that agents reach files only
through track ids (`changes.rs:779-801`).

### L1. A database error during a scan hangs the scan thread until restart (Medium, verified by reading)

In `scan.rs:185-239` the reader thread sends into a channel of 64, and
`read_receiver` lives outside the `thread::scope`. When the write closure returns an
error, `write_as_received` (`scan.rs:960-996`) returns, but the scope then waits for
the reader, which is blocked in `send` on the full channel with nobody receiving.
Cancel is only checked before a read, so it doesn't free it. `Finished` is never
sent, `scan_cancel` stays set, and every later scan is only queued.

The hang is verified; what sets it off is only plausible. It takes a database error
in the middle of the reading step with more than 64 files still unread: a full disk,
or "database is locked" (L4 below). Removing the folder being scanned would do it
too (the next `apply` fails the foreign key), but the window disables Remove while
a scan runs (`vcmp-app/ui/library-settings.slint:77, 123`), so that way is closed
from the UI.

**Simpler form:** drop the receiver before the scope ends (move it into the closure),
so the reader's `send` fails and it stops.

### L2. An agent's change interrupted while it is written can't be undone (Medium, verified by reading)

`changes.rs:458-490, 507-513`: results are saved per file as they are written, but
the set stays `pending` and the values to undo to are complete only at the end. If
VCMP quits or crashes after 300 of 3,000 files, the next start marks the set
`discarded` (`vcmp-app/src/agents.rs:136`) and `submit_undo` refuses it. The files
keep the new tags with no way back but the tag editor. No test interrupts an accept.

**Simpler form:** at startup, a set with written files becomes `partly_written`
instead of `discarded`, with undo for those files.

### L3. A cover that couldn't be saved is recorded as "no cover" (Medium, verified by reading)

`store_artwork` turns an I/O error into `None`, the same as an image that can't be
decoded (`scan.rs:876-887, 919-924`, `artwork.rs:52-79`), and the album gets
`artwork_state = 2`. A cache disk that fills during the first scan leaves every
later album without a cover and listed under "Missing covers"; state 2 isn't looked
at again until a track or folder image changes, and deleting the cache doesn't
reset it.

### L4. Smaller items

- **Symlinks inside a music folder are skipped without a word** (Medium, verified):
  `WalkDir` runs without `follow_links` (`scan.rs:490-500`), so a linked file or
  folder is not `is_file()`. `~/Music/classical -> /mnt/nas/classical` adds nothing,
  and the scan report doesn't say so. Follow links (walkdir detects loops) or
  document it.
- **Write transactions that read first fail at once** (Low; mechanism confirmed
  against SQLite outside the project, frequency plausible): they are deferred
  (`scan.rs:132-161`, `changes.rs:816-818`, `playlists.rs:121-123`,
  `check.rs:88-108`), so another connection's commit between the read and the first
  write gives "database is locked" without waiting out the busy timeout. Use
  `TransactionBehavior::Immediate`.
- **Walk errors are dropped** (Low, verified): `scan.rs:491` filters them out, so a
  subfolder that fails to list has its tracks marked unavailable with nothing in the
  report.
- **A cancelled or failed scan skips `finish_groups`** (Low, verified):
  `scan.rs:242-245`. Albums added before the cancel can't be found by search and
  have no year until the next complete scan.
- **A stale playlist-deletion can delete another playlist** (Low, plausible):
  playlist ids are reused (`schema.rs:123-128`, no `AUTOINCREMENT`), and accepting
  compares the id only (`changes.rs:445-448`).
- **The cover queue has no bound** (Low, plausible): `scan.rs:1013`. Full image
  bytes pile up when reading outruns decoding.
- **The watcher's quiet period has no upper limit** (Low, verified):
  `watch.rs:53-59`. A sync tool busy in the folder postpones the rescan for as long
  as it runs.
- **The temporary copy's name is 17 bytes longer than the file's** (Low, verified):
  `edit.rs:315-318`. A file near the 255-byte limit can never be tagged.
- **LRC times aren't checked for overflow** (Low, verified): `lyrics.rs:363,
  396-398`. `[999999999999999999:00]` panics a debug build and wraps in release.
- **M3U:** an exported relative path that starts with `#` (a folder "#1 Hits") reads
  back as a comment (`m3u.rs:414, 438-457`); import canonicalises every path, so a
  UNC path in a downloaded playlist makes Windows contact that host
  (`m3u.rs:166-179`, plausible).
- **The read-back check never looks at the audio** (Low, hardening): `edit.rs:346-372`
  compares tags, format and duration. `docs/library.md` leans on it for lofty's
  "least tested" M4A writer, but a wrong chunk offset would pass, since the duration
  comes from the headers. `tests/tag_writing.rs:145` already hashes the audio; the
  check could too.

## Playback

Read closely: every source file of `crates/vcmp-playback/src`, and the WASAPI and
CoreAudio hosts of cpal 0.18.2 for B1. Paths are under that folder.

The audio callback is clean: `Renderer::render` and what it calls hold no lock,
allocate nothing, do no I/O, have no unbounded loop and drop no `Arc`.

### B1. An output stream that dies of any other error is never reopened (Medium, VCMP's side verified, not run)

The stream's error callback marks it lost only for `DeviceNotAvailable` and
`StreamInvalidated` (`output.rs:265-271`). cpal's WASAPI worker leaves its loop on
any error, and a stopped audio service or an unmapped error code arrives as another
kind. Then the callback stops for good while the state stays Playing, the position
freezes and no event is sent. Pause and Play don't help, because `play()` reopens
only when not connected (`engine.rs:334-341`). Changing the device in Settings or a
restart recovers. Restarting the Windows Audio service while a track plays would
confirm it.

**Simpler form:** treat every kind but `Xrun`, `RealtimeDenied` and `DeviceChanged`
as lost, or watch for a stream whose read position stops moving.

### B2. Repeat switched off after the context wrapped keeps playing (Medium, verified by trace)

`view()` extends `order` until 100 tracks are upcoming (`queue.rs:429-435`, through
`context_has` at 514), which for an album is many cycles. `repeat_from` marks only
the first extension, `advance` clears it once the cursor passes it
(`queue.rs:305-307, 343-345`), and the next extension sets it to the tail
(`537-539`). So after the first wrap, `set_repeat(Off)` (`419-426`) truncates
nothing, or only the far end: a 3-track album on Repeat All plays up to about 100
more tracks after Repeat is switched off, and the queue panel shows them.
`turning_repeat_off_drops_a_cycle_shown_ahead` covers the first cycle only.

### B3. A track that opens but holds no audio makes repeat spin (Medium, verified by trace)

`MAX_FAILURES` counts failed opens and is reset by every successful one
(`engine.rs:519, 544, 642, 657`). A header-only WAV, or a file that fails on its
first chunk, opens fine and gives a segment of length zero. On Repeat One, or as a
context's only track on Repeat All, the engine loops over it without waiting, and
each round sends `TrackChanged` and `QueueChanged`; the app answers each with a
queue save on the UI thread (`vcmp-app/src/player_service.rs:1457-1462`).

**Simpler form:** a track that gave no frames counts as a failure, and the counter
resets only once audio came out.

### B4. Smaller items

- **The stream keeps running while paused or stopped** (Medium, plausible):
  `output.rs:325` is the only `play()` and nothing pauses the stream. A paused VCMP
  renders silence, which on Windows keeps an "audio stream in use" power request, so
  the PC may not sleep. `powercfg /requests` with VCMP paused would show it.
- **A failed seek abandons the track** (Low-Medium, path verified): `local.rs:201-210`
  returns the error, and `load_current` has already flushed, so the track is
  reported failed and skipped (`engine.rs:494-553`), also when restoring a saved
  position.
- **`Player::snapshot()` waits without a limit** (Low-Medium, plausible):
  `player.rs:209-215`. The engine opens files and devices inline, and the app calls
  it on the UI thread at every track start; a disk spinning up freezes the window.
- **Queue commands by index race with track changes** (Low, plausible):
  `SkipTo(3)` built from a view can hit the neighbour if a track ends first
  (`player.rs:70-78`), likelier through an agent's round trip.
- **A queue edit just after a boundary cuts audio already playing** (Low,
  plausible): commands run before `follow_playback` (`engine.rs:136-151, 699-719`).
- **A NaN volume poisons the gain** (Low, verified): `clamp` keeps NaN
  (`engine.rs:61-63`), and the renderer's gain then stays NaN until the next flush.
- **A saved queue with a bad history position can panic the engine** (Low,
  verified): `from_snapshot` accepts positions up to `order.len()`, not the cursor
  (`queue.rs:480-481`), and `previous()` then indexes out of range.
- **"Fewer repeats" is quadratic when one artist holds half the context** (Low,
  plausible): `shuffle.rs:182-211`; the speed test uses evenly spread artists.
- **Tests missing:** zero-length tracks, decode errors mid-file, a failed seek,
  repeat-off after a wrap, the stream error kinds.

## App glue (`crates/vcmp-app/src`)

Read closely: nearly every file but `title_bar.rs`, `wheel.rs` and `icons.rs`.
Paths are under that folder. Nothing was timed, so costs are counts from the code.

Found sound: answers from older requests are dropped everywhere they could arrive
late; shutdown saves the queue, stops the engine and then writes waiting edits
(`main.rs:239-249`); saved settings and theme files are parsed defensively; play
counting handles seeks and pauses.

### A1. Opening the tag editor reads every chosen file's cover on the UI thread (Medium, path verified, cost unmeasured)

`tag_editor.rs:375-384` calls `library.embedded_cover` for each track, which opens
and parses the file, and keeps all the covers in one `Vec` to see whether they
differ. Ctrl+A in Songs and "Edit tags…" freezes the window for as many file reads
as there are tracks, holding that many covers in memory, before the dialog shows.

**Simpler form:** compare cover hashes on a worker thread behind a busy state, or
decide "Several covers" only when the cover is touched.

### A2. Queue rows look paths up with a file-system call on the UI thread (Medium, mechanism verified)

Each `QueueChanged` resolves every row through `Library::track_id_for_path`
(`player_service.rs:1085-1096, 1154-1161`), which canonicalises the folder
(`vcmp-library/src/lib.rs:238-246`): up to 100 upcoming rows and all of Up next, each
time. With the music on a sleeping or unplugged network drive, the restored queue's
rows each wait on it at start. `labels.rs:97-99` already documents this cost and
caches around it; the queue could carry the track id.

### A3. Every track start rewrites the whole play context (Medium, mechanism verified)

`save_queue` (`player_service.rs:1457-1463`, `saved_queue.rs:66-90`) encodes every
context entry with its path as JSON into one settings value, on the UI thread, at
each track start. Double-click a song in Songs and the context is the whole
library. Write the context when it changes, and only the cursor, history and Up
next per track.

### A4. A panic on a worker or the engine thread leaves the window stuck (Medium, plausible)

There is no `catch_unwind` in the workspace and no drop guard that sends `Finished`.
If lofty or symphonia panics on a malformed file, the scan stays "running"
(`library_service.rs:226-255`), the tag editor and the agents' review refuse to
close because `writing` stays true (`tag_editor.rs:668-678`, `agents.rs:662-676`),
and a dead engine just ends its listener (`player.rs:194-215`,
`player_service.rs:1274-1285`): VCMP has to be killed, with no message. Both
libraries parse files VCMP didn't make, so this is the realistic way a bad file
takes the app down.

**Simpler form:** a guard in each worker whose `Drop` sends `Finished(Err)`, and a
player problem line when the engine's channel closes.

### A5. A scan that changed nothing still reloads everything (Medium, verified)

`library_service.rs:298-325` refreshes the view after every scan, whatever the
report says, and one refresh reads the albums up to three times and the tracks
twice (`library_view.rs:348-366, 554, 602, 654`, `folders.rs:112`). So every start
and every watcher scan rebuilds the Songs table and drops the selection.

### A6. On Windows a media key may be handled twice while VCMP has the focus (Medium, plausible)

The winit hook toggles on the key event (`shortcuts.rs:90-95`) and the system media
controls are registered for the same window (`media_keys.rs:37-39`). Nothing makes
the two exclude each other; if both fire, one press of Play/Pause changes nothing.
It needs one key press on Windows with the commands logged. The hook also maps the
Stop key to pause.

### A7. Smaller items

- **A second VCMP on the same library discards the first one's waiting agent
  changes** (Low, verified): `agents.rs:136` runs at every start and nothing stops a
  second instance; both then also overwrite the saved queue and fight over the MCP
  port. A lock per data directory that raises the running window would settle all
  three.
- **The start index isn't remapped when a track is left out of the queue** (Low,
  verified): `play_tracks` does it (`player_service.rs:1416-1430`), the other
  `play_*` methods don't (`753-790, 929-935, 1397-1410`). With a non-UTF-8 file name
  before the clicked row, the next track plays. One `play_context` fixes it, and is
  the earlier review's finding 5.
- **Album page actions apply a row index to a fresh query** (Low, narrow window):
  `player_service.rs:1386-1410`. During a first scan the rows refresh every 3 s, so
  a click can play whichever track holds that position now.
- **The player isn't told about changes a scan found** (Low, verified):
  `tags_changed` is wired to the tag editor and agents only (`main.rs:156-181`), so
  tags edited in another program leave the bar and queue stale.
- **The artist filter re-reads all albums and photos per keystroke** (Low,
  verified): `library_view.rs:267-271, 530-573`, with no delay as Songs has.
- **Failed library writes are only logged** (Low, verified): `playlists.rs:121-127,
  453-455, 693-698`, `labels.rs:306-308, 481-486`, `favourites.rs:175-181`. The user
  sees nothing happen.
- **English assembled in Rust reaches the screen** (Low, verified): sizes in
  "KB/MB/GB" (`agents.rs:1156-1166`), the file dialog's "Images" filter
  (`tag_editor.rs:485`, `playlists.rs:466`, `artist_photos.rs:293`), "{} copy"
  (`themes.rs:308`), and raw error texts shown as detail
  (`player_service.rs:1499, 1519`, `library_service.rs:166-205`). CLAUDE.md's rule is
  that such strings are translatable "once they appear"; they have appeared.
- **`player_service.rs` (2,186 lines) has four seams that can be named:** play
  counting (`1727-1815`, the riskiest logic and testable without a window), the
  queue panel (`1084-1271`), the Discord presence (`1298-1342, 1833-1958`) and the
  `play_*` builders (`751-958, 1384-1431`). `lyrics.rs` and `agent_playback.rs`
  already show the pattern.

## UX and usability

Each is marked as a **problem** (most users will trip on it) or an **opinion**.

1. **Problem: Escape or Cancel in the tag editor throws away everything typed,
   without asking** (`ui/tag-editor.slint:208-214`). A click beside the dialog is
   deliberately inert for this very reason. Ask through `Confirmation` when a field
   changed.
2. **Problem: a single click in the queue plays that track and drops everything
   before it** (`ui/queue-panel.slint:471-474`), the opposite of every track list,
   where a click selects. A drag shorter than 6 px counts as a click, and Clear is
   instant with no undo. Select on click and play on double-click, as elsewhere.
3. **Problem: search stops at 12 cards per kind and 50 tracks without saying so**
   (`src/library_view.rs:85-86`). Add "first 50 of N" or "Show all".
4. **Problem: cards can't play.** An album card's menu has Play next and Add to
   queue but no Play; artist and playlist cards have no menu
   (`ui/album-grid.slint:223-237`). Playing an album takes a click into it and a
   click on Play. A play button on hover, as on track rows, would match the owner's
   own pattern.
5. **Problem: Albums and Artists have a filter but no sort or grouping**
   (`ui/pages.slint:426-458`). The grid is in album artist order (seen in
   `albums-dark.png`) with nothing saying so; by title, year and date added are the
   usual choices, and "Added" already exists for Songs.
6. **Problem: dragging the seek bar doesn't show the time it will land on**
   (`ui/player-bar.slint:553-563`): the label keeps the playing time.
7. **Opinion: Settings is one column of about 20 rows in 8 sections** with no section list,
   scrolled back to the top on every visit (`ui/app-window.slint:895`). Discord,
   lyrics lookup and Last.fm sit under "Playback" here and under "Online services"
   in the setup wizard (`ui/setup.slint:210`). Use the wizard's grouping in both,
   and consider tabs or a section list.
8. **Opinion: synced lyrics re-centre on every line** (`ui/lyrics.slint:19-23,
   233`), so scrolling ahead to read is undone within seconds. Pause the following
   for a few seconds after a wheel scroll.
9. **Opinion: removing a music folder is instant** (`ui/library-settings.slint:74-80`)
   while clearing the play history asks. Tracks are soft-deleted, so nothing is
   lost, but on a large library a misclick costs a rescan; an "Undo" line would
   cover it.
10. **Opinion: Genre can be edited but is shown nowhere**: no Songs column, no
    filter (`ui/songs-page.slint:12`).
11. **Opinion: wording drifts**: "Play" in the queue's menu and "Play now" elsewhere
    (`ui/queue-panel.slint:306`, `ui/album-page.slint:588`); "Add music folder…" and
    "Add folder…" (`ui/library-settings.slint:93, 131`).
12. **Opinion: a yellow accent silently shows as pink on Light**
    (`docs/ui.md:119-123`). Say so beside the dots.

## Feature gaps

Already there, so not gaps: multi-disc headings, sorting by column in Songs, Play
next, a sleep timer, M3U import and export, the restored queue.

| Gap | Status |
|---|---|
| Opus (and other formats: WavPack, APE, WMA, DSD) | The spec's own "first more-formats candidate"; `opus-in-ogg.ogg` is a fixture for skipping it. Likely the first thing a new user hits |
| Dropping folders or files onto the window | In no spec (only dropping a playlist file is excluded) |
| ReplayGain / volume levelling | Out of scope on purpose (v1 spec, "Out of scope") |
| Equalizer / DSP | Out of scope on purpose |
| Genre and year browsing, album and artist sorting | In no spec |
| Mini player, tray icon, close to tray | In no spec |
| Crossfade, playback speed | In no spec |
| A shortcut list, more window-wide shortcuts, undo for playlist and queue edits | In no spec |
| Smart playlists | Out of scope on purpose (labels spec) |
| Online tag lookup in the tag editor | Out of scope on purpose |
| CUE sheets | A known limit (`docs/library.md`) |
| Installers, code signing, macOS downloads | Left for later (`docs/development.md`) |

## DESIGN.md, "Awaiting the owner's review"

A second opinion on each item an agent decided; they stay the owner's to settle.

- **Animated pictures: keep.** One at a time, where the picture is the subject, and
  stopped by reduced motion. Consider pausing while the window is not in front.
- **Window buttons on Windows: keep the place, size and no Tab stop; widen the drag
  area.** A 16 px strip is a thin target, and the sidebar's drag zone goes while a
  dialog or now playing is open (`ui/app-window.slint:367-369`). The grey Close
  hover is fine.
- **Hearts: keep.** Add a heart (or a Liked column) to Songs, the one place where
  liking in bulk would happen.
- **Themes: keep Darker and Midnight; change the accents.** Drop red: on the dark
  themes it is the error colour itself (`#f38ba8`). Reconsider lavender, sky and
  blue for as long as "on" is shown by tint alone (Accessibility, 1).
- **Lyrics: keep the sizes, the icon and the place; change the unconditional
  re-centring** (UX, 8). The 4 px gaps and the 52 px volume slider say the player
  bar is full: the next button needs an overflow menu.

## Process, CI, releases and docs

### P1. CI is off, and the Windows build that ships is one nothing tests (Medium, verified)

`.github/workflows/ci.yml` runs only by hand, so `fmt`, `clippy` and the tests run
when someone remembers; the merge of PRs 92 to 97 needed a follow-up commit to fix
tests and docs (`dd2d54b`). While the Release workflow can't run, the Windows archive
comes from `packaging/release-local.sh`: a MinGW cross-build
(`x86_64-pc-windows-gnu`) that `manual-testing.md` records as "seen only starting
under Wine". The workflow would build with MSVC, and the tests here ran with MSVC.
So the binary Windows users download is from a toolchain no test has run against,
on the platform the README already calls little tested.

**Suggested:** a pre-push hook or a `just check` that runs the three commands, so
the rule "run all three before every merge" doesn't rest on memory; and one run of
`cargo test --workspace --target x86_64-pc-windows-gnu` (or building releases on
this Windows machine with MSVC) before the next Windows archive.

### P2. Smaller items

- **Actions are pinned by tag, not by commit** (`actions/checkout@v4`,
  `Swatinem/rust-cache@v2`, `taiki-e/install-action@v2`,
  `dtolnay/rust-toolchain@stable`) in a workflow that holds the API keys as
  secrets. Pin by SHA once the workflows run again.
- **The minimum Rust version is never built.** `rust-version = "1.92"` is declared;
  CI uses stable, and this review ran 1.99.1. A job on 1.92, or dropping the claim,
  keeps it true.
- **No dependency audit** (`cargo audit` or `cargo deny`) for a program that parses
  untrusted media files and talks to six online services.
- **CI doesn't pass `--locked`**; the release build does.
- **The README and CLAUDE.md disagree on macOS.** The README says Windows and Linux
  and "no downloads for macOS"; CLAUDE.md says cross-platform with a `.app` archive
  per release, which the workflow would indeed attach once it runs. Decide which is
  true at the next release.
- **`docs/ui.md` is 1,500 lines.** It is accurate, but it is now a reference to
  search rather than to read; splitting it by page would help.
- **The update check finds nothing while the repository is private** (a documented
  limit): every released copy checks at each start and gets a 404. Worth
  remembering when the repository opens, since older copies will then all announce
  the newest release at once.

## The 2026-11-04 review's findings: status

None of its 14 findings appears to have been taken up; six features were merged
since, on the same patterns. This rests on `grep` finding none of the suggested
helpers (`OnlinePicker`, `Inbox`, `Origin`, `rgba_buffer`, `play_context`,
`flag`/`set_flag`, an `http` module, `IMAGE_EXTENSIONS`, `Pressable`) and on two
findings opened by line: the origin keys are still built with `format!` in
`player_service.rs` and `agent_playback.rs`, and the misplaced doc comment still
sits above `album_initial` (`albums.rs:309`). The other findings were not re-read
one by one. The repeated sites grew, counted the same way as then:

| Finding | Then | Now |
|---|---|---|
| 6. The weak-upgrade callback closure (`vcmp-app/src`) | 132 | 154 |
| 13. Slint focus boilerplate (`vcmp-app/ui`) | 27 | 30 |
| 14. `fn file_name(&Path)` | 2, in the app | the same 2, and a third in `vcmp-library/src/changes.rs:1196` |
| 12. Lists of image extensions | 4, disagreeing | 4, still disagreeing (BMP only for playlists) |

Finding 4 (queue origin keys built and parsed in three places) is still the one that
can drift into a bug. The Library check now builds a key of its own
(`library_check.rs:254`), and play history stores these keys with every play, which
raises its cost. It should go first.

## Suggested order

1. **U1** (keyboard after a click) and **U4** (the silent "Add folder" error): small
   changes, and what a new user meets first.
2. **L1** (scan hang) and **B1** (stream never reopened): each leaves VCMP needing a
   restart with nothing on screen. With them, **A4**'s drop guards.
3. **M1, M2, M5** before agents are recommended to anyone else: the path and address
   checks, and the relay's silent drop.
4. **L2** (undo after an interrupted write) and **B2, B3** (repeat).
5. A check script or hook for `fmt`, `clippy` and the tests while CI is off (**P1**),
   and one test run of the MinGW Windows build.
6. The UI-thread costs **A1, A2, A3, A5** together, measured first on a library of
   tens of thousands of tracks: this review counted calls and timed nothing.
7. From the earlier review, finding 4 (`Origin`) and finding 5 (`play_context`),
   which also closes A7's start index.
8. The usability list, starting with the tag editor's unasked discard, the queue's
   single click and a way to play from a card.

## What would confirm the "plausible" findings

- **On this Windows machine:** restart the Windows Audio service while playing (B1);
  `powercfg /requests` while paused (B4); one media key press with commands logged
  (A6); a UNC path as an agent's cover with a network capture (M1).
- **Tests to add:** a write error with 200 jobs queued (L1); a 3-track context,
  wrap, then Repeat Off (B2); a zero-frame track on Repeat One (B3); a click
  followed by a key through the real hook (U1); a `panic!` in `write_files` (A4).
- **By hand:** everything `manual-testing.md` marks "never seen working" for
  Windows is still unseen; this review ran the tests and the screenshots there, not
  the app.
