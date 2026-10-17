# VCMP

A cross-platform (Windows 10+, Linux, macOS) desktop music player for locally stored
music, written in Rust with a Slint UI. The agreed v1 scope and architecture are in
[docs/specs/2026-10-17-vcmp-music-player.md](docs/specs/2026-10-17-vcmp-music-player.md).
Read it before changing anything structural.

## Branches

- `main` is production: it only receives merged pull requests, and releases are tags
  on `main` (`vX.Y.Z`).
- All work happens on `feature/<topic>` branches that merge into `main` through a PR.
- Never commit directly to `main`.

## Technology stack

| Area | Choice |
|---|---|
| Language | Rust, edition 2024, minimum Rust 1.92 |
| UI | Slint 1.18, royalty-free license (the `AboutSlint` widget must stay on the About page) |
| Audio (planned) | symphonia (decoding) + cpal (output), own engine |
| Storage (planned) | SQLite, bundled |
| Typeface | Sora, bundled from `assets/fonts/sora/` (SIL OFL 1.1) |
| CI | GitHub Actions on Ubuntu, Windows and macOS |
| License | MIT OR Apache-2.0 |

Workspace layout:

| Crate | Role |
|---|---|
| `crates/vcmp-app` | Binary `vcmp`: the Slint UI (`ui/`) and its Rust glue (`src/`) |
| `crates/vcmp-library` | Library scanning, tags, database, artwork (placeholder) |
| `crates/vcmp-playback` | Decoding, output, queue, shuffle (placeholder) |
| `crates/vcmp-integrations` | OS media controls, Last.fm, web artwork, update check (placeholder) |

The core crates must never depend on `vcmp-app` or Slint.

## Run locally

```bash
cargo run -p vcmp-app
```

On Linux, building needs the fontconfig and xkbcommon development packages
(`libfontconfig1-dev libxkbcommon-dev` on Debian/Ubuntu; `fontconfig libxkbcommon`
on Arch).

## Test

```bash
cargo test --workspace
```

CI also runs `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings`; run both before pushing.

UI tests run headless through Slint's testing backend (`i-slint-backend-testing`,
pinned with `=` to the Slint version; bump both together). To look at the real
window in each theme and focus state:

```bash
cargo run -p vcmp-app --example screenshots
```

The PNGs land in `target/screenshots/`.

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
| `README.md` | What VCMP is, how to build, run and test |
| `CLAUDE.md` | This file: conventions for working in the repository |
| `docs/specs/` | Agreed specifications, one file per planned change |
| `docs/DESIGN.md` | The owner's design direction |
| `docs/ui.md` | UI structure, tokens, design reasons, known limits |
| `ideas.md` | The owner's original idea list |

Keep the technical documentation up to date: when a change alters behaviour,
structure, commands or conventions described in these files, update the files in the
same change.

## Localization

- v1 ships in English only, but every user-visible string in `.slint` files goes
  through `@tr()` (enforced by `tests/ui_invariants.rs`). Strings built in Rust for
  display must be translatable the same way once they appear.
- Adding a language later: extract strings with `slint-tr-extractor` into a `.pot`
  file, add a `.po` per language under `crates/vcmp-app/translations/`, and enable
  Slint's gettext or bundled-translations support.
- All code, comments and documentation are written in English.
