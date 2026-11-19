# Settings sections

Spec: docs/specs/2026-11-19-settings-sections.md
Date: 2026-11-19
Status: implemented
Area: vcmp-app (Settings page, navigation, preferences, setup wizard, photo picker, Library check, tests, screenshots example, docs)

## What was done

- Settings shows one of six sections (General, Library, Playback, Appearance,
  Online services, AI agents) beside a list of them, with a search field above
  the list; in a page too narrow for both, the search field and a dropdown of the
  sections sit above the section (`SettingsPage` in `ui/pages.slint`, new
  `ui/settings-nav.slint` with the `SettingsNav` global).
- Bundled components were split so the search can show their rows one by one:
  `AgentsSection` became `AgentsConnection`, `AgentsPlaybackRow`, `AgentsPortRow`,
  `AgentsChangesRow`, `AgentsBackupsRow`; `ThemeRows` became `ThemeRow`,
  `AccentRow`, `ThemeFilesRow` (with `ThemeRows` kept as the wizard's form);
  `UpdatesSection` is gone (`UpdateCheckRow` sits in General).
- The search (`src/settings_nav.rs`) matches every typed word against a block's
  title, its section's name and its keywords, as the page filters do; the page
  shows the found blocks themselves under their sections' names.
- The section shown is saved as `ui.settings_section` and restored
  (`src/preferences.rs`), and is a place in back and forward
  (`Place::Settings` in `src/navigation.rs`).
- `SettingsNav.open` shows Settings on a section from the wizard's agents line and
  the photo picker's refused-key message; the Library check's back returns to
  Library.
- Tests: new `tests/settings.rs`, a back/forward test in `tests/library_check.rs`,
  a save-and-restore check in `tests/preferences.rs`, the invariant
  `every_settings_block_can_be_found` in `tests/ui_invariants.rs`, unit tests in
  `src/settings_nav.rs`; every test that reached Settings rows now opens their
  section first. The screenshots example gained section, search and narrow shots.
- Docs: `docs/ui.md` (new "Settings" section, files, rules, decisions),
  `docs/DESIGN.md`, `docs/manual-testing.md`, `README.md`.
- Checks run: `cargo fmt --all --check` clean; `cargo clippy -p vcmp-app
  --all-targets -- -D warnings` clean; `cargo check -p vcmp-app --all-targets`
  (every test and example compiles); `cargo test -p vcmp-app --lib` 54 passed;
  `--test ui_invariants` 9 passed, and the new invariant fails when a block's
  index is broken. The UI test binaries (`settings`, `ui_behaviour`, `preferences`,
  `library_check`, `agents`, `lastfm`, `labels`, `themes`, `player`,
  `play_history`, `artist_photos`) were not run here: the owner runs the full
  suite. The build directory was cleaned afterwards.

## How to test

**Automated coverage.** `cargo test -p vcmp-app --test settings` (sections,
search, narrow dropdown, the wizard's link), `--test library_check` (back and
forward between sections), `--test preferences` (the section survives a restart),
`--test ui_invariants` (every block has a search entry).

**Setup.** A library of copied files: `VCMP_HOME=target/settings-test cargo run -p
vcmp-app` with `VCMP_SETUP=skip`, add a music folder from Settings, General is
shown first.

**Steps.**
1. Click Settings in the sidebar: the title, a search field and the list General,
   Library, Playback, Appearance, Online services, AI agents at the left; General
   shows Setup and Check for updates.
2. Click Library: Music folders, Update automatically, Library check, Play history
   and Labels show; nothing else.
3. Click Playback, then the mouse's back button: Library shows again; forward:
   Playback.
4. Click Appearance, open About, then Settings again: Appearance shows. Quit and
   start VCMP, open Settings: Appearance still shows.
5. Type `zoom` in the search field: only the Zoom row shows, under "Appearance",
   and its + and - buttons work there. Type `last`: Play history (and the Last.fm
   rows in a build with the Last.fm key). Type `xyz`: "No settings match".
6. Click Playback in the list: the search field empties and Playback shows.
7. Make the window 720 px wide: the list is gone and a "Settings section" dropdown
   sits above the section; choosing Online services in it shows that section.
8. Turn on AI agents, Run setup again, go to Online services, click "Open AI agents
   settings": the wizard closes and Settings shows AI agents with the token.

**Also worth trying.** Library check: Check library... then its back button
returns to Settings' Library section. A fanart.tv build with a wrong personal key:
the photo picker's "Open Online services settings" closes it and shows the key.
Search `token` with agents off: nothing; turn agents on with the search still
typed: the connection appears.

## What is missing

Nothing - the spec is implemented in full.

## Known problems

- The music folders and the labels keep their own headings inside Library (the
  spec said section components lose theirs); without them the labels list read as
  part of the folders. AI agents and Updates lost theirs as the spec says.
- Search matches only row titles, section names and hand-picked keywords; the
  keywords are a list in `SettingsPage` to keep up, English only until translated.
- The narrow dropdown repeats the first section as the `Dropdown`'s fixed first
  entry, so the list is offset by one in code (`index - 1`).
- The screenshots example and the heavier UI tests were type-checked but not run
  in this session (the owner runs them); the focus shots' Tab counts may need
  adjusting after the new tab stops.
- Code comments were kept to match the rest of the repository.

## Recommended follow-ups

- Ctrl+F on the Settings page could put the focus in its search field, as the
  filters do on other pages (left open by the spec).
- Run the screenshot example and review the list, the narrow dropdown and the
  search results with the owner (DESIGN.md "Awaiting the owner's review").

## Changelog

- Settings is split into sections - General, Library, Playback, Appearance, Online
  services and AI agents - listed beside the section shown
- Search Settings: the matching settings from every section show, ready to change
- Settings opens on the section you last used, and back and forward move between
  sections
- Links in the setup and in the artist photo picker open the right Settings section
