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
| `ui/theme.slint` | `ThemePalette` struct, `ThemeMode` and `ReduceMotion` enums, `Theme` global: colours, spacing, radii, type sizes, target height, content width, durations, drag scrolling |
| `ui/components.slint` | `FocusRing`, `EdgeScroll`, `NavItem`, `PlaylistLink`, `SegmentedChoice`, `Toggle`, `SettingRow`, `Button`, `IconButton`, `Dropdown`, `SearchField`, `TextField`, `ProgressBar` |
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
| `src/updates.rs` | The update check's switch, saved in the settings, and the newer release it found |
| `src/theme.rs` | The light and dark palettes, plus their contrast tests |

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
| Type scale 12 / 14 / 16 / 20 / 26 px, and 34 px for the track in the now playing view | Small steps suit a calm UI; the page title is the one large element per screen, and the now playing view's title takes its place there |
| Now playing covers the pages, not the player bar | The bar already holds every control, so the view needs none of its own and the controls stay where people know them |
| Titles, artists and albums of the playing track are links, underlined on hover | People expect to go from what plays to where it comes from; the underline marks a link without adding colour |
| Spacing scale 4 / 8 / 12 / 16 / 24 / 40 px | Bigger gaps separate sections, small ones group a label with its description |
| Radii 2 / 8 / 12 px, pill only for the switch | The switch track is the one shape where a pill carries meaning |
| Content column capped at 720 px | Keeps a setting's label and its control within one glance on wide windows |
| Sidebar plus content area | Agreed in the spec; it holds only the pages that exist (Search, Albums, Artists, Playlists, Songs, Folders, Settings, About), then the playlists by name |
| Folder browser built from the library, files listed by file name | Instant, and it shows the music VCMP knows; the file name comes first because the browser is where badly tagged files are found |
| Songs table without a track-number column, missing values sorted last | Numbers only mean something within an album, which the Album column already orders; untagged files at the top would hide the library behind them |
| Search first in the sidebar, the app opening on Albums | Search is the way into every other page, as in most players; Albums is still what people come to see |
| Artists in round frames, albums in rounded squares | The shape tells the two grids apart at a glance, as people know it from other players |
| An artist's picture is their photo, or without one their newest album's cover | A grid of identical initials said nothing; the cover is theirs and recognisable. Credit-only artists without a photo keep the initial |
| Album artists by default, everyone behind a switch | The owner's choice: 391 of 532 artists in their library are only credited on others' albums, and would crowd the grid |
| Chevron on the album page's back button | Shows the direction of the action; it's the one arrow in the UI |
| Accent-filled button only for "Add music folder…" on the empty Albums page | The single action that empty screen exists for; every other button is outlined |
| Scan progress at the bottom of the sidebar | Albums appear during a long first scan; progress stays visible on any page |
| The sidebar's sections and playlists scroll as one, above its foot | Eight sections and any number of playlists don't fit a short window. Scrolling them together keeps one scroll in the sidebar, and the scan status and update note stay in view below. The keyboard scrolls the focused entry into view; the page marker scrolls with its section |
| A newer VCMP is a muted link at the foot of the sidebar, and a link on About | The sidebar is on screen on every page, so the note is seen without opening anything; small, muted and in a corner, it doesn't compete with the navigation or interrupt listening. A dialog or toast would interrupt, a badge would be a second accent, and a note only in Settings or About would go unseen. A running scan takes its place, as the scan's progress matters now and the note waits |
| Accent on the play button, the playback progress and the playing track | All three mark what is playing now; the volume fill is muted because it is a setting |
| Track numbers and durations muted, titles in text colour | The title is what people scan for; numbers support it |
| Light and dark plus "System", defaulting to System | No brand reason to force a theme; following the OS is least surprising |
| Motion: marker slide, page fade, control transitions (120 / 220 ms) | MOTION 2: transitions show where the selection went; nothing loops |
| Reduce motion is System, On or Off, defaulting to System | Built like the theme choice. People who need less motion set it once in the OS for every app; On and Off still decide for VCMP alone, either way |
| "Find cover online" under the missing cover | The action sits where the gap is, and it leaves the Play and Shuffle row the width a 720 px window allows |
| The cover picker covers the sidebar and pages like the now playing view | One overlay pattern in the app; the covers get the room, and the player bar stays usable |
| Artist photo actions sit under the picture, and their picker is the cover picker's twin | The same place and pattern as an album's "Find cover online", so one way of working covers both |
| The tag editor is the same kind of overlay, not a separate window | One overlay pattern again. A second window would need its own focus, theme and close handling on three platforms, for a form that is used now and then |
| An album's differing values show as empty fields saying "Several values" | Writing only the fields the user changed is the safe default for a batch: a field left alone can't flatten twelve track numbers into one |
| A save to several files first lists its changes in the form's place, with the focus on Back | The spec asks for no bulk tag writes without a preview. The list shows exactly what goes into every file, and starts at the top even in a scrolled editor. Back has the focus, as Cancel does before deleting a playlist, so an Enter pressed twice goes back instead of rewriting an album |
| A click on a cover picks it, with no separate confirm button | The picker exists for this one choice; Cancel and Escape are the way out |
| Last.fm connects in the browser while the row waits, with Cancel | Last.fm's desktop sign-in happens on last.fm; asking every few seconds spares an "I've allowed it" button |
| One Last.fm button whose text follows the state (Connect…, Cancel, Disconnect) | The focus stays on it while the state changes under it |

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
- **Find cover online:** an album without artwork has this button under its
  missing cover. Nothing is sent anywhere before it is chosen. An album showing a
  cover fetched this way has "Change cover…" there instead, whose menu offers "Find
  another cover" (the same picker) and "Remove cover", which goes back to no cover
  and lets the automatic Discord lookup run again.
  - It opens the cover picker (`CoverPicker`) over the sidebar and pages, and
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
    Escape or Cancel closes the picker and stops the search, and the focus returns
    to the album page's back button.

The Albums page fades in through a `FadeCurtain`, created each time the page is
shown, because the page itself stays alive while hidden.

## Artists page

- **The grid** lists album artists by default (138 in the owner's library). The
  "All artists" choice adds everyone credited on a track (532), saved as
  `ui.artists_all`. A card says how many albums an artist has, or how many tracks
  for an artist with no albums of their own. Its picture is the artist's photo, or
  without one the cover of their newest album; pictures load in the background like
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
- **The artist's photo:** under the picture, "Find photo online", "Choose photo…"
  and, once there is one, "Remove photo".
  - "Find photo online" opens a picker over the sidebar and pages, like the cover
    picker. The artist is looked up on MusicBrainz (by the ID the files are tagged
    with, then by name; only sure, same-name matches), and their photos come from
    fanart.tv, the best liked first, up to 8. Each shows the name MusicBrainz has
    and its disambiguation ("UK rock band"), so two artists of the same name can be
    told apart. A click or Enter picks one, which is downloaded in full; Escape or
    Cancel closes. Nothing is sent until the button is used, and the button is
    there only in builds with VCMP's fanart.tv key (`VCMP_FANARTTV_API_KEY`).
  - "Choose photo…" takes a JPEG, PNG or WebP file. One that can't be read says so
    under the buttons.
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
  "File not found" where the artist would be. Play, Shuffle and a click play the
  entries that are there, as one context (`playlist:<id>`, named in the queue
  panel); a click on a missing entry plays from the next one that is there.
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
- **A whole album:** "Add to playlist…" under the album page's cover adds every
  track of the album; New playlist there names the playlist after the album.
- **Getting back:** the back button, Escape, or the Playlists sidebar entry. A
  playlist opened from search goes back to Search.

## Tag editor

- **Opening it:** "Edit tags…" in every track menu (album, artist, search, folder and
  playlist lists, and the songs table) edits that track, or every selected track
  (see [Selecting tracks](#selecting-tracks)). "Edit tags…" under the cover on an
  album page edits every track of the album at once. The heading names the album
  for tracks of one album, or counts the tracks of several.
- **The form:** the cover, with Choose image… (JPEG or PNG) and Remove cover, then
  Artist, Album, Album artist, Disc, Year and Genre, plus Title and Track for one
  track. The focus starts in the first field.
- **What gets written:** only the fields the user changed. An emptied field removes
  its tag. For an album, a field whose value differs between tracks starts empty
  with "Several values" as its placeholder, and keeps each track's own value unless
  something is typed in it. The cover works the same way ("Several covers").
- **Saving:** Save or Enter in a field. A wrong value (a track number that isn't a
  whole number, a year outside 1 to 9999, a line break) writes nothing and says
  which field is wrong. Escape or Cancel closes without writing. Saving with
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

- **Mouse:** Ctrl+click (Cmd+click on macOS) adds or takes away a track; Shift+click
  selects the tracks from the last one clicked. A plain click still plays, and
  clears the selection.
- **Keyboard:** Shift with Up/Down, Home/End (and Page Up/Down in the songs table)
  selects as the current track moves; Ctrl+Space adds or takes away the current
  track; Ctrl+A selects all; Escape clears, and only a second Escape goes back. The
  window's Space shortcut leaves Ctrl+Space to the lists.
- **The menu** acts on every selected track when it opens on one of them, and says
  how many ("3 tracks selected"): Play next and Add to queue put them into Up next in
  list order, Add to playlist adds them all, Edit tags… edits them together, and in a
  playlist Remove from playlist (or Delete) removes them. Play now and Move up/down
  are for one track and hide. A right click on a track outside the selection clears
  it first, so the menu acts on that track alone.
- **Look:** a selected row has the hover fill and a 3 px accent bar at its edge, so
  it reads apart from the row under the pointer; its secondary text turns full
  strength, because muted text on that fill would fall under 4.5:1 in the light
  theme. Assistive technology hears each row as selectable and selected.
- **Where it lives:** each row model holds `selected`. Slint can't loop over rows,
  so `src/selection.rs` applies a toggle, range, all or none and returns the count;
  a new list (another album, a new sort or filter) starts with nothing selected. The
  queue and remove handlers take the list's model and the menu's row and act on the
  selected rows when that row is one of them; Add to playlist and Edit tags go
  through the `TrackSelection` and `SongSelection` globals by file path.

## Search

- **The Search page** is first in the sidebar; Ctrl+F (Cmd+F on macOS) opens it
  from anywhere with the field focused. The window's shortcut scope holds the focus
  at start, so the shortcut works before anything was clicked.
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
- **Tracks:** a click or Enter plays the found tracks from that one (the queue
  panel says "Next from your search"); the track menu offers Play now, Play next
  and Add to queue.
- **`TrackList`** is the track list on album, artist, folder and playlist pages and
  search: one tab stop, Up/Down, Home/End, Enter, and the Menu key or a right click
  for the track menu (Play now, Play next, Add to queue, Add to playlist).

## Page filters

The Albums, Artists and Songs pages each have a filter field beside their title
(`SearchField` in its compact form). A row stays when every typed word appears in
one of its names, ignoring case: an album's title or artist, an artist's name, a
song's title, artist or album. Albums and artists filter as you type; the songs
table, which can hold tens of thousands of rows, once typing pauses for 150 ms.
The page says when nothing matches, and Escape or the clear button empties the
field. Unlike Search, a filter only narrows the page it is on.

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
- **Playing:** a click or Enter plays every song in the table's order from that one;
  the queue panel says "Next from all songs".

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
  and pages and fades in; the player bar stays below it with the controls, and
  the queue panel can be open beside it. Escape, the close button (top left) or a
  click on the background returns; the focus starts on the close button so Escape
  works at once.
- **Leading on:** as in the player bar, the title, the album and the cover lead to
  the album's page, and the artist to the artist's page; the view closes on the
  way. The next track is a button that skips to it.
- **Content:** the cover as large as the view allows (up to 560 px), then the
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
    volume slider while muted unmutes at the new value.
- **Seeking** moves the bar to the new position at once. Positions the engine sent
  before it took the seek are ignored for up to a second, so the bar doesn't jump
  back.
- **On the album page:**
  - Play starts the album; Shuffle turns shuffle on and starts it;
  - clicking a track, or pressing Enter on it, plays the album from there;
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
- **The queue panel** slides in from the window's edge (at once with reduced
  motion). It opens from the queue button in the player bar, and stays open
  or closed across restarts (`ui.queue_open`). In a window at least 1000 px wide
  the page narrows to make room for it; in a narrower one the panel lies over the
  page's right edge, so the page keeps the width its controls need. It lies over
  the now playing view too.
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
  names the device playing instead.
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
    `VCMP_LASTFM_SHARED_SECRET` (README, "Integration keys"). Without both there is
    no row and nothing is sent.
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

## Known limits

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

- Going back is one step: an album opened from an artist found by search returns
  to the artist, and the artist to Search, but there is no general history.
- The queue panel slides in when it opens but closes at once: the panel is made when
  it opens and dropped when it closes, so there is nothing left to slide out.
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
- The Windows media flyout may name VCMP "Unknown app" until the installer registers
  an AppUserModelID; on Linux the widget's icon needs the installed `vcmp.desktop`.
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
