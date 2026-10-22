# Listening and library features

Status: agreed
Date: 2026-10-22
Area: vcmp-library (schema, play history, favourites, finder queries, M3U, formats),
vcmp-playback (formats, stop after this track), vcmp-app (Home page, heart buttons,
sleep timer, finder view, first-run wizard, Settings); all three platforms

## Context of the change

VCMP v1 ([2026-10-17-vcmp-music-player.md](2026-10-17-vcmp-music-player.md)) left
several things out on purpose: favourites, ratings, play counts and "recently added"
were "not requested and not planned" (play history is kept for scrobbling only),
M3U import/export was offered and not chosen, formats beyond MP3, FLAC, WAV and Ogg
Vorbis were out of scope, and a first-run wizard was planned for later. A sleep
timer, a Home page and a duplicate or missing-tag finder were never discussed.

What exists today on `main` (schema version 9):

- **Library.** Tracks are identified by path (`tracks.path`); a missing file is
  marked unavailable, never deleted, and a moved file re-links to its old row when
  title, artist and album match and the duration is within 1 s
  ([library.md](../library.md)). `tracks.added_ms` is set when a row is first
  inserted, so every track of the first scan carries the scan's time; it is not
  exposed on `Track`. Nothing stores play counts or last-played times; the
  `scrobble_queue` table keeps tags, not track ids, and its rows are deleted once
  sent. Settings live in the `settings` key/value table.
- **Playlists.** Name (not unique), an optional own picture or a mosaic of the first
  four album covers, entries by position, duplicates allowed. There is no special or
  built-in playlist, and no import or export.
- **Plays.** The Last.fm rule (track longer than 30 s, heard for half its length or
  4 minutes, seeks add nothing) is computed in the app on every Position event, even
  when Last.fm is not connected; it produces no play for tracks without an artist
  tag or outside the library. The engine emits `TrackChanged`, but no event tells a
  natural end from a skip.
- **Playback.** Stop and Pause are distinct. Gapless playback opens the next track
  about 2 s before the current one ends and writes it straight after, so stopping
  at a track boundary needs the engine to not prepare the next track
  ([playback.md](../playback.md)). The only volume ramp is 10 ms per gain change.
- **Formats.** The scanner accepts `mp3`, `flac`, `ogg`/`oga` and `wav`, checked by
  extension and by lofty's detected type. symphonia 0.6.1 is built with mp3, flac,
  pcm, wav, ogg and vorbis; it also offers `aac`, `alac`, `isomp4` and `aiff`, but
  no Opus, WMA, APE or WavPack decoder. lofty 0.25 already reads MP4, AIFF, Opus,
  APE and WavPack tags. Playback probes content and is not limited by extension.
- **UI.** The sidebar is Search | Albums, Artists, Playlists, Songs, Folders |
  Settings, About, then playlists by name; the app starts on Albums. Page indices
  are hard-coded in `app-window.slint`, `navigation.rs` and several tests. Track
  rows (`TrackLine`, `SongLine`, `QueueLine`) carry a path only and have no in-row
  buttons except the queue's remove button. The player bar fits a 720 px window with
  about 24 px to spare; only its left section stretches. Icons are 16 SVGs in
  `ui/icons/`; there is no heart. Dialogs use the `Modal` component. A new user
  sees the empty Albums page with "Add music folder...". Discord presence is on by
  default and explained only in Settings.
- **Pending work.** The MCP server PR
  ([dalibor-osu/vcmp#42](https://github.com/dalibor-osu/vcmp/pull/42), not yet
  tested) adds schema versions 10 (`track_tags`, every tag of every file) and 11
  (reviewed change sets), and `Library::tracks_by_tag` for finding tracks missing a
  tag.

## User request

Work on these features: play history (play count, last played, recently added);
more formats; a sleep timer that includes two things, "stop after this track" and a
timer that stops the music after a selected time; favourites, Spotify style, with a
simple heart icon on the track row or in the bottom playing bar, the liked tracks
being in a special "Favourites" playlist; a Home page; a duplicate and missing-tag
finder; M3U import/export; a first-run wizard. Create a new spec with these changes
and brainstorm how they should work. The MCP server PR is yet to be tested.

## Decisions

### Should this work build on top of the MCP server PR (#42) being merged first?

| Option | What it means |
|---|---|
| After PR #42 (Recommended) | New schema versions follow its 10 and 11, and the missing-tag finder can reuse its every-tag table; waits for #42 to be tested and merged. |
| Independent of #42 | Can start on main now; whichever lands second renumbers its migrations, and the finder only knows the nine fields VCMP shows. |

**Answer: After PR #42**

### How should the eight features be delivered?

| Option | What it means |
|---|---|
| One PR per feature (Recommended) | Small reviews and features ship in any order after shared groundwork; more merges and more manual-testing rounds. |
| A few grouped PRs | E.g. library data (history, favourites, finder, M3U), playback (formats, sleep timer), UI (Home, wizard); fewer merges, larger reviews. |
| One PR for everything | One review and one release note; a large diff, and a problem in one feature holds back all of them. |

**Answer: One PR per feature**

### What counts as a play for the play count and last played?

| Option | What it means |
|---|---|
| Last.fm rule (Recommended) | Over 30 s and half the track or 4 min heard, seeks add nothing; matches Last.fm, and quick skips don't count. Counted for every library track, tagged or not. |
| Played to the end | Only tracks that finish naturally count; strict (stopping 10 s early loses the play) and needs a new engine event. |
| Any start | Every track start counts; simplest, but skipping through a playlist inflates counts. |

**Answer: Last.fm rule**

### What should be stored for play history?

| Option | What it means |
|---|---|
| Every play with its time (Recommended) | A play log (track, time); counts and last played derive from it, and Home can show recent plays or periods later. Grows by a few MB per year. |
| Counters only | Play count and last played on each track; tiny and simple, but no history behind them, so no per-period stats ever. |

**Answer: Every play with its time**

### What date does "recently added" use for tracks already in the library?

| Option | What it means |
|---|---|
| File dates (Recommended) | Earlier of the file's created and modified time, for old and new files alike; a file copied in with its old dates shows as added long ago. |
| First seen, backfill old | New files get the moment VCMP first sees them; only today's library is backfilled from file dates. Precise for new music, two rules. |
| Keep first-scan time | No change; the current library shares one moment, so recently added is useful only for music added from now on. |

**Answer: First seen, backfill old**

### Which formats should this change add?

Several could be chosen.

| Option | What it means |
|---|---|
| M4A: AAC and ALAC (Recommended) | The most common missing format (iTunes, purchases, ALAC rips); pure Rust in symphonia. AAC in MP4 has no gapless info there, so AAC albums may get tiny gaps. |
| AIFF (Recommended) | Lossless Apple format, pure Rust in symphonia with gapless; rare in most libraries but nearly free to add. |
| Opus | Common for modern rips and downloads; symphonia has no decoder, so it needs libopus (a C library to build or bundle on all three platforms). |

**Answer: M4A (AAC and ALAC) and AIFF**

### Can the tag editor write tags to the new formats?

| Option | What it means |
|---|---|
| Read and write (Recommended) | New formats behave like the old four, including covers and the read-back check before replacing; each needs edit_check runs on real files. |
| Read only for now | Play, scan and show them; the tag editor says it can't edit that format. Less risk to the files, but a visible gap. |

**Answer: Read and write**

### Where does the sleep timer live?

| Option | What it means |
|---|---|
| Button in the player bar (Recommended) | A moon button by the queue button opens the choices and shows the time left in the accent while running; the 720 px bar needs ~16 px more, taken from the title section. |
| Now playing menu only | Choices in the now playing view's "..." menu; no layout change, but hidden, and a running timer shows only there. |
| Bar's context menu | Right-click on the player bar; no new button, but undiscoverable and hard to reach by keyboard. |

**Answer: Button in the player bar**

### Which lengths can be picked? ("End of this track" is always one of the choices.)

| Option | What it means |
|---|---|
| Presets (Recommended) | End of this track, 15, 30, 45, 60, 90 min; one click covers almost every use, no odd lengths. |
| Presets and custom | The presets plus a minutes field for any length; flexible, one more input to build and keyboard-test. |

**Answer: Presets**

### What happens when the timer runs out?

| Option | What it means |
|---|---|
| Fade out, then pause (Recommended) | Volume fades to silence over ~10 s, playback pauses, the volume comes back; gentle, and the position is kept for later. |
| Finish the track, then stop | When time is up the current track plays to its end; no cut mid-song, but may run several minutes over. |
| Pause at once | Simplest; an abrupt cut, which can wake someone falling asleep. |

**Answer: Finish the track, then stop**

### What is the Favourites playlist underneath?

| Option | What it means |
|---|---|
| Own list shown as a playlist (Recommended) | Liked tracks with the time liked, shown pinned first among playlists, newest first, fixed name, no delete or reorder (like Spotify's Liked Songs); the playlist page needs a read-only variant. |
| A real playlist, flagged | A playlists row marked as Favourites; reuses the page, picture and M3U export as is, but every playlist action must guard it (no delete, rename, duplicates). |

**Answer: Own list shown as a playlist**

### Where do heart buttons appear?

Several could be chosen.

| Option | What it means |
|---|---|
| Player bar (Recommended) | Next to the playing track's title; the most used spot, fits the bar's stretching left section. |
| Track lists (Recommended) | Album, artist, playlist, search and folder rows; shown on hover and always when liked. Rows must start carrying track ids. |
| Songs table and queue | The same in the Songs table and the queue panel; consistent everywhere, but more crowded rows. |
| Now playing view | A large heart by the title in the full-screen view. |

**Answer: Player bar, Track lists, and "Also add "Add to favourites" option to the right-click menu"**

### Does a heart go anywhere outside VCMP?

| Option | What it means |
|---|---|
| Local only (Recommended) | Hearts live in VCMP's database and follow re-linked files; lost only if the database is. |
| Also love on Last.fm | When connected, a heart loves the track on Last.fm and removing it unloves; one-way, loves made elsewhere don't come back. |
| Also a tag in the file | Written as a rating tag so other players see it; every heart rewrites a file, and formats store ratings differently. |

**Answer: "Local + a setting to also reflect this on Last.fm. It would be nice to also add a support to load loved/favoutites from Last.fm. This should live as a button in settings."**

### Which right-click menus get "Add to Favourites"?

| Option | What it means |
|---|---|
| Every track menu (Recommended) | Track lists, Songs table, queue and the now playing "..." menu; keyboard users can like a track anywhere, even where no heart is drawn. |
| Only where hearts show | Track list menus only, next to the row hearts; consistent with the hearts, but the Songs table and queue can't like at all. |

**Answer: Every track menu**

### What does the Home page show?

Several could be chosen.

| Option | What it means |
|---|---|
| Recently played (Recommended) | Albums and playlists you last played from, newest first; the fastest way back into music. |
| Recently added (Recommended) | Albums whose tracks VCMP saw first most recently. |
| Most played (Recommended) | Top tracks and albums of the last 30 days, from the play log. |
| Forgotten favourites | Liked tracks or much-played albums not heard for months; nice for rediscovery, empty until history builds up. |

**Answer: All four: Recently played, Recently added, Most played, Forgotten favourites**

### Where does Home sit and what does VCMP open on?

| Option | What it means |
|---|---|
| First, opens on Home (Recommended) | Home goes above Albums and is the start page; a sidebar change recorded in DESIGN.md as your decision. |
| First, opens on last page | Home goes above Albums, but VCMP reopens the section you left; a new saved preference. |
| First, still opens on Albums | Home is just another section; the start stays as today. |

**Answer: "First, add a setting to either always open on Home or on last page"**

### How are Home's sections laid out?

| Option | What it means |
|---|---|
| One row each, Show all (Recommended) | Each section is one row of cards that fits the width, with Show all opening the full list; no sideways scrolling, simple keyboard paths. |
| Sideways-scrolling shelves | Spotify-like rows that scroll horizontally; more at a glance, but needs horizontal scrolling with mouse and keys. |

**Answer: One row each, Show all**

### Where does the duplicate and missing-tag finder live?

| Option | What it means |
|---|---|
| Opened from Settings (Recommended) | A "Check library" entry under Music folders, next to the unreadable-files list, opens a page; out of the way of listening, it's an occasional task. |
| Own sidebar section | A "Tidy up" section in the sidebar; one click away, but a permanent entry for something used now and then. |
| Filters on the Songs page | "Missing tags" and "Duplicates" filters on the Songs table; reuses its selection and tag editor, but no room to group or explain duplicates. |

**Answer: Opened from Settings**

### What makes two tracks duplicates?

| Option | What it means |
|---|---|
| Title, artist and length (Recommended) | Same title and artist ignoring case, lengths within 2 s, like the re-link rule; finds one song in two formats or folders, may flag identically named live versions. |
| Same audio | A fingerprint of the decoded audio; nearly no false matches, but decodes every file (many minutes on a large library) and misses re-encodes. |
| Title and artist only | Ignores length; catches more, including live and remastered versions that aren't real duplicates. |

**Answer: Title, artist and length**

### What can be done from the finder?

| Option | What it means |
|---|---|
| Review and fix, no deleting (Recommended) | Play, edit tags, show in the file manager, and mark a group "not duplicates" to hide it; VCMP never removes music files. |
| Also move a copy to the trash | Faster clean-up from inside VCMP; the first feature that removes music files, recoverable from the OS trash. |

**Answer: Review and fix, no deleting**

### Which missing things does the finder report?

Several could be chosen.

| Option | What it means |
|---|---|
| Title, artist, album (Recommended) | The fields VCMP groups by; titles that fell back to the file name count as missing. |
| Track number and year (Recommended) | Ordering and dating; common gaps in ripped or downloaded files. |
| Album artist and genre | Compilations group wrongly without album artist; genre is often left empty on purpose. |
| Cover (Recommended) | Albums with no embedded or folder cover, with "Find cover online" right there. |

**Answer: Title, artist, album; Track number and year; Cover**

### What is the start-page setting's default?

| Option | What it means |
|---|---|
| Home (Recommended) | New and existing users land on the new page and see it exists; anyone who prefers can switch to the last page. |
| Last page | Existing users keep landing where they were used to; Home is easy to miss at first. |

**Answer: Home**

### How do M3U playlists get in and out?

| Option | What it means |
|---|---|
| Import and export by hand (Recommended) | "Import playlist..." on the Playlists page, "Export..." in a playlist's menu (Favourites too); an imported playlist is a normal VCMP playlist from then on. |
| Also pick up .m3u in music folders | Playlist files found in the music folders appear and update by themselves; convenient, but edits in VCMP and in the file can fight. |

**Answer: Import and export by hand**

### How does an exported playlist point at its files?

| Option | What it means |
|---|---|
| Relative where possible (Recommended) | Relative to the playlist file's folder when on the same drive, absolute otherwise; survives moving the whole music folder or using it on another machine. |
| Always absolute | Works wherever the playlist file is moved on this machine; breaks on another machine or a different mount point. |

**Answer: Relative where possible**

### What happens to imported entries that aren't in the library?

| Option | What it means |
|---|---|
| Path, then tags; list the rest (Recommended) | Match by path, then by the #EXTINF artist and title; anything still unfound is left out and listed after the import. |
| Path only; list the rest | Exact and predictable; a playlist from another machine matches nothing unless paths happen to line up. |

**Answer: Path, then tags; list the rest**

### Who sees the first-run wizard?

| Option | What it means |
|---|---|
| New libraries only (Recommended) | Shown when VCMP starts with no music folders and no saved settings; existing users never see it. |
| Everyone once | Existing users see it once after updating too; introduces Home and favourites, but walks them through choices they already made. |

**Answer: "New libraries only, but add a button to settings to "rerun" the wizard."**

### Which steps does the wizard have (after a welcome)?

Several could be chosen.

| Option | What it means |
|---|---|
| Music folders (Recommended) | Add one or more folders, subfolders included or not; the scan starts in the background while the wizard goes on. |
| Look (Recommended) | Theme (System, Light, Dark) and reduce motion, shown live behind the dialog. |
| Online services (Recommended) | Discord presence, Last.fm connect, the update check and artist photos, each explained with what it sends where. |
| Start page | Open on Home or on the last page; one more step for a choice most people never change. |

**Answer: Music folders, Look, Online services**

### What form does the wizard take?

| Option | What it means |
|---|---|
| Dialog over the window (Recommended) | The existing Modal with Back, Next and Skip, Escape skips it all; the app is visible behind and fills as the scan runs. |
| Full-window pages | Covers the whole window like an installer; more room for explanation, the app appears only at the end. |

**Answer: Dialog over the window**

### Where do play count, last played and date added show for tracks?

Several could be chosen.

| Option | What it means |
|---|---|
| Songs table columns (Recommended) | Plays, Last played and Added as sortable columns; the table gains a way to hide columns, since eight don't fit a 720 px window. |
| Tag editor, one track | Read-only lines in the tag editor when it edits a single track; no new UI, but only seen while editing. |
| Album page header | The album's total plays and last played under its title. |

**Answer: Songs table columns, and "A statistics page would be nice in the future. Just mark it as planned."**

### Can play history be cleared or paused?

| Option | What it means |
|---|---|
| Clear in Settings (Recommended) | A "Clear play history" button with confirmation; counts, last played, Home's played sections and the play log are emptied. |
| Clear and pause | Plus a switch that stops recording plays (e.g. for someone else's music); Home's sections freeze while it is off. |
| Neither | History is only ever added to; simplest, but no way to start over short of deleting the database. |

**Answer: Clear in Settings**

### What are the thresholds for "Forgotten favourites" and "Most played"?

Left open in the first draft; the owner settled it without offered options.

**Answer: "For forgotten favourites, let's do 3 months and for most played add a selection of time range. Either last week, last month, last year or all time."**

### Where is the Most played time range picked?

| Option | What it means |
|---|---|
| On Home and Show all (Recommended) | A small range picker in the section's heading row on Home, the same picker on its Show all page; the two stay in step. |
| Only on Show all | Home's row always uses the default range; quieter Home, but changing the range takes a click into the full list. |

**Answer: On Home and Show all**

### Which range does Most played start with, and is the choice remembered?

| Option | What it means |
|---|---|
| Last month, remembered (Recommended) | Starts on last month; whatever you pick is saved and kept after a restart. |
| Last month, not remembered | Every start goes back to last month; a pick only lasts for the session. |
| All time, remembered | Starts on all time; fills sooner on a new install but changes slowly once history is long. |

**Answer: Last month, remembered**

### What does "Forgotten favourites" include?

| Option | What it means |
|---|---|
| Liked tracks only (Recommended) | Liked tracks not played for 3 months (or never played since liking); a clear rule, empty if you never use hearts. |
| Liked tracks and much-played albums | Also albums played often before but not in 3 months; needs a play count that counts as "often", still to be set. |

**Answer: Liked tracks only**

## High-level plan

All work starts after PR #42 is tested and merged. Each feature is its own PR into
`main`, with its own documentation and manual-testing steps. Features that add a
schema version land one after another, so each takes the next number.

**Play history.** The play log, counting by the Last.fm rule for every library
track, the backfill of added dates from file dates, and Plays, Last played and
Added as sortable Songs columns, with a way to hide Songs columns. "Clear play
history" in Settings. Depends only on #42.

**Favourites.** The liked-tracks list, track ids on the rows, hearts in the
player bar and track lists, "Add to Favourites" / "Remove from Favourites" in
every track menu, the pinned read-only Favourites playlist, and in Settings the
"Love on Last.fm" switch and the "Import loved tracks from Last.fm" button.

**Home page.** Home as the first section, the start-page setting (Home or last
page, default Home), and the four sections with Show all and the Most played range
picker. Depends on play history and favourites.

**Sleep timer.** "Stop after the current track" in the engine, the moon button in
the player bar, and the presets. Independent of the others.

**More formats.** M4A (AAC and ALAC) and AIFF: scan, play, tag reading and
writing with covers, fixtures and edit_check runs. Independent of the others,
but builds on #42's tag writing.

**Library check.** The finder page opened from Settings, Music folders: duplicate
groups, missing tags and missing covers, with play, edit tags, show in the file
manager, find cover online and "Not duplicates". Uses #42's tag table.

**M3U import and export.** Depends on favourites, so Favourites can be exported too.

**First-run wizard.** Last, so it can introduce the finished features; includes
the Settings button that runs it again.

## Architecture decisions

**Play history.**
- A `plays` table in `vcmp-library` (track id, time, origin of the play context:
  album, artist, playlist, Favourites or none). Counts and last played are derived
  through an index on (track, time); nothing is cached on `tracks`. Keeping every
  play beat counters because Home, "Forgotten favourites" and the planned statistics
  page all need time ranges.
- A play is recorded once, when the Last.fm rule is met. The counting reuses
  `lastfm::Play` but not `start_play`'s gate: tracks without an artist tag count.
  Plays keep counting while Last.fm is disconnected or not built in.
- `tracks.added_ms` keeps "first seen" for new rows. Existing rows are backfilled
  once, on the first scan after the migration (SQL can't read file dates): the
  earlier of the file's creation and modification time, or modification time where
  the file system has no creation time. The backfill never moves a date later.
  `Track` exposes the added date.
- "Clear play history" deletes the play log only. Scrobbles waiting to be sent are
  untouched, and added dates are not history.

**Favourites.**
- A `favourites` table (track id, time liked), not a `playlists` row, so no
  playlist action needs a guard. Following the track id means hearts survive
  re-links; an unavailable track stays liked and shows "File not found" like in a
  playlist.
- Favourites is drawn as a playlist: pinned first in the sidebar list and the
  playlists grid, fixed name, newest first, picture made from the newest four
  albums. Its page plays, shuffles and exports, but can't be renamed, deleted or
  reordered; removing a track there means unliking it.
- `TrackRow`, `SongRow` and `QueueRow` carry the track id and a liked flag, and one
  source updates every visible heart. Row hearts are `tab-stop: false` like the
  queue's remove button; the menu entry is the keyboard path, and a test checks
  every track menu has it.
- "Love on Last.fm" (off by default) sends love and unlove through the existing
  Last.fm service when connected. The import button reads the account's loved
  tracks and likes the library tracks that match by artist and title (ignoring case
  and accents), then reports matched and unmatched counts. It only adds; it never
  unlikes, and it can run again.

**Home page.**
- The `Page` enum and navigation stop using hard-coded indices, so adding a section
  is a one-place change; tests refer to pages by name. The new sidebar order goes
  into `docs/DESIGN.md` as the owner's decision.
- `ui.start_page` is `home` (default) or `last`. "Last" reopens the last place in
  the navigation history, or Home if it no longer exists.
- Sections are library queries, refreshed when Home opens and after a play is
  recorded, never on Position events. Empty sections are hidden. A Home with
  nothing to show offers "Add music folder..." like the other pages.
  - **Recently played:** play origins, newest first.
  - **Recently added:** albums ordered by their newest added date.
  - **Most played:** plays in a chosen range: last week, last month, last year or
    all time. The same range picker sits in the section's heading on Home and on
    its Show all page, and both follow one setting, `ui.most_played_range`
    (default last month), which is kept after a restart. The ranges are rolling
    periods counted back from now (7, 30 and 365 days), not calendar periods.
  - **Forgotten favourites:** liked tracks with no play in the last 3 months. This
    includes tracks never played since they were liked, once they have been liked
    for 3 months.

**Sleep timer.**
- "Stop after the current track" is a new engine command and flag. While it is set,
  `prepare_next` takes its "no next track" path and an already prepared track is
  cut. At the boundary the engine stops with the queue moved to the next track,
  ready from its start. Repeat One stops instead of repeating. Doing this in the app
  would let a moment of the next track through, so the engine owns it.
- The timer itself is app state, on a Slint timer on the UI loop: when the time is
  up it sets the engine flag, so the track finishes (as decided) and nothing fades.
  It is not saved. Choosing Off, pressing Stop, or the flag firing clears it.
- The moon button sits next to the queue button. While a timer runs it shows in the
  accent with the minutes left, and with "End of this track" while only the flag is
  set. The title section's minimum shrinks by the button's width so
  `smallest_window_keeps_everything_inside` still passes at 720 px.

**Formats.**
- symphonia gains `isomp4`, `aac`, `alac` and `aiff`. The scanner accepts `m4a`,
  `aif`, `aiff` and `aifc`, again checked against lofty's detected type; an MP4
  holding video is skipped.
- `Format` gains M4A and AIFF. Tags are written as MP4 atoms for M4A (covers as
  `covr`) and as ID3v2 for AIFF. Both go through the same copy, read-back check and
  replace as today, and #42's "every untouched key reads back unchanged" check
  covers MP4 freeform atoms.
- The fixture scripts generate the new formats. Gapless tests cover ALAC and AIFF;
  AAC's gapless behaviour is measured and documented.

**Library check.**
- These are read-only queries in `vcmp-library`, and the page never writes or
  removes music files; a test enforces this. The page is a navigation place, so
  Back works.
- **Duplicates:** available tracks grouped by title and artist (ignoring case and
  accents) whose lengths lie within 2 s. "Not duplicates" stores the group's track
  ids; a group that later gains a member shows again.
- **Missing:** title, artist, album, track number and year per track, using #42's
  tag table to tell a real title from the file-name fallback. Missing covers are
  reported per album.

**M3U.**
- Parsing and writing live in `vcmp-library`.
- Export writes UTF-8 `.m3u8` with `#EXTM3U` and `#EXTINF` (seconds, "Artist -
  Title"). Paths are relative to the playlist file when they share a root, and
  absolute otherwise.
- Import reads `.m3u` and `.m3u8` and creates a new playlist named after the file.
  It matches entries by path first (relative paths resolved against the file),
  then by `#EXTINF` artist and title with the closest length. Unmatched entries are
  listed afterwards. An imported playlist is an ordinary playlist with no link back
  to the file.

**First-run wizard.**
- It is shown when the database has no music folders and no `ui.setup_done`
  setting. Finishing or skipping sets the setting, and an existing library sets it
  silently on first start after the update.
- It is a `Modal` with Welcome, Music folders, Look, Online services and Done. Only
  services present in the build are shown, and current defaults are kept. "Run
  setup again" in Settings opens it with the current values.
- Screenshot and test runs with a fresh `VCMP_HOME` need a way to skip it.

## Weaknesses and risks

| Risk | Likelihood | Cost | Mitigation |
|---|---|---|---|
| Everything waits on #42, which is not yet tested | High | Delay | Test #42 first. The sleep timer is the one feature with no tie to it, if waiting gets long |
| Positional migrations: PRs merged out of order collide on a version number, and a dev database that ran an unmerged migration reads as "too new" | Medium | Rework, broken dev databases | Merge schema PRs in the plan's order, rebase and renumber before merging, and test with a throwaway `VCMP_HOME` |
| File-date backfill is wrong for copied libraries or files VCMP already edited (an edit replaces the file, giving both dates the edit's time) | Medium | Odd Recently added | One-time only; documented in `library.md`; new music is exact |
| AAC in MP4 has no gapless info in symphonia: short gaps between AAC tracks | High for AAC | Audible on live albums | Measure, document as a known limit, and look for upstream support |
| MP4 tag writing (freeform atoms, covers) is the least tested lofty path | Medium | File damage | The read-back check refuses to replace a file that reads back wrong; edit_check on real M4A files before merging. Whether the owner's library has M4A files to test with is not known yet |
| Duplicate false positives (live or remastered versions with the same names); the same album in FLAC and MP3 already merges into one album with doubled tracks | Medium | Noise | "Not duplicates"; the finder explains the rule; fixing album grouping by format is out of scope |
| Last.fm import misses tracks whose names differ slightly ("feat.", punctuation); a large loved list takes many pages | Medium | Partial import | Report what didn't match; run in the background; can be repeated |
| Player bar gets tighter at 720 px | Low | Clipped titles | The existing smallest-window test |
| Clearing play history can't be undone | Low | Lost history | Confirmation naming what is lost |
| "Forgotten favourites" is empty for 3 months after the first hearts, and "Most played" is empty until the chosen range holds plays | High | Quiet Home | Empty sections are hidden; the Most played heading and its range picker stay, so another range can be picked |

Left open:
- Whether the Most played row shows albums, tracks or both, and how its Show all page
  looks. Settle this during implementation and record it in `docs/ui.md`.

## Out of scope

- A statistics page. Planned for a later change; the play log is shaped to serve it.
- Opus, WMA, APE and WavPack, and `.m4b` audiobooks and MP4 video files.
- Hearts in the Songs table, the queue and the now playing view (menu entries only
  there); ratings; favourites written to file tags; two-way Last.fm sync (the
  import is a manual button, and loves removed elsewhere are not mirrored).
- Custom sleep lengths and fading out.
- Deleting or moving files from the finder; audio fingerprints; finding missing
  album artist or genre; fixing album grouping across formats.
- Picking up M3U files in music folders automatically; PLS and XSPF; exporting all
  playlists at once.
- Sideways-scrolling Home shelves and recommendations beyond the four sections;
  much-played albums in "Forgotten favourites"; custom date ranges for "Most
  played".
- Showing the wizard to existing users (they can run it from Settings); a
  start-page step in the wizard.
- Pausing play history recording.
- MCP tools for play history and favourites (reading them, or liking through a
  change set). Left for a later change.
