# UI foundation

How VCMP's Slint UI is put together and why. Visual direction comes from
[DESIGN.md](DESIGN.md); this file records how the code applies it. Keep it in sync
with `crates/vcmp-app/ui/`.

Design Read: a desktop music player shell for music enthusiasts, in a calm Catppuccin
pastel language with one subtle neon accent, dial ENERGY 1 / RHYTHM 2 / MOTION 2 (the
dials come from DESIGN.md).

## Files

| File | Holds |
|---|---|
| `ui/theme.slint` | `ThemePalette` struct, `ThemeMode` enum, `Theme` global: colours, spacing, radii, type sizes, target height, content width, durations |
| `ui/components.slint` | `FocusRing`, `NavItem`, `SegmentedChoice`, `Toggle`, `SettingRow` |
| `ui/pages.slint` | `AlbumsPage`, `SettingsPage`, `AboutPage` and their shared page frame |
| `ui/album-grid.slint` | `AlbumGrid` and its cards, created only for the rows in view |
| `ui/album-page.slint` | `AlbumView`: one album's cover, details and tracks by disc |
| `src/albums.rs` | `AlbumBrowser`: the grid's model and background cover loading |
| `src/library_view.rs` | Connects the library to the grid and opens album pages |
| `ui/library-state.slint` | `LibraryState` global: music folders, scan progress and results |
| `ui/library-settings.slint` | The Music folders section of Settings |
| `src/library_service.rs` | `LibraryService`: folders, background scans, automatic updates |
| `ui/player-state.slint` | `PlayerState` global: the current track, position, volume, shuffle, repeat |
| `ui/player-bar.slint` | `PlayerBar`, its `IconButton`s, the `Slider`, `Icon` and the `Glyph` names |
| `ui/icons/*.svg` | The media control icons, painted in `currentColor` |
| `src/icons.rs` | `IconArt`: writes the tint into an icon's SVG and keeps the result |
| `src/player_service.rs` | `PlayerService`: commands to the playback engine, its events into `PlayerState` |
| `ui/app-window.slint` | `AppWindow`: sidebar, current-page marker, content area |
| `src/theme.rs` | The light and dark palettes, plus their contrast tests |

## Rules enforced by tests

| Rule | Test |
|---|---|
| No colour literals in `.slint` files; every colour comes from `Theme.colors` | `tests/ui_invariants.rs` |
| Durations only in `theme.slint`, so reduced motion turns all motion off in one place | `tests/ui_invariants.rs` |
| Every user-visible string goes through `@tr()` (exceptions: "VCMP", "Sora", the slint.dev URL) | `tests/ui_invariants.rs` |
| Every text pairing reaches 4.5:1 and every control boundary 3:1, in both palettes | `src/theme.rs` |
| Clicking, keyboard-only use, theme choice and reduced motion behave as designed; nothing leaves a 720 px wide window | `tests/ui_behaviour.rs` |

## Palettes

Catppuccin Latte (light) and Mocha (dark), with Pink as the single accent.

| Token | Light | Dark | Use |
|---|---|---|---|
| base | `#eff1f5` | `#1e1e2e` | Content background |
| mantle | `#e6e9ef` | `#181825` | Sidebar |
| surface | `#ccd0da` | `#313244` | Hover, selected item, control track |
| outline | `#7c7f93` (overlay2) | `#6c7086` (overlay0) | Control boundaries |
| text | `#4c4f69` | `#cdd6f4` | Text |
| text-muted | `#5c5f77` (subtext1) | `#a6adc8` (subtext0) | Descriptions, only on base or mantle |
| accent | `#a1197d` | `#f5c2e7` (pink) | Current page, selected option, switched-on state |
| on-accent | `#eff1f5` | `#11111b` | Text and marks on accent |
| glow | `#ea76cb` at 69% | `#f5c2e7` at 69% | Neon glow around the page marker |

Latte's own pink (`#ea76cb`) reaches only 2.3:1 on Latte base, so the light accent is
the same hue darkened to 6.3:1. The original pink survives as the light glow. Latte's
subtext0 and overlay0 fail contrast on its backgrounds, so the light theme uses
subtext1 and overlay2 instead.

## Design decisions

| Decision | Reason |
|---|---|
| Palettes live in Rust, not in Slint | One source for the UI and the contrast tests; future custom themes only swap values |
| Pink marks what is active (page, option, switch) | Gives the neon accent a job, so it stays an accent rather than decoration |
| Glow only on the current-page marker | DESIGN.md asks for subtle neon; one glowing element per screen keeps it subtle |
| Sora for everything, two weights (400, 600) | The owner's choice; two weights carry hierarchy without a second family at ENERGY 1 |
| Type scale 12 / 14 / 16 / 20 / 26 px | Small steps suit a calm UI; the page title is the one large element per screen |
| Spacing scale 4 / 8 / 12 / 16 / 24 / 40 px | Bigger gaps separate sections, small ones group a label with its description |
| Radii 2 / 8 / 12 px, pill only for the switch | The switch track is the one shape where a pill carries meaning |
| Content column capped at 720 px | Keeps a setting's label and its control within one glance on wide windows |
| Sidebar plus content area | Agreed in the spec; it holds only the pages that exist (Albums, Settings, About) |
| Chevron on the album page's back button | Shows the direction of the action; it's the one arrow in the UI |
| Accent-filled button only for "Add music folder…" on the empty Albums page | The single action that empty screen exists for; every other button is outlined |
| Scan progress at the bottom of the sidebar | Albums appear during a long first scan; progress stays visible on any page |
| Accent on the play button, the playback progress and the playing track | All three mark what is playing now; the volume fill is muted because it is a setting |
| Track numbers and durations muted, titles in text colour | The title is what people scan for; numbers support it |
| Light and dark plus "System", defaulting to System | No brand reason to force a theme; following the OS is least surprising |
| Motion: marker slide, page fade, control transitions (120 / 220 ms) | MOTION 2: transitions show where the selection went; nothing loops |

## Album grid

The grid creates cards only for the rows in view plus one, places them by index,
and reuses them while scrolling. It reports the range in view through
`visible-range-changed`. `AlbumBrowser` responds:

- It queues covers for that range first, then one screen below, then one above.
- Background threads decode each cached 400 px thumbnail and shrink it to the tile's
  pixel size.
- Covers more than three screens away are released, so memory stays flat however
  long the library is.

Covers are uploaded as RGBA. With RGB images the GPU renderer converts every cover
on every frame, which doubled the grid's render time.

Measured with `examples/grid_benchmark.rs`: 5,000 synthetic albums, a 1256x1388
window, Radeon RX 7800 XT (Mesa), femtovg renderer, scrolling top to bottom.

| | RGB covers | RGBA covers |
|---|---|---|
| Render time p50 / p95 / max | 10.8 / 12.8 / 21.9 ms | 5.1 / 7.7 / 11.4 ms |
| Frames missing two refreshes | 0 | 0 |
| Covers on screen after opening | 0.1 s | 0.1 s |
| Covers kept in memory at the end | 165 | 165 |
| Process memory after scrolling | 234 MiB | 250 MiB |

Without text labels a frame took 6.2 ms (with RGB covers), so text is about 4.5 ms
of the RGB frame. Rounded clipping of the covers made no measurable difference.

```bash
cargo run --release -p vcmp-app --example grid_benchmark -- 5000 120
```

## Album page

Choosing an album shows `AlbumView` in place of the grid. The grid stays alive, so
its scroll position and current card survive.

- **Getting back:** the back button, Escape, or the Albums sidebar entry. Focus
  returns to the album's card.
- **Discs:** a "Disc n" heading appears above each disc only when the album has
  more than one.
- **Track artist:** shown only where it differs from the album artist, as on a
  compilation.
- **No play controls yet.** Playback isn't merged into this branch, and a control
  without behaviour would be a dead control.

The Albums page fades in through a `FadeCurtain`, created each time the page is
shown, because the page itself stays alive while hidden.

## Music folders and scanning

`LibraryState` is the UI's view of the folders and of scanning. `LibraryService`
keeps it current and implements its callbacks.

- **Folders:**
  - **Adding** opens the system folder picker (`rfd`: the desktop portal on Linux,
    native dialogs on Windows and macOS), then scans.
  - **Changing "Include subfolders"** rescans.
  - **Removing** needs no confirmation: tracks become unavailable, and re-adding the
    folder brings them back with playlists intact.
  - **Locked during a scan:** the subfolder switch and Remove can't be used while a
    scan runs, since the change would race it.
- **Scans:**
  - **Where they run:** a background thread with its own database connection.
  - **Progress:** reported to the UI thread at most every 100 ms.
  - **Albums appear as found:** the grid refreshes every 3 s while tags are read.
  - **One at a time:** a scan requested during another runs once it is done.
- **Automatic updates** (on by default): a scan at startup, plus a watcher that
  triggers one 2 s after the folders stop changing.
- **Where the status shows:**
  - Settings shows the phase and count while scanning, then the number of tracks.
  - Unreadable files can be expanded into a list with the reason for each.
  - While a scan runs, the bottom of the sidebar shows a progress bar on every page.
- **Errors** reach the UI as a kind plus the system's detail. Slint words the
  sentence, so it can be translated.

The Albums page tells apart why it is empty:
- the library could not be opened;
- a scan is running;
- there are no folders yet, with the one primary button in the UI, "Add music
  folder…";
- the folders hold no tagged albums.

## Playback

- **The player bar** runs along the bottom of the window:
  - the current track with its cover;
  - shuffle, previous, play/pause, next and repeat (off, all, one);
  - the position with a seek slider, and volume;
  - the speaker icon mutes and unmutes, coming back to the earlier volume. Moving the
    volume slider while muted unmutes at the new value.
- **Seeking** moves the bar to the new position at once. Positions the engine sent
  before it took the seek are ignored for up to a second, so the bar doesn't jump
  back.
- **On the album page:**
  - Play starts the album; Shuffle turns shuffle on and starts it;
  - clicking a track, or pressing Enter on it, plays the album from there;
  - the track list is one tab stop (Up/Down, Home/End, Enter or Space);
  - the playing track gets a play mark and an accent title.
- **Settings → Playback** chooses how shuffle picks: "Fewer repeats" (the engine's
  less-repeats mode) or "Fully random". Volume, shuffle and repeat are saved in the
  library's settings.
- **`PlayerService`** sends `Command`s to the engine. A thread forwards the engine's
  `Event`s to the UI thread. Tests replace the engine with a recorder through the
  `PlayerControl` trait.
- **Problems:** a missing sound output and skipped tracks are shown in the bar.
- **Discord Rich Presence** (`vcmp-integrations::discord`, on by default, switch
  under Settings → Playback):
  - shows "Listening to VCMP" with the track, artist and a progress bar while a track
    plays, and clears on pause or stop;
  - after a seek the progress bar is refreshed;
  - albums tagged with a MusicBrainz release ID show their Cover Art Archive cover,
    which Discord fetches itself; other albums show no picture until there is an app
    logo;
  - the client runs on its own thread and reconnects every 15 s while Discord is
    away;
  - the application ID is built in (`APPLICATION_ID`) and can be replaced at build
    time with `VCMP_DISCORD_APPLICATION_ID`.

The icons are simple shapes drawn on a 24-point grid (`ui/icons/*.svg`), not taken
from an icon set. Media controls are the one place in the UI where shapes read
faster than words. Every icon button still has a spoken label.

The colour is written into each SVG in Rust (`IconArt.paint`), and Slint draws the
result as a plain image, rasterized on the CPU (resvg) at the size it is shown. Both
ways of tinting inside Slint misdrew small glyphs with the GPU renderer:

- a filled `Path` loses pixels at the corner where its outline starts, which left a
  speck in the play and pause buttons; moving the start point only moved the flaw;
- `Image.colorize` paints the image into a texture before drawing it, and on the
  owner's GPU that step cut a block out of the next button. It did not happen on
  other machines, so it could not be caught by screenshot tests.

## Accessibility patterns

- Every control is a `FocusScope` with Space or Enter (arrows in `SegmentedChoice`)
  and an accent `FocusRing` drawn 3 px outside it. Mouse clicks do not move focus
  (`focus-on-click: false`).
- Rings show only in keyboard mode (`Theme.keyboard-navigation`), like
  `:focus-visible` on the web. It switches on when a key is pressed (the window
  sees every key first) or a control gains focus through Tab. It switches off when
  a control is clicked. So focus can move for keyboard users, as when an album
  opens, without leaving a ring in front of mouse users.
- The album grid is a single tab stop. Arrow keys, Home/End and Page Up/Down move
  the current card and scroll it into view; Enter or Space opens it.
- Controls set `accessible-role`, `accessible-label`, the checked or selected state,
  and `accessible-action-default`. Tests find elements through these properties.
- Interactive targets are at least 40 px tall (`Theme.target-height`).
- State is never shown by colour alone: the selected option is bold and filled, and
  the switch knob changes side.

## Known limits

- There is no queue panel yet, the queue isn't restored after a restart, and the
  output device can't be chosen. Those are the next playback step.
- Tracks whose file path isn't valid Unicode can't be queued: the engine names
  tracks by a string path.

- The Slint attribution (`AboutSlint`) picks its logo from the operating system's
  colour scheme, not VCMP's theme, so it sits on the system palette's background. It
  can look out of place when VCMP's theme differs from the OS.
- Theme and reduced-motion choices are saved in the library database's settings
  (`src/preferences.rs`). If the database can't be opened, they last for the session.
- Reduced motion does not yet follow the OS preference. The spec asks for that where
  a platform exposes one; it needs per-platform code.
- `examples/screenshots.rs` cannot force a window size under tiling window managers.
  Layout at the smallest window size is covered by a test instead.
