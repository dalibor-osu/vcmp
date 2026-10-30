# UI foundation

How VCMP's Slint UI is put together and why. Visual direction comes from
[DESIGN.md](DESIGN.md); this file records how the code applies it. Keep it in sync
with `crates/vcmp-app/ui/`.

Design Read: a desktop music player shell for music enthusiasts, in a calm Catppuccin
pastel language with one pink accent and no glow (the owner dropped the neon on
2026-10-18), dial ENERGY 1 / RHYTHM 2 / MOTION 2 (the dials come from DESIGN.md).

## Files

| File | Holds |
|---|---|
| `ui/theme.slint` | `ThemePalette` struct, `ThemeMode` and `ReduceMotion` enums, `Theme` global: colours, spacing, radii, type sizes, target height, content, column and dialog widths, the detail pages' cover size, the zoom, durations, drag scrolling |
| `ui/components.slint` | `FocusRing`, `EdgeScroll`, `NavItem`, `PlaylistLink`, `SegmentedChoice`, `Toggle`, `SettingRow`, `Button`, `IconButton`, `Dropdown`, `SearchField`, `TextField`, `ProgressBar`, `Modal` (the dialog frame of the tag editor and the pickers) |
| `ui/icons.slint` | The `Glyph` names, the `IconArt` global and `Icon` |
| `ui/pages.slint` | `SearchPage`, `AlbumsPage`, `ArtistsPage`, `PlaylistsPage`, `SongsPage`, `FoldersPage`, `SettingsPage`, `AboutPage` and their shared page frame |
| `ui/album-grid.slint` | `AlbumGrid` (cards created only for the rows in view; round for artists, track counts for playlists), `AlbumCard`, and `CardGrid` for a few cards inside a page |
| `ui/playlist-page.slint` | `PlaylistView`: one playlist's picture, name, actions and tracks |
| `ui/playlist-state.slint` | `Playlists` global: the playlist names every Add to playlist menu lists |
| `src/playlists.rs` | `PlaylistBrowser`: the playlist grid and page, the sidebar list, and the edits behind them |
| `ui/artist-page.slint` | `ArtistView`: one artist's picture, albums and appearances |
| `ui/songs-page.slint` | `SongTable`: every track with sortable columns, only the rows in view created |
| `ui/folders-page.slint` | `FolderBrowser`: the path back up, the folders and the files of one folder |
| `src/folders.rs` | `FolderBrowser`: the folder tree built from the library's tracks |
| `ui/album-page.slint` | `AlbumView`: one album's cover, details and tracks by disc; `TrackList`, the track list every page shares, and its `TrackSelection` global |
| `ui/selection.slint` | `SelectionChange`: how a track list's selection changes |
| `ui/columns.slint` | `ColumnWidths` global (the saved column shares) and `ColumnEdge`, the resizable edge between two column headings |
| `src/selection.rs` | Applies selection changes to a list's rows, and picks the rows a menu acts on |
| `ui/cover-picker.slint` | `CoverSearch` global and `CoverPicker`: "Find cover online" and the covers to pick from |
| `src/cover_fetch.rs` | `CoverFetch`: runs the cover search, stores the picked cover and shows it everywhere |
| `ui/photo-picker.slint` | `PhotoSearch` global and `PhotoPicker`: an artist's photo, found online or from a file |
| `src/artist_photos.rs` | `ArtistPhotos`: runs the photo search, keeps the chosen photo and shows it on the page and the cards |
| `ui/tag-editor.slint` | `TagEditorState` global and `TagEditorView`: one track's or an album's tags and cover, and the list of changes before writing several files |
| `src/tag_editor.rs` | `TagEditor`: fills the editor, works out what changed, lists it before writing several files, writes it in the background and refreshes the views |
| `src/albums.rs` | `CoverGrid`: a card grid's model with background picture loading, used by `AlbumBrowser` (the album grid), the artist and playlist grids, an artist's albums and search |
| `src/library_view.rs` | Connects the library to the album and artist grids, search and the songs table, and opens album and artist pages; owns the folder and playlist browsers |
| `ui/library-state.slint` | `LibraryState` global: music folders, scan progress and results |
| `ui/library-settings.slint` | The Music folders section of Settings |
| `src/library_service.rs` | `LibraryService`: folders, background scans, automatic updates |
| `ui/player-state.slint` | `PlayerState` global: the current track, position, volume, shuffle, repeat |
| `ui/player-bar.slint` | `PlayerBar`, its play button and the `Slider` |
| `ui/now-playing.slint` | `NowPlayingView`: the playing track's large cover and details over the pages |
| `ui/queue-panel.slint` | `QueuePanel`: the track playing, Up next and what follows |
| `ui/icons/*.svg` | The control icons, painted in `currentColor` |
| `src/icons.rs` | `IconArt`: writes the tint into an icon's SVG and keeps the result |
| `src/player_service.rs` | `PlayerService`: commands to the playback engine, its events into `PlayerState` |
| `src/saved_queue.rs` | The queue kept across restarts, as versioned JSON in the settings |
| `ui/lastfm-state.slint` | `LastfmState` global: whether the build has Last.fm, the account, connecting, and what went wrong |
| `ui/lastfm-settings.slint` | `LastfmRow`: the Last.fm setting and its Connect, Cancel or Disconnect button |
| `src/lastfm_service.rs` | `LastfmService`: the Last.fm row, the account, and plays into the scrobble queue |
| `ui/app-window.slint` | `AppWindow`: sidebar, current-page marker, content area |
| `ui/updates.slint` | `UpdateState` global, `UpdateLink`, the sidebar's update note and the Updates section of Settings |
| `ui/agents.slint` | `AgentState` global, the AI agents section of Settings (switch, connection, backups), `ChangeHistory` (the dialog listing the changes by agents), the sidebar's review note and `ChangeReview`, the dialog that reviews a change, with `ChangeTable` for each file's changes |
| `src/agents.rs` | `AgentService`: the switch and the MCP server (`vcmp-mcp`), the history, the review (accepting writes in the background like the tag editor), undo, the backups, and the upkeep at start and daily; `AppChanges` brings agents' requests from the server's thread |
| `src/updates.rs` | The update check's switch, saved in the settings, and the newer release it found |
| `ui/labels.slint` | `Labels` global, `LabelEntry`, `LabelDot`, `LabelChip(s)`, `LabelFilter` (the Songs and Albums pages' label filter), `LabelEditor` (the dialog that makes or edits a label, with the palette and the custom colour picker) and the Labels section of Settings |
| `ui/label-page.slint` | `LabelsPage`: the Labels section's list, and `LabelView`, one label's tracks with Play and Shuffle |
| `src/labels.rs` | `LabelBrowser`: labels behind the `Labels` global: the menus' choices and toggles, the dialog, the filters, the label page, the chips rows show, the colour picker's images |
| `src/theme.rs` | The light and dark palettes and the labels' colours, plus their contrast tests |
| `src/zoom.rs` | The zoom: its steps, and the window's scale factor as the system's times the zoom |

## Rules enforced by tests

| Rule | Test |
|---|---|
| No colour literals in `.slint` files; every colour comes from `Theme.colors` | `tests/ui_invariants.rs` |
| Durations only in `theme.slint`, so reduced motion turns all motion off in one place | `tests/ui_invariants.rs` |
| Every user-visible string goes through `@tr()` (exceptions: "VCMP", "Sora", the slint.dev URL) | `tests/ui_invariants.rs` |
| Every text pairing reaches 4.5:1 and every control boundary 3:1, in both palettes | `src/theme.rs` |
| Every wrapped text sets `min-height: self.preferred-height` (or its height outright). Slint 1.18.1 measures an element an `if` creates at its single-line width, so a wrapped text there lost its last lines ([slint-ui/slint#13655](https://github.com/slint-ui/slint/issues/13655), fixed after 1.18.1); a text's own preferred height follows its real width. Drop the rule once Slint is past the fix | `tests/ui_invariants.rs` |
| Clicking, keyboard-only use, theme choice and reduced motion (the user's choice against the system's) behave as designed; nothing leaves a 720 px wide window | `tests/ui_behaviour.rs` |

## Palettes

Catppuccin Latte (light) and Mocha (dark), with Pink as the single accent and Red for
errors.

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
| error | `#d20f39` (red) | `#f38ba8` (red) | What is wrong, as text on base only |
| scrim | `#4c4f69` at 45 % (text) | `#11111b` at 70 % (crust) | Dims the sidebar and pages behind a dialog |

Latte's own pink (`#ea76cb`) reaches only 2.3:1 on Latte base, so the light accent is
the same hue darkened to 6.3:1. Latte's subtext0 and overlay0 fail contrast on its
backgrounds, so the light theme uses subtext1 and overlay2 instead. Even subtext1 is
4.05:1 on Latte's surface, so a row filled for hover or selection turns its
secondary text to `text`. Latte's red is 4.80:1 on base but 4.46:1 on mantle, so
errors are written on base only. Nothing is drawn on the scrim; a dialog's card is
base, with a surface border, so the dialog's text keeps base's contrast.

## Design decisions

| Decision | Reason |
|---|---|
| Palettes live in Rust, not in Slint | One source for the UI and the contrast tests; future custom themes only swap values |
| Pink marks what is active (page, option, switch) | Gives the accent a job, so it stays an accent rather than decoration |
| No glow | The owner dropped the neon (2026-10-18). The page marker's glow, the one neon touch, barely showed in the light theme |
| Red only for errors, with words | The owner's choice (2026-10-18): pink means active, so a wrong value marked in pink read as selected. The red label is bold and the sentence under the form names the field, so the colour never carries the message alone |
| Sora for everything, two weights (400, 600) | The owner's choice; two weights carry hierarchy without a second family at ENERGY 1 |
| Type scale 12 / 14 / 16 / 20 / 26 px, and 34 px for the track in the now playing view (42 px when the view is wider than 1800 px) and for the title of an album, artist or playlist page | Small steps suit a calm UI; the page title is the one large element per screen, and the now playing view's and a detail page's titles take its place there, a step larger next to the large cover. A detail page's artist line is 20 px and its counts 14 px, so the header reads at a glance beside its cover (the owner's request, 2026-10-28) |
| Now playing covers the pages, not the player bar | The bar already holds every control, so the view needs none of its own and the controls stay where people know them |
| Titles, artists and albums of the playing track are links, underlined on hover | People expect to go from what plays to where it comes from; the underline marks a link without adding colour |
| Spacing scale 4 / 8 / 12 / 16 / 24 / 40 px | Bigger gaps separate sections, small ones group a label with its description |
| Radii 2 / 8 / 12 px; round (half the element's size) for icon-only buttons, the play button, progress and slider tracks, the slider knob and drop lines; the switch is a pill | Circles and round-ended tracks are what those controls are; everything else uses the three radii. `tests/ui_invariants.rs` refuses a radius written in bare pixels |
| Content column capped at 720 px, except track lists and the songs table | Keeps a setting's label and its control within one glance on wide windows. Track lists span the page: the owner found half a wide window left empty beside them unfinished (2026-10-18) |
| Settings and About in a column in the middle of the page, 60 % of its width, from 720 to 960 px (`Theme.column-max-width`) | The owner's request (2026-10-20): the column at the left of a wide window, with the right side blank, looked weird. 960 px still keeps a setting's label and its control within reach; About shares the frame, so the two pages next to each other in the sidebar look alike |
| Sidebar plus content area | Agreed in the spec; it holds only the pages that exist (Search, Albums, Artists, Playlists, Songs, Folders, Settings, About), then the playlists by name |
| The sidebar's sections in three groups: Search; Albums to Folders; Settings and About | The owner's request (2026-10-18): Search and the app's own pages are not views of the library. A gap with a faint line sets the groups apart, the order stays; the page marker adds the gaps above its section |
| A sidebar section always opens on its grid | The owner's request (2026-10-18): the album, artist or playlist last open is not where someone choosing Albums means to go |
| A click selects a track; the play button in the number's place, a double-click or Enter plays | The owner's request (2026-10-18), as in Spotify and MusicBee: a stray click no longer starts music, and the button keeps playing one click away |
| Column widths as shares, dragged at the edge between headings | The owner's request (2026-10-18). Shares fit any window width; one setting serves every track list, another the songs table |
| Folder browser built from the library, files listed by file name | Instant, and it shows the music VCMP knows; the file name comes first because the browser is where badly tagged files are found |
| Songs table without a track-number column, missing values sorted last | Numbers only mean something within an album, which the Album column already orders; untagged files at the top would hide the library behind them |
| Search first in the sidebar, the app opening on Albums | Search is the way into every other page, as in most players; Albums is still what people come to see |
| Artists in round frames, albums in rounded squares | The shape tells the two grids apart at a glance, as people know it from other players |
| An artist's picture is their photo, or without one their newest album's cover | A grid of identical initials said nothing; the cover is theirs and recognisable. Credit-only artists without a photo keep the initial |
| Album artists by default, everyone behind a switch | The owner's choice: 391 of 532 artists in their library are only credited on others' albums, and would crowd the grid |
| Chevron on the album page's back button | Shows the direction of the action; it's the one arrow in the UI |
| Accent-filled button only for "Add music folder…" on the empty library pages (Albums, Artists, Songs, Folders) | The single action those empty screens exist for; every other button is outlined. Not yet true: Play, the tag editor's save buttons and the review's Write button are filled too, which audit 001 (finding 5) leaves open with the accent's jobs (finding 3) |
| Scan progress at the bottom of the sidebar | Albums appear during a long first scan; progress stays visible on any page |
| The sidebar's sections and playlists scroll as one, above its foot | Eight sections and any number of playlists don't fit a short window. Scrolling them together keeps one scroll in the sidebar, and the scan status and update note stay in view below. The keyboard scrolls the focused entry into view; the page marker scrolls with its section |
| A newer VCMP is a muted link at the foot of the sidebar, and a link on About | The sidebar is on screen on every page, so the note is seen without opening anything; small, muted and in a corner, it doesn't compete with the navigation or interrupt listening. A dialog or toast would interrupt, a badge would be a second accent, and a note only in Settings or About would go unseen. A running scan takes its place, as the scan's progress matters now and the note waits |
| Accent on the play button, the playback progress and the playing track | All three mark what is playing now; the volume fill is muted because it is a setting |
| Track numbers and durations muted, titles in text colour | The title is what people scan for; numbers support it |
| Light and dark plus "System", defaulting to System | No brand reason to force a theme; following the OS is least surprising |
| Motion: marker slide, page fade, control transitions (120 / 220 ms) | MOTION 2: transitions show where the selection went; nothing loops |
| Reduce motion is System, On or Off, defaulting to System | Built like the theme choice. People who need less motion set it once in the OS for every app; On and Off still decide for VCMP alone, either way |
| An album's and an artist's other actions in a More menu ("…") beside Play and Shuffle | The owner's request (2026-10-28): the buttons under the cover (Add to playlist…, Edit tags…, the photo's) were distracting and seldom used. The playlist page already worked this way, so the three detail pages now share it. The album's menu holds Find cover online (or Find another cover and Remove cover for a cover fetched online), Add to playlist and Edit tags…; the artist's Find photo online, Choose photo… and Remove photo |
| The cover or picture of an album, artist or playlist page as large as the page allows, from 128 to 280 px (`Theme.detail-cover-size`) | The owner's request (2026-10-28): the room the buttons under it took goes to the cover. It shrinks first in a narrow window, so the title and the Play, Shuffle and More buttons keep about 240 px beside it down to a 720 px window |
| The cover picker is a dialog in the middle of the window, over the dimmed sidebar and pages | The owner's request (2026-10-20): a form or a few covers at the top left of a full-window overlay left most of a wide window empty. One dialog pattern (`Modal`) for the tag editor and both pickers; the page stays in sight behind the scrim, and the player bar stays usable. The pickers' card is 960 px wide (`Theme.dialog-wide-width`), room for five covers in a row, and as tall as the window allows from the start, so it doesn't grow as covers arrive |
| The artist photo picker is the cover picker's twin | The same pattern as an album's "Find cover online", so one way of working covers both. The fanart.tv credit stays under the photo, as its terms ask |
| The tag editor is the same kind of dialog, not a separate window | One dialog pattern again, 720 px wide (`Theme.dialog-width`) and as tall as the form. A second window would need its own focus, theme and close handling on three platforms, for a form that is used now and then |
| A dialog keeps the keyboard: Tab and Shift+Tab go round its controls | The page behind is in sight but not in use. A focus that moved to it would be hidden behind the scrim, and Escape would no longer reach the dialog |
| A click beside a picker closes it; beside the tag editor it does nothing | A picker holds nothing to lose. The tag editor holds typed values, so only Cancel and Escape close it |
| The tag editor's buttons stay at the foot of the dialog while the form scrolls | Save and Cancel are in view in a short window however far the form is scrolled, and so is the sentence saying why nothing was written |
| An album's differing values show as empty fields saying "Several values" | Writing only the fields the user changed is the safe default for a batch: a field left alone can't flatten twelve track numbers into one |
| A save to several files first lists its changes in the form's place, with the focus on Back | The spec asks for no bulk tag writes without a preview. The list shows exactly what goes into every file, and starts at the top even in a scrolled editor. Back has the focus, as Cancel does before deleting a playlist, so an Enter pressed twice goes back instead of rewriting an album |
| A click on a cover picks it, with no separate confirm button | The picker exists for this one choice; Cancel and Escape are the way out |
| Last.fm connects in the browser while the row waits, with Cancel | Last.fm's desktop sign-in happens on last.fm; asking every few seconds spares an "I've allowed it" button |
| One Last.fm button whose text follows the state (Connect…, Cancel, Disconnect) | The focus stays on it while the state changes under it |
| AI agents are a section of Settings, off by default | The owner's decision (MCP spec, 2026-10-19): the server runs only while VCMP is open and the switch is on. Its connection (address, token, port, the Claude Code command and the Claude Desktop entry for `vcmp mcp`) shows only while it is on. The Claude Desktop entry carries the path of the VCMP that runs, and its `VCMP_HOME` when set, so the relay reads the same library |
| Values to copy are read-only fields with a Copy button | The token and command are too long to read out; a field shows they are text, selects like text, and Copy needs no clipboard library (`TextField.copy-all`). A long value scrolls inside its field, which clips it |
| A waiting change is a note at the foot of the sidebar, above the update note | The owner chose a notice that doesn't interrupt (spec, "Notice, review when ready"). The sidebar is on every page, like the update note; the note is in text colour, not muted, because it asks for a decision. A dialog that opens by itself would interrupt listening and typing |
| The review is a dialog like the tag editor's, focus on Decide later | The same `Modal` pattern. Three answers: write, reject, or leave it waiting; the focus starts on the one that changes nothing, so an Enter pressed by accident writes nothing. A click beside it does nothing, and Escape decides later |
| The review lists the changes by file: what every file gets first, once ("In all 12 files"), then each file with changes of its own under its title and file name, by path | The owner's request (2026-10-28): rows of values with counts were hard to follow back to the songs they touch. A change to a whole album still lists its shared values once, so the spec's risk (an owner who accepts what is too long to read) stays answered |
| The review's values in two columns, Before and After, of equal width beside the field's name | The columns line up from row to row and file to file, so the eye runs down one of them. A screen reader hears each value with its column ("After: 1979") |
| A new cover in the review is the picture itself, 64 px | The owner decides on what the cover looks like; "a new cover" in words gave nothing to decide on |
| Before in muted text, after in text colour | The new value is what the owner decides on; the old one is context |
| Writing tags is the filled button; deleting a playlist is not | Written tags can be undone for 30 days, so filling the button that writes them doesn't push a risk. A deleted playlist can't come back, so its button stays outlined, under the warning |
| Deleting a playlist warns that it can't be undone, whoever asks | The owner's decision (2026-10-20): the playlist page's own Delete question says it too |
| The changes by agents list newest first, with Review or Undo where they apply, in a dialog of their own opened from Settings (Show changes…) | The decision is about the latest change; older ones are history. An undo is itself a change, so it can be undone (it writes the agent's tags again). The owner's request (2026-10-28): a busy month of changes cluttered Settings, so the page keeps one row that counts them. A review opened from the dialog takes its place and gives it back when it closes; a click beside the dialog closes it, as it holds nothing to lose |
| Zoom in Settings, from 80 to 200 %, with Ctrl+= and Ctrl+- (Ctrl+0 for 100 %) | The owner's request (2026-10-28): on a large 2K monitor VCMP looked small. It scales the whole window on top of the system's own scale, so every size in `theme.slint` grows together and the layout rules still hold. Buttons on either side of the value step it, as zooming goes a step at a time; the shortcuts are the ones browsers use |
| Small marks after the playing track's artist: lossless (a waveform) and Last.fm (signal arcs, with a tick once the play counts) | The owner's request (2026-10-28). Muted and 14 px, so they inform without competing with the title; the scrobbled mark turns to text colour and changes shape, so the state never rests on colour. Their name shows above them under the pointer and is what a screen reader hears. The Last.fm mark shows only while an account is connected, and only for plays Last.fm would count (an artist, and longer than 30 s) |
| The wheel over the volume slider changes the volume, 5 % a notch; over the position slider it scrolls the page | The owner's request (2026-10-28). Turning the wheel to set the volume is what people expect of a player; seeking by wheel would happen by accident while scrolling. The volume is saved once the wheel rests (`Theme.wheel-commit-delay`), not on every step |
| On Windows no system title bar: VCMP's own minimise, maximise and close buttons in the top right corner (`ui/window-frame.slint`, `src/title_bar.rs`), 46 × 32 px as Windows' own. The window drags by a 16 px strip along its top and by the sidebar down to its first section. The frame's border is `surface` and Windows' dark frame follows the theme | The owner's choice (issue #55, 2026-10-30): Windows painted its bar in the accent colour, a pink strip over the dark theme, on a row of its own. The window keeps Windows' styles and only gives the title bar's room to the app, so Windows still snaps, maximises on a double-click of a drag area, opens the window menu on Alt+Space or a right-click, and resizes at the edges. The maximise button answers Windows as its own (`HTMAXBUTTON`), so Windows 11 shows Snap layouts over it. Top right is where Windows users look for the buttons, and the page headers start 40 px down, so the 32 px buttons cover nothing. The strip is 16 px because detail pages, dialogs and the now playing view put controls from 16 or 24 px down; the now playing view's menu moves below the buttons. **The placement, sizes and hover are awaiting the owner's review** (DESIGN.md) |

## Album grid

The grid creates cards only for the rows in view plus one, places them by index,
and reuses them while scrolling. It reports the range in view through
`visible-range-changed`. `AlbumBrowser` responds:

- It queues covers for that range first, then one screen below, then one above.
- Background threads decode each cached 400 px thumbnail and shrink it to the tile's
  pixel size.
- Covers more than three screens away are released, so memory stays flat however
  long the library is.

A card under the pointer sits on a `surface` fill reaching into the gap around it,
as hovered rows do, and its muted second line turns to `text` on it.

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

- **Unknown albums:** the library puts tracks without an album tag into an
  "Unknown" album of their artist, and those naming no artist into one Unknown
  album by "Unknown artist", last in the grid (see `docs/library.md`). Their card
  and page say "Unknown" (and "Unknown artist"), words of the UI rather than tags,
  with a question mark in place of a cover, and offer no "Find cover online".
  Editing the album tag of their tracks moves them to a real album, and the page
  follows.

- **Getting back:** the back button, Escape, or the Albums sidebar entry. Focus
  returns to the album's card. The sidebar entry opens the grid from any section,
  not the album last open (the same for Artists and Playlists).
- **A renamed album:** when its title or album artist changes (the tag editor, or
  another program and the watcher), the page follows its tracks to the album most
  of them now make up (`LibraryView::reload`). It says the album is gone only when
  its tracks are.
- **Discs:** a "Disc n" heading appears above each disc only when the album has
  more than one.
- **Track artist:** shown only where it differs from the album artist, as on a
  compilation.
- **The More menu** ("More for …", beside Play and Shuffle) holds the album's other
  actions: Find cover online, Add to playlist (with New playlist and every
  playlist) and Edit tags…. It opens below the button, from a click or Enter.
- **Find cover online:** an album without artwork has this item in its More menu.
  Nothing is sent anywhere before it is chosen. An album showing a cover fetched
  this way has "Find another cover" (the same picker) and "Remove cover" there
  instead; removing goes back to no cover and lets the automatic Discord lookup run
  again.
  - It opens the cover picker (`CoverPicker`), a dialog over the sidebar and pages, and
    `CoverFetch` asks `vcmp-integrations::musicbrainz::CoverPicker` for covers. That
    sends the album title and artist to musicbrainz.org, at one request a second
    (shared with the Discord lookup), and fetches 250 px previews from the Cover
    Art Archive. A release the files are tagged with comes first, then the matching
    releases, their release groups when a release has no art, and the matching
    release groups: at most 8 covers, each image once, from at most 12 image
    requests.
  - Covers appear as they arrive, each with its release title, year and country
    (a worldwide release names none). The picker's sentence says whether it is
    still looking, found nothing, could not reach the services (with Search
    again), or broke off after finding some.
  - Choosing a cover (a click, or Enter on the current one) downloads it at up to
    1200 px and stores it as the album's artwork (`Library::set_fetched_cover`, see
    [docs/library.md](library.md#covers-fetched-online)). The picker closes, and
    the album page, the grids and, while the album plays, the player bar, the now
    playing view, the media widget and the queue panel show it at once. Discord
    shows the picked cover too. A failed download keeps the picker open with a
    sentence, so another cover can be picked.
  - Keyboard: the focus starts on Cancel and moves to the first cover when it
    arrives; the covers are one tab stop (`CardGrid`: arrows, Home/End, Enter).
    Tab and Shift+Tab stay in the picker. Escape, Cancel or a click beside the
    dialog closes the picker and stops the search, and the focus returns to the
    album page's back button. Both pages that show albums hold an album page, so
    only the one shown takes the focus back.

The Albums page fades in through a `FadeCurtain`, created each time the page is
shown, because the page itself stays alive while hidden. The Albums, Artists and
Playlists pages hold one more curtain per view they switch between (the grid, an
album, an artist, a playlist), so opening a page or going back to the grid fades
the new view in too.

## Artists page

- **The grid** lists album artists by default (138 in the owner's library). The
  "All artists" choice adds everyone credited on a track (532), saved as
  `ui.artists_all`. A card says how many albums an artist has, or how many tracks
  for an artist with no albums of their own. Its picture is the artist's photo, or
  without one the cover of their newest album; pictures load in the background like
  album covers (`CoverGrid`).
- **An artist's page** (`ArtistView`) shows their picture, name and counts, Play,
  Shuffle and the More menu (the photo's actions), their albums as a small grid (`CardGrid`, one tab stop with arrow
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
- **The artist's photo:** in the More menu, "Find photo online", "Choose photo…"
  and, once there is one, "Remove photo".
  - "Find photo online" opens a picker, a dialog like the cover picker. The artist is looked up on MusicBrainz (by the ID the files are tagged
    with, then by name; only sure, same-name matches), and their photos come from
    fanart.tv, the best liked first, up to 8. Each shows the name MusicBrainz has
    and its disambiguation ("UK rock band"), so two artists of the same name can be
    told apart. A click or Enter picks one, which is downloaded in full; Escape,
    Cancel or a click beside the dialog closes. Nothing is sent until the button is used, and the button is
    there only in builds with VCMP's fanart.tv project key (`VCMP_FANARTTV_API_KEY`).
    When fanart.tv refuses a key, the picker says so apart from an unreachable
    service, and names the personal key when one was sent.
  - **The user's own key:** Settings → Artist photos takes a fanart.tv personal
    API key (only in builds with the project key). It is optional: fanart.tv's
    terms ask apps to let users give their own, which goes with VCMP's key on each
    request (`client_key`) and gets them new photos sooner. Enter or Save keeps it
    (trimmed), Escape puts the kept one back, Remove forgets it. It is kept in the
    library's settings (`integrations.fanart_personal_key`) and never written to
    the log: fanart.tv's error messages have both keys scrubbed.
  - **Credit:** a photo found on fanart.tv says "Photo from fanart.tv" under it, a
    link to the site; a file of the owner's says nothing. About names fanart.tv,
    the Cover Art Archive and MusicBrainz as the sources of what VCMP finds online,
    with links, as fanart.tv's terms ask.
  - "Choose photo…" takes a JPEG, PNG or WebP file. One that can't be read says so
    under the picture.
  - The photo is kept by the library (see `docs/library.md`) and shows at once on
    the page, the artist cards and search.
- **Getting back:** the back buttons, Escape, or the Artists sidebar entry, which
  returns to the grid from anywhere on the page.

## Playlists page

- **In the sidebar:** Playlists comes after Artists. Below the sections, the
  playlists are listed by name, and each opens its page; the open one is
  highlighted.
- **The grid** shows each playlist's picture, name and number of tracks. New
  playlist makes "Playlist n" and opens it with the name field focused and
  selected, ready to be typed over. Without playlists the page says how to make one.
- **A playlist's page** (`PlaylistView`) shows its picture, name, number of tracks,
  minutes of the tracks that can play and how many files are not found, then
  Play, Shuffle and a More button (a right click on it does the same) with
  Rename, Choose picture…, Use album covers (after choosing a picture) and Delete
  playlist….
  - **Rename** turns the name into a field: Enter or Save keeps it, Escape or Cancel
    leaves it. An empty name keeps the old one.
  - **Delete** asks first, on the page, with the focus on Cancel; Escape cancels.
    The tracks stay in the library.
  - **Choose picture** opens the system's file dialog for images. A file that can't
    be read as a picture leaves the picture as it was and says so on the page.
- **Entries** are numbered by position. One whose file is gone is dimmed and says
  "File not found" where the artist would be. Play, Shuffle and playing an entry
  (its play button, a double-click or Enter) play the entries that are there, as
  one context (`playlist:<id>`, named in the queue panel); playing a missing entry
  plays from the next one that is there.
- **Editing:** a playlist's `TrackList` adds Move up, Move down and Remove from
  playlist to its track menu, and Alt+Up, Alt+Down and Delete to its keys. A row
  can also be dragged: past 6 px of movement the press becomes a drag, an accent
  line shows where the row would land, and letting go moves it without playing it.
  The keyboard's current track follows a moved one.
  - Held near the top or bottom of the page (within 60 px, or a third of a short
    view), a drag scrolls the page, faster the closer the pointer is to the edge
    and fastest at or past it (about 750 px a second). `EdgeScroll`
    (`components.slint`) does this on a timer (`Theme.drag-scroll-tick`), since
    the pointer may be held still. The place the row would land is worked out from
    the pointer's window position each time the list moves under it. Reduced
    motion leaves it on: the pointer drives it, it is not an animation.
- **Adding:** every track menu (album, artist, search, folder and playlist lists,
  the songs table and the queue panel) has Add to playlist, with New playlist and
  every playlist by name. New playlist there makes "Playlist n" with the track,
  without leaving the page. The now playing view has the same in its "…" menu, for
  the track playing.
- **A whole album:** Add to playlist in the album page's More menu adds every
  track of the album; New playlist there names the playlist after the album.
- **Getting back:** the back button, Escape, or the Playlists sidebar entry. A
  playlist opened from search goes back to Search.

## Tag editor

- **Opening it:** "Edit tags…" in every track menu (album, artist, search, folder and
  playlist lists, and the songs table) edits that track, or every selected track
  (see [Selecting tracks](#selecting-tracks)). "Edit tags…" in an album page's More
  menu edits every track of the album at once. The heading names the album
  for tracks of one album, or counts the tracks of several.
- **The form:** the cover, with Choose image… (JPEG or PNG) and Remove cover, then
  Artist, Album, Album artist, Disc, Year and Genre, plus Title and Track for one
  track. The focus starts in the first field. The form fills a dialog in the middle
  of the window; in a short window it scrolls, and the buttons stay at the foot.
  An album without an album tag is "the Unknown album" in the subtitle, as its
  page calls it "Unknown".
- **What gets written:** only the fields the user changed. An emptied field removes
  its tag. For an album, a field whose value differs between tracks starts empty
  with "Several values" as its placeholder, and keeps each track's own value unless
  something is typed in it. The cover works the same way ("Several covers").
- **Saving:** Save or Enter in a field. A wrong value (a track number that isn't a
  whole number, a year outside 1 to 9999, a line break) writes nothing, turns the
  field's label bold in the error red, and says above the buttons, in red, which
  field is wrong. Escape or Cancel closes without writing. Saving with
  nothing changed closes, writing nothing.
- **Several files ask first:** when a save would write more than one file, the
  editor lists the changes in place of the form before anything is written (the
  `confirming` state, filled by `TagEditor::save`).
  - It says how many files, then each changed field under its form label, in form
    order, with the new value: "Removed" (muted) for an emptied field, and the new
    cover as a thumbnail. Fields not listed keep each file's own value.
  - "Write N files" writes. Back or Escape returns to the form with the typed values,
    the focus where the save came from (the field Enter was pressed in, or Save) and
    the scroll position it had. The focus starts on Back, so a second Enter goes back
    rather than writing.
  - One track still writes at once, and so does "Try again" for a single file. The
    list holds what this save changes, not an edit that waits for playback from an
    earlier save. A cover chosen while the list shows (the file chooser was still
    open) brings the form back.
- **Writing** runs on its own library connection in the background, through the
  library's verified copy-and-replace (see `docs/library.md`), with a progress bar.
  When every file was written the editor closes, and the library pages, the queue
  and the player bar show the new tags. Files that kept their old tags are listed
  with the reason; "Try again" writes only those.
- **The file the player has open** (`Event::Decoding`: the playing or paused track,
  or, in the last two seconds of a track, the one that follows) can't be replaced on
  Windows (`OPEN_FILES_LOCKED`). Its edit waits; the rest of an album is written at
  once.
  - The editor stays open in the `waiting` status: it names the file, says the new
    tags are written when playback stops or the next track starts, and offers only
    Close. Closing leaves the edit waiting.
  - When the engine reports that it let go of the file, the edit is written in the
    background. An editor still showing that wait closes once it is written, or lists
    the file if it failed, and "Try again" builds on the failed edit.
  - Opened again on a waiting track, the editor shows the waiting tags and the note,
    and saving adds the new changes to the waiting edit.
  - If the engine opened the file again before it could be replaced, the edit waits
    once more. Any other failure after the editor was closed is said in the player
    bar ("The new tags could not be written to …") while the next track plays.
  - When VCMP closes, `main` drops the player (which stops the engine and closes the
    file), then writes the edits still waiting.
  - On Linux and macOS the replace works on an open file, and the engine reads on
    from the old one, so every edit is written at once. Tests turn the waiting on
    with `TagEditor::set_open_files_locked`.

## Selecting tracks

Every track list (album, artist, search, folder and playlist pages) and the songs
table can select several tracks for the track menu.

- **Mouse:** a click selects that track alone; Ctrl+click (Cmd+click on macOS) adds
  or takes away a track; Shift+click selects the tracks from the last one clicked.
  Playing is separate: under the pointer the track number turns into a play button
  (a play cell starts each row of the songs table), and a click on it, or a
  double-click anywhere on the row, plays from that track. With Ctrl or Shift held,
  a click on the button selects instead. The playing track keeps its play mark in
  the accent. Assistive technology's default action on a row plays it.
- **Keyboard:** Shift with Up/Down, Home/End (and Page Up/Down in the songs table)
  selects as the current track moves; Ctrl+Space adds or takes away the current
  track; Ctrl+A selects all; Escape clears, and only a second Escape goes back. The
  window's Space shortcut leaves Ctrl+Space to the lists.
- **The menu** acts on every selected track when it opens on one of them, and says
  how many ("3 tracks selected"): Play next and Add to queue put them into Up next in
  list order, Add to playlist adds them all, Labels puts a label on them all or takes
  it off (see [Labels](#labels)), Edit tags… edits them together, and in a
  playlist Remove from playlist (or Delete) removes them. Play now and Move up/down
  are for one track and hide. A right click on a track outside the selection clears
  it first, so the menu acts on that track alone.
- **Look:** a selected row has the hover fill and a 3 px accent bar at its edge, so
  it reads apart from the row under the pointer. Selected rows next to each other
  read as one block: only its outer corners are rounded and the bar runs through
  it (each row knows whether its neighbours are selected; a disc heading between
  two rows parts them). A selected or hovered row's secondary text turns full
  strength, because muted text on that fill would fall under 4.5:1 in the light
  theme. Assistive technology hears each row as selectable and selected.
- **Where it lives:** each row model holds `selected`. Slint can't loop over rows,
  so `src/selection.rs` applies a toggle, range, all or none and returns the count;
  a new list (another album, a new sort or filter) starts with nothing selected. The
  queue and remove handlers take the list's model and the menu's row and act on the
  selected rows when that row is one of them; Add to playlist and Edit tags go
  through the `TrackSelection` and `SongSelection` globals by file path.

## Table columns

- **Resizing:** the edge between two column headings (`ColumnEdge`,
  `ui/columns.slint`) moves room from one column to the other: dragged, or with
  Left and Right while it has the focus (a fiftieth of the two columns' room per
  press). A double-click shares the room evenly. Each column keeps at least 64 px.
  The edge is a faint line that turns strong while pointed at, dragged or focused,
  with the resize cursor; assistive technology hears a slider named after the
  column before it, with its share in percent.
- **Widths** are shares of the room the resizable columns have together
  (`ColumnWidths`), so they fit any window. The text columns have no minimum width
  of their own (`min-width: 0`), so a column's width follows its share exactly and
  the rows line up under the headings. The drag is measured in window coordinates,
  as the edge moves with it.
- **One setting per kind of table:** every track list shares the title and the
  column beside it; the songs table has title, artist and album. They are saved in
  the library's settings when a drag or key press ends (`ui.track_columns`,
  `ui.song_columns`, numbers separated by spaces) and read at start
  (`src/preferences.rs`); anything else saved there is ignored.

## Search

- **The Search page** is first in the sidebar. Ctrl+F (Cmd+F on macOS) puts the
  focus in the filter of the page shown when it has one (the Albums and Artists
  grids, the Songs table), and opens Search with its field focused from anywhere
  else. The window's shortcut scope holds the focus at start, so the shortcut works
  before anything was clicked.
- **The field** (`SearchField`) searches 150 ms after typing pauses
  (`SEARCH_DELAY`): a one-letter search takes about 50 ms on a few thousand tracks.
  Escape or the clear button empties it.
- **Results:** up to 12 artists, 12 albums and 12 playlists as cards, and up to 50
  tracks with their artist and album. Playlists are found by their names. A search finds any part of a name, so Japanese titles
  without spaces are found by a word inside them (see `docs/library.md`). Without
  a search the page says what it finds; when nothing matches, it says so.
- **Opening a result:** an artist, album or playlist opens on its own page, and
  its back button (or Escape) returns to Search, where the results are still
  shown. Opening something from those pages' own grids goes back to the grid as
  before.
- **Tracks:** playing one (its play button, a double-click or Enter) plays the
  found tracks from there (the queue panel says "Next from your search"); the track
  menu offers Play now, Play next and Add to queue.
- **`TrackList`** is the track list on album, artist, folder and playlist pages and
  search: one tab stop, Up/Down, Home/End, Enter, and the Menu key or a right click
  for the track menu (Play now, Play next, Add to queue, Add to playlist). It spans
  the page, under a row of headings: #, the title (File on the folders page), the
  column beside it when the list has one (Artist on a compilation's album page,
  Album under "Appears on", Artist and album elsewhere, Title and artist for files)
  and Time, with a small gap between the line under them and the first row. See
  [Table columns](#table-columns).

## Back and forward

`src/navigation.rs` keeps a history like a browser's: the mouse's back and forward
buttons (caught in the winit hook, `src/shortcuts.rs`) and Alt+Left and Alt+Right
(the window's shortcut scope, not while typing) move through it.

- **A place** is a section on its grid, or the album, artist (or an artist's
  album) or playlist page open in it.
- **Remembering:** the window calls `navigated` whenever the section or the page
  open in it changes, however that came about: the sidebar, a card, a back button,
  Escape, a link in the player bar. Moving somewhere new drops what was ahead, as a
  browser does; up to 100 places are kept behind.
- **Going back or forward** shows the place through the same openers a click uses
  (`show-section`, `open_album_by_id`, ...). The `navigated` calls that follow find
  the window where the history already is. A page that is gone meanwhile (an album
  removed) leaves the window where it ended up.

## Mouse wheel

A free-spinning or high-resolution wheel (Logitech's, for one) sends hundreds of
wheel events a second; Linux reports eight per notch for them. Slint handles each
one on its own, finding what is under the pointer and starting a scroll animation,
so a fast spin queued more work than a second held, above all in debug builds: the
window froze, then scrolled the whole way at once.

The winit hook hands mouse-wheel events (line deltas) to `WheelScroller`
(`src/wheel.rs`) instead. It adds them up, and a 16 ms timer scrolls a third of what
is left at the pointer, at least 2 px, so a flick glides to a stop in about a
quarter of a second; one scroll event a frame, however fast the wheel spins. With
reduced motion the whole distance goes at once. A touchpad's pixel deltas, with
their gesture phases, still go to Slint as they come, for its kinetic flicks.

## Page filters

The Albums, Artists and Songs pages each have a filter field beside their title
(`SearchField` in its compact form). A row stays when every typed word appears in
one of its names, ignoring case: an album's title or artist, an artist's name, a
song's title, artist or album. Albums and artists filter as you type; the songs
table, which can hold tens of thousands of rows, once typing pauses for 150 ms.
The page says when nothing matches, and Escape or the clear button empties the
field. Unlike Search, a filter only narrows the page it is on.

While labels exist, the Songs and Albums pages also have a Labels button before the
field (see [Labels](#labels)).

## Songs page

- **The table** (`SongTable`) lists every available track: Title, Artist, Album,
  Year and Time. Like the album grid, it only creates the rows in view (a test
  checks that 5,000 songs create fewer than 60 rows).
- **Sorting:** a header sorts by its column; the same header again reverses the
  order. Ties keep album and track order, and missing artists, albums and years
  go last. The order is saved (`ui.songs_sort`), and starts by artist.
- **Loading:** the tracks are read the first time the page is shown, not at
  startup, and again after library changes once they were loaded.
- **Keyboard:** the headers are buttons; the rows are one tab stop with Up/Down,
  Page Up/Down, Home/End and Enter. The Menu key or a right click opens
  the track menu. Rows are selected as in every track list (see
  [Selecting tracks](#selecting-tracks)).
- **Playing:** a row's play button, a double-click or Enter plays every song in the
  table's order from that one; the queue panel says "Next from all songs".
- **Columns:** the edges between the Title, Artist and Album headings resize those
  columns (see [Table columns](#table-columns)); Year and Time keep their widths.
  While labels exist, a Labels column of 140 px before Year shows a track's first
  label and how many more it has.

## Folders page

- **The top level** lists the music folders with how many tracks are under each;
  one that is not there now (an unplugged drive) says "Not connected".
- **Inside a folder:** its subfolders that hold tracks, by name with their track
  counts, then its files by file name, each with its title and artist after it.
  The tree comes from the library's tracks (`src/folders.rs`), read the first time
  the page is shown and again after library changes; a folder that lost all its
  files gives way to the nearest one above that still has some.
- **Getting around:** the path above the lists ("Music folders › Hudba › …"), whose
  steps are buttons; Backspace or Alt+Up on the page goes up one folder. The
  folders are one tab stop (arrows, Enter opens), the files another (`TrackList`).
- **Playing:** a file plays its folder's files from that one; the queue panel names
  the folder.

## Now playing view

- **Opening:** the now playing button in the player bar, next to the queue
  button, opens and closes it. The view (`NowPlayingView`) lies over the sidebar
  and pages and fades in and out (the window animates its opacity, and keeps the
  view until it has faded); the player bar stays below it with the controls, and
  the queue panel can be open beside it. Escape, the close button (top left) or a
  click on the background returns; the focus starts on the close button so Escape
  works at once.
- **Leading on:** as in the player bar, the title, the album and the cover lead to
  the album's page, and the artist to the artist's page; the view closes on the
  way. The next track is a button that skips to it.
- **Content:** the cover as large as the view allows (up to 880 px, within the
  1200 px it is decoded at), then the
  title, artist and album, and the next track as the queue stands. Beside each
  other in a wide window, stacked and centred in a narrow one. With the queue
  panel open in a wide window, the view narrows to leave it room.
- **The large cover** is the full-size artwork, decoded off the UI thread at up to
  1200 px when the view opens (and for each new track while it is open). The bar's
  small cover stands in until then; a decode that arrives after the track changed
  is dropped.
- Lyrics would go here later.

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
  - the current track with its cover: the title and the cover lead to its album's
    page, the artist to the artist's page (links, underlined on hover, and tab
    stops; the cover is for the pointer, the title the keyboard's way);
  - shuffle, previous, play/pause, next and repeat (off, all, one);
  - the position with a seek slider, and volume;
  - the speaker icon mutes and unmutes, coming back to the earlier volume. Moving the
    volume slider while muted unmutes at the new value. The mouse wheel over the
    volume slider changes it, 5 % a notch (a third of that each frame of a
    `WheelScroller` glide, which adds up the same), and the volume is saved once
    the wheel has rested for 400 ms;
  - after the artist, a mark for a lossless file (FLAC or WAV, from the library's
    `tracks.format`) and, while a Last.fm account is connected, a mark for a play
    Last.fm will count, which changes to a tick once it has (`PlayerState.scrobble`,
    set from the same `lastfm::Play` that hands the scrobble over). Pointing at a
    mark shows its name above it;
  - the sleep timer, a moon next to the queue button. Its menu offers "End of this
    track" and 15, 30, 45, 60 and 90 minutes, plus "Turn off sleep timer" while one
    is on. While it is on the moon is in the accent; while it counts down, the minutes
    left sit in the moon's hollow, and the button's label says them. When the time is
    up, the track playing then finishes and playback stops with the next track
    waiting (the engine's stop after the current track, [playback.md](playback.md#queue)).
    A timer that runs out while nothing plays just ends, and Stop, or playback
    stopping by itself, ends a running one. It counts in whole minutes on a Slint
    timer and isn't saved.
- **Width at 720 px** (at 100 % zoom): the bar's three parts have fixed minimums: 136 px for the
  track, 240 px for the controls and 256 px for the buttons and volume. With its
  padding and gaps that is exactly 720 px; another button on the right needs room
  taken elsewhere (`smallest_window_keeps_everything_inside` checks it).
- **Seeking** moves the bar to the new position at once. Positions the engine sent
  before it took the seek are ignored for up to a second, so the bar doesn't jump
  back.
- **On the album page:**
  - Play starts the album; Shuffle turns shuffle on and starts it;
  - a track's play button (in its number's place under the pointer), a
    double-click, or Enter on it plays the album from there; a click selects it;
  - the track list is one tab stop (Up/Down, Home/End, Enter);
  - a track's menu (right click, or the Menu key on the current track) offers Play
    now, Play next (front of Up next) and Add to queue (end of Up next);
  - the playing track gets a play mark and an accent title.
- **Space** plays and pauses everywhere except while typing in a text field, so
  controls are activated with Enter. Slint routes keys only to the focused element
  and drops the focus when that element is removed or hidden (the now playing
  view's close button, an album page's back button), after which keys reach
  nothing. So the app catches Space from the native window events before Slint
  routes them (`src/shortcuts.rs`, through Slint's winit hook), and the window's
  own key handler does the same where the hook cannot reach, as in tests. Whether
  a text field has the focus comes from Slint's `TextInputInterface`.
- **A click takes the focus off** whatever has it first (the same hook), so focus
  rings go away and Space works again after clicking anywhere. A click on a text
  field gives it the focus straight back, as fields take the focus when pressed;
  the app's other controls never take focus on a click.
- **The queue panel** slides in from the window's edge and back out when it closes
  (at once with reduced motion): the window animates how far out it is
  (`queue-out`) and keeps the panel until it is out. It opens from the queue button in the player bar, and stays open
  or closed across restarts (`ui.queue_open`). In a window at least 1000 px wide
  the page narrows to make room for it. In a narrower one the panel covers the
  pages (and the now playing view) from the sidebar to the window's edge, so it
  never hides half a page's controls; the sidebar stays. Once it has slid in, the
  pages under it are hidden, so the keyboard can't reach controls nobody can see,
  and Escape closes it.
  - Each track shows its album's cover, decoded off the UI thread at 96 px and
    kept per album (up to 500), and filled into the rows when it arrives.
  - Tracks added to Up next can also be dragged to another place, like a
    playlist's rows, and the list scrolls the same way while a drag is held near
    its edge.
  - The track menu offers what applies: Play, then Remove, Move up and Move down
    for tracks added to Up next, or Play next and Add to queue for the album's (or
    list's) own tracks, which stay in their place too. The engine can't take a
    track out of the album being played. It shows the track
  playing, Up next (with Clear, and a remove button per track), then "Next from" the
  album being played, at most 100 tracks ahead, as the engine reports them.
  - Clicking a track, or Enter on it, plays it; the tracks before it are
    dropped, as the engine's `SkipTo` does.
  - The tracks are one tab stop: Up/Down and Home/End move, Delete removes from Up
    next, Alt+Up/Alt+Down reorder it, and the Menu key opens the track's menu. The
    remove buttons are not tab stops, since Delete does the same.
- **The queue comes back after a restart**, paused at the saved position: context,
  Up next, history, shuffle and repeat. `PlayerService` saves the engine's snapshot
  in the settings (`playback.queue`, `src/saved_queue.rs`) whenever a track starts
  and when the window closes, and restores it at start. A save from another version
  is ignored with a warning.
- **Settings → Playback → Output device** lists "System default" and the devices,
  listed off the UI thread each time Settings opens and when the list opens. Up and
  Down change the choice directly; a click or Enter opens the list. The
  choice is saved by name (`playback.output_device`) and used when the engine
  starts. A chosen device that is unplugged stays in the list, and the description
  names the device playing instead. On Linux with a sound server, the engine goes
  back to the chosen device when it returns; elsewhere it waits for another choice
  (see [docs/playback.md](playback.md#linux-pulseaudio-then-alsa)).
- **Settings → Playback** chooses how shuffle picks: "Fewer repeats" (the engine's
  less-repeats mode) or "Fully random". Volume, shuffle, repeat and the output
  device are saved in the library's settings.
- **`PlayerService`** sends `Command`s to the engine. A thread forwards the engine's
  `Event`s to the UI thread. Tests replace the engine with a recorder through the
  `PlayerControl` trait.
- **Problems:** a missing sound output, skipped tracks and tags that waited for the
  player but could not be written are shown in the bar.
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
    failed request after 10 minutes. A cover picked with "Find cover online"
    replaces the lookup's answer. Albums without a cover show no picture until
    there is an app logo;
  - the client runs on its own thread and reconnects every 15 s while Discord is
    away; it tells `PlayerService` when Discord is reached or lost;
  - the application ID is built in (`APPLICATION_ID`) and can be replaced at build
    time with `VCMP_DISCORD_APPLICATION_ID`.
- **Last.fm scrobbling** (`vcmp-integrations::lastfm`, under Settings → Playback,
  only in builds made with VCMP's API key):
  - Connect… asks Last.fm for a token, opens Last.fm's page where the user allows
    VCMP, and asks every 3 s whether they have. Meanwhile the row says what to do
    and offers Cancel and "Open the Last.fm page again"; it names the error when
    the browser doesn't open. Connecting gives up after 15 minutes.
  - The session key goes into the OS keyring (service `io.github.dalibor-osu.vcmp`,
    entry `lastfm:<user>`), only the user name into the settings (`lastfm.user`).
    At start the scrobbler reads the key back; when it is missing or unreadable,
    the row asks to connect again.
  - A play counts by Last.fm's rules: the track is longer than 30 s and has played
    for half its length or 4 minutes, whichever comes first. `lastfm::Play` adds up
    the steps between the engine's positions while playing, so pauses and the part
    a seek jumps over add nothing (a step back, or of more than 2 s, is a jump). A
    repeated track is a new play; going back within a play does not scrobble it
    twice. Tracks without an artist tag are not scrobbled.
  - Counted plays wait in the library's `scrobble_queue` and go out in batches of
    up to 50, so they survive a restart or a day offline. After a failure the
    scrobbler waits 30 s, then twice as long each time up to 30 minutes; a "now
    playing" update that goes through ends the wait. A play Last.fm refuses
    outright is found by sending one at a time and dropped, so it cannot hold up
    the rest. When Last.fm stops taking the key (access revoked on last.fm), the
    key is forgotten, the plays are kept and the row asks to connect again.
    Disconnecting forgets the key and the plays not sent, which the row says while
    any wait.
  - "Now playing" goes out each time a track starts or resumes, and is not retried.
    Tracks are not loved. Nothing is sent or kept while no account is connected.
  - Requests, the keyring and the scrobbler's own database connection live on the
    `vcmp-lastfm` thread; its reports come back to the UI thread. Tests replace it
    with a recorder (`LastfmWorker`) and the browser with another, and
    `PlayerService` hands plays over through `ScrobbleSink`.
  - The API key and shared secret are compiled in from `VCMP_LASTFM_API_KEY` and
    `VCMP_LASTFM_SHARED_SECRET` (docs/development.md, "Integration keys"). Without
    both there is no row and nothing is sent.
- **OS media controls** (`vcmp-integrations::media_controls`, always on): MPRIS on
  Linux, System Media Transport Controls on Windows, Now Playing on macOS.
  - Media keys and the system's media widget play, pause, stop, skip, seek, set
    shuffle, repeat and volume, raise the window (un-minimise it) and quit VCMP.
    A desktop that turns media keys into MPRIS calls (GNOME, KDE, a window manager
    bound to `playerctl`) reaches VCMP whatever has the focus. One that binds
    nothing, such as a bare awesome or i3 setup, hands the keys to the focused
    window: there the winit hook (`src/shortcuts.rs`) plays and pauses, skips and
    stops itself. `playerctl` without `--player` picks the first player it lists,
    not the one playing; `playerctld` follows the one used last.
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

## Update check

- When VCMP starts, unless "Check for updates" (Settings, Updates) is off, a worker
  thread asks GitHub's REST API for the repository's ten newest releases
  (`vcmp_integrations::updates`). It takes the highest version among the published
  ones, skipping drafts, pre-releases (flagged as such, or with a version like
  `2.0.0-beta.1`) and tags that are no version, and compares it with the running
  version by semver precedence. The list is read rather than GitHub's "latest"
  release, because a fix for an older line published after a newer release would
  otherwise count as the newest.
- A newer version fills `UpdateState`, and the note "VCMP 1.2.0 is available" shows
  at the foot of the sidebar and under the version on About. Activating it opens the
  release page in the browser; VCMP downloads nothing. The page address is built
  from the tag, not taken from the answer.
- The request carries VCMP's User-Agent (its name and version). GitHub sees the IP
  address and nothing about the library; the setting's description says so.
- Network errors and answers other than 200 (offline, the rate limit, 404) are
  logged and show nothing.
- The switch is saved as `integrations.update_check` ("false" when off; absent
  means on). Turning it on checks at once, at most once per run; turning it off
  hides a note already shown.

## Accessibility patterns

- Every control is a `FocusScope` activated with Enter (arrows in `SegmentedChoice`);
  Space is play and pause everywhere but in text fields
  and an accent `FocusRing` drawn 3 px outside it. Mouse clicks do not move focus
  (`focus-on-click: false`).
- Rings show only in keyboard mode (`Theme.keyboard-navigation`), like
  `:focus-visible` on the web. It switches on when a key is pressed (the window
  sees every key first) or a control gains focus through Tab. It switches off when
  a control is clicked. So focus can move for keyboard users, as when an album
  opens, without leaving a ring in front of mouse users.
- The album grid is a single tab stop. Arrow keys, Home/End and Page Up/Down move
  the current card and scroll it into view; Enter opens it.
- Controls set `accessible-role`, `accessible-label`, the checked or selected state,
  and `accessible-action-default`. Tests find elements through these properties.
- Interactive targets are at least 40 px tall (`Theme.target-height`).
- **Zoom** (Settings → Appearance, or Ctrl+= and Ctrl+-, Ctrl+0 back to 100 %) goes
  in steps from 80 to 200 % and is saved as `ui.zoom`. Slint draws, lays out and
  maps the pointer by its window's scale factor, so `src/zoom.rs` hands the window
  the system's factor times the zoom (a `ScaleFactorChanged` event). The native
  window keeps its size in pixels, and the pages lay out in the logical size that
  leaves. Slint lays a window out again only on a resize, which a native window
  that keeps its pixels never sends, so a `Resized` event with that logical size
  follows; without it the pages kept their old size and the player bar fell below
  the window until it was resized by hand. Slint sets the system's factor when it makes the native window and when
  the system's changes (the window moved to another monitor); the winit hook then
  applies the zoom on top again. With `SLINT_SCALE_FACTOR` set, that is the
  system's factor.
- Reduced motion turns every duration in `theme.slint` to 0 ms. Under "System" (the
  default) it does what the operating system asks: Windows' "Animation effects",
  macOS' "Reduce motion", and on Linux the settings portal's `reduced-motion` key,
  or where the portal lacks it GNOME's `enable-animations` or KDE's animation speed
  set to "Instant" (`vcmp_integrations::reduce_motion`). VCMP asks at start, on a
  worker thread (`preferences::follow_system_motion`), and asks again whenever the
  system announces a change: the portal's `SettingChanged` signal, `WM_SETTINGCHANGE`
  (received by a hidden top-level window, since message-only windows miss
  broadcasts), or AppKit's accessibility display notification. Each answer that
  differs from the last sets `Theme.system-reduces-motion`. Until the first answer,
  and where the system has no such setting, motion stays on. "On" and "Off" override
  the system. Saved values are `system`, `on` and `off`; the older switch's `true`
  reads as On and `false` as System, because `false` was saved with every other
  appearance change, chosen or not.
- State is never shown by colour alone: the selected option is bold and filled, and
  the switch knob changes side.
- A dialog (`Modal`) keeps the focus: an empty `FocusScope` before and after its
  controls catches Tab past the last control and Shift+Tab before the first, and
  the dialog moves the focus to its other end (`focus-first`, `focus-last`). Only
  Tab goes round, so the player bar can't be reached by keyboard while a dialog is
  open; Space and the media keys still play and pause.

## Labels

The owner's own names for tracks ([labels spec](specs/2026-10-28-user-labels.md),
storage in [library.md](library.md#labels)). `LabelBrowser` (`src/labels.rs`, owned
by `LibraryView`) keeps the `Labels` global current.

- **A colour is only a dot** (`LabelDot`) beside the name, in the text colour (muted
  on rows until the row is hovered or selected, like their other secondary text).
  So any colour the owner picks leaves the name readable in both themes. The
  palette is Catppuccin: Mocha's colours on the dark theme, Latte's darkened in
  the same hue on the light one until each reaches 3:1 on `surface`, the fill of a
  hovered row (`src/theme.rs` tests it). A custom colour is drawn as it is.
- **Menus.** Every track menu (track lists, the songs table, the queue, the now
  playing "…" menu) has a Labels submenu: each label as a checkable item, and New
  label…. Album cards (the Albums grid, an artist's albums, search) have a menu of
  their own, for every track the album has now, and so does the album page's More
  menu (the three dots beside Shuffle), which asks for the album's labels when its
  button is pressed. Slint's menus have no mixed state, so a label only
  some chosen tracks hold reads "chill (on some)". Choosing a label puts it on all of
  them unless all hold it, which takes it off all. A menu asks for its choices just
  before it opens (`prepare-labels`, `prepare-path`, `prepare-album`): a right click
  through the list's `show-menu`, the Menu key (or Shift+F10) through the list's key
  handler, which then lets the key through to `ContextMenuArea`. The action works
  out its tracks again from the list's rows, so old choices can't act on other
  tracks.
- **Rows** show a track's labels after its title, the first two and then "+n"; the
  row's spoken label names them. Every row builder takes them from
  `labels::chips(track)`, which reads what `LabelBrowser` keeps for the UI thread,
  so no builder needs the browser. After any label change `LibraryView::labels_changed`
  builds the rows again without reading the albums or tracks.
- **Filters.** The Labels button opens a menu of every label to check; picked
  labels show as chips with a remove mark beside the button, and "Show every label"
  clears them. Songs keep the tracks holding every picked label; albums stay when
  one of their tracks does. The filters combine with the typed one and aren't saved.
- **The Labels section** sits after Playlists, and the labels are listed below the
  playlists in the sidebar, each with its dot. The section's page lists them with
  their counts and New label…; a label's page (`LabelView`) has its tracks in the
  library's order with the artist and album, Play and Shuffle. They play as a
  context of their own (`label:<id>`), named in the queue panel. A label deleted
  while its page is open says so until the user leaves.
- **The dialog** (`LabelEditor`, a `Modal`) takes the name and a colour: the palette
  as one tab stop (Left and Right choose, like `SegmentedChoice`), then Custom,
  which shows a picker: the shades of a hue to click or drag in, a strip of hues,
  and the colour as a hex code, which is the keyboard's way to pick one. Rust paints
  the strip and the shades (`Labels.hue-strip`, `Labels.shades`). What is wrong
  (no name, too long, taken, not a colour) shows in the error colour. Made from a
  menu, the new label goes on the menu's tracks.
- **Settings** lists every label with its dot and count, Edit… (the dialog) and
  Delete…, which asks first, on the page, with the focus on Cancel and how many
  tracks lose it.

## Known limits

- A label held by only some of the chosen tracks is marked by words in the menu
  ("on some"), not by a mixed check mark: Slint's `MenuItem` has none.
- The custom colour picker's shades and hues work with the mouse only; the keyboard
  picks a custom colour by typing its hex code.
- The tag editor can't empty a field that differs between an album's tracks in one
  go: an empty "Several values" field keeps the values. Type a value, or edit the
  tracks one by one.
- The list of changes before writing several files shows the new values only, not
  the values they replace.
- A selection lives in one list: tracks from two pages (an album and a search) can't
  be selected together, and a selection goes when its list is rebuilt (another
  album, a new sort or filter, a library change).
- On Windows, an edit to the track the player has open waits until playback stops
  or moves on, also while it is paused. Pause doesn't let go of the file.
- The waiting and the release were tested on Linux, with the lock simulated
  (`set_open_files_locked`). On Windows they are built but not yet seen working.
- A play that started before its tags were edited is scrobbled to Last.fm with the
  tags it started with.
- On Windows, a change an agent asked for can't be accepted while the player has
  one of its files open: the review says which file and keeps the change waiting
  until playback moves on. The tag editor instead writes the rest and holds that
  one file back; a change set is accepted once, with a result per file, so it waits
  rather than leave one file for later. Tested on Linux with the lock simulated.
- The More menus are Slint's own, like the track menus, and Slint's testing backend
  can't reach their items: tests find the More button and run the item's action
  directly.
- Zoom was tested headless and against the window's scale factor; how it looks on
  a real high-resolution monitor, and on moving the window to another monitor, is
  in [manual-testing.md](manual-testing.md).
- VCMP's own window buttons on Windows are built and checked to compile for
  Windows, but not yet seen working. A page scrolled under the 16 px drag strip
  can't be clicked there; the strip drags instead. The system's title bar shows for
  a moment at start, until the native window exists and it comes off. On Windows 10
  the window's top border line is missing: Windows 10 draws it as part of the title
  bar. The border's colour needs Windows 11. The caption buttons aren't in the Tab
  order; the keyboard reaches them through the window menu (Alt+Space), as with
  Windows' own. On macOS and Linux the system's title bar stays and follows the
  system's light or dark mode, not VCMP's theme.

- The history (back and forward) remembers sections and the album, artist and
  playlist pages, not a folder opened on the Folders page or the search typed, and
  an album reached from search comes back as an Albums page.
- An artist's photo is keyed by their name: two different artists with the same
  name in one library share it.
- Photos found online were tried only against scripted answers; a real search needs
  a build with VCMP's fanart.tv key.
- The track menus are Slint's own, styled by the operating system's colour scheme
  like `AboutSlint` below, so they can look out of place when VCMP's theme differs
  from the OS.
- Tracks whose file path isn't valid Unicode can't be queued: the engine names
  tracks by a string path.
- The OS media controls were tried by hand on Linux (MPRIS) only; Windows and macOS
  are built and tested in CI but not yet seen working.
- Windows takes media widget covers only from the web, so there the widget shows
  the Cover Art Archive cover when one is known and none otherwise.
- The Windows media flyout may name VCMP "Unknown app": releases are archives, and
  only an installer (left for later) would register an AppUserModelID. On Linux the
  widget's icon needs a `vcmp.desktop`, which only the .deb and AppImage installers
  install.
- Raising the window from the media widget only un-minimises it: Wayland compositors
  don't let an app take focus without an activation token.
- The update check asks GitHub without signing in, so it finds nothing while the
  repository is private (GitHub answers 404). It runs once per start: a VCMP left
  open for days learns of a release at its next start.
- Last.fm's API was exercised through scripted answers; connecting a real account
  is still to be tried.
- On Linux the Last.fm key needs a Secret Service (GNOME Keyring, KWallet). Without
  one, an account stays connected until VCMP closes, and the row says so.
- VCMP doesn't watch for the network coming back: queued plays wait out the backoff,
  or go as soon as a "now playing" update gets through.

- The Slint attribution (`AboutSlint`) picks its logo from the operating system's
  colour scheme, not VCMP's theme, so it sits on the system palette's background. It
  can look out of place when VCMP's theme differs from the OS.
- Theme and reduced-motion choices are saved in the library database's settings
  (`src/preferences.rs`). If the database can't be opened, they last for the session.
- The operating system's reduce-motion setting was tried on Linux only: GNOME's key
  through the real portal (both portal methods), and following changes against a
  stand-in portal on a private bus. The Windows and macOS code and the KDE key are
  still to be tried on those systems: `cargo run -p vcmp-integrations --example
  reduce_motion` prints what VCMP reads and, except on macOS, each change.
- `examples/screenshots.rs` cannot force a window size under tiling window managers.
  Layout at the smallest window size is covered by a test instead.
