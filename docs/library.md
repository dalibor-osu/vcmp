# Library

How `crates/vcmp-library` turns source directories into the music library. The
agreed decisions behind it are in the
[v1 spec](specs/2026-10-17-vcmp-music-player.md). Keep this file in sync with the
crate.

## Storage

| What | Where (from `directories::ProjectDirs` with the name `vcmp`) |
|---|---|
| Database | data directory, `library.sqlite3` (Linux: `~/.local/share/vcmp/`) |
| Artwork originals | cache directory, `artwork/<hash>.<ext>` (Linux: `~/.cache/vcmp/`) |
| Thumbnails | cache directory, `thumbnails/<hash>.jpg`, longest side 400 px |

The cache can be deleted at any time: the next scan rebuilds it. The database runs in
WAL mode, so one connection can scan while others read. Open one `Library` per
thread.

Music files are only ever read by this crate. Tag writing arrives with the tag editor
(workstream 6).

## Schema

Version 1, applied by `src/schema.rs`. `PRAGMA user_version` counts the applied
migrations; a database from a newer build is refused instead of being altered.

| Table | Holds |
|---|---|
| `settings` | Key/value preferences: `library.auto_update`, `ui.theme`, `ui.reduced_motion` |
| `source_dirs` | Source directories (canonical path) and their `recursive` flag |
| `tracks` | One row per file ever seen: path, size, mtime, `available`, format, tags, duration |
| `albums` | Album groups, their artist, year and artwork state |
| `artists` | Artist names (unique, case-insensitive) and MusicBrainz artist ID |
| `track_search`, `album_search`, `artist_search` | FTS5 indexes, diacritics removed |

Paths are stored as platform-native bytes (`src/path_codec.rs`), so file names that
aren't valid UTF-8 keep working.

## Scanning

1. **Discover.** Walk every source (only its top level when `recursive` is off) and
   collect `.mp3`, `.flac`, `.ogg`/`.oga` and `.wav` files with their size and
   modification time. A source that isn't a readable directory is reported as
   unreachable.
2. **Compare.**
   - An unchanged file (same size and mtime) is left alone; if it was unavailable, it
     becomes available again.
   - A changed file is re-read.
   - A file the database has but the disk doesn't is marked unavailable. Rows are
     never deleted, so playlists keep their entries.
3. **Read** new and changed files in path order on one reader thread, which runs
   ahead of the database writes (one transaction per 200 files).
   - The file type is detected from content, not the extension, so an Opus stream
     named `.ogg` is skipped as unsupported. Unreadable files are skipped and listed
     in the report.
   - The first file of each folder is read with its embedded cover, which sits next
     to the tags, so the album's artwork costs no extra disk seek.
   - Covers go to a pool of threads that decode, shrink and cache them while the next
     files are read.
   - Files are read one at a time; eight reader threads measured no faster on a
     hard disk (see Measured below).
4. **Re-link.** A new file takes over an unavailable track's row when exactly one
   such row has the same title, artist and album (ignoring case) and a duration
   within one second. Otherwise it becomes a new track.
5. **Group.**
   - An album is keyed by its MusicBrainz release ID when tagged. Otherwise it is
     keyed by album artist (or track artist, when there is no album artist) plus
     title, lowercased.
   - The album year is the earliest track year.
   - Albums and artists nothing refers to any more are deleted, and the artist and
     album search indexes are rebuilt.
6. **Artwork** for albums still without it (no cover captured while reading, or
   cached files deleted): a picture embedded in one of the first two tracks (front
   cover preferred), otherwise `cover`, `folder`, `front` or `album` with a `.jpg`,
   `.jpeg`, `.png` or `.webp` extension in the tracks' folders, matched
   case-insensitively. Files are read in order on one thread; the same cover pool
   does the image work.

`Library::scan` takes a cancel flag and a progress callback, and returns a
`ScanReport` with counts, skipped files and unreachable sources. Removing a source
marks its tracks unavailable; adding it back and scanning restores them with the same
IDs.

A file without tags gets its file name (without the extension) as its title; its
artist and album stay empty.
Display names for empty values ("Unknown artist") belong to the UI, so they can be
translated.

## Watching

`LibraryWatcher` watches the reachable sources and calls back once no change has
arrived for a quiet period, so copying an album produces one notification. It never
scans by itself. The app isn't wired to it yet: starting the watcher and scanning at
startup while `library.auto_update` is on (the default) comes with the library views
in workstream 4.

## Search

`Library::search` matches every typed word as a word prefix, ignoring case and
diacritics: `dvorak` finds "Dvořák", and `floyd wall` finds the album "The Wall" by
Pink Floyd. Artists, albums and tracks are searched separately and ranked by FTS5.

## Measured

The library used below has 3,621 tracks, 127 GB, mostly FLAC, on an NTFS hard disk
mounted through `ntfs-3g` (FUSE, single-threaded).

Cold runs start with an empty OS file cache
(`sudo sh -c 'sync; echo 3 > /proc/sys/vm/drop_caches'`).

| Version | Discovering | Reading tags | Artwork | Total |
|---|---|---|---|---|
| First version, cold | 4.8 s | 30.9 s | 10.2 s | 45.9 s |
| Covers captured while reading, 8 reader threads, cold | 6.5 s | 34.7 s | 2.0 s | 43.2 s |
| Covers captured while reading, 1 reader thread, cold (current) | 4.5 s | 34.4 s | 1.7 s | 40.6 s |
| First version, files cached | 0.1 s | 0.5 s | 5.7 s | 6.3 s |
| Current, files cached | 0.1 s | 0.7 s | 0.2 s | 1.0 s |
| Rescan, nothing changed | | | | 0.15 s |

The Discovering phase is the same code in every version; its 4.5 to 6.5 s spread
shows the run-to-run noise of cold measurements.

Where the time goes (`examples/scan_profile.rs`):

- **Cold tag reading is seek-bound.** About 8 ms per file, one disk seek each,
  against 0.05 ms once cached. FLAC files need about 35 KB read each. MP3 files
  average 1.3 MB, because lofty reads the whole ID3 tag, embedded pictures included.
- **Reading covers with the tags moves about 3.5 s into tag reading and saves about
  8.5 s of artwork time.** About 400 embedded covers (roughly 150 MB) are now read
  right after their tags instead of with a second seek per album later.
- **Parallel reading doesn't help this disk.** One reader thread (34.4 s) and eight
  (34.7 s) are the same within noise, since `ntfs-3g` serves one request at a time.
  On an SSD, parallel reads would save under a second for a library this size, so
  VCMP keeps one reader thread and reads in path order.
- **Artwork work is CPU-bound.** Decoding embedded covers (up to 4688x4688 px),
  shrinking and re-encoding took about 5 s on one core. It now runs on all cores
  while the disk keeps reading.
- **`ntfs-3g` handles one request at a time.** Linux's in-kernel `ntfs3` driver
  would make the per-file reads cheaper; that's a system choice outside VCMP.

To measure your own library (files are only read):

```bash
cargo run --release -p vcmp-library --example scan -- /path/to/music
cargo run --release -p vcmp-library --example scan_profile -- /path/to/music
```

## Known limits

- CUE sheets aren't read: an album ripped as one file plus a `.cue` shows up as a
  single long track.
- An album with no artwork is checked again only when one of its tracks changes.
  Adding or replacing a `cover.jpg` alone isn't noticed.
- SQLite's case-insensitive comparison only folds ASCII. Artist names that differ
  only in the case of a non-ASCII letter ("Dvořák" and "DVOŘÁK") become two artists,
  and sorting is not locale-aware. Album grouping lowercases with full Unicode rules.
- Multi-value artist tags are joined with ", " into one artist name.
- MP3 durations count whole frames, so they can be a fraction of a second longer
  than what plays.
