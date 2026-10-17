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
| `ui/components.slint` | `FocusRing`, `NavItem`, `SegmentedChoice`, `Toggle`, `SettingRow`, `Button`, `IconButton`, `Dropdown`, `ProgressBar` |
| `ui/icons.slint` | The `Glyph` names, the `IconArt` global and `Icon` |
| `ui/pages.slint` | `AlbumsPage`, `ArtistsPage`, `SettingsPage`, `AboutPage` and their shared page frame |
| `ui/album-grid.slint` | `AlbumGrid` (cards created only for the rows in view; round for artists), `AlbumCard`, and `CardGrid` for a few cards inside a page |
| `ui/artist-page.slint` | `ArtistView`: one artist's picture, albums and appearances |
| `ui/album-page.slint` | `AlbumView`: one album's cover, details and tracks by disc |
| `src/albums.rs` | `CoverGrid`: a card grid's model with background picture loading, used by `AlbumBrowser` (the album grid), the artist grid and an artist's albums |
| `src/library_view.rs` | Connects the library to the album and artist grids and opens album and artist pages |
| `ui/library-state.slint` | `LibraryState` global: music folders, scan progress and results |
| `ui/library-settings.slint` | The Music folders section of Settings |
| `src/library_service.rs` | `LibraryService`: folders, background scans, automatic updates |
| `ui/player-state.slint` | `PlayerState` global: the current track, position, volume, shuffle, repeat |
| `ui/player-bar.slint` | `PlayerBar`, its play button and the `Slider` |
| `ui/queue-panel.slint` | `QueuePanel`: the track playing, Up next and what follows |
| `ui/icons/*.svg` | The control icons, painted in `currentColor` |
| `src/icons.rs` | `IconArt`: writes the tint into an icon's SVG and keeps the result |
| `src/player_service.rs` | `PlayerService`: commands to the playback engine, its events into `PlayerState` |
| `src/saved_queue.rs` | The queue kept across restarts, as versioned JSON in the settings |
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
| Sidebar plus content area | Agreed in the spec; it holds only the pages that exist (Albums, Artists, Settings, About) |
| Artists in round frames, albums in rounded squares | The shape tells the two grids apart at a glance, as people know it from other players |
| An artist's picture is their newest album's cover until there are artist photos | A grid of identical initials said nothing; the cover is theirs and recognisable. Credit-only artists keep the initial |
| Album artists by default, everyone behind a switch | The owner's choice: 391 of 532 artists in their library are only credited on others' albums, and would crowd the grid |
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

Release builds are what these numbers describe. A debug build with unoptimised
dependencies rendered a grid frame in 46 ms and decoded a cover in 58 ms, which made
scrolling stutter; with the dependencies optimised in debug builds too (see
`Cargo.toml`) it is 6.8 ms and 3.7 ms. Today's release run: 0.8 ms p50, 7.9 ms p95.

The album page, measured with `examples/album_page_benchmark.rs` on a real library's
longest album (116 tracks): it opens in 33 ms and renders in 0.7 ms p50 (release),
1.4 ms (debug).

## Album page

Choosing an album shows `AlbumView` in place of the grid. The grid stays alive, so
its scroll position and current card survive.

- **Getting back:** the back button, Escape, or the Albums sidebar entry. Focus
  returns to the album's card.
- **Discs:** a "Disc n" heading appears above each disc only when the album has
  more than one.
- **Track artist:** shown only where it differs from the album artist, as on a
  compilation.

The Albums page fades in through a `FadeCurtain`, created each time the page is
shown, because the page itself stays alive while hidden.

## Artists page

- **The grid** lists album artists by default (138 in the owner's library). The
  "All artists" choice adds everyone credited on a track (532), saved as
  `ui.artists_all`. A card says how many albums an artist has, or how many tracks
  for an artist with no albums of their own. Pictures load in the background like
  album covers (`CoverGrid`).
- **An artist's page** (`ArtistView`) shows their picture, name and counts, Play
  and Shuffle, their albums as a small grid (`CardGrid`, one tab stop with arrow
  keys), and "Appears on": tracks credited to them on other artists' albums or on
  no album, each with its album name.
  - Play and Shuffle play their albums' tracks, then the appearances, as one
    context (`artist:<id>`, named in the queue panel). A click or Enter on an
    appearance plays that list from there; its menu offers Play now, Play next and
    Add to queue.
  - An album chosen there opens on the Artists page, and its back button names the
    artist. Only one album page is open at a time: opening one closes the other
    page's.
  - An artist who leaves the library while their page is open stays on screen,
    marked as gone, until the user leaves.
- **Getting back:** the back buttons, Escape, or the Artists sidebar entry, which
  returns to the grid from anywhere on the page.

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
  - a track's menu (right click, or the Menu key on the current track) offers Play
    now, Play next (front of Up next) and Add to queue (end of Up next);
  - the playing track gets a play mark and an accent title.
- **The queue panel** opens from the queue button in the player bar, and stays open
  or closed across restarts (`ui.queue_open`). In a window at least 1000 px wide
  the page narrows to make room for it; in a narrower one the panel lies over the
  page's right edge, so the page keeps the width its controls need. It shows the track
  playing, Up next (with Clear, and a remove button per track), then "Next from" the
  album being played, at most 100 tracks ahead, as the engine reports them.
  - Clicking a track, or Enter or Space on it, plays it; the tracks before it are
    dropped, as the engine's `SkipTo` does.
  - The tracks are one tab stop: Up/Down and Home/End move, Delete removes from Up
    next, Alt+Up/Alt+Down reorder it, and the Menu key opens the track's menu (Play,
    Remove from queue, Move up, Move down). The remove buttons are not tab stops,
    since Delete does the same.
- **The queue comes back after a restart**, paused at the saved position: context,
  Up next, history, shuffle and repeat. `PlayerService` saves the engine's snapshot
  in the settings (`playback.queue`, `src/saved_queue.rs`) whenever a track starts
  and when the window closes, and restores it at start. A save from another version
  is ignored with a warning.
- **Settings → Playback → Output device** lists "System default" and the devices,
  listed off the UI thread each time Settings opens and when the list opens. Up and
  Down change the choice directly; a click, Enter or Space opens the list. The
  choice is saved by name (`playback.output_device`) and used when the engine
  starts. A chosen device that is unplugged stays in the list, and the description
  names the device playing instead.
- **Settings → Playback** chooses how shuffle picks: "Fewer repeats" (the engine's
  less-repeats mode) or "Fully random". Volume, shuffle, repeat and the output
  device are saved in the library's settings.
- **`PlayerService`** sends `Command`s to the engine. A thread forwards the engine's
  `Event`s to the UI thread. Tests replace the engine with a recorder through the
  `PlayerControl` trait.
- **Problems:** a missing sound output and skipped tracks are shown in the bar.
- **Discord Rich Presence** (`vcmp-integrations::discord`, on by default, switch
  under Settings → Playback):
  - shows "Listening to VCMP" with the track, artist and a progress bar while a track
    plays, and clears on pause or stop;
  - after a seek the progress bar is refreshed;
  - the album cover comes from the Cover Art Archive, which Discord fetches itself.
    Albums tagged with a MusicBrainz release ID use it directly. For the others,
    `vcmp-integrations::musicbrainz` looks the playing album up while Discord is
    running and the presence is on (it sends the album title and artist to
    musicbrainz.org). Each album is looked up once: the answer is kept in the library
    (`cover_lookups`), an album with no match is tried again after 30 days, and a
    failed request after 10 minutes. Albums without a cover show no picture until
    there is an app logo;
  - the client runs on its own thread and reconnects every 15 s while Discord is
    away; it tells `PlayerService` when Discord is reached or lost;
  - the application ID is built in (`APPLICATION_ID`) and can be replaced at build
    time with `VCMP_DISCORD_APPLICATION_ID`.
- **OS media controls** (`vcmp-integrations::media_controls`, always on): MPRIS on
  Linux, System Media Transport Controls on Windows, Now Playing on macOS.
  - Media keys and the system's media widget play, pause, stop, skip, seek, set
    shuffle, repeat and volume, raise the window (un-minimise it) and quit VCMP.
    `PlayerService::media_control` turns each request into what the player bar's
    own control does, so the bar and the saved settings follow.
  - The widget shows the title, artist, album, cover, length, position, volume,
    shuffle, repeat, and playing or paused. `PlayerService` sends the whole state
    on every engine event and volume change; the controls publish only what
    changed.
  - On Linux the bus name is `org.mpris.MediaPlayer2.vcmp`, the identity "VCMP" and
    the desktop entry `vcmp`, so the widget finds the icon once `vcmp.desktop` is
    installed. The cover is the cached thumbnail as a `file://` address.
  - Windows attaches the controls to the window by its `HWND` (Slint's
    `raw-window-handle-06` feature, Windows only), so `src/media_keys.rs` starts
    them from the event loop and waits up to 5 s for the window to exist.
  - The controls run on their own thread on Linux and Windows, so a stalled D-Bus or
    WinRT call never holds up the UI. macOS wants Now Playing touched from the main
    thread, so there they are updated in place.
  - The crate is [`playwire`](https://crates.io/crates/playwire), not `souvlaki`.
    souvlaki's last release (0.8.3, June 2025) still has the macOS cover crash fixed
    only on its main branch, and depends on windows 0.44, cocoa/objc 0.2 and zbus 3
    (or libdbus), all older than what VCMP already builds. playwire uses the
    windows 0.62, objc2 0.6 and zbus 5 already in the tree (no libdbus to install),
    and publishes `DesktopEntry`, shuffle, repeat and real capability flags, which
    souvlaki does not. It is young (first release August 2026) and has one
    maintainer; it was read through before adopting it, and only
    `media_controls.rs` names it, so it can be swapped.

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

- Up next can be filled from album and artist pages only, and reordered by keyboard
  or menu, not by dragging.
- The queue panel appears without an animation; one is planned.
- Artists have no photos of their own yet: choosing a photo, and the web search for
  one (fanart.tv), come with the integrations.
- The track menus are Slint's own, styled by the operating system's colour scheme
  like `AboutSlint` below, so they can look out of place when VCMP's theme differs
  from the OS.
- Tracks whose file path isn't valid Unicode can't be queued: the engine names
  tracks by a string path.
- The OS media controls were tried by hand on Linux (MPRIS) only; Windows and macOS
  are built and tested in CI but not yet seen working.
- Windows takes media widget covers only from the web, so there the widget shows
  the Cover Art Archive cover when one is known and none otherwise.
- The Windows media flyout may name VCMP "Unknown app" until the installer registers
  an AppUserModelID; on Linux the widget's icon needs the installed `vcmp.desktop`.
- Raising the window from the media widget only un-minimises it: Wayland compositors
  don't let an app take focus without an activation token.

- The Slint attribution (`AboutSlint`) picks its logo from the operating system's
  colour scheme, not VCMP's theme, so it sits on the system palette's background. It
  can look out of place when VCMP's theme differs from the OS.
- Theme and reduced-motion choices are saved in the library database's settings
  (`src/preferences.rs`). If the database can't be opened, they last for the session.
- Reduced motion does not yet follow the OS preference. The spec asks for that where
  a platform exposes one; it needs per-platform code.
- `examples/screenshots.rs` cannot force a window size under tiling window managers.
  Layout at the smallest window size is covered by a test instead.
