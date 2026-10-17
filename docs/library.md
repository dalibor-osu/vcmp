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
| Playlist pictures made from covers | cache directory, `mosaics/<hash>.jpg`, 400 px |
| Playlist pictures the owner chose | data directory, `images/playlists/<id>-<hash>.jpg`, longest side 600 px |
| Artist photos the owner chose | data directory, `images/artists/<hash>.jpg`, longest side 800 px |

Covers fetched online (see [Covers fetched online](#covers-fetched-online)) are cached
the same way.

The cache can be deleted at any time: the next scan rebuilds it, and playlist
pictures are made again when next shown. Fetched covers are the exception: nothing
downloads them again. Pictures the owner chose are kept next to the database
instead, because nothing could rebuild them. The database runs in
WAL mode, so one connection can scan while others read. Open one `Library` per
thread.

Music files are only read, except by `Library::write_tags`, the tag editor's write
path (see [Writing tags](#writing-tags)).

## Schema

Version 8, applied by `src/schema.rs`. `PRAGMA user_version` counts the applied
migrations; a database from a newer build is refused instead of being altered.

| Table | Holds |
|---|---|
| `settings` | Key/value preferences: `library.auto_update`, `ui.theme`, `ui.reduced_motion` |
| `source_dirs` | Source directories (canonical path) and their `recursive` flag |
| `tracks` | One row per file ever seen: path, size, mtime, `available`, format, tags, duration |
| `albums` | Album groups, their artist, year and artwork state |
| `artists` | Artist names (unique, case-insensitive) and MusicBrainz artist ID |
| `cover_lookups` | What the online cover lookup found for an album (a release or release-group MBID, or nothing) and when, keyed by the album's group key so it survives rescans (version 2) |
| `track_search`, `album_search`, `artist_search` | FTS5 trigram indexes, case and diacritics ignored (version 3; version 1 indexed words) |
| `playlists` | Playlist name, the file name of the owner's picture (or NULL) and when it was made (version 4) |
| `playlist_entries` | A playlist's tracks by position (from 0); the same track may appear more than once (version 4) |
| `fetched_covers` | Covers the user fetched for albums whose files have no art: the cached image's key, the Cover Art Archive release or release group it came from, and when, keyed by the album's group key (version 5) |
| `scrobble_queue` | Plays waiting to be sent to Last.fm: artist, title, album, album artist, track number, length and when the play started, oldest first (version 6). They copy the tags rather than reference tracks, so a rescan or tag edit can't change what was listened to |
| `folder_art` | Each music folder's cover images (`cover.jpg` and the like) as a fingerprint of their names, sizes and modification times, so a scan notices one added, replaced or removed (version 7) |
| `artist_photos` | The photo chosen for an artist: the file under `images/artists/`, the web address it came from (NULL for a file of the owner's) and when. Keyed by the artist's name, ignoring case, so it outlives artist rows a scan removes and adds (version 8) |

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
     to the tags, so a new album's artwork costs no extra disk seek.
   - A new or changed file sends an album that already had art to step 7, which
     picks it again from the album's first tracks: another program may have
     replaced or removed the files' art. A changed later track can't swap the
     cover this way.
   - Covers go to a pool of threads that decode, shrink and cache them while the next
     files are read.
   - Files are read one at a time; eight reader threads measured no faster on a
     hard disk (see Measured below).
4. **Re-link.** A new file takes over an unavailable track's row when exactly one
   such row has the same title, artist and album (ignoring case) and a duration
   within one second. Otherwise it becomes a new track.
5. **Folder images.** The discovery walk also notes the cover images in each
   folder. A folder whose images were added, replaced or removed since the last
   scan sends its albums to step 7; a folder seen for the first time is only
   recorded. Changing only an image changes no music file, so this is how it is
   noticed.
6. **Group.**
   - An album is keyed by its MusicBrainz release ID when tagged. Otherwise it is
     keyed by album artist (or track artist, when there is no album artist) plus
     title, lowercased.
   - The album year is the earliest track year.
   - Albums and artists nothing refers to any more are deleted, and the artist and
     album search indexes are rebuilt.
7. **Artwork** for albums still without it, or sent here by steps 3 and 5 (or whose
   cached files were deleted): a picture embedded in one of the first two tracks (front
   cover preferred), otherwise `cover`, `folder`, `front` or `album` with a `.jpg`,
   `.jpeg`, `.png` or `.webp` extension in the tracks' folders, matched
   case-insensitively. Files are read in order on one thread; the same cover pool
   does the image work. An album found without art loses the artwork key it had,
   so a fetched cover shows instead of stale art.

`Library::scan` takes a cancel flag and a progress callback, and returns a
`ScanReport` with counts, skipped files and unreachable sources. Removing a source
marks its tracks unavailable; adding it back and scanning restores them with the same
IDs.

A file without tags gets its file name (without the extension) as its title; its
artist and album stay empty.
Display names for empty values ("Unknown artist") belong to the UI, so they can be
translated.

## Writing tags

`Library::write_tags` takes `(TrackId, &TagEdit)` pairs: one track, or a batch that
passes the same edit for many tracks (the album artist of a whole album). A
`TagEdit` sets, clears or keeps each of title, artist, album, album artist, track
number, disc number, year, genre and the embedded front cover. `TagEdit::validate`
runs first, for every edit. An invalid edit (a control character, a year outside
1 to 9999, a cover that isn't JPEG or PNG) fails the whole call before any file is
touched. Values are trimmed, and an empty value clears the field.

Each file is written through a verified copy:

1. Follow a symlink to the real file. Refuse a read-only file, and read the tags the
   way a scan does.
2. Copy the file next to itself as `.<name>.<random>.vcmp-tmp`, with the same
   permissions. The name hides it from scans and file managers. A failure at any
   later step deletes the copy.
3. Edit the tag VCMP reads in the copy: ID3v2 in MP3 and WAV, Vorbis comments in
   FLAC and Ogg Vorbis. The tag is edited in place, so frames and fields the editor
   doesn't know (comments, composer, lyrics, ReplayGain, MusicBrainz IDs, other
   pictures) are kept. An ID3v2.3 tag stays ID3v2.3 when that loses nothing. Taggers
   also put ID3v2.4 frames (full original and release dates) into ID3v2.3 tags, and
   ID3v2.3 can hold only the year of those, so such a tag is written as ID3v2.4.
   When the file has
   no such tag, it is created with what the scan read from the file's other tags, so
   an MP3 with only ID3v1 loses no values.
4. Read the copy back and compare:
   - every item and picture of the written tag with what was meant to be written;
   - the scan's reading of every field, MusicBrainz IDs included, with the old reading
     plus the edit;
   - the file type and duration with the original's.
5. Check that the original has not changed since it was copied (size and
   modification time), then rename the copy over it. The rename is atomic, so the
   original is either untouched or fully replaced. On Windows a failed rename is
   retried for up to about a second, in case an antivirus scanner or the indexer
   holds the new copy.

A file that fails is reported in `TagWriteReport::failed` with a `TagWriteError`.
Its variant is the reason, for the UI to translate, and the text is English detail
for logs. The other files of the batch are still written. `cancel` stops before the
next file.

The cover edit replaces "the cover": every front-cover picture or, when there is
none, a leading picture without a type, which is how many taggers store covers.
Other pictures (back cover, artist) are kept. `Library::embedded_cover` reads the
current cover from a track's file for the editor.

Written files are then re-read into the database through the scan's own code. A
changed album or album artist regroups the tracks, so an album can get a new id
(`Library::track` gives it), and emptied albums and artists are removed. Every
album involved looks for its artwork again, so a new embedded cover becomes the
album artwork. Albums and artists match ignoring case, and a scan keeps the
spelling it met first. A spelling the user typed in an edit replaces it, so
"the wall" can be renamed to "The Wall".

`examples/edit_check.rs` copies a sample of a real collection into
`target/edit-check/`, then edits every copy twice: once setting every field and a
cover, once restoring the fields and removing the cover. It prints every failure
(the collection itself is only read):

```bash
cargo run --release -p vcmp-library --example edit_check -- /path/to/music 40
```

## Covers fetched online

"Find cover online" on the album page of an album without artwork (see
[docs/ui.md](ui.md#album-page)) searches MusicBrainz and the Cover Art Archive
(`vcmp-integrations::musicbrainz`), and the image the user picks is stored with
`Library::set_fetched_cover`:

- The image goes into the artwork cache like extracted art: the same hash key and
  400 px thumbnail.
- `fetched_covers` keeps that key and the release or release group the image came
  from, by the album's group key. Scans never touch it, and an album row that a
  scan removes and adds again finds it by the key.
- `Album::artwork` is the files' own art when they have some, otherwise the fetched
  cover. If the files gain art, theirs shows; if the tag editor removes it again,
  the fetched cover comes back.
- A fetched image whose cached files were deleted counts as no cover, and the album
  offers the search again. Fetching again replaces the row.

## Watching

`LibraryWatcher` watches the reachable sources and calls back once no change has
arrived for a quiet period, so copying an album produces one notification. It never
scans by itself. The app's `LibraryService` (see [ui.md](ui.md#music-folders-and-scanning))
starts it, and scans at startup, while `library.auto_update` is on (the default).

## Search

`Library::search` finds every typed word anywhere in a title, artist or album name,
ignoring case and diacritics: `dvorak` finds "Dvořák", `floyd wall` finds the album
"The Wall" by Pink Floyd, and `ライブ` finds "ラブライブ!". Artists, albums and
tracks are searched separately.

- **Why parts of words:** the first index matched word starts, and words end at
  spaces and punctuation. Japanese and Chinese titles have no spaces, so on the
  owner's library `ライブ` found 1 of the 88 tracks whose title or album contains it,
  and `穂乃果` 3 of 135. The trigram tokenizer indexes every three-character run.
- **Words of three characters or more** go into one full-text query, ranked by FTS5.
  **Shorter words** (`高坂`, `μ`) cannot be looked up in a trigram index, so they
  become `LIKE` conditions over the search table's columns, which scan it.
- **Speed:** each full-text query runs once, in a `MATERIALIZED` CTE that the other
  tables join. Joined directly, SQLite looked the full-text query up again for
  every album row: 237 ms for one search instead of a few. On the owner's library
  (3,621 tracks) searches take 7 to 16 ms, a one-letter search about 50 ms.
  `examples/search.rs` times searches on any library.

## Playlists

`src/playlists.rs`. A playlist is a name and an ordered list of track ids.

- **Entries outlive their files.** Tracks are never deleted (see
  [Scanning](#scanning)), so an entry keeps pointing at its track while the file is
  gone, and `PlaylistEntry::track.available` says so. The playlist counts every
  entry, and its duration only the available ones. A moved file that is re-linked
  keeps its entries; a test checks it.
- **Edits** (add, remove, move) read the entry list, change it, and write it back in
  one transaction. Shifting positions one by one would collide on the
  `(playlist_id, position)` key, and playlists are small enough that a rewrite costs
  nothing noticeable.
- **The picture** is the owner's own image when they chose one: decoded, bounded to
  600 px, and stored as a JPEG named by playlist id and content hash, so a new one
  never reuses the old file name. Otherwise it is made from the covers of the first
  four different albums in playlist order (the files' own cover or a fetched one,
  if it is in the cache): a 2 x 2 grid of 200 px crops, cached under `mosaics/`
  and named by the four covers, so it changes when the order brings other albums
  to the front. With fewer than four albums the picture is the first album's
  cover; with none there is no picture. Deleting a playlist or going back to
  covers deletes the owner's file.
- **Search:** `Library::search_playlists` keeps the playlists whose name holds every
  typed word, ignoring case.

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
- A folder image is noticed by its name, size and modification time. An image
  replaced by another of the same size with its old modification time restored
  goes unnoticed until a track of the album changes.
- A fetched cover belongs to the album's group key. When the album's title or
  album artist changes, the album gets a new key and offers the search again.
- SQLite's case-insensitive comparison only folds ASCII. Artist names that differ
  only in the case of a non-ASCII letter ("Dvořák" and "DVOŘÁK") become two artists,
  and sorting is not locale-aware. Album grouping lowercases with full Unicode rules.
- Multi-value artist tags are joined with ", " into one artist name.
- MP3 durations count whole frames, so they can be a fraction of a second longer
  than what plays.
- Tag writing updates only the tag VCMP reads. An ID3v1 or APE tag in an MP3, or a
  RIFF INFO list in a WAV file, keeps its old values, and players that prefer those
  tags (Windows Explorer reads RIFF INFO in WAV files) show them.
- On Windows a file that another program holds open cannot be replaced, so the
  edit fails with `TagWriteError::Replace`. The app must release the playing track
  before writing its tags.
- Replacing a file gives it a new inode (Unix) or file ID (Windows). Hard links keep
  the old content, and on Unix the file's owner becomes the user running VCMP.
- A crash between the copy and the rename can leave a `.<name>.<random>.vcmp-tmp`
  file next to the music file. Scans ignore it, and it can be deleted.
- Setting the cover of one track doesn't change the album's other tracks. The album
  artwork comes from its first two tracks, so to change it, edit the whole album.
- A file whose tag lofty can only read in part (a damaged frame) may lose that part
  when it is written. The read-back check compares what lofty can read.
- Playlist pictures made from covers that no longer fit any playlist stay in
  `mosaics/` until the cache is deleted. Each is about 30 KB.
- Playlist search compares names in Rust after reading every playlist, and lowercases
  them the simple way, so it ignores case but not diacritics.
