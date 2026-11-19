# VCMP streaming server

Spec: docs/specs/2026-11-11-vcmp-streaming-server.md
Date: 2026-11-11
Status: partially implemented
Area: vcmp-library (schema versions 18 and 19: change feed, server tracks), new crates
vcmp-protocol, vcmp-server and vcmp-client, vcmp-playback (server source,
buffering), vcmp-app (Music servers in Settings, album marks, Folders page, player
bar, tag editor), vcmp-mcp (agents kept to local music), packaging/server, docs

## What was done

All seven workstreams, with the gaps listed under What is missing.

- **Server** (workstreams 1 and 2): `vcmp-server` with `serve`, `tokens` and
  `new-token`; configuration from a TOML file and `VCMP_SERVER_*` variables; a user
  and an admin token, configured ones owning their value; one writer thread, pooled
  readers; the change feed (`Library::track_feed`, revisions kept by triggers, the
  feed's `head` on every page), artwork, audio with ranges and `If-Range`, folders,
  rescans, tag writes and token replacement. `vcmp-protocol` holds the shared types.
- **Mirror** (workstream 3, schema version 19, `vcmp-library/src/mirror.rs`): a
  server track is a `tracks` row plus a `remote_tracks` row, its `path` the key
  `vcmp-server:<server>:<id>`; `Track::file` is None for it, so every use of the
  old path field had to be looked at. Code that turns a row's path string back into
  a file path was checked by hand: the only "show in file manager" is the library
  check's, which lists local tracks only, and lyrics skip server keys. Server albums group with the scanner's own
  functions under a `server:<id>:` prefix; artists stay shared. The scanner,
  re-linking, the artwork pass, the library check and M3U import leave server rows
  alone; M3U export leaves them out and the playlist page says how many.
  `vcmp-client::sync` fetches a page, then writes it with the cursor in one
  transaction, syncs again from the start when the app is ahead of the server, marks
  unseen rows unavailable then, and fetches missing covers.
- **Playback** (workstream 4): `SourceKind::Server`, `PlaybackError::NotReady`,
  `TrackSource::prepare` and `Event::Buffering` in the engine (a not-ready track
  waits without failing; a not-ready next track ends the run and starts once it is
  there); `vcmp_client::ServerSource` downloads whole files on one thread into a
  bounded cache (2 GB by default) and decodes them with `LocalSource`. A file changed
  on the server since the last sync is kept under the name the sync knows (it used
  to download again on every poll); a failed download is tried again after 30 s.
  `examples/play_server.rs` plays a folder through an in-process server, the server
  source and a real `Player`.
- **App** (workstream 5): Settings' Music servers section (address and token, the
  plain http warning, each server's state, Sync now, Remove, the cache size and
  Empty), `ServerService` (tokens in the OS keyring, a sync at start and every 5
  minutes), album cards with "Artist · server", muted titles while a server can't be
  reached, servers' folders on the Folders page, "Loading" in the player bar while a
  track downloads.
- **Admin** (workstream 6): Manage… on a server connected with the admin token:
  folders, Rescan with the report (polled while the server scans, synced after), new
  user and admin tokens. The tag editor sends server tracks' edits to their server
  and syncs before it reports.
- **AI agents:** the MCP tools that search, list, read tags, propose changes or play
  take local tracks only; playlists and the queue keep server entries in place as
  `{"server_track": true, "title", "artist"}`.
- **Packaging and docs:** the Dockerfile; docs/server.md, library.md ("Change feed",
  "Server tracks"), ui.md ("Music servers"), playback.md, mcp.md, manual-testing.md,
  development.md, the README's "Music servers" and CLAUDE.md's tables.
- **Checks** (Windows 11): `cargo fmt --all --check` and `cargo clippy --workspace
  --all-targets -- -D warnings` pass. `cargo test --workspace --no-fail-fast`: 758
  passed, 1 failed, the flaky `vcmp-mcp` token test below.
- **Run for real:** the server binary (every command, then the How to test curl
  steps) and the Docker image (amd64, see Follow-up changes 1). The app's side ran
  in its tests only, not in the real window.

## Follow-up changes

1. Build and try the Docker image. Built with Docker 29.9.2 (linux/amd64, Docker
   Desktop on Windows) in about 2 minutes, 130 MB; tokens through `docker exec`, the
   API, a byte-identical download, read-only and writable music, restart and stop
   behaved as described. No code changed.
2. Continue with the client side on the same branch and pull request. The M3U
   question was put to the owner first (export skips server tracks and lists them,
   import matches local tracks only) and recorded in the spec; then workstreams 3 to
   6 were built as above, the engine and the MCP rules by parallel agents.

## How to test

**Automated coverage.**
- `crates/vcmp-server/tests/api.rs`: the server over HTTP as the app uses it.
- `crates/vcmp-client/tests/sync.rs`: an in-process server synced into a library
  (albums, covers, search, removal and return, a restored server, a local scan that
  leaves the mirror alone) and a server track decoded through `ServerSource`,
  compared sample for sample with the original file.
- `crates/vcmp-app/tests/servers.rs`: the Settings section against an in-process
  server: Connect, the album card "Afar, Remote · VCMP server", Play sending server
  queue entries, a server that goes away (dimmed card), Remove, a wrong token, the
  plain http warning, a restart with the server already connected (the start page's
  cards name it), and the admin path: Manage… lists the folders, a new admin token is
  kept and the app stays connected, a title edit routed to the server shows in the
  library. Breaking the album mark makes it fail. The tag editor's own window isn't
  driven: the test calls the routing (`servers::write_tags`) directly.
- `vcmp-client/tests/sync.rs` also plays a file changed on the server since the
  sync, which used to loop.
- `crates/vcmp-playback` engine tests (a slow fake source), `crates/vcmp-mcp/tests/server.rs`
  (agents and server tracks), `crates/vcmp-library/tests/feed.rs`.
- Run all with `cargo test --workspace`.

**Setup.** A server over a copy of some albums: in Docker as in docs/server.md
(`127.0.0.1:8268`), or `VCMP_SERVER_DATA=target/server VCMP_SERVER_MUSIC=target/server-music cargo run -p vcmp-server`.
`vcmp-server tokens` (or `cargo run -p vcmp-server -- tokens` with the same
variables) prints the user and admin tokens. The app on a scratch library, never
the real one (it migrates to schema 19, which 26.11.0 refuses):
`VCMP_HOME=target/try-server VCMP_SETUP=skip cargo run -p vcmp-app`.

**Steps.**

1. Settings, Music servers: type `127.0.0.1:8268` in Server address and the user
   token in Token, press Connect. The row shows "Syncing: n tracks received", then
   "Connected".
2. Albums: the server's albums are there, their cards reading "Artist · VCMP server"
   (or the configured name), with covers.
3. Open one and press Play: the bar says "Loading" for a moment, then the track
   plays; seeking works; the next track follows.
4. Play the album again: it starts at once. Settings shows the downloaded size.
5. Folders: "VCMP server · music" opens the server's folder tree.
6. Stop the server and press Sync now: the row says it can't be reached and the
   server's album titles are dimmed; the album played before still plays.
7. Start the server, Sync now, then Remove: its albums leave the pages. Connect again
   with the same token: they come back, and a playlist holding them is whole again.
8. Connect with the admin token instead: Manage… lists the folders, Rescan runs and
   reports, New user token shows a token to copy (the old one stops working), and a
   title changed in the tag editor shows on the album page.

**Also worth trying.**
- A wrong token: "The server refused that token"; nothing is added.
- `http://music.example.com` with any token: the plain http warning, with Connect
  anyway and Cancel.
- Export a playlist with server tracks: the page says how many were left out because
  they play from a server.
- An AI agent asked for that playlist: it lists the server entries as server tracks,
  without tags, and can't play them.
- The server alone, with curl: `/health`, a 401 without a token, the feed with
  `limit=2`, a range request on `/api/v1/tracks/<id>/audio`, an admin tag write
  (docs/server.md has the API).

## What is missing

- **A source filter** on Albums, Artists, Songs and Search (spec, workstream 5): only
  album cards carry the server's name; song rows and search results don't, and no
  page filters by source.
- **Greyed song rows** while a server is away: only album cards are dimmed.
- **Releases:** the image is not published, ARM64 was never built, and the release
  workflow and `packaging/release-local.sh` don't build the server.
- **The app was not run in its real window** against a server; its tests cover the
  flows headless.

## Known problems

- **Whole files:** a server track starts only once its whole file is downloaded
  (one download at a time), so the first track of a FLAC album waits for its file,
  and a join into a track still downloading is not gapless. The wait was not
  measured over a real link.
- **Tag editor:** it can't show a server track's current cover, and without the admin
  token or on read-only music each file is reported as read-only only after Save.
- **AI agents:** `next` and `previous` still reach server entries in the queue, and
  several tools read every track per call to drop server ones (docs/mcp.md).
- **A server away:** each track of it not downloaded before waits for its download
  to fail, then is skipped, one after another.
- **Polling:** changes on a server show at the next sync, every 5 minutes or on Sync
  now; the cache sizes are fixed choices (1 to 20 GB).
- **Docker Desktop** on Windows and macOS doesn't pass a mounted folder's file
  changes into the container: the server sees them at its hourly rescan or an
  admin's.
- **Schema 19** is applied to every app library this build opens; 26.11.0 then
  refuses it.
- **Server side:** the feed's tracks and `head` are two reads; trigger cost on a
  large scan unmeasured; admin calls wait behind a running scan; stopping waits for
  downloads in flight; a configured folder missing at start is skipped; wrong tokens
  aren't rate-limited; covers travel base64 in JSON (64 MB limit).
- **No comments in the new code**, following the owner's rule, unlike the rest of
  the repository; the docs carry what they would have said.
- **Flaky test:** `vcmp-mcp`'s `only_requests_with_the_token_and_without_a_browser_origin_get_in`
  fails about one run in three on Windows (connection reset, 10054). This branch
  changed that crate, but not the code under that test; it was not run on `main`.
- **Flaky on `main` too:** after merging `main` (#101),
  `vcmp-playback`'s `stop_and_the_end_of_the_queue_clear_stop_after_the_current_track`
  fails about one run in four, one buffer period (1,024 frames) longer than
  expected. `main`'s own playback crate, run alone, fails it 2 times in 8.

## Recommended follow-ups

- Run the app against a real server (Docker on Linux, behind the owner's nginx) with
  docs/manual-testing.md's "Music servers in the app", and measure the first track's
  wait.
- Build the source filter and the marks on song rows and search results.
- Stream while downloading (decode a growing file) so a track starts before its whole
  file is there.
- Add the server to the release workflow and publish the image for both
  architectures.
- Give vcmp-library local-only queries for the MCP tools, instead of filtering every
  track per call.

## Changelog

- Connect to VCMP servers in Settings and play their music next to your own: their
  albums show with the server's name, stream when played, and stay on your computer
  up to a size you choose
- See a server's folders on the Folders page
- Manage a server's music folders, rescans and tokens, and edit its tracks' tags,
  when connected with its admin token
- Run your own VCMP server in Docker or as a program, to share a library from a NAS,
  a VPS or another computer
