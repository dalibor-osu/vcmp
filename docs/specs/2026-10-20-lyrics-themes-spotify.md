# Lyrics, custom themes and Spotify

Status: agreed
Date: 2026-10-20
Area: vcmp-app (theme, Settings, now playing view, sidebar, Spotify pages),
vcmp-library (lyrics from tags and .lrc files, lyrics cache, playlist schema),
vcmp-integrations (LRCLIB, Last.fm, Discord), vcmp-playback (a second track source),
a new Spotify crate; all three platforms

## Context of the change

`ideas.md` lists three "Future features": Spotify connection and playback, a lyrics
view and custom themes. The v1 specification
([2026-10-17-vcmp-music-player.md](2026-10-17-vcmp-music-player.md)) kept all three
out of scope and left only seams for them:

- **Spotify.** v1 chose in-app playback through librespot and built a track source
  abstraction for it: `TrackSource` / `TrackStream` in
  `crates/vcmp-playback/src/source.rs`, and `SourceKind` plus `TrackRef { source, id }`
  in `track.rs` ([playback.md, Data path](../playback.md#data-path)). `SourceKind`
  has only `Local`. The rest of the app still assumes local tracks: playlists store
  library track ids, and the queue, the songs table and search show scanned rows only
  ([library.md, Playlists](../library.md#playlists)).
- **Lyrics.** Nothing reads them. The tag writer keeps lyrics frames but never reads
  them ([library.md, Writing tags](../library.md#writing-tags)). The now playing view
  is the agreed future home for lyrics: a comment in `ui/now-playing.slint` marks
  the spot, and [ui.md, Now playing view](../ui.md#now-playing-view) says so too.
- **Themes.** `ui/theme.slint` has one `ThemePalette` of 9 colour tokens on `main`. It is
  filled from Rust (`src/theme.rs`) with Catppuccin Latte and Mocha, pink accent and
  red errors. A `ThemeMode` (System / Light / Dark, saved as `ui.theme`) picks which
  one is shown. Contrast tests run on those two palettes only
  ([ui.md, Palettes](../ui.md#palettes)). The v1 invariant holds: no component
  contains a colour, so a theme only has to supply other values.

Things that changed outside the repository since v1 was agreed (checked 2026-10-20):

- librespot still plays Spotify, but only for Premium accounts and only after an
  OAuth login (password login is gone). In 2026 its issue tracker reports OAuth and
  device-visibility breakage.
- Spotify's Web API is effectively closed to an app like VCMP. Development Mode
  apps are capped at 5 users and the app owner needs Premium (February 2026).
  Extended quota is only for registered businesses with at least 250k monthly users
  (Spotify developer blog, 2026-07-23).
- LRCLIB (lrclib.net) is a free lyrics database that needs no key. It serves both
  plain and time-synced (LRC) lyrics.

## User request

Analyze and brainstorm the future features in `ideas.md` (Spotify connection and
playback, a lyrics view, custom themes): work out what it would take to implement
all three, and plan them.

For Spotify, VCMP should read, play, like and save. Full two-way editing of Spotify
playlists would be nice in the future. For themes, the dark/light switch should go
away; VCMP should simply offer different themes.

## Decisions

### Which of the three future features should this spec plan for implementation? (multi-select)

| Option | What it means |
|---|---|
| Lyrics view | Self-contained, low risk, uses the existing now playing seam. Best value for effort. |
| Custom themes | Small code surface (9 tokens), but touches the owner's design direction and contrast rules. |
| Spotify | Largest by far and on unstable ground (librespot unofficial, Web API capped at 5 users). May end up a personal-build-only feature. |

**Answer: Lyrics view, Custom themes, Spotify** (all three)

### How should VCMP reach Spotify's audio and catalogue (search, playlists, liked songs)?

| Option | What it means |
|---|---|
| librespot for both (Recommended) | librespot plays audio; its login token also calls the Web API, as ncspot and spotify-player do. No developer app, works for any Premium user; fully unofficial, Spotify can break or block it. |
| librespot + user's own app | Audio via librespot; catalogue via a Spotify developer app each user registers and pastes a Client ID for. Catalogue access is sanctioned; setup is clumsy and audio is still unofficial. |
| Remote-control Spotify Connect | No audio in VCMP; it controls the official Spotify app via the Web API (user's own app). Most official, but no mixed queues and it reverses the v1 seam decision. |
| Personal build only | Like option 1, but compiled only behind a build flag for the owner, never in released archives. Avoids shipping ToS risk to others; nobody else gets the feature. |

**Answer: librespot for both**

### Where does Spotify content appear in VCMP?

| Option | What it means |
|---|---|
| Own sidebar section (Recommended) | A Spotify group: search, liked songs, saved albums, Spotify playlists. Clear what is local and what streams; local views stay as fast and offline as today. |
| Merged into the library | Saved Spotify albums and artists show in the Albums/Artists grids next to local ones. One library feel; grids depend on the network, duplicates of albums you also own. |
| Search only | Spotify results only appear in global search, playable into the queue. Smallest UI; no browsing of your Spotify library. |

**Answer: Own sidebar section**

### Can Spotify tracks be mixed with local tracks in the queue and in VCMP playlists?

| Option | What it means |
|---|---|
| Queue and playlists (Recommended) | Up next and VCMP playlists hold both kinds. The point of in-app playback; needs a playlist schema change for non-local entries. |
| Queue only | Mixed Up next and contexts, but VCMP playlists stay local. No schema change; a playlist can't keep a Spotify song. |
| No mixing | Playing Spotify replaces the queue with Spotify-only context. Simplest; loses the main benefit of librespot. |

**Answer: Queue and playlists**

### How far does VCMP manage the user's Spotify library?

| Option | What it means |
|---|---|
| Read and play only (Recommended) | Browse and play Spotify playlists, liked songs and albums; changes are made in VCMP playlists, never written to Spotify. Fewer write calls on an unofficial token; no two-way sync. |
| Like and save too | Also heart tracks and save albums to the Spotify library. Small write surface users expect; more calls that a token block can break. |
| Full two-way | Also edit Spotify playlists from VCMP. Feels native; largest API surface, conflict handling, most exposure. |

**Answer (Other): "Let's do Read, play, like and save. But having a full two-way in the future would be nice."**

### How is Spotify offered in released builds?

| Option | What it means |
|---|---|
| Off until connected (Recommended) | In every build, but nothing runs or appears until the user connects in Settings, with a note that it is unofficial and needs Premium. Fits the network policy; honest about risk. |
| Experimental flag | Hidden behind an 'Experimental features' switch in Settings first. Signals instability; one more setting to explain. |
| Cargo feature, off in releases | Compiled only into builds made with a feature flag; released archives lack it. No ToS exposure for VCMP releases; users must build it themselves. |

**Answer: Off until connected**

### Do Spotify plays go through VCMP's Last.fm scrobbling and Discord presence like local ones?

| Option | What it means |
|---|---|
| Yes, both (Recommended) | Same behaviour for every track; Spotify gives covers, so no MusicBrainz lookup. Users who also linked Last.fm inside Spotify may get double scrobbles, so a note in Settings. |
| Discord yes, Last.fm no | Leaves scrobbling to Spotify's own Last.fm link. No duplicates; users without that link lose Spotify scrobbles. |
| Separate switch | A 'Scrobble Spotify plays' toggle under Last.fm, default on. Covers both cases; one more setting. |

**Answer: Separate switch**

### Where do lyrics come from? (multi-select)

| Option | What it means |
|---|---|
| Embedded tags | USLT/SYLT in ID3, LYRICS in Vorbis comments, read at scan time. Offline, already in many tagged libraries; mostly unsynced. |
| Sidecar .lrc files | song.lrc next to song.mp3, synced. Common among enthusiasts; one more file per track for the scanner and watcher. |
| LRCLIB online | Free, no key, synced lyrics; also covers Spotify tracks. Sends title/artist/album/duration to lrclib.net, so it needs a switch per the network policy. |
| Spotify's own lyrics | For Spotify tracks only, via librespot's unofficial lyrics endpoint. Good coverage; more unofficial surface, Musixmatch-licensed text. |

**Answer: Embedded tags, Sidecar .lrc files, LRCLIB online**

### How are lyrics shown?

| Option | What it means |
|---|---|
| In now playing, synced (Recommended) | A lyrics column in the now playing view, the current line highlighted and kept in view, click a line to seek; plain lyrics just scroll. Uses the agreed seam; one place for it. |
| Own side panel | A lyrics panel like the queue panel, usable over any page. Visible while browsing; competes with the queue for the same space. |
| Both | Now playing column plus a side panel. Most flexible; two layouts to build and test. |

**Answer: In now playing, synced**

### What happens to lyrics fetched from LRCLIB?

| Option | What it means |
|---|---|
| Kept in VCMP's data only (Recommended) | Stored per track in the database (or cache), never written to files, like the Discord cover lookup. Music files stay untouched; lyrics don't travel with the files. |
| Offer to save as .lrc | Cached as above, plus a 'Save to file' action that writes a sidecar .lrc. User decides per track; a write into the music folders. |
| Write into tags | Saved into the file's lyrics tag through the safe tag writer. Travels with the file; modifies music files, and synced text fits poorly in USLT. |

**Answer: Kept in VCMP's data only**

### When a track has lyrics from more than one place, which wins?

| Option | What it means |
|---|---|
| Synced first, local first (Recommended) | .lrc, then embedded synced, then LRCLIB synced, then embedded plain, then LRCLIB plain. A synced view whenever one exists; online can override a local plain text. |
| Local always first | .lrc, embedded, then LRCLIB only if nothing local exists. The user's own files are never overridden; plain local text hides synced online lyrics. |

**Answer: Synced first, local first**

### When does VCMP ask LRCLIB for lyrics?

| Option | What it means |
|---|---|
| Auto, behind a switch (Recommended) | A Settings switch 'Find lyrics online' (off by default); when on, the playing track is looked up if nothing synced is local. Lyrics are just there; off by default per network policy. |
| Auto, on by default | Same, but the switch starts on, like the Discord presence. Works out of the box; sends track info without the user choosing it. |
| Only on request | A 'Find lyrics online' button in the empty lyrics column. Nothing sent without a click; lyrics never appear by themselves. |

**Answer: Auto, behind a switch**

### What is a custom theme in VCMP?

| Option | What it means |
|---|---|
| Presets + theme files (Recommended) | Built-in Catppuccin flavours (Latte, Frappe, Macchiato, Mocha) with an accent picker, plus user theme files (the 9 tokens in a TOML file) loaded from the data folder. Covers most users and power users; no editor UI to build. |
| Presets and accent only | Flavours and accent picker, no user files. Smallest, every combination contrast-tested ahead; no truly own colours. |
| In-app theme editor | Presets plus colour pickers per token with live preview and import/export. Most approachable for own themes; Slint has no colour picker, so one must be built. |

**Answer: Presets + theme files**

### How does a theme choice combine with today's System / Light / Dark setting?

| Option | What it means |
|---|---|
| A light and a dark slot (Recommended) | Settings picks a theme for light and one for dark; System switches between them as now. Keeps following the OS; two pickers to explain. |
| One theme replaces the mode | Choosing a theme fixes it; System/Light/Dark become the three default entries in the same list. One simple list; a custom theme no longer follows the OS. |

**Answer (Other): "I think we should get rid of dark/light switch and just provide different themes."**

### A user theme file fails the WCAG AA contrast checks. What does VCMP do?

| Option | What it means |
|---|---|
| Load it, show a warning (Recommended) | The theme applies, Settings names the failing pairs. User's choice respected; an unreadable theme is possible but explained. |
| Refuse it | The file is listed as invalid and not applied. Accessibility guaranteed; frustrating for someone who wants a low-contrast look. |
| Fix it automatically | Failing colours are lightened or darkened until they pass, as Latte's pink is today. Always readable; the result differs from what the file says. |

**Answer: Load it, show a warning**

### With the light/dark switch gone, is there still a way to follow the OS dark mode?

| Option | What it means |
|---|---|
| Keep a 'System' entry (Recommended) | The theme list starts with 'System (Latte / Mocha)', the default, which follows the OS; every other entry is fixed. New users still match their OS; one special entry in the list. |
| No OS following | Every entry is a fixed theme; new installs start on Mocha. Simplest model; a light-mode OS user starts dark until they change it. |
| Default follows once | No System entry; on first start VCMP picks Latte or Mocha from the OS, then it stays fixed. Good first impression; doesn't track later OS changes. |

**Answer: Default follows once**

### In which order should the three features be built?

| Option | What it means |
|---|---|
| Themes, lyrics, Spotify (Recommended) | Smallest and safest first; Spotify last, when the source seam and playlist schema change get full attention. Usable wins early; Spotify waits longest. |
| Lyrics, themes, Spotify | The most visible listening feature first. Same risk profile; themes slip a little. |
| Spotify first | Tackle the biggest unknown while motivation is high. Early proof librespot works; the two easy features wait on the hard one. |

**Answer: Themes, lyrics, Spotify**

### What can a user theme file change?

| Option | What it means |
|---|---|
| Colours only (Recommended) | The 9 palette tokens, a name, and light/dark kind (for icons and the OS title bar). Fits the 'themes only swap values' invariant; no layout breakage. |
| Colours and font | Also a font family from the system. More personal; text metrics change, so layouts and the 720 px rule need testing per font. |
| Colours, font, radii | Also corner radii and spacing scale. Closest to a full skin; many more combinations that tests can't cover. |

**Answer: Colours only**

## High-level plan

Three workstreams, built in the agreed order. Each one ships on its own and leaves
the app releasable.

**1. Themes.** The Light / Dark / System switch becomes a single theme list. It
offers the four Catppuccin flavours (Latte, Frappe, Macchiato, Mocha) with an accent
picker, then any user theme files. Settings shows a contrast warning for a user theme
that fails WCAG AA. The saved `ui.theme` value is migrated, and DESIGN.md and ui.md
record the new model. Depends on nothing.

**2. Lyrics.** Lyrics are read from embedded tags and sidecar `.lrc` files, with an
optional LRCLIB lookup behind an off-by-default "Find lyrics online" switch. Fetched
lyrics are cached in VCMP's data. The now playing view gets a lyrics column: synced
lyrics highlight and follow the current line, and a click on a line seeks to it;
plain lyrics scroll. Depends on nothing in workstream 1 except the theme tokens it
already uses. The LRCLIB lookup is written source-neutral, so Spotify tracks get
lyrics in workstream 3 without changes.

**3. Spotify.** Built in this order inside the workstream:

1. **Identity seam.** `SourceKind::Spotify`, and a playlist and queue model that
   holds non-local entries. A versioned schema migration stores a metadata snapshot
   of every Spotify track VCMP keeps. Nothing visible yet; local behaviour unchanged.
2. **Connection and audio.** OAuth login through librespot, credentials in the OS
   keyring, a Spotify `TrackSource` feeding the existing engine. Gapless, resampling,
   output and media controls stay the engine's.
3. **Spotify section.** A sidebar group, present only while connected: Search,
   Liked songs, Albums, Playlists. Its pages are built from the existing grid and
   track list components. Spotify tracks can be added to Up next and to VCMP
   playlists.
4. **Like and save.** Hearting a track, and saving or removing an album, in the
   Spotify library.
5. **Integrations.** Discord uses Spotify's cover. Last.fm gets a "Scrobble Spotify
   plays" switch, on by default. LRCLIB lyrics work for Spotify tracks.

Every workstream updates ui.md, library.md, playback.md and manual-testing.md as it
changes them; workstream 3 also updates CLAUDE.md's stack and crate tables.

## Architecture decisions

### Themes

- **A theme is a name, a kind (light or dark) and every `ThemePalette` token.**
  Built-in themes stay in Rust next to their contrast tests; user themes are TOML
  files in `<data dir>/themes/`. A token added to the palette later gets a default
  derived from the others, so older theme files keep loading; a file missing one of
  today's tokens is listed as invalid. The kind sets Slint's `Palette.color-scheme`, so std widgets
  (scroll bars, `AboutSlint`) and the OS title bar match. Reason: one shape for both
  sources, and the v1 invariant "themes only swap values" survives unchanged.
- **The `Theme` global takes one palette and the kind from Rust;** `ThemeMode`,
  `light-colors` and `dark-colors` go. Reason: with no mode, the choice lives in one
  place, Rust, which also validates files.
- **`ui.theme` holds a theme id** (`latte`, `mocha`, ..., or `file:<name>`) plus the
  accent for built-ins. Migration: `light` becomes Latte and `dark` becomes Mocha.
  `system` and a fresh install get Latte or Mocha from the OS once, then stay fixed.
  A user theme whose file is gone falls back to the built-in of its kind, and
  Settings says so. Reason: the owner's "default follows once"; nobody's look
  changes on upgrade.
- **The accent picker offers the Catppuccin accents on built-in flavours only.**
  On light flavours an accent is darkened in hue until it reaches 4.5:1, as pink is
  today. The contrast tests cover every flavour and accent pair. User files define
  their own accent. Reason: every built-in combination stays guaranteed AA.
- **The contrast check moves from test-only code into the app,** and both the tests
  and the Settings warning use it. Reason: one definition of "passes".
- **Theme handling stays in vcmp-app.** Reason: it is UI-only, and core crates must
  not depend on Slint.

### Lyrics

- **One lyrics model:** the source, optional timed lines, optional plain text. The
  LRC parser handles `[offset:]`, several timestamps on one line, and ignores
  word-level tags. Embedded text that is LRC-formatted counts as synced. Reason:
  taggers often put LRC text into USLT or LYRICS.
- **Local lyrics are read when a track starts playing,** not at scan time. Read
  through lofty: SYLT and USLT in ID3v2, LYRICS in Vorbis comments, plus the
  `.lrc` file of the same name. Reason: no schema change or rescan, an `.lrc` added
  later is found without the watcher, and the database doesn't grow by every
  song's text. Cost: lyrics can't be searched (out of scope).
- **The lyrics lookup lives in vcmp-library and LRCLIB in vcmp-integrations.**
  LRCLIB gets an exact match on title, artist, album and duration (within a
  tolerance), sends a VCMP User-Agent and makes one request at a time. Results,
  including "not found", are cached in SQLite and keyed by the track's stable
  identity: the library track id (which survives re-links) or the Spotify track
  id. Misses are retried after 30 days, failures on a later play, as the Discord
  cover lookup does. Reason: the same proven pattern; no lookup per play.
- **The priority order is fixed:** `.lrc`, embedded synced, LRCLIB synced, embedded
  plain, LRCLIB plain. LRCLIB is asked only when nothing synced is local.
- **Sync uses the engine's position events** (every 250 ms), interpolated between
  events while playing. Line changes follow `Theme.reduced-motion`: they scroll
  smoothly, or jump with reduced motion on. The current line uses the accent: it
  marks what is playing now, the accent's existing job.

### Spotify

- **A new crate, `vcmp-spotify`,** owns the librespot session, the Spotify
  `TrackSource` and the Web API client. It depends on vcmp-playback (for the trait),
  never on vcmp-app or Slint. Reason: librespot brings tokio and a large dependency
  tree that vcmp-integrations' small blocking clients don't need, and this keeps it
  in one place that a build can leave out.
- **librespot is used below its player.** VCMP takes librespot's session, metadata
  and decrypted audio file, skips Spotify's file header, and decodes the Ogg Vorbis
  itself with symphonia. librespot's own player and output are not used. Reason:
  gapless joining, resampling, device selection and the allocation-free callback
  stay VCMP's, for Spotify and local tracks alike.
- **`TrackSource` gets a prepare hint** for the next queue entry, and a Spotify
  stream downloads in the background. `open` returns once the first audio is there.
  Reason: the engine thread opens the next track only seconds before the boundary.
  A network fetch done inside `open` would leave gaps and stall command handling.
- **`TrackRef` for Spotify:** source `spotify`, id the track URI
  (`spotify:track:<id>`). The saved queue already writes the source by name, so it
  needs no format change.
- **Non-local entries in the database:** a table of remote tracks (source, id,
  title, artists, album, duration, cover URL, when fetched). Playlist entries then
  point to a library track or a remote track, never both, enforced by a CHECK.
  The change comes in a versioned migration; existing playlists are untouched.
  Reason: playlists and the queue render offline and without API calls. The same
  identity serves a later two-way sync.
- **Unavailable Spotify entries** (disconnected, offline, removed from Spotify,
  region-locked) are shown greyed and skipped, as unplugged local tracks are.
- **The login is librespot's OAuth in the browser** with a local redirect, waiting
  in the Settings row like Last.fm's connect. librespot's reusable credentials go
  into the OS keyring through the existing keyring dependency. Reason: same UX and
  storage as Last.fm, and no secret in SQLite.
- **The catalogue comes from the Web API with librespot's token** (read the library,
  playlists and search; write liked tracks and saved albums). A 429 is backed off
  and shown as "Spotify is busy". A Free account gets a clear "needs Premium" state.
- **Off until connected.** Nothing Spotify-related runs, appears or talks to the
  network until the user connects in Settings. The Settings row says plainly that
  the connection is unofficial and needs Premium. Reason: the network policy, and
  honesty about the risk.
- **Spotify artwork** is cached in the cache directory like other fetched images.
  The Discord presence uses the cover URL directly. MPRIS, SMTC and Now Playing get
  the cached file.
- **Scrobbling:** a `lastfm.scrobble_spotify` switch, on by default, decides whether
  Spotify plays enter the existing scrobble queue.

## Weaknesses and risks

| Risk | Likelihood | Cost | Mitigation |
|---|---|---|---|
| Spotify blocks or changes what librespot depends on (login, audio keys, the token's Web API access) | Medium-high over the feature's life | High: Spotify stops working for everyone at once | Isolated crate, off until connected, a clear error state; local playback never depends on it; follow librespot releases |
| Shipping librespot conflicts with Spotify's terms; account or app consequences | Medium | Medium-high | Stated plainly in Settings; nothing on by default; the owner may still fall back to a build flag (a later decision) |
| Network stalls cause gaps or block the engine thread | Medium | Medium: gaps, unresponsive controls | Prepare hint and background download; tests with a slow fake Spotify source |
| librespot's tokio runtime and dependency tree inflate build time and binary size | High | Low-medium | Separate crate; measure size before and after; trim librespot features |
| The playlist schema change breaks existing playlists, or an older VCMP opens the migrated database | Low | High: lost playlists | Versioned migration with tests on a copy of real data; refuse to open a newer schema in an older build (check the current behaviour first) |
| Double scrobbles when Spotify itself scrobbles to Last.fm | Unknown: whether Spotify counts librespot plays is not known | Low | The switch, and a note under it; check with a real account during manual testing |
| LRCLIB is down, slow or retired | Low-medium | Low: no online lyrics | Cached results, local sources first, failures retried later |
| LRCLIB returns lyrics for the wrong version (live, remaster) | Medium | Low | Exact match with a duration tolerance, no fuzzy search |
| Synced lyrics drift by up to 250 ms | Medium | Low | Interpolation between position events; `[offset:]` honoured |
| A user theme makes the UI unreadable | Medium | Low: the user chose it | Contrast warning that names the failing pairs; built-in themes always one click away |
| Removing the System entry: users who switch their OS between light and dark lose automatic following | Medium | Low | The owner's decision; record it in DESIGN.md; reconsider if users ask |
| Light-flavour accents darkened for contrast may stop looking like their Catppuccin colour | Medium | Low | Leave out an accent that can't pass recognisably, rather than shipping it pale |

**Questions deliberately left open**

- Two-way editing of Spotify playlists: wanted later. The remote track identity is
  designed for it, but conflict rules aren't designed.
- Whether Spotify's global search results join VCMP's global Search page. For now
  the Spotify section has its own search.
- Spotify audio quality: VCMP plays 320 kbps where the account allows; no setting
  yet.
- Whether released archives keep Spotify if librespot breaks for a long time. The
  owner decides when it happens.

## Out of scope

- Spotify's own lyrics (Musixmatch through librespot), and writing lyrics into
  files or `.lrc` files.
- Editing lyrics, lyrics search, word-by-word karaoke highlighting, and a lyrics side
  panel outside the now playing view.
- An in-app theme editor or colour picker, and themes that change fonts, radii or
  spacing.
- Following the OS light/dark setting after the first start.
- Editing, creating or reordering Spotify playlists (later two-way sync), following
  artists, podcasts and audiobooks, Spotify Connect (VCMP as a target or as a remote),
  offline downloads of Spotify tracks, and Spotify Free accounts.
- Merging Spotify albums and artists into the local Albums and Artists grids.
