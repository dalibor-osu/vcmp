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
| Playlist pictures the owner chose | data directory, `images/playlists/<id>-<hash>.jpg`, longest side 600 px; an animated one also as it was, `<id>-<hash>.gif` (or `.webp`, `.png`) |
| Artist photos the owner chose | data directory, `images/artists/<hash>.jpg`, longest side 800 px; an animated one also as it was, `<hash>.gif` (or `.webp`, `.png`) |
| Animated album covers the owner chose | data directory, `images/covers/<hash>.gif` (or `.webp`, `.png`), the original bytes, named by the hash of the still the files hold (see [Animated pictures](#animated-pictures)) |
| Covers kept for change sets | data directory, `images/undo/<hash>.<ext>`, the original bytes (see [Change sets](#change-sets)) |

Covers fetched online (see [Covers fetched online](#covers-fetched-online)) are cached
the same way.

The cache can be deleted at any time: the next scan rebuilds it, and playlist
pictures are made again when next shown. Fetched covers are the exception: nothing
downloads them again. Pictures the owner chose are kept next to the database
instead, because nothing could rebuild them. The database runs in
WAL mode, so one connection can scan while others read. Open one `Library` per
thread.

Labels (see [Labels](#labels)), favourites (see [Favourites](#favourites)) and the
play history (see [Play history](#play-history)) live in the database only and never
touch a file.

Music files are only read, except by `Library::write_tags`, the tag editor's write
path (see [Writing tags](#writing-tags)), and `Library::accept_change_set` (see
[Change sets](#change-sets)), which writes through the same path.

## Schema

Version 19, applied by `src/schema.rs`. `PRAGMA user_version` counts the applied
migrations; a database from a newer build is refused instead of being altered.
Migrations run with foreign keys off (`Library::open` turns them on afterwards), so
a migration that rebuilds a table, as version 12 rebuilds `change_sets` to widen
the CHECK on its kind, doesn't delete the rows that refer to it.

| Table | Holds |
|---|---|
| `settings` | Key/value preferences: `library.auto_update`, `library.added_backfilled` (the one-time backfill of added dates ran; see [Play history](#play-history)), `ui.theme` (the theme and accent as JSON; [ui.md](ui.md#themes)), `ui.reduced_motion` (`system`, `on` or `off`), `ui.setup_done` (the setup wizard was shown, or the library had folders when it came; [ui.md](ui.md#setup-wizard)), `ui.lyrics_shown` and `integrations.lyrics_online` (see [Lyrics](#lyrics)); in a VCMP server's library also `server.id`, `server.user_token` and `server.admin_token` ([server.md](server.md)) |
| `source_dirs` | Source directories (canonical path) and their `recursive` flag |
| `tracks` | One row per file ever seen: path, size, mtime, `available`, format, tags, duration, `added_ms`, when VCMP first saw it (see [Play history](#play-history)), and `revision`, its place in the change feed (version 18; see [Change feed](#change-feed)) |
| `albums` | Album groups, their artist, year and artwork state, and whether the album is an artist's Unknown album (`unknown`, version 9) |
| `artists` | Artist names (unique, case-insensitive) and MusicBrainz artist ID |
| `cover_lookups` | What the online cover lookup found for an album (a release or release-group MBID, or nothing) and when, keyed by the album's group key so it survives rescans (version 2) |
| `track_search`, `album_search`, `artist_search` | FTS5 trigram indexes, case and diacritics ignored (version 3; version 1 indexed words) |
| `playlists` | Playlist name, the file name of the owner's picture (or NULL) and when it was made (version 4) |
| `playlist_entries` | A playlist's tracks by position (from 0); the same track may appear more than once (version 4) |
| `fetched_covers` | Covers the user fetched for albums whose files have no art: the cached image's key, the Cover Art Archive release or release group it came from, and when, keyed by the album's group key (version 5) |
| `scrobble_queue` | Plays waiting to be sent to Last.fm: artist, title, album, album artist, track number, length and when the play started, oldest first (version 6). They copy the tags rather than reference tracks, so a rescan or tag edit can't change what was listened to |
| `folder_art` | Each music folder's cover images (`cover.jpg` and the like) as a fingerprint of their names, sizes and modification times, so a scan notices one added, replaced or removed (version 7) |
| `artist_photos` | The photo chosen for an artist: the file under `images/artists/`, the web address it came from (NULL for a file of the owner's) and when. Keyed by the artist's name, ignoring case, so it outlives artist rows a scan removes and adds (version 8) |
| `track_tags` | Every text item of the tag VCMP reads from each track's file, by key, in order (version 10; see [All tags](#all-tags)) |
| `change_sets` | Tag edits, label edits and playlist deletions agents asked for: who, what kind (`tags`, `labels`, `playlist_deletion`), status, the set an undo reverses, when made and finished (version 11; `labels` since 12; see [Change sets](#change-sets)) |
| `change_files` | A tag set's files: track, path, the cover before and after (file names under `images/undo/`) and each file's result (version 11) |
| `change_edits` | What a set asks for each file, by field name or `tag:` and a key, the values as a JSON array (version 11) |
| `change_values` | The values of every tag a set may change in each file, as JSON arrays: before (when the set was made) and after (once written) (version 11) |
| `labels` | The owner's labels: name (unique, case-insensitive), colour (`palette:` and a name, or `#rrggbb`), when made (version 12; see [Labels](#labels)) |
| `track_labels` | Which track holds which label, and since when; deleting a label deletes its rows (version 12) |
| `change_labels` | The labels a label set creates when accepted, and whether it did or found one of that name (version 12) |
| `favourites` | The tracks the owner liked, one row each, and when (version 13; see [Favourites](#favourites)) |
| `not_duplicates` | Duplicate groups the owner marked "Not duplicates": one row per track, with a group id shared by the tracks marked together (version 14; see [Library check](#library-check)) |
| `plays` | Every play that counted: track, when it started and the origin of the context it played from, indexed by track and time and by time (version 15; see [Play history](#play-history)) |
| `lyrics_lookups` | What LRCLIB answered for a track (LRC text, plain text, or nothing), the question it answered (title, artist, album, duration in seconds) and when, keyed by source and track id (`local` and the library track id) rather than a reference (version 16; see [Lyrics](#lyrics)) |
| `servers` | The VCMP servers the app is connected to, or was: the server's own id (unique), its name and address, whether it is connected (removing one keeps its row), the feed cursor reached and when it last synced (version 19; see [Server tracks](#server-tracks)) |
| `remote_tracks` | Which `tracks` rows are a server's: the server, the server's own track id (unique per server), its music folder and its path inside it (version 19) |
| `server_folders` | Each server's music folders by the server's id and name, for the Folders page (version 19) |
| `love_queue` | Loves and unloves waiting to be sent to Last.fm: artist, title and love or unlove, oldest first, one row per artist and title (version 17). A newer entry replaces the waiting one with a new id, so the one being sent as it is replaced isn't mistaken for it |

Paths are stored as platform-native bytes (`src/path_codec.rs`), so file names that
aren't valid UTF-8 keep working.

## Scanning

1. **Discover.** Walk every source (only its top level when `recursive` is off) and
   collect `.mp3`, `.flac`, `.ogg`/`.oga`, `.wav`, `.aif`/`.aiff`/`.aifc` and `.m4a`
   files with their size and modification time. `.mp4` files are never looked at. A source that isn't a readable directory is reported as
   unreachable.
2. **Compare.**
   - An unchanged file (same size and mtime) is left alone; if it was unavailable, it
     becomes available again.
   - A changed file is re-read.
   - A file the database has but the disk doesn't is marked unavailable. Rows are
     never deleted, so playlists keep their entries.
   - Once, on the first scan after schema version 15, the same transaction backfills
     the added dates of the rows that existed before it (see
     [Play history](#play-history)).
3. **Read** new and changed files in path order on one reader thread, which runs
   ahead of the database writes (`write_as_received`: at most 200 files to a
   transaction).
   - The scan commits before it waits for the reader, so it holds the write lock
     only while it writes files already read. Held while a hard disk read 200 files,
     it locked the window's connection out for longer than its 5-second busy
     timeout: turning on AI agents at the first start of 26.10.1, which re-reads
     every file, was not saved. Re-reading files after a tag write reads them all
     first for the same reason.
   - The file type is detected from content, not the extension, so an Opus stream
     named `.ogg` is skipped as unsupported. An `.m4a` file is stored as AAC or
     ALAC, the codec lofty reads; one holding anything else (MP3, FLAC) or a video
     track (a `vide` handler in `moov/trak/mdia/hdlr`, which VCMP reads itself since
     lofty reads only the audio track) is skipped as unsupported. Unreadable files
     are skipped and listed in the report.
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
   - Every track belongs to an album. One without an album tag goes into the
     "Unknown" album of its album artist (or track artist): an album row with
     `unknown = 1`, an empty title and the group key `unknown:<artist id>`. A track
     naming no artist either goes into the one Unknown album without an artist
     (`unknown:`). Keying by the artist's id rather than the name lets the
     migration that first made them (version 9, in SQL, whose `lower()` folds only
     ASCII) build the same keys as a scan. Unknown albums get no artwork, no cover
     lookups and no "Find cover online"; they sort after their artist's other
     albums, and the one without an artist last of all.
   - The album year is the earliest track year.
   - Albums and artists nothing refers to any more are deleted, and the artist and
     album search indexes are rebuilt.
7. **Artwork** for albums still without it, or sent here by steps 3 and 5 (or whose
   cached files were deleted): a picture embedded in one of the first two tracks (front
   cover preferred), otherwise `cover`, `folder`, `front` or `album` with a `.jpg`,
   `.jpeg`, `.png`, `.webp` or `.gif` extension in the tracks' folders, matched
   case-insensitively. Files are read in order on one thread; the same cover pool
   does the image work, through a queue of twice its threads, so image bytes don't
   pile up when reading outruns decoding. An album found without art loses the
   artwork key it had, so a fetched cover shows instead of stale art. A cover the
   cache couldn't take (a full disk) leaves the album to be looked at again at the
   next scan, rather than marked as having none.

`Library::scan` takes a cancel flag and a progress callback, and returns a
`ScanReport` with counts, skipped files and unreachable sources. Removing a source
marks its tracks unavailable; adding it back and scanning restores them with the same
IDs. A folder that can't be listed is reported among the skipped files, and its
tracks keep their state rather than going unavailable. A scan that is cancelled or
fails still brings album years and search up to date with what it added.

Writes that read first take the write lock as they begin (`BEGIN IMMEDIATE`): a
deferred transaction that another connection's commit overtakes fails at once with
"database is locked", where an immediate one waits out the busy timeout.

A file without tags gets its file name (without the extension) as its title; its
artist and album stay empty, and it lands in the Unknown album without an artist.
Display names for empty values ("Unknown artist") belong to the UI, so they can be
translated.

## All tags

Besides the fields VCMP shows, a scan stores every text item of the tag it reads
(ID3v2 in MP3, WAV and AIFF, Vorbis comments in FLAC and Ogg Vorbis, MP4 atoms in
M4A) in `track_tags`, and
`Library::track_tags` returns them. They are there for AI agents (see the
[MCP spec](specs/2026-10-19-mcp-library-server.md)), which can find, for example,
every track without a MusicBrainz ID: `Library::tag_keys` lists every key with how
many tracks hold it, and `Library::tracks_by_tag` the available tracks that hold a
key (with a value, when given) or lack it, a page at a time;
`Library::count_tracks_by_tag` counts them all.

- **Keys.** An item lofty has a format-independent name for is stored under that
  name (`TrackTitle`, `Composer`, `MusicBrainzReleaseId`, `ReplayGainTrackGain`),
  the same in every format. Anything else keeps the format's own key behind a
  prefix: `id3:` and the frame ID, plus the frame's description after a colon where
  it has one (`id3:TXXX:MY FIELD`, `id3:COMM:iTunNORM`, `id3:POPM:<email>`),
  `vorbis:` and the field name in capitals (`vorbis:MYFIELD`), or `mp4:` and the
  atom: its four characters as lofty reads them (`mp4:stik`; byte `0xA9` is the
  copyright sign, which many atom names start with) or, for a freeform atom, `----`,
  its mean and its name (`mp4:----:com.apple.iTunes:MY FIELD`). An atom lofty has a
  name for keeps that name even when it holds a number lofty's split leaves alone
  (the integer `tmpo` is `IntegerBpm`).
- **Values** are stored as the file holds them, untrimmed. A field with several
  values (Vorbis comments may repeat a field, ID3v2.4 separates values with a NUL)
  is one row per value, and `position` keeps the order. `TRCK` and `TPOS` split into
  number and total (`TrackNumber`, `TrackTotal`); one that isn't a number stays
  whole under `TrackNumber`, so malformed values stay visible. An MP4 atom's
  numbers and flags are listed in decimal (`mp4:stik` `1`).
- **What is left out:** pictures, binary frames (`PRIV`, `GEOB`, chapters) and
  atoms (binary data), and a file's other tags (ID3v1 and APE in an MP3, RIFF INFO
  in a WAV, text chunks in an AIFF). A file without
  the tag VCMP reads stores the items of the tag its fields came from instead.
- **Reading** goes through each format's own lofty type, not the probe's
  format-independent view: that view drops the items it has no name for.
- **Upgrading** to version 10 sets every track's stored modification time to -1, so
  the next scan re-reads every file once as if it had changed. That includes looking
  for every album's artwork again (step 7). Tracks whose files are unavailable at
  that scan get their tags when they come back.

## Writing tags

`Library::write_tags` takes `(TrackId, &TagEdit)` pairs: one track, or a batch that
passes the same edit for many tracks (the album artist of a whole album). A
`TagEdit` sets, clears or keeps each of title, artist, album, album artist, track
number, disc number, year, genre and the embedded front cover. `TagEdit::validate`
runs first, for every edit. An invalid edit (a control character, a year outside
1 to 9999, a cover that isn't JPEG or PNG) fails the whole call before any file is
touched. Values are trimmed, and an empty value clears the field.

`TagEdit::other` edits any other tag by its [`track_tags` key](#all-tags): it
replaces the tag's values with a list, written exactly as given, or removes the tag
(`Clear`, or an empty list). See [Other tags](#other-tags).

Each file is written through a verified copy:

1. Follow a symlink to the real file. Refuse a read-only file, and read the tags the
   way a scan does.
2. Copy the file next to itself as `.vcmp-<random>.tmp`, with the same
   permissions. The name hides it from scans and file managers. A failure at any
   later step deletes the copy.
3. Edit the tag VCMP reads in the copy: ID3v2 in MP3, WAV and AIFF, Vorbis
   comments in FLAC and Ogg Vorbis, MP4 atoms (`ilst`) in M4A. The tag is edited in
   place, so frames, fields and atoms the editor
   doesn't know (comments, composer, lyrics, ReplayGain, MusicBrainz IDs, other
   pictures) are kept. An ID3v2.3 tag stays ID3v2.3 when that loses nothing. Taggers
   also put ID3v2.4 frames (full original and release dates) into ID3v2.3 tags, and
   ID3v2.3 can hold only the year of those, so such a tag is written as ID3v2.4.
   When the file has
   no such tag, it is created with what the scan read from the file's other tags, so
   an MP3 with only ID3v1 loses no values. In an M4A file the album artist is
   `aART`, the year `©day`, and the cover the first picture of `covr`, since MP4
   keeps no picture types: a new cover goes first, and removing the cover removes
   the first picture.
4. Read the copy back and compare:
   - every item, every `track_tags` entry and every picture of the written tag with
     what was meant to be written;
   - the scan's reading of every field, MusicBrainz IDs included, with the old reading
     plus the edit (MusicBrainz IDs are edited as other tags);
   - every other tag the edit names with the values asked for, and every tag it
     doesn't touch with what the file held before, in the same order (skipped when
     VCMP had to create the tag);
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

### Other tags

`src/other_tags.rs`. The keys an agent reads from `track_tags` are the keys it
writes with.

- **A key means what it reads back as.** A common name becomes an ID3v2 frame in an
  MP3 and a Vorbis field in a FLAC file, and some native keys have common names
  (`id3:TCOM` and `vorbis:COMPOSER` are listed as `Composer`). So each key is
  resolved per file by writing its first value into an empty tag of the file's kind
  and reading it back with the scan's code; the key it reads back under is the one
  the checks compare. The value itself is written because some tags take only
  values of a form: lofty lists a rating as `email|stars|play count`
  (`Popularimeter`, `Easy CD-DA Extractor|4|0`), and ID3v2 drops anything else.
- **Refused before any file is touched** (`InvalidTagEdit`): a key that isn't one
  (`UnknownKey`), a key of the nine fields under any name (`ShownField`: `TrackTitle`,
  `id3:TIT2`, `vorbis:ALBUMARTIST`, `mp4:©nam`), a native key whose frame VCMP
  can't build (`UnwritableKey`: `id3:POPM:<email>` and `id3:TIPL:<role>`; clearing
  those works), a value with a control character other than a line break or tab or
  an empty value, a value that isn't a whole number for a key lofty keeps as a
  number (`TrackTotal`, `MovementNumber`), and a value no kind of tag VCMP writes
  keeps (`BadValue`). Every common name lofty knows has a place in ID3v2, Vorbis
  comments or MP4 atoms, so none is refused as unwritable before the files are
  known: `AppleXid`, an MP4 atom only, is refused for each MP3 or FLAC file.
- **Refused for one file** (`TagWriteError::UnsupportedKey`): a native key of
  another kind of tag (`id3:` in a FLAC file, `mp4:` in an AIFF), a common name the
  format has no field for (ID3v2 has no `Lyrics`, only `UnsyncLyrics`), a value only
  another kind keeps (a rating as free text is kept in a FLAC file, not in an MP3),
  and two keys of one edit that name the same tag. An MP4 atom that holds numbers
  in the file and is given something else (`mp4:stik` `music`), or that holds binary
  data, fails that file with `TagWriteError::Write`.
- **ID3v2.** A native key replaces its frames (by frame ID and description) with one
  new frame; several values are joined with NULs. A number or its total is written
  by VCMP into the frame holding both as `number/total` (`TRCK` for `TrackNumber`
  and `TrackTotal`, `TPOS` for the disc, `MVIN` for the movement), keeping the part
  not edited. lofty would build the frame from the edited part alone, so a total
  added to `TRCK` `7` would become `0/15`. Any other common name goes through lofty's
  generic tag, but only for the frames holding that key: they are split into lofty's
  generic form, edited and merged back, so a frame holding several keys keeps the
  ones not edited (`TIPL` holds the producer and the engineer), and every other
  frame stays as it was.
- **Vorbis comments.** The fields of the key are removed (for a common name, every
  field name lofty reads as it) and one field per value is added under the key's
  name. lofty's own `remove` swaps the fields it keeps around, which could reverse
  two values of one key (an `ORIGINALDATE` and an `ORIGINALYEAR` are both
  `OriginalReleaseDate`), so every Vorbis edit, the nine fields' included, puts the
  fields it keeps back in their original order.
- **MP4 atoms.** A native key replaces the atom with one holding the values, as
  text, or as numbers or flags when the atom it replaces held those (players read
  `tmpo` and `stik` only as numbers). A common name goes through lofty's generic
  tag like ID3v2, for the atoms holding that key and the atom lofty writes it to
  (a `trkn` without a total lists only the number, and a total goes into it), so
  `trkn` and `disk` keep the part not edited. A number lofty's split leaves in its
  atom (the integer `tmpo`) is replaced too, and written back as a number.

`examples/edit_check.rs` copies a sample of a real collection into
`target/edit-check/`, then edits every copy twice: once setting every field, a
cover and other tags (a composer, a comment, a key of the format's own, and the
first other tag the file has), once putting everything back and removing the cover.
After the second edit every tag outside the nine fields must read as before. Then
the first edit goes through a [change set](#change-sets), is accepted and undone, and
every tag and the cover must be as before. It prints every failure (the collection
itself is only read):

```bash
cargo run --release -p vcmp-library --example edit_check -- /path/to/music 40
```

## Change sets

`src/changes.rs`. The tag edits, label edits and playlist deletions AI agents ask
for (see the [MCP spec](specs/2026-10-19-mcp-library-server.md) and the
[labels spec](specs/2026-10-28-user-labels.md)) are recorded as change sets, which
the owner reviews; nothing is written, labelled or deleted before a set is
accepted.

- **Submitting** (`Library::submit_tag_changes`) checks every edit (`TagEdit::validate`),
  that every track is in the library and available and listed once, and that every
  key of `TagEdit::other` fits its file's kind of tag. It records what each file
  should get and, for every tag the edit may change, the values the database holds
  now (`change_values.before`): the nine fields' keys it edits, the keys of its other
  edits, and what the nine fields' writes remove on the way (album artists in user
  text frames, ID3v2.3 date parts, a total cleared with its number). An edit of the
  cover also keeps the file's current cover under `images/undo/`, named by its hash.
- **Accepting** (`Library::accept_change_set`) writes each file through the tag
  writer, which first checks that the file still holds the recorded values (and
  cover). A file that doesn't is left alone and reported as a conflict, so an agent
  can't overwrite what changed since it looked. Each file's result is `written`,
  `conflict` or `failed`; the set is `accepted` when every file was written,
  otherwise `partly_written`. What each written file reads back as is recorded
  (`change_values.after`) as soon as it is written, read from the file itself, and
  again from the library once the set is done.
- **Undo** (`Library::submit_undo`) is a new pending set, reviewed like any other. It
  writes back the `before` values of every written file, raw: all values of a key
  and in their form, so two `ARTIST` fields or a full `DATE` come back as they were
  rather than as the nine fields' reading ("First, Second", the year). Only for undo,
  the writer takes the nine fields' keys in `TagEdit::other`; it then checks them by
  the entries and leaves the comparison of the scan's reading out, since that
  follows from them. The files must still hold the `after` values, or they are
  conflicts. Once every file is written, the first set is `undone`. A set is undone
  at most once; a set with an undo pending can't get a second one.
- **Label sets** (`Library::submit_label_changes`) take, per track, labels to put on
  and take off by name, and the new labels to create. Every name must be a label
  or one of the new ones (ignoring case; stored as the label spells it), a new name
  must be free, a track may be named once and can't both get and lose a label.
  Edits that change nothing are left out. Every new label is kept, whether a track
  gets it or not, so a set may only create labels (no `change_files` rows); a set
  with neither is refused.
  They reuse the tag sets' tables: a track is a `change_files` row (with its path),
  each label `label:` and the name in `change_edits` (the name to put it on, NULL
  to take it off) and `change_values` (the name when the track holds it, `[]` when
  not). Accepting (in one transaction) creates the new labels, or takes a label of
  that name made meanwhile as it is (`change_labels.created` 0), then changes each
  track whose labels are still as the set found them. A track whose labels
  changed, or that would get a label deleted since, is a `conflict`. Undo works as
  for tags, and accepting the undo also deletes the labels the first set created
  that no track holds any more. A set that only created labels can be undone while
  one of them still exists; its undo has no tracks and only deletes them. Sets
  never mix tags and labels.
- **Playlist deletions** (`Library::submit_playlist_deletion`) wait for review too, and
  can't be undone. Playlist ids are reused, so accepting checks that the id still
  names a playlist of the name asked about; when it doesn't, nothing is deleted and
  the set stays pending for the owner to reject.
- **Rejected and discarded.** `Library::reject_change_set` finishes a set unwritten.
  `Library::discard_pending_change_sets` marks every set still pending as discarded,
  for the app to call when it starts: VCMP keeps no set waiting across a restart.
  A pending set with written files was cut off while it was written (VCMP quit or
  crashed): it becomes `partly_written` instead, so the files it wrote can be undone.
- **Retention.** A finished set is kept for 30 days (`CHANGE_SETS_KEPT`), with the
  covers only it refers to: `Library::expire_change_sets` deletes older ones.
  `Library::delete_finished_change_sets` deletes every finished set at once and
  returns the bytes its covers took (`Library::finished_change_sets_size` tells
  beforehand); the database rows are small next to them. Pending sets are never
  deleted. A deleted set can't be undone.

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
- `Album::fetched_cover` says the artwork shown is a fetched cover.
  `Library::clear_fetched_cover` forgets it, together with the automatic lookup's
  result for the album, so that lookup may run again.

## Lyrics

`src/lyrics.rs` ([spec](specs/2026-11-07-lyrics-in-now-playing.md)). Lyrics are
for the now playing view; nothing here writes a music file.

- **Read when a track starts**, never at scan time (`read_local_lyrics`, called off
  the UI thread by the app): the `.lrc` file of the same name (the track's path
  with the extension `lrc` or `LRC`), and the tag VCMP reads. In ID3v2 (MP3, WAV):
  SYLT frames timed in milliseconds (lofty keeps SYLT as binary data and parses it
  on request) and USLT frames, those without a description first. In Vorbis
  comments (FLAC, Ogg Vorbis): `LYRICS` and `UNSYNCEDLYRICS`. Plain lyrics are in
  `track_tags` too, but the file is read again so one read always matches it.
- **LRC** (`parse_lrc`): `[mm:ss]` to `[mm:ss.xxx]` stamps, several on one line
  (the line repeats at each), `[offset:+/-ms]` (positive shows lines earlier),
  other `[tag:value]` lines ignored, word stamps `<mm:ss.xx>` removed, and lines
  without a stamp dropped. Text with at least one stamp counts as synced, wherever
  it is: an `.lrc` file, USLT, `LYRICS`. A SYLT whose entries start new lines with a
  line break and continue them without one (syllables) is joined into lines.
- **`.lrc` files** are read up to 1 MiB, as UTF-8 (with or without a BOM), UTF-16
  with a BOM, and otherwise Latin-1.
- **Which lyrics show** (`choose_lyrics`): synced first, then plain, and within
  each the `.lrc` file, then the tags, then LRCLIB. An `.lrc` file without stamps
  is plain text and comes before the tags' plain text.
- **LRCLIB** (`vcmp-integrations::lrclib`, behind "Find lyrics online",
  `integrations.lyrics_online`, off unless set): asked only when nothing synced is
  on disk and the track has an artist. Its exact `get` with the title, artist,
  album (when there is one) and duration in whole seconds; LRCLIB matches the
  duration within 2 s. A 404 or an instrumental record is "not found"; anything
  else that isn't an answer is a failure, and a 429 waits at least for its
  Retry-After. One request at a time, at least 300 ms apart; a lookup asked for
  while another runs replaces any that waits.
- **The cache** (`lyrics_lookups`, `Library::lyrics_lookup` and
  `set_lyrics_lookup`): every answer, "not found" included, with the question it
  answered. An answer counts only for that question, so a track whose title,
  artist, album or duration changed (a tag edit, an agent's change, a rescan) is
  looked up again (`lyrics_lookup_due`). "Not found" is asked again after 30 days
  (`LYRICS_RETRY_NOT_FOUND`). Failures aren't stored; the app waits 10 minutes, or
  the Retry-After, before asking again for that track. Rows are keyed by source
  and id, so tracks of another source can use the table later; tracks are never
  deleted, so rows are never orphaned.

## Animated pictures

`src/animation.rs`. A GIF, an animated WebP or an APNG can be an album's cover, an
artist's photo or a playlist's picture. Everything that shows pictures small
(grids, lists, the player bar, playlist mosaics) keeps using a still JPEG made from
the first frame; the original is kept beside it for the few places that play it
(see [docs/ui.md](ui.md#animated-pictures)).

- **Playlist pictures and artist photos:** `set_playlist_image` and
  `set_artist_photo` make the JPEG as before and, when the image has more than one
  frame and is at most 16 MiB (`MAX_ANIMATION_BYTES`), write the original next to
  it under the same name with its own extension. `animation::beside` finds it from
  the JPEG's path. Replacing, removing or deleting the picture deletes both files.
- **Album covers stay VCMP's:** an animation never goes into the music files. The
  tag editor writes a PNG of its first frame as the cover, which other players show,
  and `Library::keep_cover_animation` keeps the original under `images/covers/`,
  named by the hash of that PNG: the same hash that names the album's cached
  artwork once the scan reads the cover back. `Library::cover_animation` looks for
  that file, so an album plays the animation exactly while its files hold that
  still; a cover changed later, by VCMP or another program, ends it without any
  bookkeeping.
- **A folder's `cover.gif`** (or an animated `cover.webp` or `cover.png`) is album
  art like any folder image, cached with its original under `artwork/`, so
  `cover_animation` returns the cached original for GIF, WebP and PNG artwork. Only
  decoding says whether it moves. Folder images still come after art embedded in
  the files.
- **Decoding:** `animation::decode_frames` gives every frame, composited to the
  full canvas, as RGBA at most the size asked for, with the delay the file gives (0
  where it leaves the delay to the viewer). It keeps at most 1,000 frames
  (`MAX_FRAMES`) and 64 MiB (`MAX_DECODED_BYTES`): past the budget, the frames so far
  are shrunk and the rest follow at the smaller size. A still picture, or a format
  that can't move, gives `None`. The checks read the header where the format has
  one (APNG's animation chunk, WebP's animation flag) and decode two frames of a
  GIF.

## Change feed

`src/feed.rs`. What a VCMP server gives the apps that mirror its library
([server.md](server.md#the-change-feed)); the app's own library keeps the revisions
too, unused.

- **Revisions.** Every track has one. Three triggers of schema version 18 keep them:
  a new track takes the next revision; an update that changes anything the feed
  carries (path, folder, size, modification time, availability, format, the shown
  fields, duration, recording ID, added date, album) does too; and an album whose
  `artwork_hash` changes gives all its tracks the next revision, since new folder art
  changes no track row. Triggers compare old and new values, so a rescan that
  changes nothing leaves every revision alone. Migration 18 numbers the tracks
  already there by id.
- **Reading.** `Library::track_feed` gives up to a page of tracks after a
  `FeedCursor` (a revision and a track id, since an album's tracks share one), in
  that order, unavailable ones included. The MusicBrainz release, artist and album
  artist IDs come from `track_tags`, the first non-blank value as the scan reads
  them; a changed file always changes its size or modification time, so they never
  change without a new revision. `Library::feed_head` is the newest cursor.
- **Artwork by key.** `Library::artwork_by_key` gives the cached files of an
  artwork key only when the key has a key's shape (hex, a dot, a lowercase
  extension), since keys come from clients.

## Server tracks

`src/mirror.rs`, with the writes in `scan::apply_remote`. The app mirrors the
catalogue of every VCMP server it is connected to into its own library
([server.md](server.md), [ui.md](ui.md#music-servers)), so server music is in the
same pages, search, playlists, favourites, labels and play history as local music.
The client side (the HTTP calls and the sync loop) is `crates/vcmp-client`.

- **A server track is a `tracks` row** with a `remote_tracks` row beside it. Its
  `path` is `vcmp-server:<server row id>:<server's track id>` (`remote_key`), which no
  file has: it is the track's identity, and `Track::key()` hands it to the window
  and the player (`TrackRef::server`), which find the row again with
  `track_id_for_path` as for local files. `Track::file` is None for a server track;
  everything that opens a file takes `file`, so the compiler lists every such place.
  `Track::remote` holds the server, its id, folder and path inside the folder.
- **Grouping** uses the scanner's own functions with the key prefixed
  `server:<server row id>:`, so an album held locally and on a server is two albums
  (`Album::server` says whose), while artists stay one by name. MusicBrainz release
  IDs group as they do locally.
- **Applying a feed page** (`Library::apply_server_page`) writes its tracks and the
  new cursor in one transaction, after the page was fetched: the write lock is never
  held across the network. A row keeps its id through every change, so playlists and
  hearts follow it. The album's artwork key is the server's (checked to have a key's
  shape before it names a file); the app fetches the original and the thumbnail into
  its own artwork cache under the same names (`missing_server_artwork`,
  `store_server_artwork`).
- **From the start again** (a new server, a removed one added again, or a server
  behind the app's cursor, as after a restore from a backup): the feed lists every
  track the server ever had, so the rows of that server it doesn't list are marked
  unavailable (`forget_unseen_server_tracks`).
- **Removing a server** marks its tracks unavailable and resets its cursor; its rows
  stay. Adding the same server again (same server id) updates the row, and the full
  sync makes the tracks available again with the same ids.
- **Local only:** the scanner's known files, re-linking, the added-date backfill
  and the artwork pass leave server rows and albums alone (a test syncs a server,
  scans a local folder holding a file with the same tags, and checks that no
  mirrored row or album changed). The library check, M3U import (by path and by tags)
  and change sets take local tracks only; M3U export leaves server tracks out and
  lists them in `M3uExport::from_servers` (the owner's choice).
- **Reachability is not stored:** a server that can't be reached is the app's
  connection state, never written to rows; its tracks stay available here.

## Watching

`LibraryWatcher` watches the reachable sources and calls back once no change has
arrived for a quiet period, so copying an album produces one notification. It never
scans by itself. Changes that never pause (a sync tool busy in a folder) are reported
at the latest 30 quiet periods after the first one. The app's `LibraryService` (see [ui.md](ui.md#music-folders-and-scanning))
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
- **M3U export** (`src/m3u.rs`, `Library::export_playlist`) leaves a VCMP server's
  tracks out (a playlist file can only point at files) and names them in
  `M3uExport::from_servers`. It writes a playlist as a
  UTF-8 file without a byte order mark and with `\n` line ends, as the spec decided
  (`docs/specs/2026-10-22-listening-and-library-features.md`, "M3U"):
  `#EXTM3U`, then per entry, in order and repeats included, an
  `#EXTINF:<seconds>,<Artist> - <Title>` line (whole seconds, rounded; `-1` for an
  unknown length; the title alone without an artist; line breaks in tags become
  spaces) and the file's path. An existing file is overwritten; the save dialog
  asks first.
  - **Paths** are relative to the playlist file's folder when the track is on the
    same drive: the same filesystem (`st_dev`) on Linux and macOS, the same drive
    letter or network share on Windows. They are written with `/` on every
    platform, which Windows accepts too, so the file also works on a phone or
    another system. Otherwise the path is absolute, in the platform's own form. The
    folder is resolved (symlinks, `..`) first, as track paths are. A relative path
    that would start with `#` (a folder "#1 Hits" beside the playlist) is written
    as `./#1 Hits/…`, so it isn't read back as a comment.
  - **Missing files:** an entry whose file is gone is written with the path the
    library last saw, so the playlist is whole again when the drive is back; other
    players skip it meanwhile. `M3uExport::missing` counts them for the page.
  - **Left out:** a path that a UTF-8 file of lines can't hold (not valid UTF-8,
    as old Linux file names can be, or holding a line break) is left out and
    listed in `M3uExport::left_out`.
  - `m3u_file_name` suggests the file name: the playlist's name with `<>:"/\|?*`
    and control characters replaced by `_`, trailing dots and spaces dropped, and
    `.m3u8`.
- **M3U import** (`src/m3u.rs`, `Library::import_playlist`) makes a new, ordinary
  playlist from a `.m3u` or `.m3u8` file, with no link back to it
  (`docs/specs/2026-11-07-m3u-playlist-import.md`):
  - **Reading:** the whole file is UTF-8 (a byte order mark is dropped) whatever its
    extension; when it isn't valid UTF-8, the whole file is read as Windows-1252.
    Lines end in `\n`, `\r\n` or `\r`. Every line that isn't blank and doesn't start
    with `#` is an entry; the `#EXTINF:<seconds>,<text>` line before it gives its
    length (a negative one is unknown; attributes after the length are skipped)
    and its title. Other `#` lines are skipped.
  - **By path first:** a `file:` URL is decoded; other URLs (`http://`,
    `content://`) have no path. A path takes `/` and `\` alike, is resolved against
    the playlist file's folder when relative, and has `.` and `..` resolved
    without the filesystem. It matches the track stored at that path, available or
    not, like `track_id_for_path`; failing that, a file that exists is looked up
    again by its real path (letter case on Windows, links). A Windows network or
    device path is never looked up unless it lies in a music folder: a downloaded
    playlist naming `\\host\share\…` would make Windows contact that host.
  - **Then by tags:** an entry whose path gives no library track (gone, outside the
    music folders, a URL) and that has an `#EXTINF` text is matched by it. The text
    is split at the first " - " into artist and title, compared with every track's
    artist and title after folding (lowercase, diacritics removed through Unicode
    decomposition, one space between words); when that finds nothing, the whole
    text is compared with titles alone, whatever the artist. Of several tracks,
    the one whose length is closest to the entry's wins, however far; then an
    available one; then the first in the library. The library's tags are read
    once, on the first entry that needs them.
  - **The playlist** holds the matched tracks in file order, repeats included, and
    is named after the file without its extension (the caller's fallback name when
    that is blank). A name another playlist has (ignoring case, as the database
    does) gets " 2", " 3" and so on. The playlist and its entries are written in
    one transaction. When nothing matched, or the file had no entries, nothing is
    made.
  - **`M3uImport`** says what was made (`playlist`, `name`), how many entries the
    file had, and lists the unmatched ones (`M3uEntry`: location, `#EXTINF` text
    and length) in file order.

## Labels

`src/labels.rs`. A label is the owner's own name for tracks ("chill", "workout") with
a colour, kept in the database only: nothing is read from or written into files.
The agreed decisions are in the [labels spec](specs/2026-10-28-user-labels.md).

- **Tracks only.** A label belongs to tracks; labelling an album labels the tracks it
  has now (`Library::album_tracks`), so a track added to the album later has none.
- **Links outlive files**, like playlist entries: tracks are never deleted, so a
  label stays on a track whose file is gone and shows again with it; a re-linked
  file keeps its labels (a test checks both). Counts (`Label::track_count`), label
  pages (`Library::label_tracks`) and the filters' queries count available tracks.
- **Names** are trimmed, 1 to 40 characters (`LABEL_NAME_MAX`), and unique ignoring
  case. SQLite's `COLLATE NOCASE` folds ASCII only, so `create_label` and
  `rename_label` also compare lowercased names in Rust (`Library::label_named`).
- **Colours** are a name from `LABEL_PALETTE` behind `palette:`, which the UI draws
  in a light and a dark variant, or `#rrggbb` in lower case, drawn as it is
  (`valid_label_colour`).
- **Reads for the window:** `label_links` (every link, for the chips rows show),
  `album_labels` (for each album the labels any of its available tracks hold, for
  the album filter), `tracks_with_labels` (tracks holding every label given, for
  agents).

## Favourites

`src/favourites.rs`. The tracks the owner liked, with when, kept in the database
only. The agreed decisions are in the
[favourites spec](specs/2026-11-04-favourites-and-lastfm-loves.md).

- **Its own table, not a playlist.** `favourites` holds a track id and the time it
  was liked, so no playlist operation needs to guard against it. The window draws it
  as a playlist ([ui.md](ui.md#playlists)).
- **Likes outlive files**, like playlist entries and labels: a heart stays on a track
  whose file is gone and follows a re-linked file (a test checks both).
- **Liking** (`Library::like`) a set of tracks happens in one transaction; a track
  liked already keeps the time it was first liked. `unlike` takes tracks out.
- **Reads:** `liked_tracks` (the set, for the hearts rows show), `favourites` (newest
  first, unavailable tracks included; tracks liked together keep their album order)
  and `favourites_summary` (count, available count, duration, for the card).
- **M3U export:** `Library::export_favourites` writes them newest first through the
  same writer as `export_playlist`.
- **Last.fm import:** `like_at` likes tracks as of given times (Last.fm's love
  dates); a track liked already keeps its own time, and an id that is no track is
  skipped. `track_names` gives every track's id, artist and title, available or
  not, for the tracks with an artist; the matching itself is
  `vcmp_integrations::lastfm::match_loved` ([ui.md](ui.md#playback), "Last.fm
  scrobbling").
- **Love queue** (`src/loves.rs`): `queue_loves`, `queued_loves`,
  `queued_love_count`, `remove_loves` and `clear_loves`, like the scrobble queue.
  Loves hold the artist and title as they were, which is how Last.fm knows a track.

## Library check

`src/check.rs`. What the Library check page lists: duplicate tracks, tracks with
missing tags and albums without a cover. The agreed decisions are in the
[library check spec](specs/2026-11-07-library-check.md).

- **Read-only.** Every query only reads. The one thing the module writes is the
  "Not duplicates" marks; it never touches a music file. A test checks that the
  queries and the marks leave every music file's bytes and modification time, and
  every other table, as they were; another checks the module's source and the
  page's glue reach for no file system call.
- **Duplicates** (`Library::duplicate_groups`): available tracks with a track artist
  and a title the file holds (not the file-name fallback). Title and artist are
  folded in Rust (Unicode NFKD, the accents of the diacritical mark blocks dropped,
  lower case, recomposed, runs of white space made one), since SQLite folds only
  ASCII. Other marks stay, so the Japanese voicing marks keep "ga" apart from "ka". Tracks with the same folded pair
  are sorted by length, and a group runs on while each track is at most 2 s longer
  than the one before (`DUPLICATE_WINDOW`), so a group can span more than 2 s.
  Groups of one are dropped; groups come by artist, then title.
- **"Not duplicates"** (`mark_not_duplicates`) stores the group's track ids under a
  new group id. A group the check finds is `hidden` while every one of its tracks is
  in one stored group: a copy going missing keeps it hidden, a new copy shows it
  again. `unmark_not_duplicates` forgets every stored group that holds all the
  tracks given. Track ids outlive moved files, like favourites.
- **Missing tags** (`missing_tags`): available tracks lacking a title, artist, album,
  track number or year, grouped by album in library order (`AlbumGaps`). The title
  is missing when `track_tags` holds no non-blank `TrackTitle` item: `tracks.title`
  always has a value, the file name when the file holds no title. The other four
  are their `tracks` columns being empty.
- **Missing covers** (`albums_without_cover`): albums with available tracks, not an
  Unknown album, whose artwork state says a scan looked and found none
  (`artwork_state = 2`), and that have no fetched cover. Albums a scan hasn't looked
  at yet are left out until one has.

## Play history

`src/plays.rs`. Every play that counted, kept in the database only. The agreed
decisions are in the [play history spec](specs/2026-11-07-play-history.md).

- **A log, not counters.** `plays` holds one row per play: the track id, when the
  play started (Unix ms) and the origin of its context as the app named it
  (`album:12`, `playlist:3`, `label:5`, `folder:<path>`, `search`, `songs`,
  `favourites`), or NULL for a track from Up next. The origin is kept as text, so a
  new kind of context needs no migration, and an id in it may no longer exist (a
  deleted playlist). Counts and last played are worked out through the
  `(track_id, played_ms)` index; nothing is cached on `tracks`.
- **What counts** is decided by the app (Last.fm's rule, for every library track;
  [ui.md](ui.md#play-history)). `Library::record_play` writes one row.
- **Plays outlive files**, like playlist entries and favourites: they stay with a
  track whose file is gone and follow a re-linked file.
- **Reads:** `play_stats` (every played track's count and last play, over all time
  or since a moment, for ranges such as "last month"), `track_play_stats` (one
  track).
- **Clearing** (`clear_play_history`) deletes every play and nothing else: the
  scrobbles waiting for Last.fm, the added dates and the favourites stay.
- **Added dates.** `tracks.added_ms` is set when the scanner first inserts a row.
  Before play history every track of a library's first scan shared that scan's
  time, so the first scan after migration 15 backfills the rows that existed
  before it, once: each takes the earliest of its current value, its file's
  modification time and, where the file system keeps one, its creation time. A date
  only ever moves earlier, rows first seen in that same scan keep "now", and a row
  whose file that scan doesn't find keeps its date. The scan records
  `library.added_backfilled` in the same transaction, so it never runs again. SQL
  can't read file dates, which is why this is in the scanner and not the migration.
  `Track::added` exposes the date.

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
| Upgrade to schema version 10 (every file re-read, every album's artwork looked for), files not read for two days but cache not dropped | 4.9 s | 38.8 s | 4.6 s | 48.3 s |

The Discovering phase is the same code in every version; its 4.5 to 6.5 s spread
shows the run-to-run noise of cold measurements.

Storing all tags (version 10) took the database from 3.6 to 8.5 MB on this library:
42,637 entries under 155 keys for 3,486 tagged tracks. The values hold 1.7 MB of
text, two thirds of it lyrics (the largest single value is an 825 KB lyrics tag);
the keys, their index and SQLite's page overhead take the rest. A rescan afterwards
took 0.4 s.

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

- Added dates of tracks already in the library before play history come from their
  files: a library copied from elsewhere, or files VCMP or another program edited
  (a tag write replaces the file), get the copy's or the edit's date. A track whose
  file was missing at that first scan keeps its first-scan date, and with
  "Update automatically" off the backfill waits for the next rescan.
- CUE sheets aren't read: an album ripped as one file plus a `.cue` shows up as a
  single long track.
- Links inside a music folder aren't followed: a linked file or folder
  (`~/Music/classical -> /mnt/nas/classical`) adds nothing, and the scan doesn't
  say so. Add the link's target as a music folder of its own.
- The tag writer's read-back compares tags, format and duration, not the audio: a
  writer that damaged the audio data but kept the headers would pass.
- A folder image is noticed by its name, size and modification time. An image
  replaced by another of the same size with its old modification time restored
  goes unnoticed until a track of the album changes.
- A fetched cover belongs to the album's group key. VCMP's tag editor carries it to
  the album a renamed title or album artist leads to, but when another program
  changes those tags, the album gets a new key and offers the search again.
- SQLite's case-insensitive comparison only folds ASCII. Artist names that differ
  only in the case of a non-ASCII letter ("Dvořák" and "DVOŘÁK") become two artists,
  and sorting is not locale-aware. Album grouping lowercases with full Unicode rules.
- Multi-value artist tags are joined with ", " into one artist name.
- MP3 durations count whole frames, so they can be a fraction of a second longer
  than what plays.
- Tag writing updates only the tag VCMP reads. An ID3v1 or APE tag in an MP3, a
  RIFF INFO list in a WAV file, or the text chunks of an AIFF file keep their old
  values, and players that prefer those tags (Windows Explorer reads RIFF INFO in
  WAV files) show them.
- MP4 keeps no picture types, so in an M4A file the cover is the first picture. A
  file whose first picture is another picture (a back cover saved first) has that
  picture replaced or removed by a cover edit.
- lofty upgrades an old numeric `gnre` genre atom to a `©gen` text atom when it
  reads an M4A file, so any write turns `gnre` into `©gen`; when the file has
  both, the `gnre` atom is dropped. The scan lists the genre the same before and
  after.
- lofty writes an MP4 number with as few bytes as it needs, so a number atom VCMP
  writes (`tmpo`, `stik`) may be shorter than the four bytes some players write.
- An `.m4a` file holding chapter pictures as a video track (some enhanced podcasts)
  may count as video and be skipped.
- Tag writing on M4A files goes through lofty's least tested writer.
  Run `examples/edit_check.rs` on copies of real files before relying on it; the
  read-back check refuses to replace a file that reads back wrong.
- A comment or lyrics frame VCMP writes gets the unknown language `XXX`, whatever
  language the frame it replaces had.
- Lyrics: a SYLT frame timed in MPEG frames is ignored, and an `.lrc` file in an
  encoding other than UTF-8, UTF-16 with a BOM or Latin-1 (Windows-1250, GBK)
  shows wrong characters. Lyrics in an MP3's APE or ID3v1 tag aren't read, and
  lyrics can't be searched.
- ID3v2 keeps a total only behind its number. A total for an MP3 or WAV file
  without the number, or clearing a movement number while its total stays, would
  read back as the number 0, so the read-back check refuses that file
  (`TagWriteError::Verify`).
- When VCMP has to create the tag it writes (an MP3 with only ID3v1, a WAV with only
  RIFF INFO), only the fields the scan reads go into it, and the check that
  untouched tags keep their values is skipped for that file. From then on the
  file's tags are listed from the new tag, some under other names: lofty reads
  RIFF INFO's encoder as `EncoderSoftware` and the ID3v2 frame it writes for it
  (`TSSE`) as `EncoderSettings`.
- Some files hold values with control characters: 8 values on the owner's library,
  URL frames from one store that start with a stray text-encoding byte. VCMP
  writes the cleaned value but refuses to write such a value back. An undo, which
  puts back what a file had, is the exception.
- An undo puts the old cover back as the front cover. A file whose cover was a
  leading picture without a type gets the same image, typed as the front cover.
- An undo doesn't remove a tag VCMP created (the ID3v2 tag of an MP3 that had only
  ID3v1); it puts the values back into it.
- On Windows a file that another program holds open cannot be replaced, so the
  edit fails with `TagWriteError::Replace`. The tag editor holds back the edit of
  the file VCMP's own playback has open until the engine lets go of it (see
  `docs/ui.md`, "Tag editor").
- Replacing a file gives it a new inode (Unix) or file ID (Windows). Hard links keep
  the old content, and on Unix the file's owner becomes the user running VCMP.
- A crash between the copy and the rename can leave a `.vcmp-<random>.tmp`
  file next to the music file. Scans ignore it, and it can be deleted.
- Setting the cover of one track doesn't change the album's other tracks. The album
  artwork comes from its first two tracks, so to change it, edit the whole album.
- A file whose tag lofty can only read in part (a damaged frame) may lose that part
  when it is written. The read-back check compares what lofty can read.
- Playlist pictures made from covers that no longer fit any playlist stay in
  `mosaics/` until the cache is deleted. Each is about 30 KB.
- Playlist search compares names in Rust after reading every playlist, and lowercases
  them the simple way, so it ignores case but not diacritics.
- An animated cover is kept as the tag editor starts writing it, so one whose
  writes all failed still leaves its file in `images/covers/`, and so does an
  animated cover later replaced by another. Nothing deletes those files yet.
- An animated folder image moves only when the files hold no cover of their own,
  since embedded art comes first.
- A GIF's loop count is not read: every animation loops.
- M3U export decides "same drive" on Linux and macOS by filesystem. A missing file
  counts from its nearest folder that exists: an unmounted drive whose mount point
  is a folder on the system disk gives a relative path through that folder, which
  still works on this machine. Btrfs subvolumes count as separate filesystems, so
  a playlist saved in another subvolume than the music gets absolute paths.
- M3U import reads a file that isn't UTF-8 as Windows-1252. A legacy file in
  another code page (CP1250 for Czech, CP1251 for Cyrillic) gets wrong letters in
  its paths and titles, so those entries match neither way. Saving the file as
  `.m3u8` in the player that made it fixes that.
- Tag matching lets the length only break ties, so a live or remastered version
  with the same artist and title can stand in for the one the file meant, and the
  import counts it as matched. An entry with a title alone matches that title by
  any artist.
- A path in other letter case than the file's matches on Windows, where the real
  path gives the file's own case; on macOS it falls back to the tags.
- Labels are in the database only. Deleting `library.sqlite3` (or losing it) loses
  every label, and nothing in the files can bring them back.
- The owner's own label edits keep no history and can't be undone; only agents'
  label sets can.
- The Library check folds accents by Unicode decomposition, so letters that don't
  decompose ("o" with a stroke, the German sharp s) don't match their plain forms,
  and live or remastered versions with the same title and artist within 2 s show as
  duplicates until marked "Not duplicates". The same album in FLAC and MP3 already
  merges into one album with doubled tracks, so each of its tracks shows as a
  duplicate.
