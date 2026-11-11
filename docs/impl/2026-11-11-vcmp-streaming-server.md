# VCMP streaming server

Spec: docs/specs/2026-11-11-vcmp-streaming-server.md
Date: 2026-11-11
Status: partially implemented
Area: vcmp-library (schema version 18, change feed), new crates vcmp-protocol and
vcmp-server, packaging/server, docs

## What was done

The server side of the spec: workstreams 1 (server core) and 2 (protocol), plus the
Dockerfile and documentation from workstream 7. The app can't connect yet.

- **Change feed in vcmp-library** (`src/feed.rs`, schema version 18): a `revision`
  on every track, kept by triggers that fire only when a mirrored value really
  changes (an unchanged rescan changes nothing), and that renumber an album's tracks
  when its artwork changes. `Library::track_feed` pages by `(revision, id)`, and
  every feed page carries the `head`, so an app ahead of a server restored from a
  backup can tell and sync again; `Library::artwork_by_key` refuses anything that
  isn't a key's shape. The scan report gives users file and folder names only.
- **vcmp-protocol**: paths, request and answer types, error codes with an `Other`
  fallback for codes a newer server adds, the protocol version, `audio_etag`. serde
  only.
- **vcmp-server**: the `vcmp-server` binary with `serve`, `tokens` and
  `new-token user|admin`; configuration from a TOML file and `VCMP_SERVER_*`
  variables, the environment winning, `_FILE` variants for Docker secrets; the
  server id and both tokens made on first start, configured ones taking precedence
  and refusing replacement; one writer thread (scans, folders, tag writes, watcher,
  hourly rescan), pooled read connections; every API path of the spec, audio with
  single-range requests and `If-Range`, `Cache-Control: private, no-store` on every
  answer that needs a token.
- **Packaging and docs**: `packaging/server/Dockerfile` (multi-stage, non-root,
  `/data` and `/music`), `.dockerignore`, the new `docs/server.md`, and updates to
  library.md, development.md, manual-testing.md and CLAUDE.md.
- **Checks** (Windows 11): `cargo fmt --all --check` and `cargo clippy --workspace
  --all-targets -- -D warnings` pass. `cargo test --workspace --no-fail-fast`: 735
  passed, 1 failed. The failure is
  `vcmp-mcp`'s `only_requests_with_the_token_and_without_a_browser_origin_get_in`:
  a connection reset (OS error 10054) when the MCP server refuses a request before
  reading its body. It fails about one run in three, also on its own, and passes
  otherwise; this branch doesn't change `vcmp-mcp`, though the test was not run on
  `main` to compare. After the code's comments were removed and the feed's `head`
  and the users' scan report were added, the changed crates' tests and clippy
  passed again.
- **The binary, run for real** (Windows, port 18268, two fixture files): `--version`,
  `--help`, `tokens`, `new-token` with a bad role (exit 2), a short configured token
  (refused, exit 1), `new-token user` against a configured one (refused), then
  `serve` and the How to test steps 1 to 8 below with curl, all as described; the log
  held no token. Stopping by signal was not tried: the process was killed.

## How to test

**Automated coverage.** `crates/vcmp-server/tests/api.rs` drives the server over
real HTTP as the app's sync will; `crates/vcmp-library/tests/feed.rs` covers the
revisions. Run them with `cargo test -p vcmp-server -p vcmp-library --test feed`.
Breaking the `If-Range` check makes `an_app_mirrors_plays_and_edits_the_library`
fail, which shows the test can fail. Nothing in the app is user-facing yet, so there
is no UI test.

**Setup.** A folder with a copy of a few albums (tags may be edited), for example
`target/server-music`. Then, from the repository's root:

```bash
VCMP_SERVER_DATA=target/server VCMP_SERVER_MUSIC=target/server-music cargo run -p vcmp-server
```

In a second shell, `VCMP_SERVER_DATA=target/server cargo run -p vcmp-server -- tokens`
prints the server id, the user token (`<user>`) and the admin token (`<admin>`).

**Steps.**

1. `curl http://127.0.0.1:8268/health` prints `ok`.
2. `curl -i http://127.0.0.1:8268/api/v1/handshake` answers 401 with
   `{"code":"unauthorized",...}`.
3. `curl -H "Authorization: Bearer <user>" http://127.0.0.1:8268/api/v1/handshake`
   shows the server id `tokens` printed, `"protocol":1`, `"role":"user"` and
   `"can_write_tags":true`.
4. `curl -H "Authorization: Bearer <user>" "http://127.0.0.1:8268/api/v1/feed?limit=2"`
   lists two tracks with `path` relative to the music folder (`Album/01.flac`),
   `artwork` keys, and `"more":true` when there are more tracks; repeat with
   `after_revision` and `after_track` from `next` until `"more":false`, where
   `next` equals `head`.
5. `curl -H "Authorization: Bearer <user>" http://127.0.0.1:8268/api/v1/thumbnails/<artwork key> -o cover.jpg`
   saves the album's cover as a JPEG.
6. `curl -H "Authorization: Bearer <user>" -H "Range: bytes=0-99" -i http://127.0.0.1:8268/api/v1/tracks/<id>/audio`
   answers 206 with `Content-Range: bytes 0-99/<size>` and an `ETag`.
7. As the admin, rename a track:
   `curl -H "Authorization: Bearer <admin>" -H "Content-Type: application/json" -d '{"groups":[{"tracks":[<id>],"edit":{"title":{"op":"set","value":"Renamed"}}}]}' http://127.0.0.1:8268/api/v1/tags`
   answers `{"written":[<id>],"failed":[]}`.
8. The feed after the last `next` cursor of step 4 lists only that track, titled
   "Renamed", with a new `modified_ms`; its audio's `ETag` changed too.

**Also worth trying.**

- The user token on step 7: 403 `admin_only`.
- `"year":{"op":"set","value":0}` in step 7: 400 `invalid_tag_edit`, no file touched.
- `DELETE /api/v1/folders/<id>` of the configured folder: 409 `configured_folder`.
- `POST /api/v1/folders` with `{"path":"/no/such/folder"}`: 400 `not_a_folder`.
- `cargo run -p vcmp-server -- new-token user` while the server runs: the old user
  token gets 401 within 2 seconds, the new one works.
- `VCMP_SERVER_USER_TOKEN=short`: the server refuses to start and says why.
- Copy another album into the music folder: within about 5 seconds of the copy ending
  the feed after the last cursor lists only its tracks.

## What is missing

- Workstream 3, the mirror in the app (the app's tracks from servers, the list of
  servers, the sync, grouping, the local-only guard and the agent rules): not
  started.
- Workstream 4, playback of server tracks (the `server` source, prepare hint,
  buffering, the bounded audio cache): not started.
- Workstream 5, the app's UI (Settings' server list and cache size, source marks and
  filter, server folders on the Folders page, greyed music while a server is away,
  the plain http warning): not started.
- Workstream 6, admin in the app: the server side of every admin call exists; the
  app's controls don't.
- Workstream 7: the image is not published, the release workflow and
  `packaging/release-local.sh` don't build the server, and the README has no
  section, since the app can't use a server yet.
- The open question on M3U for server tracks is still to be decided before
  workstream 3. The other, reaching `main` together, is answered by building
  everything on this one branch.

## Known problems

- The Docker image was never built: Docker wasn't running on the machine this was
  written on. The base image tags (`rust:1.99.1-trixie`, `debian:trixie-slim`) are
  unverified.
- Everything ran on Windows only; the server targets Linux, where it has never run.
- Schema version 18 is shared, so every app library gets the revision column and
  triggers on its next start, though the app doesn't use them yet; older builds then
  refuse that database. Running this branch's app on the real library makes the
  released 26.11.0 refuse to open it: try the branch's app only with a `VCMP_HOME`
  copy.
- The feed's tracks and its `head` are two reads, not one transaction: a write
  committed between them can make `head` newer than a last page's `next`, and the
  app would then ask once more.
- The triggers run on every scan's track write; their cost on a large library wasn't
  measured.
- Adding or removing a folder and writing tags wait behind a running scan on the one
  writer thread: during a large library's first scan (40 s on the owner's 3,600
  tracks from a cold disk) an admin's request waits that long.
- Graceful shutdown waits for downloads in flight, so `docker stop` may kill a server
  mid-download after its grace period.
- A configured music folder missing at start is skipped with a warning and isn't
  listed as unreachable.
- Whether folders are writable is checked by making a hidden temporary file in each
  music folder, at start and after folder changes.
- Covers in tag edits travel base64 in JSON, a third larger than the image, with a
  64 MB limit per request.
- Wrong tokens are not rate-limited.
- The new code carries no comments, following the owner's own rule to remove them,
  unlike the rest of the repository; what they explained is in docs/server.md and
  library.md's "Change feed".
- `vcmp-mcp`'s token test is flaky on Windows (see the checks above). This branch
  doesn't touch that crate, so it most likely fails on `main` too, but that wasn't
  run.

## Recommended follow-ups

- Build and run the image on Linux (x64 and ARM64) and behind the owner's nginx,
  following docs/manual-testing.md, before building the app's side on it.
- Decide M3U for server tracks, then start workstream 3, the app's mirror.
- Measure a scan of the owner's library with the triggers, against the numbers in
  library.md.
- Add the server to the release workflow and `packaging/release-local.sh`, and publish
  the image, once the app can connect.
- Make `vcmp-mcp`'s token test tolerate a reset connection on Windows (or have the
  server read the body before refusing), so the workspace tests pass reliably there.

## Changelog

No user-visible changes.
