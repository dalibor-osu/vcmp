# Custom themes

Spec: docs/specs/2026-11-05-custom-themes.md
Date: 2026-11-06
Status: implemented
Area: vcmp-app (theme, Settings, setup wizard, title bar scheme, screenshots), docs

## What was done

- **Themes** (`crates/vcmp-app/src/theme/mod.rs`):
  - There are four built-in themes. Light and Dark keep today's palettes exactly.
    Darker is warm and deeper; Midnight is neutral and close to black.
  - There are 10 accents, one per label hue. Each has a hand-picked light colour and
    a dark colour shared by the dark themes. Yellow has no light colour.
  - `apply` sets the `Theme` global's palette and kind, and sets
    `Palette.color-scheme`.
- **Contrast check** (`theme/contrast.rs`): one list of pairs and thresholds, used
  by the tests and by the Settings warning.
- **Theme files** (`theme/file.rs`): TOML with `name`, `kind` and a `[colors]`
  table. The module reads, writes and lists the themes folder. Unknown keys are
  ignored.
- **Choice** (`src/themes.rs`):
  - `ui.theme` is now JSON holding the theme, the accent and the kind of a file
    theme.
  - Saved `light` and `dark` become the built-in themes of the same name.
  - `system`, a fresh install or an unreadable value shows Light until the OS
    reports its scheme. Then it picks Light or Dark once and saves it.
  - A chosen file that is gone or broken falls back to the built-in theme of its
    kind, with a note.
  - Reload, Open themes folder and Save as theme file (`<id>-copy.toml`) are here.
- **UI:**
  - `ui/themes.slint` holds the `Themes` global, the swatch tiles (one tab stop;
    arrows move) and the accent dots. It also shows the file buttons, invalid files,
    the missing-file note and the contrast warning.
  - The `Theme` global takes one palette and a `ThemeKind`; `ThemeMode` and
    `ThemeRow` are gone.
  - The wizard shows the built-in themes and the accent only.
- **Tests:** unit tests in `theme/` and `themes.rs`, plus `tests/themes.rs`
  (8 end-to-end cases). `ui_behaviour`, `setup` and `preferences` are updated.
- **Screenshots:** the example adds Albums and Settings shots for Darker and
  Midnight, and `appearance-light`/`-dark` (tall Settings with an example theme
  file).
- **Docs:** ui.md ("Palettes", new "Themes"), DESIGN.md, library.md, README.md,
  CLAUDE.md (`VCMP_HOME` themes folder, contrast rule) and manual-testing.md.
- **Checks:** `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` are clean.
  `cargo test --workspace` passed 604 tests with 0 failures. Screenshots of Settings
  and the wizard were looked at in Light, Dark, Darker and Midnight.

## How to test

**Automated coverage.** `cargo test -p vcmp-app --test themes` drives the picker
through the real window. `cargo test -p vcmp-app --lib theme` covers contrast,
files, migration and fallback.

**Setup.** Run on a throwaway library: `VCMP_HOME=target/theme-check VCMP_SETUP=skip
cargo run -p vcmp-app`. The themes folder is `target/theme-check/themes/`.

**Steps.**
1. Open Settings and scroll to Appearance. Four tiles show: Light, Dark, Darker,
   Midnight. One is ringed: Dark if the OS is dark, Light otherwise.
2. Click Midnight. The whole window turns near-black and the Accent row shows 10
   dots.
3. Click the Teal dot. The page marker, switches and focus rings turn teal.
4. Click Light. Yellow is missing from the dots, and teal is a dark teal.
5. Quit and start again with the same command. Light with teal is back.
6. Choose Darker, then press Save as theme file. The line "Saved as
   darker-copy.toml in the themes folder." shows, and a "Darker copy" tile appears.
7. Press Open themes folder. The file manager opens `target/theme-check/themes/`.
8. In `darker-copy.toml`, set `base = "#203040"` and press Reload. Click "Darker
   copy": the page background is that blue. The accent dots are replaced by "A theme
   file brings its own accent."
9. Set `text = "#203040"` (the same as base) and press Reload. The theme still
   applies, under "Some colours of this theme are hard to read (WCAG AA):", with
   "text on base: 1:1, needs 4.5:1" among the lines.
10. Delete `darker-copy.toml` and press Reload. Dark shows, with "The theme file
    darker-copy.toml is gone or can't be used, so VCMP shows Dark instead."

**Also worth trying.**
- A file without `outline` is listed as "<file>: the colour outline is missing."
  and gets no tile.
- `kind = "sepia"` gives "kind must be "light" or "dark"".
- Saving twice gives `darker-copy-2.toml`.
- Keyboard: Tab to the tiles, then Left/Right change the theme. The next Tab
  reaches the accents.
- An older library with `ui.theme` = `dark` opens on Dark with pink, unchanged.
- Settings, Run setup again, then the Look step, shows four tiles and the accents, without file
  buttons.

## What is missing

Nothing - the spec is implemented in full.

## Known problems

- On a dark OS, the first start can show Light for a moment before it picks Dark.
  Slint reports the OS scheme only once the window exists. The pick itself was
  checked on Windows 11 set to dark: a fresh `VCMP_HOME` saved
  `{"theme":"dark","accent":"pink"}`. macOS and Linux are not checked.
- No palette token has been added since theme files exist, so the "derived default
  for a newer token" rule has no code yet. The next token added has to add it to
  `theme/file.rs`.
- Without a library database the theme isn't saved and the first-start pick doesn't
  run. The app starts on Light rather than the OS scheme, unlike before.
- "Open themes folder" spawns `explorer`, `open` or `xdg-open`. None of them has
  been tried by hand yet (manual-testing.md lists it). A missing `xdg-open` shows
  the OS error under the buttons.
- Lavender on Light reaches 4.5:1 only as a vivid blue-violet, close to blue (see
  DESIGN.md, "Awaiting the owner's review").
- The colour scheme handed to `Palette.color-scheme` is not yet checked on macOS
  and Linux title bars.

## Recommended follow-ups

- The owner reviews the new palettes, the names Darker and Midnight, and the dropped
  yellow on Light (DESIGN.md).
- The screenshots example's `agents-*` shots open page 6, which has been Folders
  since Labels joined the sidebar. They should open Settings (page 7).
- The `focus-theme-dark` shot's Tab count no longer reaches the theme picker.

## Changelog

- Choose from four themes: Light, Dark, Darker and Midnight
- Pick an accent colour for the built-in themes from ten hues
- Use your own theme files: save a built-in theme as a file, edit its colours and reload
- See which colours of your own theme are hard to read, right in Settings
- The light/dark/system switch is replaced by the theme list; VCMP picks light or dark from your system on the first start only
