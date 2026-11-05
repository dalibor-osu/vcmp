# Custom themes

Status: draft
Date: 2026-11-05
Area: vcmp-app (theme, Settings, setup wizard, title bar, labels, screenshots);
all three platforms

## Context of the change

This is workstream 1 ("Themes") of the agreed spec
[2026-10-20-lyrics-themes-spotify.md](2026-10-20-lyrics-themes-spotify.md), filed as
issue 84. That spec already settled the shape of the feature; this spec settles what
it left open and does not reopen the rest.

What exists today:

- `crates/vcmp-app/src/theme.rs` holds two hand-made palettes, `light()` and
  `dark()`, of 10 `ThemePalette` tokens (`scrim` was added after the earlier spec,
  which still says 9). Pink is the single accent, red marks errors. The same file
  holds `LABEL_COLOURS`: 10 pastel hues, each with a dark-theme colour and a light
  one darkened in hue to 3:1.
- The WCAG AA contrast check (`luminance`, `contrast`, `assert_pairs`) lives only in
  that file's test module ([ui.md, Palettes](../ui.md#palettes)).
- `crates/vcmp-app/ui/theme.slint` takes both palettes and picks one through
  `ThemeMode` (System / Light / Dark). `Theme.dark` is read by `components.slint`,
  `labels.slint` and `src/title_bar.rs` (the Windows caption buttons).
- `ThemeRow` in `ui/setting-rows.slint` is a three-way `SegmentedChoice`, used on the
  Settings page and in the setup wizard (`ui/setup.slint`).
- `src/preferences.rs` saves the mode as `ui.theme` (`system`, `light`, `dark`) in
  the library database's settings table.

Inherited from the earlier spec, and taken as decided here:

- Built-in themes plus user theme files in TOML; no in-app editor.
- One theme list replaces the Light / Dark / System switch.
- The OS light/dark setting is followed once, at first start, never after.
- A theme is a name, a kind (light or dark) and every `ThemePalette` token; theme
  files change colours only.
- A theme file that fails contrast still loads; Settings names the failing pairs.
- Saved `light` and `dark` become the built-in themes made from today's palettes, so
  nobody's look changes on upgrade.
- Accents apply to built-in themes only; theme files bring their own.
- The contrast check moves into the app; theme handling stays in `vcmp-app`.

## User request

Brainstorm the feature described in issue 84 ("Custom themes: built-in themes,
accents and theme files", https://github.com/dalibor-osu/vcmp/issues/84). The issue
asks for workstream 1 of the 2026-10-20 spec as that spec decided it: one theme list
of four built-in themes (one light, three dark of different depths) with an accent
picker, then any user theme files. A theme is a name, a kind and every palette token.
Built-in themes stay in Rust next to their contrast tests; user themes are TOML files
in `<data dir>/themes/`, colours only. A token added later gets a default derived
from the others; a file missing one of today's tokens is listed as invalid. The kind
sets Slint's `Palette.color-scheme`. The `Theme` global takes one palette and the
kind from Rust; `ThemeMode`, `light-colors` and `dark-colors` go. `ui.theme` holds a
theme id plus the accent for built-ins, migrated as described above; a user theme
whose file is gone falls back to the built-in of its kind and Settings says so. A set
of built-in accents, today's pink among them, darkened on light themes to 4.5:1; an
accent that can't pass recognisably is left out. The contrast check moves into the
app and drives both the tests and a Settings warning. Out of scope: an in-app editor
or colour picker, fonts, radii or spacing in themes, following the OS after the
first start. Also: DESIGN.md and ui.md updated, tests for migration, file loading,
fallback and the contrast check, screenshots of each flavour, manual-testing.md, and
every new string through `@tr()`.

During the session the user chose: the three dark themes differ in depth and in
temperature; plain descriptive names; the 10 label hues as accents, with hand-picked
light variants, one accent for all built-in themes; theme files read at start and on
a Reload button; "Open themes folder" and "Save as theme file" to start a file;
swatch tiles with accent dots in Settings; built-ins and accent only in the wizard.

## Decisions

### How should the three dark built-in themes differ from each other? (Today's dark palette is one of them.)

| Option | What it means |
|---|---|
| Same hues, darker bases (Recommended) | Today's dark plus a darker and a near-black variant with the same text and accent hues. Feels like one family and is easy to contrast-test, but the three are only subtly different. |
| Lighter, today, darker | A softer, lighter-grey dark theme, today's dark, and a near-black one. Covers more tastes, but the lighter dark is harder to get past 4.5:1 for muted text. |
| Depth and temperature | They also differ in tint (cool blue-grey, neutral grey, warm). More character per theme, but harder to keep matching the pastel identity in DESIGN.md. |
| I'll supply palettes | You give the colours (or a reference) for the two new ones. Exactly your look, but implementation waits for those values. |

**Answer: Depth and temperature**

### How should the four built-in themes be named?

| Option | What it means |
|---|---|
| Plain descriptive (Recommended) | e.g. Light, Dark, Darker, Midnight. Instantly clear in a list, but has no character of its own. |
| Evocative names | e.g. Dawn, Dusk, Night, Void. More brand personality, but the name alone doesn't say light or dark, so the list needs swatches. |
| I'll name them | You pick the four names. Your call, and it can be filled in later without blocking anything. |

**Answer: Plain descriptive**

### Which accent colours should the built-in themes offer?

| Option | What it means |
|---|---|
| The 10 label hues (Recommended) | Pink, mauve, red, peach, yellow, green, teal, sky, blue and lavender, which labels already use with tested light variants. Consistent and cheap, but red clashes with errors and yellow may not pass on light, so some may be dropped. |
| A curated 5-6 | Pink plus a few chosen hues that work as accents (e.g. mauve, blue, teal, green, peach). Every one is a good accent, but it's a second colour list next to the labels'. |
| Labels minus red | The label hues except red, which stays reserved for errors. Keeps the 'red means wrong' rule, but still 9 hues to test on 4 themes. |

**Answer: The 10 label hues**

### How is an accent's light-theme variant (darkened to 4.5:1) produced?

| Option | What it means |
|---|---|
| Hand-picked constants (Recommended) | Each accent stores its light colour, as labels and today's pink do, and a test checks 4.5:1. You judge whether it is still recognisable; more values to maintain. |
| Computed at runtime | Darken in hue until 4.5:1 against the light theme. No constants, but 'recognisable' can't be judged by code, so a muddy result can ship. |

**Answer: Hand-picked constants**

### Is the accent one choice for all built-in themes, or remembered per theme?

| Option | What it means |
|---|---|
| One accent for all (Recommended) | Switching themes keeps your accent. Simplest to understand and store, but you can't have pink on Light and teal on Midnight. |
| Remembered per theme | Each built-in theme keeps its own accent. More personal, but there's more to store and switching themes can change the accent unexpectedly. |

**Answer: One accent for all**

### When does VCMP read the files in `<data dir>/themes/`?

| Option | What it means |
|---|---|
| On start + Reload button (Recommended) | Read at start and when you press Reload in Settings. Predictable and cheap, but you have to press Reload after every edit. |
| Watch the folder | Changes apply live as you save the file. Great for tweaking colours, but needs a file watcher and has to cope with half-written files. |
| Only on start | Simplest, but you have to restart VCMP to see any edit. |

**Answer: On start + Reload button**

### How does someone start writing their own theme file?

| Option | What it means |
|---|---|
| Open folder + copy a theme (Recommended) | Settings has 'Open themes folder' and 'Save as theme file', which writes the current built-in theme (with its accent) as a TOML file to edit. Easy start from a known-good theme; two buttons to add. |
| Open folder + docs | Only an 'Open themes folder' button, and the file format is documented. Least UI, but people start from a blank file. |
| Bundled example file | VCMP writes an example theme into the folder the first time it's opened. No extra button, but it's a file people didn't ask for and may later wonder about. |

**Answer: Open folder + copy a theme**

### How does Settings present the theme list (4 built-ins + any user files) and the accent?

| Option | What it means |
|---|---|
| Swatch tiles + accent dots (Recommended) | A row of small preview tiles (base, surface, text, accent), user themes after the built-ins, and a row of round accent dots under them. You see each theme before picking it, but it's new components and the most layout work. |
| Dropdown + accent dots | A ComboBox of theme names and a row of accent dots. Compact and scales to many files, but you only see a theme by picking it. |
| Radio list with swatches | A vertical list of names, each with a small strip of its colours. Readable and keyboard-friendly, but the Settings page gets long with many files. |

**Answer: Swatch tiles + accent dots**

### What does the setup wizard's theme step offer?

| Option | What it means |
|---|---|
| Built-ins + accent (Recommended) | The same picker as Settings, but only the four built-ins (no file buttons). Brand-new users have no files anyway; it keeps the step simple. |
| Same as Settings | The full row, including theme files and their buttons. One component everywhere, but the buttons are noise on first start. |
| Theme only, no accent | The four built-ins only, and the accent is left to Settings. Shortest step, but a visible choice is hidden at the moment people set up. |

**Answer: Built-ins + accent**

## High-level plan

**1. Theme model and contrast check.** A theme type (id, name, kind, palette) in
`vcmp-app`, the four built-in themes, the 10 accents with their light and dark
constants, and the contrast check as app code with the pair list it checks today.
Today's light and dark palettes stay as they are. The two new dark themes are drawn
up here: one neutral and one warm, at different depths from today's cool dark, in
the pastel family DESIGN.md describes. The tests cover every built-in theme with
every accent, and every label colour on every built-in theme. Depends on nothing.

**2. One palette in the UI.** The `Theme` global takes one palette and the kind;
`ThemeMode` and the two palette properties go. `Theme.dark` stays, now derived from
the kind, so components, labels and the title bar keep working. The kind also sets
`Palette.color-scheme`. Depends on 1.

**3. Saved choice and migration.** `ui.theme` holds the theme id and the accent.
`light`, `dark` and `system` are migrated as the earlier spec decided, and a fresh
install takes Light or today's Dark from the OS once. Depends on 1 and 2.

**4. Theme files.** Reading `<data dir>/themes/*.toml` at start and on Reload,
validation (missing token, unreadable colour, bad kind), the contrast warning, the
fallback when the chosen file is gone, "Open themes folder" and "Save as theme
file". Depends on 1 and 3.

**5. The picker.** Swatch tiles for the themes and accent dots in Settings, with
the file buttons, invalid files and the warning; the wizard's step with the
built-ins and the accent only. Fully keyboard operable, every string in `@tr()`.
Depends on 2 to 4.

**6. Documentation and screenshots.** DESIGN.md records the owner's decisions (the
switch and OS following gone, the new themes, the accent set). ui.md describes the
model, the four palettes, the accents and the theme file format. The `screenshots`
example covers every built-in theme; manual-testing.md gets the steps tests can't
check. Runs alongside 1 to 5.

## Architecture decisions

- **Built-in themes are a palette without an accent, plus one accent.** The accent
  sets `accent` and `on-accent`; every other token belongs to the theme. Reason: one
  accent for all built-in themes means the accent is not part of a theme's identity,
  and the saved value can name the two separately.
- **Each accent has two hand-picked constants: one for the light theme, one shared
  by the three dark themes.** An accent ships only if its constants pass every
  accent pair (4.5:1 on base, mantle and surface, and `on-accent` on it) on every
  built-in theme of that kind. One that can't pass recognisably is left out, as the
  earlier spec decided. Reason: the owner's eye judges "recognisable", the tests
  judge "passes", and three dark variants per accent would triple the constants for
  little visible gain. Where a dark constant fails on one dark theme only, the
  implementation reports it to the owner rather than adding a per-theme variant.
- **The accents reuse the label hues but not the label constants.** A label's light
  colour reaches 3:1 on surface; an accent needs 4.5:1 for text, so several light
  accents are darker than their label. The list of hue names is shared with
  `vcmp_library::LABEL_PALETTE` and a test keeps them in step.
- **Today's light and dark palettes keep their exact values.** They become the
  Light theme and the cool Dark theme, and pink keeps today's two constants.
  Reason: nobody's look changes on upgrade.
- **The kind picks the label colours.** Light themes use the light label set, dark
  themes the dark one, user themes those of their kind. The label tests run on every
  built-in theme, so a deeper or warmer dark theme can't make a label dot vanish.
- **`ui.theme` records the kind of a file theme, not only its id.** Without it the
  "falls back to the built-in of its kind" rule can't work once the file is gone.
  The accent is kept while a file theme is chosen, so going back to a built-in
  restores it.
- **A file theme's id is `file:` plus the file name without `.toml`.** The name
  shown is the `name` inside the file. Two files may carry the same name; the tiles
  still tell them apart by their colours, and the id stays unique.
- **Theme file format:** `name`, `kind` (`light` or `dark`) and one entry per
  `ThemePalette` token by its Slint name, as `#rrggbb`, or `#rrggbbaa` for `scrim`.
  A file missing a token VCMP had when the file format was introduced is invalid; a
  token added later is derived from the others when missing. Unknown keys are
  ignored, so a newer file still loads in an older VCMP. "Save as theme file" writes
  this format, so the format is defined once.
- **Files are read on start and on Reload, never watched.** A Reload that finds the
  chosen file gone or invalid falls back as on start. Reason: the owner's choice;
  no watcher and no half-written files to cope with.
- **The themes folder follows `VCMP_HOME`:** `<VCMP_HOME>/themes/` when it is set,
  the platform data directory otherwise, next to `library.sqlite3`. Reason: tests
  and trial libraries stay apart from the real one.
- **The contrast warning checks the same pairs as the tests.** One list of pairs,
  thresholds and names in app code, used by both. Label dots are not part of the
  warning, since a user theme brings no label colours of its own.

## Weaknesses and risks

| Risk | Likelihood | Cost | What reduces it |
|---|---|---|---|
| A warm or neutral dark theme drifts away from the pastel identity in DESIGN.md | Medium | Medium: the owner rejects the palettes and they are redone | Present the two palettes to the owner as screenshots before the picker is built; record the approval in DESIGN.md |
| Red as an accent reads as an error, since red means "wrong" today | Medium | Low-medium: confusing marks for whoever picks it | Errors are always written in words, which the owner kept (2026-10-18); DESIGN.md notes that red as accent is the user's choice. Revisit if it confuses |
| Yellow, peach or other pale hues can't reach 4.5:1 on Light and stay recognisable | High for yellow | Low | Left out on purpose, as the earlier spec decided; the picker shows only accents that ship |
| A pastel accent fails on one dark theme only (e.g. on the lightest dark base) | Medium | Low | Reported to the owner during implementation; options are a darker base for that theme or dropping the accent |
| Swatch tiles get crowded with many theme files | Low | Low | Tiles wrap into rows; most people have a handful of files at most |
| A theme file with a broken `scrim` (opaque) hides the page behind a dialog | Low | Low: the user wrote it | The warning can't catch it (no text is on the scrim); ui.md says what scrim is for |
| New dependencies: a TOML reader, and opening a folder in the OS file manager | High | Low: build time, three platforms to check | Pick small, maintained crates; check "Open themes folder" on all three platforms in manual testing |
| The migration from `system` reads the OS scheme too early at start, before Slint knows it | Medium | Low: a light-OS user starts dark once | Test the first-start pick on each platform in manual testing |

**Questions deliberately left open**

- The four themes' final names. Plain descriptive names, settled with the owner
  when the two new palettes are shown (e.g. Light, Dark, Darker, Midnight).
- Which accents finally ship on which kind, once the constants are tested.
- Whether the accent dots are hidden or shown disabled while a file theme is
  chosen. Decided when the picker is laid out, with the owner.

## Out of scope

- An in-app theme editor or colour picker.
- Themes that change fonts, radii, spacing or icons.
- Following the OS light/dark setting after the first start.
- Watching the themes folder for changes.
- Accents on theme files, and label colours defined by theme files.
- Sharing or downloading themes from anywhere online.
