# VCMP

A cross-platform (Windows 10+, Linux, macOS) desktop music player for locally stored
music, written in Rust with a Slint UI. The agreed v1 scope and architecture are in
[docs/specs/2026-10-17-vcmp-music-player.md](docs/specs/2026-10-17-vcmp-music-player.md).
Read it before changing anything structural.

## Branches

- `main` is production: it only receives merged pull requests, and releases are tags
  on `main` (`vYY.MM.PATCH`, a calendar version such as `v26.10.0`;
  docs/development.md, "Versions"). Pushing such a tag runs
  `.github/workflows/release.yml`, which drafts a GitHub Release with an archive of
  the build per platform; docs/development.md, "Releases", has the steps.
  Pre-releases have no version suffix: they are releases published with GitHub's
  pre-release box ticked.
- All work happens on `feature/<topic>` branches that merge into `main` through a PR.
- Never commit directly to `main`.

## Technology stack

| Area | Choice |
|---|---|
| Language | Rust, edition 2024, minimum Rust 1.92 |
| UI | Slint 1.18, royalty-free license (the `AboutSlint` widget must stay on the About page) |
| Audio | Own engine: symphonia 0.6 (decoding), cpal 0.18 (output; on Linux its pure-Rust PulseAudio backend, ALSA when no server answers), rubato 5 (resampling), rtrb (lock-free ring buffer) |
| Storage | SQLite (bundled via rusqlite), FTS5 search; tags read and written with lofty |
| OS media controls | playwire 2 (MPRIS, SMTC, Now Playing) |
| OS reduce-motion setting | zbus to the XDG settings portal (Linux), windows-sys (Windows), objc2-app-kit, objc2-foundation and block2 (macOS); all already come with Slint |
| Last.fm | ureq for the API, md5 for its request signatures, keyring 4 for the session key (Keychain, Credential Manager, Secret Service over zbus) |
| AI agents (MCP) | rmcp 3.6 (the official Rust SDK) serving Streamable HTTP through axum 0.8 on 127.0.0.1, on a tokio runtime of its own; getrandom for the token |
| Typeface | Sora, bundled from `assets/fonts/sora/` (SIL OFL 1.1) |
| CI | GitHub Actions, turned off for now: `.github/workflows/ci.yml` runs only when started by hand (Ubuntu, Windows and macOS). With its triggers restored: all three for pull requests into `main` and pushes to it, Ubuntu only for pull requests between feature branches, and a newer push to a pull request cancels its running checks |
| Releases | An unsigned archive per platform (zip of `vcmp.exe`, zip of `VCMP.app`, tar.gz of `vcmp`), built by `.github/workflows/release.yml`; while that can't run, `packaging/release-local.sh` builds the Windows and Linux ones locally in Docker (docs/development.md, "Building the archives locally"). Installers are left for later: the cargo-packager configuration for MSI, NSIS, dmg, AppImage and .deb stays in `[package.metadata.packager]` in `crates/vcmp-app/Cargo.toml` (icons in `packaging/icons/`); the macOS `.app` already comes from it |
| License | MIT OR Apache-2.0 |

Workspace layout:

| Crate | Role |
|---|---|
| `crates/vcmp-app` | Binary `vcmp`: the Slint UI (`ui/`) and its Rust glue (`src/`) |
| `crates/vcmp-library` | Source directories, scanning, tags (reading every tag, writing any), SQLite database, artwork cache, search, playlists, change sets agents ask for (review, undo, retention), watcher |
| `crates/vcmp-playback` | Playback engine: decoding, output and devices, gapless, queue, shuffle ([docs/playback.md](docs/playback.md)) |
| `crates/vcmp-integrations` | OS media controls, the OS reduce-motion setting, Discord Rich Presence and its MusicBrainz cover lookup, cover search for "Find cover online", artist photos from fanart.tv, the update check, Last.fm scrobbling |
| `crates/vcmp-mcp` | The MCP server AI agents connect to: its tools, the token and Origin checks; changes go to the app through its `Changes` trait |

The core crates must never depend on `vcmp-app` or Slint.

## Run locally

```bash
cargo run -p vcmp-app
```

`VCMP_HOME=<dir>` makes the app use `<dir>/library.sqlite3` and `<dir>/cache/`
instead of the platform data and cache directories. For example,
`VCMP_HOME=target/scan-example cargo run -p vcmp-app` opens the library that
`cargo run --example scan` built.

`cargo run -p vcmp-app -- mcp` runs the stdio relay for MCP clients instead of the
window: it relays standard input and output to a VCMP running with the same
`VCMP_HOME` and its AI agents switch on ([docs/mcp.md](docs/mcp.md)). Accepting an
agent's change writes tags, so try agents on a `VCMP_HOME` library of copied files.

Last.fm exists only in builds made with VCMP's API key: set `VCMP_LASTFM_API_KEY`
and `VCMP_LASTFM_SHARED_SECRET` when building (read with `option_env!`; without both
the Settings row is hidden and nothing is sent). Never commit them; CI and the
release workflow take them from the repository secrets of the same names
(docs/development.md, "Integration keys"). The connected account's session key lives in the OS keyring,
shared by every build and `VCMP_HOME` on the machine. "Find photo online" on artist
pages likewise needs `VCMP_FANARTTV_API_KEY` at build time.

Debug builds optimise the dependencies (`[profile.dev.package."*"]` in
`Cargo.toml`): unoptimised Slint, its renderer and image decoding made the album grid
take 46 ms a frame. The first debug build is slower for it; later ones only rebuild
VCMP's crates. Dependencies carry line tables only, and CI strips debug info from
test binaries (`CARGO_PROFILE_DEV_*` in the workflow): with full debug info each test
binary was 400 to 660 MB and filled the runner's disk. Measure performance with
`--release`.

On Linux, building needs the fontconfig, xkbcommon and ALSA development packages
(`libfontconfig1-dev libxkbcommon-dev libasound2-dev` on Debian/Ubuntu;
`fontconfig libxkbcommon alsa-lib` on Arch).

## Test

```bash
cargo test --workspace
```

CI also runs `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings`; run both before pushing.
While CI is turned off, run all three and the tests locally before every merge.

UI tests run headless through Slint's testing backend (`i-slint-backend-testing`,
pinned with `=` to the Slint version; bump both together). To look at the real
window in each theme and focus state:

```bash
cargo run -p vcmp-app --example screenshots
```

The PNGs land in `target/screenshots/`. Set `VCMP_HOME` to show a real library in
the album shots, and `VCMP_SHOT_ONLY=review` to take only the shots whose names hold
that text. `--example grid_benchmark` scrolls the album grid through thousands
of synthetic albums and reports render times and memory (see
[docs/ui.md](docs/ui.md#album-grid)). `--example album_page_benchmark` (with
`VCMP_HOME`) opens the library's longest album and scrolls its page.

Library tests use the tiny silent files in `crates/vcmp-library/tests/fixtures/`
(see the README there for how they were made) and write tags into temporary copies.
To check the scanner against a real collection (files are only read; the database
goes to `target/scan-example/`):

```bash
cargo run --release -p vcmp-library --example scan -- /path/to/music
```

`--example scan_profile` breaks a scan down by step (tag reads per format, cover
read, decode, resize, encode) on Linux. [docs/library.md](docs/library.md#measured)
has the reference numbers.

To check tag writing against real files, `--example edit_check -- /path/to/music 40`
copies 40 files per format into `target/edit-check/` and edits only the copies.
Never point a test or example that writes at the real collection itself.

Playback tests need no sound card: the engine runs against a test device that the
test pulls audio from. To hear the engine on the real device (or check it silently
with `--volume 0`):

```bash
cargo run -p vcmp-playback --example play -- path/to/album
```

## Playback work

- How the engine works, and why, is in [docs/playback.md](docs/playback.md).
- The audio callback (`Renderer::render`) must never allocate, lock, wait or do
  I/O. `no_allocation_in_the_callback` enforces the allocation part; keep the
  callback to the rtrb queues and atomics.
- The test fixtures in `crates/vcmp-playback/tests/fixtures/` are generated by
  `make-fixtures.sh` there; change the script, not the files.

## UI work

- Visual direction is the owner's [docs/DESIGN.md](docs/DESIGN.md). Don't change it
  without the owner; record their decisions there.
- How the UI implements that direction, and the rules the tests enforce, are in
  [docs/ui.md](docs/ui.md). In short: no colour literals and no durations outside
  `ui/theme.slint`, every visible string in `@tr()`, WCAG AA contrast in both themes,
  everything operable by keyboard.
- The owner uses the antislop plugin in "during" mode for UI work.

## Documentation structure

| Path | Contents |
|---|---|
| `README.md` | For people using VCMP: what it does, download, first start, building it |
| `docs/development.md` | Build, test, integration keys, versions and releases |
| `CLAUDE.md` | This file: conventions for working in the repository |
| `docs/specs/` | Agreed specifications, one file per planned change |
| `docs/DESIGN.md` | The owner's design direction |
| `docs/ui.md` | UI structure, tokens, design reasons, known limits |
| `docs/library.md` | Library storage, schema, scan and re-link rules, search, known limits |
| `docs/playback.md` | Playback engine: threads, data path, queue and shuffle rules, known limits |
| `docs/mcp.md` | The MCP server for AI agents: running, security, tools, how changes reach the app, known limits |
| `docs/manual-testing.md` | What only a person can check: real devices, online services, Windows and macOS |
| `ideas.md` | The owner's original idea list and improvements noticed in use |

Keep the technical documentation up to date: when a change alters behaviour,
structure, commands or conventions described in these files, update the files in the
same change. A change that tests can't fully check adds its manual steps to
`docs/manual-testing.md`.

## Localization

- v1 ships in English only, but every user-visible string in `.slint` files goes
  through `@tr()` (enforced by `tests/ui_invariants.rs`). Strings built in Rust for
  display must be translatable the same way once they appear.
- Adding a language later: extract strings with `slint-tr-extractor` into a `.pot`
  file, add a `.po` per language under `crates/vcmp-app/translations/`, and enable
  Slint's gettext or bundled-translations support.
- All code, comments and documentation are written in English.
