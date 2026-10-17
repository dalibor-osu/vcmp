# VCMP music player

Status: agreed
Date: 2026-10-17
Area: whole application (new project) - audio engine, library/database, Slint UI,
web integrations, packaging for Windows 10+, Linux and macOS

## Context of the change

Nothing is built yet. The directory `/home/dalibor/repos/vcmp` contains only
`ideas.md`, the author's feature and technology notes. There is no source code, no
`CLAUDE.md`/`README.md` and no git repository. This document turns `ideas.md` into
an agreed specification for the first release (v1). It also records the choices
`ideas.md` left open (UI framework, audio stack, metadata model, integrations) and
the extra scope the author added during the session.

## User request

Plan a music player app called VCMP and get everything ready for building it. The
author's ideas, including some technical solutions, are in `ideas.md`, and it is far
from complete.

VCMP is a cross-platform (Windows 10+, Linux, macOS) music player for locally stored
music. It aims for a modern look, ease of use, good UX and every feature a music
enjoyer needs. It is written in Rust. Supported formats are mp3, wav, flac and ogg,
with more in the future. The UI framework is still to be decided. The database is
fully local and needs no running server (SQLite is the idea). A settings checkbox
turns automatic updating and detection of changes in the source directories on or
off. The output device is selectable in settings.

The user picks multiple source directories that are scanned and used for the
library. Subdirectories are included by default, and a checkbox controls that. Users
can make playlists. Each playlist has a name and any number of songs. Its thumbnail
is generated automatically from the first 4 album covers in the playlist, and the
user can override it with a custom image. The library has several views: an album
view (grid of covers with artist and album name), an artist view (grid of artists
with photos and names; the user can override a photo, or have it searched on the web
when they explicitly ask) and a playlist view (grid of the user's playlists). Shuffle
play has two settings: true shuffle and less repeats. VCMP supports Last.fm
scrobbling and has a light and a dark theme.

Future features: Spotify connection and playback, a lyrics view, custom themes.

Added after the spec was agreed, while setting the design direction: the author
likes clean animations, and an option for reduced motion would be nice for users
who dislike them. The visual direction itself lives in `docs/DESIGN.md`.

Added later, during implementation: v1 should also have Discord Rich Presence. It
should be fairly easy to implement. It is on by default for now; a future setup
wizard that introduces the user to the app should mention it. The presence shows the
album cover when the album is tagged; otherwise it should show the app logo, which
does not exist yet, so for now it shows nothing.

Added after that: the presence works, but most songs in the owner's library carry no
MusicBrainz tag, so most albums showed no cover. Asked how to approach this, the
owner chose an automatic MusicBrainz lookup, with a test on about 20 of their albums
first.

## Decisions

### How should the spec scope the first release of VCMP?

| Option | What it means |
|---|---|
| All core ideas, seams for future (Recommended) | v1 ships every non-future feature in ideas.md; Spotify/lyrics/themes get only architectural hooks. Bigger v1, but no rework later. |
| Thin MVP, then milestones | v1 = library scan + playback + album view; playlists, artists, shuffle modes, Last.fm follow in milestones. Faster to something usable, more sequencing to plan. |
| Everything, incl. future features | Spotify, lyrics and custom themes fully designed now. Most complete plan, but designs things far from being built. |

**Answer: All core ideas, seams for future**

### Which of these, not in ideas.md, should be part of the plan? (multi-select)

| Option | What it means |
|---|---|
| OS media controls | Media keys + system now-playing widget (MPRIS on Linux, SMTC on Windows, MPNowPlaying on macOS). Expected by users, three platform backends. |
| Gapless playback | No silence between tracks of an album. Matters for live/concept albums; constrains the audio pipeline design. |
| ReplayGain normalization | Even loudness across tracks using tags. Cheap if planned early, awkward to retrofit. |
| M3U import/export | Playlists portable to/from other players. Small effort, but path handling across OSes needs care. |

**Answer: OS media controls, Gapless playback**

### Which UI framework should VCMP use?

| Option | What it means |
|---|---|
| Tauri 2 + web frontend (Recommended) | Rust core, UI in TypeScript/CSS in the system webview. Easiest modern look, large cover grids and future custom themes (CSS); UI code is not Rust and Linux WebKitGTK can be slower. |
| Slint | Rust-native declarative UI, fast, small binaries. Good styling, but fewer ready-made components; GPLv3 or royalty-free license terms apply. |
| iced | Pure Rust, Elm-style architecture (used by COSMIC desktop). Everything in one language, but theming and virtualized grids need more custom work. |
| Dioxus desktop | React-like UI written in Rust, rendered in a webview. All-Rust code with CSS styling, but a younger ecosystem with more churn. |

**Answer: Slint**

### Under which license will VCMP be released? (Slint's license depends on it)

| Option | What it means |
|---|---|
| Open source, GPLv3 (Recommended) | Uses Slint under GPLv3, no attribution badge needed; all VCMP code must stay GPL-compatible. |
| Open source, permissive (MIT/Apache) | VCMP code is permissive; Slint used under its royalty-free license, which requires a visible 'Made with Slint' attribution. |
| Closed source, free | Slint royalty-free license (desktop is allowed) with attribution; no source published. |

**Answer: Open source, permissive (MIT/Apache)**

### How should VCMP be distributed to users?

| Option | What it means |
|---|---|
| GitHub Releases installers (Recommended) | CI builds MSI/exe (Windows), dmg (macOS), AppImage + .deb (Linux). Simple; macOS signing/notarization still needed to avoid Gatekeeper warnings. |
| Releases + Flatpak/winget/Homebrew | Same plus package-manager channels. Easier installs and updates for users, more packaging to maintain. |
| Personal use, build from source | No installers or signing in scope for now. Cheapest, but users must have a Rust toolchain. |

**Answer: GitHub Releases installers**

### Which audio stack should the playback engine be built on?

| Option | What it means |
|---|---|
| symphonia + cpal, own engine (Recommended) | Pure Rust decode + output; one continuous output stream makes gapless and device selection straightforward. We own the mixing/resampling code. |
| rodio | Higher-level wrapper over symphonia + cpal, quick start. Gapless and switching devices mid-playback are awkward; likely outgrown. |
| GStreamer | Mature, gapless via playbin, huge format list. Heavy native runtime to bundle on Windows/macOS, complicates installers. |
| libmpv | Robust playback and formats with gapless built in. External C library to ship per platform; less control over output. |

**Answer: symphonia + cpal, own engine**

### What is the source of truth for track metadata (title, artist, album...)?

| Option | What it means |
|---|---|
| File tags, read-only (Recommended) | DB is a rebuildable cache of the tags; VCMP never writes music files. Simplest and safest; fixing bad tags needs an external tagger. |
| File tags + in-app tag editor | VCMP can edit and write tags back to files. Convenient, but writing 4 formats safely is real work and risks corrupting files. |
| Tags + DB-only overrides | Edits stored in the DB layered over tags, files untouched. Safe, but edits are invisible to other players and lost if the DB is. |

**Answer: File tags + in-app tag editor**

### How should tracks be grouped into albums?

| Option | What it means |
|---|---|
| Album artist + album title (Recommended) | Tag-based; falls back to track artist when album artist is missing, MusicBrainz album ID used when present. Handles compilations; bad tags split albums. |
| By folder | Each folder is an album. Robust to messy tags, wrong for multi-disc folders or loose-file folders. |
| Album title only | Simplest, but merges different albums with the same name (e.g. 'Greatest Hits'). |

**Answer: Album artist + album title**

### How should VCMP identify a track when its file is moved or renamed?

| Option | What it means |
|---|---|
| Path + re-link by tags (Recommended) | Path is the key; a vanished file reappearing elsewhere with matching tags+duration is re-linked, keeping playlists and play counts. Cheap; rare false matches. |
| Path only | Move = delete + new track; playlists and stats lose it. Simplest, but reorganizing folders breaks playlists. |
| Audio content hash | Hash of the audio stream survives moves and tag edits. Most robust, but the first scan reads every file fully (slow on big libraries). |

**Answer: Path + re-link by tags**

### What happens to a playlist entry whose file is gone (deleted, drive unplugged, source dir removed)?

| Option | What it means |
|---|---|
| Keep, shown as unavailable (Recommended) | Entry stays greyed out and is skipped in playback; comes back when the file does. Safe for external drives; playlists can accumulate dead entries. |
| Remove automatically | Playlists always clean, but unplugging a drive silently empties playlists. |

**Answer: Keep, shown as unavailable**

### Where should 'search artist photo on the web' get images from?

| Option | What it means |
|---|---|
| fanart.tv via MusicBrainz (Recommended) | Artist resolved to a MusicBrainz ID, images from fanart.tv. Purpose-built, good quality; needs a project API key and an MBID match. |
| Deezer public API | No key, excellent coverage by artist name. API terms are restrictive and name-only matching picks wrong same-name artists. |
| Wikidata / Wikimedia Commons | Free, openly licensed images via MusicBrainz -> Wikidata. Weak coverage for smaller artists; attribution needed. |

**Answer: fanart.tv via MusicBrainz**

### When the user asks for an artist photo search, what do they get?

| Option | What it means |
|---|---|
| Pick from candidates (Recommended) | A dialog shows several found images, the user picks one. One extra click, avoids wrong-artist photos. |
| First result applied | Fastest, but a wrong match must be noticed and overridden by hand. |

**Answer: Pick from candidates**

### Albums without embedded or folder cover art:

| Option | What it means |
|---|---|
| Also fetch on request (Recommended) | Same explicit-action model as artist photos, from Cover Art Archive. Consistent and fills gaps; one more web integration. |
| Local art only | Embedded tags and cover.jpg/folder.jpg only, otherwise a placeholder; user can set an image by hand. No network for covers. |

**Answer: Also fetch on request**

### What should 'less repeats' shuffle mean (vs 'true shuffle' = independent random picks)?

| Option | What it means |
|---|---|
| No repeats + spread artists (Recommended) | Every track plays once before any repeats, and the order avoids the same artist/album back to back. Feels most 'random' to people; slightly more logic. |
| No repeats only | Plain shuffled permutation of the queue, each track once per cycle. Simple; same-artist clusters still happen. |
| History-weighted | Tracks played recently (across sessions) get lower odds. Good for huge libraries on endless shuffle; needs play history and tuning. |

**Answer: No repeats + spread artists**

### How should the play queue work?

| Option | What it means |
|---|---|
| Context + 'Up next' (Recommended) | Playing an album/playlist sets the context; 'Play next'/'Add to queue' go to a manual Up-next list played first. Spotify-like, familiar; two lists to show. |
| Single editable queue | Playing something replaces one queue the user can reorder and append to (foobar/Winamp-like). Simple model, easy to clobber by accident. |

**Answer: Context + 'Up next'**

### What should the main window layout be?

| Option | What it means |
|---|---|
| Sidebar + content + player bar (Recommended) | Left nav (views, playlists), main grid/detail area, bottom now-playing bar, toggleable queue panel on the right. Familiar desktop pattern, scales to more views. |
| Top tabs + player bar | Views as tabs across the top, more horizontal room for grids. Playlists are one click further away. |
| Now-playing centric | Big cover/now-playing screen is home, library is a drawer. Pretty for listening, slower for browsing large libraries. |

**Answer: Sidebar + content + player bar**

### Which views/features besides the album, artist and playlist grids belong in v1? (multi-select)

| Option | What it means |
|---|---|
| Global search | One search box over tracks, albums, artists and playlists. Almost essential for larger libraries. |
| All songs table | Sortable list of every track (title, artist, album, duration...). Power-user staple; needs a fast virtualized table. |
| Full-screen now playing | Large cover art + controls; also where lyrics would later live. |
| Folder browser | Browse the source directories as a file tree. Helps with badly tagged files. |

**Answer: Global search, All songs table, Full-screen now playing, Folder browser** (all four)

### How should future Spotify support fit in, so v1 leaves the right seam?

| Option | What it means |
|---|---|
| Playback inside VCMP (Recommended) | Spotify audio decoded in-app (librespot, Premium only) and fed into our engine; v1 gets a 'track source' abstraction. Unified queue/playlists; librespot is unofficial and may break. |
| Remote-control Spotify Connect | VCMP controls the official Spotify app/device via Web API; v1 gets a 'player backend' abstraction. Officially supported; mixed local+Spotify queues impossible. |
| Decide later, no seam | v1 ignores Spotify entirely. Least up-front work; likely refactor of queue/player when it comes. |

**Answer: Playback inside VCMP**

### Which UI languages should v1 support?

| Option | What it means |
|---|---|
| English, i18n-ready (Recommended) | Ship English only, but all strings go through Slint's translation (gettext) from day one. Adding Czech etc. later is just a .po file. |
| English + Czech in v1 | Two languages from the start. Forces i18n discipline early; every UI string translated twice. |
| English only, hardcoded | No i18n layer. Fastest now, costly to retrofit. |

**Answer: English, i18n-ready**

### How does VCMP authenticate to Last.fm (needs an API key + shared secret)?

| Option | What it means |
|---|---|
| Project key in release builds (Recommended) | Official key injected at CI build time, not in the repo; users just click 'Connect'. Common practice for open-source players; the secret is extractable from binaries. |
| User supplies own key | Settings ask for the user's own API key/secret. Nothing secret shipped; much worse UX. |

**Answer: Project key in release builds**

### How far should the v1 tag editor go?

| Option | What it means |
|---|---|
| Single + batch, incl. cover (Recommended) | Edit one track or many at once (e.g. set album artist for a whole album), including embedded cover art. Covers the real use case of fixing albums; more UI states. |
| Single track, text fields only | One track at a time, common text tags. Smallest, but fixing a whole album is tedious. |
| Full editor + online lookup | Batch editing plus filling tags from MusicBrainz. Most powerful; close to building a separate tagger app. |

**Answer: Single + batch, incl. cover**

### How do users learn about new versions?

| Option | What it means |
|---|---|
| Check + notify (Recommended) | On start (can be disabled) VCMP checks GitHub Releases and shows a 'new version' link. Cheap; user downloads the installer manually. |
| Self-updater | Downloads and installs updates in-app. Best UX, but signing, per-OS install logic and rollback are substantial work. |
| Nothing in-app | Users watch GitHub themselves. Zero network calls, users stay on old versions. |

**Answer: Check + notify**

### Add a 'reduced motion' setting to the agreed v1 spec? (asked after agreement)

| Option | What it means |
|---|---|
| Yes, add to v1 spec (Recommended) | Settings toggle (also follows the OS reduce-motion preference where available) recorded in the spec as a small scope addition. |
| Only note in DESIGN.md | Stays a design wish; the spec is unchanged and it may be dropped from v1. |

**Answer: Yes, add to v1 spec**

### Discord Rich Presence (asked after agreement): on or off by default?

| Option | What it means |
|---|---|
| Off, opt-in in Settings (Recommended) | Nothing about your listening leaves the machine until you switch it on. Respects privacy; fewer people discover the feature. |
| On by default | Works out of the box for Discord users, like most players. Broadcasts what you play without being asked. |

**Answer: "For now, make it on by default. It would be nice to add some kind of setup wizard in the future that would introduce the user to the app and mention this as well."**

### Discord Rich Presence: which picture should the presence show?

| Option | What it means |
|---|---|
| Album cover when tagged (Recommended) | Cover Art Archive URL for albums with a MusicBrainz release ID, else no picture. Discord fetches the image itself, so nothing is uploaded. |
| No picture | Text only: track, artist, album and elapsed time. Simplest; looks plainer in Discord. |
| Upload covers somewhere | Every album gets its cover shown. Needs an image host and sends your covers off the machine. |

**Answer: "Let's go with cover when tagged and when not, it should show the app logo. Logo is non existent yet, so let's show nothing for now and add this later."**

### Discord Rich Presence: where does the Discord application ID come from?

| Option | What it means |
|---|---|
| Build it configurable (Recommended) | The ID comes from a build-time setting, like the Last.fm key; without it the feature is hidden. You create the app later in Discord's developer portal. |
| I'll create it now | You create the 'VCMP' application at discord.com/developers and paste its Application ID here to put into the build. |

**Answer: I'll create it now**

### How should VCMP find covers for albums without a MusicBrainz ID? (asked during implementation)

| Option | What it means |
|---|---|
| Auto MusicBrainz lookup (Recommended) | Look up the playing album once while Discord presence is on, cache the result, use Cover Art Archive. Covers most albums; sends album and artist names to MusicBrainz. |
| Lookup on request | A 'Find on MusicBrainz' action on the album page; you confirm the match. Nothing leaves unasked; most albums stay coverless. |
| Store catalogue (iTunes/Deezer) | Search a store for the cover URL directly. Very good mainstream coverage; their terms are not meant for this, and no ID for other features. |
| Wait for tag editor | Leave it until tags can be fixed in VCMP. Cleanest data, slowest. |

**Answer: Auto MusicBrainz lookup**

### Before building it, may I test MusicBrainz matching on ~20 of your albums (sends their album and artist names to musicbrainz.org)?

| Option | What it means |
|---|---|
| Yes, test first (Recommended) | Shows the real match rate on your collection before committing to the approach. |
| No, just build it | Skip the measurement; nothing about your library is sent until the feature runs. |

**Answer: Yes, test first.** The test drew 20 random albums without a MusicBrainz
tag; 12 of them got a cover.

### Which artists should the Artists page show? (asked during implementation)

Asked with the owner's library as the example: 138 artists with their own albums,
394 more who only appear in track credits on other artists' albums.

| Option | What it means |
|---|---|
| Album artists + switch (Recommended) | Shows the 138 album artists by default; a switch on the page shows all 532. Clean grid, credits still browsable; one more control. |
| Album artists only | Just the 138 with albums; credit-only artists are reached through search and album pages. Cleanest, but e.g. a character credit has no page. |
| All artists | All 532 in one grid, each page listing the tracks they appear on. Complete, but compound credits like 'A, B, C, D' fill the grid. |

**Answer: Album artists + switch**

## High-level plan

v1 is one release with all core features. It is built in workstreams so that a
usable player exists early and features are added on top of it.

**1. Foundation.** A Cargo workspace, CI that builds and tests on Windows, Linux and
macOS, the MIT/Apache-2.0 license, the repository's `CLAUDE.md`/`README.md`, the Slint
app shell with theme tokens (light/dark) and translatable strings. Depends on nothing.

**2. Library core.** The SQLite database with versioned migrations, source directory
management (recursive toggle), the scanner (incremental, tag reading, album/artist
grouping, re-linking moved files, availability tracking), the optional file watcher,
artwork extraction and the thumbnail cache. Depends on 1.

**3. Playback engine.** Decoding (symphonia), the output stream (cpal) with device
selection and fallback, resampling, gapless transitions, the track source
abstraction, the queue (context + Up next) and both shuffle modes. Depends on 1 and
can be built alongside 2, because it only needs file paths.

**4. UI shell and views.** The sidebar, content area, player bar and queue panel.
Album, artist and playlist grids with detail pages, the all-songs table, the folder
browser, global search, full-screen now playing, settings, light/dark switching,
a reduced-motion setting. Visual direction comes from `docs/DESIGN.md`.
Depends on 2 and 3. This is the first point where VCMP is a usable player.

**5. Playlists.** Create, rename, delete, add/remove/reorder entries, the generated
4-cover mosaic, the custom image override, unavailable entries. Depends on 2 and 4.

**6. Tag editor.** Single and batch editing, embedded cover art, write-back to files
followed by a re-read into the library. Depends on 2 and 4.

**7. Integrations.** OS media controls (MPRIS / SMTC / MPNowPlaying), Last.fm
scrobbling, Discord Rich Presence (with its MusicBrainz cover lookup), artist photo
search (MusicBrainz + fanart.tv) with a candidate picker, cover fetch (Cover Art
Archive), update check. Depends on 3 (media controls, scrobbling) and 2/4 (artwork).

**8. Packaging and release.** Installers on GitHub Releases (MSI/exe, dmg, AppImage +
.deb), code signing/notarization, build-time injection of API keys. It can start
right after 1 and is finished last.

## Architecture decisions

**UI-free core, thin Slint front end.** Library, playback and integrations live in
crates that do not depend on Slint. The UI crate turns user actions into commands
for the core and turns core events into Slint models and properties. Reason: the core
can be tested headless, and a heavy UI framework doesn't leak into audio code. The
seam is a command/event channel pair, not shared mutable state.

**Threading.** The Slint event loop owns the main thread. The cpal callback runs on
a real-time thread and only reads from a lock-free ring buffer: it never allocates,
locks or does I/O. This invariant gets a test or debug assertion. A decoder thread
fills the ring buffer. Scanning, image processing and network calls run on worker
threads and post results back through the event loop. Reason: dropouts and a
frozen UI are the most visible quality failures of a player.

**One persistent output stream.** The output stream stays open across tracks. The
decoder starts the next track before the current one ends and writes its samples
straight after, trimming encoder delay/padding where the file declares it. Every
track is resampled to the device rate, so a sample-rate change never reopens the
stream. The output device is stored by name. If it is missing, VCMP falls back to
the system default and tells the user. Reason: this is what makes gapless playback
and device switching reliable, and it is why the engine is our own rather than
rodio.

**Track source abstraction.** Queue entries are references of the form (source,
id), and a source produces a stream of decoded audio with its format. `Local` is the
only v1 source. Spotify (librespot) is added later as a second source, behind the
same queue, shuffle, scrobbling and media controls. Reason: the agreed Spotify plan
is in-app playback, and retrofitting a source abstraction into the queue later is
the expensive part.

**Database and file layout.** SQLite (bundled, no system dependency) in the OS data
directory. Main entities: source directories, tracks (unique path, tag fields,
duration, mtime/size, availability), albums (keyed by MusicBrainz album ID when
present, otherwise by album artist or artist + album title), artists, playlists and
ordered playlist entries that reference track ids, a scrobble queue, settings.
Migrations are versioned from the first release. Artwork is never stored as DB
blobs. Extracted and fetched images and thumbnails go to the cache directory, which
can be deleted and rebuilt. User-chosen overrides (artist photos, playlist images)
go to the data directory, because they cannot be rebuilt. Search uses SQLite FTS5.

**Tracks are soft-deleted.** A file that disappears marks its track unavailable;
the row is not deleted. A new file is re-linked to an unavailable track only on a
unique match of title, artist, album and duration (within a tolerance). Reason:
this one rule gives both agreed behaviours, playlists surviving moves and greyed-out
entries for unplugged drives. Unavailable tracks are hidden from library views. An
invariant test covers it: rescanning an unchanged library writes nothing, and a
move keeps the playlist entry and the track id.

**Scanning modes.** A manual rescan is always available. With the "automatic
update" setting on, VCMP runs an incremental scan at startup (mtime + size) and
watches the source directories live. With it off, the library changes only on a
manual rescan.

**Tags are the source of truth.** The DB is a cache of the tags. The tag editor
writes to the file, safely: write to a copy, read it back, then atomically replace
the original. The file is then re-read through the same scanner path, so the DB and
the file never disagree. Edits to the track that is playing wait until it is
released (Windows file locks).

**Shuffle and queue semantics.** Shuffle applies to the context. Up next always
plays first, in order. "True shuffle" draws independent random picks.
"Less repeats" is a permutation (each track once per cycle) that keeps the same
artist and the same album out of adjacent slots where possible.

**Theme and i18n invariants.** Components never hardcode colours. Every colour comes
from one theme global whose values are switched for light/dark, and custom themes
later only load other values into it. Every user-visible string uses Slint's
translation macro. Every animation duration comes from the same global, so the
reduced-motion setting (defaulting to the OS preference where the platform exposes
one) turns all motion off in one place. These rules are checked in code review or CI.

**Network policy.** VCMP talks to the network only for things the user turned on
(Last.fm, the update check, which can be disabled, the Discord presence) or
explicitly started (photo and cover search). MusicBrainz requests respect its 1
request/second limit and send an identifying User-Agent.

**Discord cover lookup.** While the Discord presence is on and Discord is running,
VCMP looks up the playing album on MusicBrainz when its files carry no release ID,
which sends the album title and album artist to musicbrainz.org. A match must agree
on the normalised title and artist and have a high search score; a second try drops
trailing edition notes such as "(Deluxe Edition)" and extra credited artists, and a
last one searches release groups. The cover is the Cover Art Archive image of the
release, or of its release group when the release has none. The result, found or
not, is stored per album (by its grouping key, so it outlives rescans), so an album
is looked up once; albums with no match are tried again after 30 days, and a failed
request (offline, a server error) on a play at least ten minutes later. The found ID only picks the Discord picture: it does not
change album grouping and is never written to files. The Last.fm key/secret and the fanart.tv key are injected at
CI build time, never committed. Dev builds read them from the environment and turn
the feature off when they are missing. The Last.fm session key is kept in the OS
keyring, not in SQLite. Scrobbles are queued persistently and sent in batches when
online.

**Slint attribution.** VCMP is MIT/Apache-2.0 and uses Slint under its royalty-free
license, which requires the "Made with Slint" attribution (for example in the About
dialog).

## Weaknesses and risks

| Risk | Likelihood | Cost | Mitigation |
|---|---|---|---|
| Slint struggles with large virtualized image grids (thousands of covers, async thumbnail loading) | Medium | High: the core views feel slow | Early spike with a ~5k-album synthetic library before building the other views; downscaled thumbnails only |
| Gapless MP3 depends on LAME/iTunSMPB delay/padding info that symphonia may not fully expose | Medium | Medium: small gaps or clicks on MP3 albums | Test corpus of gapless albums per format; read the info ourselves if needed |
| Tag writing corrupts or loses data in some files | Low-medium | High: the user's music is damaged | Copy-verify-replace writes, round-trip tests per format, an "edit tags" confirmation, no bulk writes without preview |
| "ogg" can mean Vorbis or Opus; symphonia has no built-in Opus decoder | High (users will try Opus) | Low-medium | v1 promises Ogg Vorbis; Opus is the first "more formats" candidate (open) |
| OS media control backends differ (Windows needs the window handle, macOS needs the main run loop) | Medium | Medium | Prototype on all three platforms during workstream 7, not at the end |
| Linux device list through ALSA is confusing next to PipeWire/PulseAudio | Medium | Low-medium | Show a "System default" entry first; filter or label raw hardware devices |
| Re-link heuristic matches the wrong file (duplicates, live versions) | Low | Low-medium | Re-link only on a unique match; otherwise treat the file as a new track |
| API keys shipped in binaries can be extracted and revoked | Low | Medium: Last.fm/fanart features break | Keys rotatable in CI; features degrade gracefully when a key is rejected |
| Unsigned builds trigger SmartScreen/Gatekeeper warnings | High without certificates | Medium: trust and adoption | Decide on an Apple Developer account and a Windows signing certificate (open) |
| librespot (future Spotify) is unofficial, Premium-only and may break or conflict with Spotify ToS | Medium | The v1 source seam may go unused | The seam is kept small; Spotify is a separate decision when it is planned |
| The Discord cover lookup matches the wrong release, or finds nothing | Medium: 8 of 20 untagged albums in the owner's library found no cover | Low: a wrong or missing picture in Discord | Strict matching, so misses are more likely than wrong covers; tagging the files with a release ID always wins |
| v1 scope is large (tag editor, 4 extra views, 3 OS integrations, web lookups) | High | Delayed release | Workstream order makes a usable player available by workstream 4; later workstreams can slip without blocking it |

**Questions deliberately left open**

- Should the queue and playback position be restored after a restart? Decided by
  the owner on 2026-10-17, during workstream 3: yes. The context, Up next, shuffle
  and repeat modes and the current track come back, loaded paused at the saved
  position.
- Signing budget (Apple Developer Program, Windows code-signing certificate). Has to
  be decided before the first public release.
- Last.fm extras beyond scrobbling ("now playing" updates, loving tracks): not
  discussed. Decide in workstream 7.
- Favourites, ratings, play counts or "recently added" in the UI: not requested and
  not planned. Play history is stored for scrobbling only.
- The target library size for performance budgets: needed for the grid spike in
  workstream 4.

## Out of scope

- Spotify connection/playback, the lyrics view and custom themes. v1 only provides
  the seams (track source abstraction, the full-screen now-playing view as the
  future lyrics host, the theme token global).
- ReplayGain/volume normalization and M3U import/export. Both were offered and not
  chosen for v1.
- Online tag lookup (MusicBrainz autotagging) in the tag editor, and DB-only
  metadata overrides. The Discord cover lookup is not tagging: its IDs stay in the
  database and only pick the presence picture.
- A self-updater, and Flatpak/winget/Homebrew distribution.
- A first-run setup wizard (it would introduce the app and mention the Discord
  presence), and the app logo in the Discord presence for untagged albums. Both are
  planned for later.
- UI languages other than English (the i18n layer is in v1, translations are not).
- Formats beyond mp3, wav, flac and Ogg Vorbis. An equalizer/DSP, streaming
  services other than the future Spotify, network/DLNA sources, mobile platforms,
  and syncing the library between devices.
