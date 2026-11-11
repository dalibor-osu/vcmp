# Codebase review: reuse and duplication

Reviewed `main` at `2419170` on 2026-11-04. The review looks for code that repeats,
code that re-implements a helper the codebase already has, and code that could be
simpler. It changes no code; each finding names the simpler form and what a merge
has to keep.

## Scope

- Read closely: the services in `crates/vcmp-app/src` (player, library, tag editor,
  agents, cover fetch, artist photos, albums, library view), `vcmp-integrations`
  (MusicBrainz, fanart.tv, the Last.fm and update-check transports, Discord) and the
  two Slint pickers.
- Read in part: `vcmp-mcp` (cover loading in `tools.rs`, track choosing in
  `playback.rs`) and `vcmp-library` (`lib.rs`, `artwork.rs`, `artist_photos.rs`,
  `playlists.rs`).
- Outlined only: `vcmp-playback`, the library's scan, edit and change-set modules,
  and most Slint pages. A second pass would start there.

The codebase is consistent and well commented. Most duplication comes from features
built one after another on the same pattern (two online pickers, three services with
a background thread), where the second copy was made by adapting the first.

## Summary

| # | Finding | Sites | Payoff |
|---|---|---|---|
| 1 | The online picker exists twice, in three layers | 6 files | High |
| 2 | Background-message plumbing copied in three services | 3 files, plus `post` in 4 | High |
| 3 | Decoding an image into a Slint RGBA buffer, 7 times | 6 files | High |
| 4 | Queue origin keys built and parsed in three places | 2 files | High (can drift) |
| 5 | `play_*` methods repeat the shuffle and start logic, with different guards | 4 methods | Medium |
| 6 | The weak-upgrade callback closure, 132 times | 21 files | Medium |
| 7 | "Everything by an artist" assembled twice; artist page read twice | 2 crates | Medium |
| 8 | Stored pictures: artist photo and playlist picture repeat each other | 3 library files | Medium |
| 9 | MusicBrainz `search` and `candidates` repeat their query set-up | 1 file | Medium |
| 10 | Boolean settings parsed and written by hand | 14 sites | Low |
| 11 | HTTP transports repeat the response tail; `USER_AGENT` lives in `musicbrainz` | 4 files | Low |
| 12 | Four lists of image extensions that disagree | 4 files | Low |
| 13 | Slint focus boilerplate on every pressable row | 12 files | Low |
| 14 | Small items | 4 files | Low |

Suggested order: 4 first (smallest change, removes a drift risk), then 3, 2 and 1
(each makes the next feature of its kind cheaper), then the rest as files are
touched anyway.

## 1. The online picker exists twice, in three layers

"Find cover online" and "Find photo online" are the same feature with different
sources, and each layer was copied.

- **Worker** (`vcmp-integrations`): `CoverPicker` (`musicbrainz.rs:593-699`) and
  `PhotoPicker` (`fanart.rs:207-310`) have the same job enum (search with a stop
  flag, download), the same worker loop that skips `Searched` once stopped, and the
  same `current` / `replace_current` / `cancel`.
- **App glue** (`vcmp-app`): `CoverFetch` (`cover_fetch.rs`) and `ArtistPhotos`
  (`artist_photos.rs`) repeat the search counter, `next_search`, the found keys plus
  tiles model, `chosen`, `find` / `choose` / `cancel` / `close`, the "drop answers
  of old searches" check in `handle`, the `thread_local` plus `post` pair, and a
  byte-identical `decode` (`cover_fetch.rs:330`, `artist_photos.rs:379`).
- **Dialog** (Slint): `cover-picker.slint` and the first 200 lines of
  `photo-picker.slint` differ only in texts, one status, round tiles and the key row.

The copies have already drifted:

- The photo picker focuses a result that arrived before the dialog was drawn
  (`photo-picker.slint:80-86`); the cover picker always focuses Cancel
  (`cover-picker.slint:70`). The fix landed in one copy.
- A search that failed after finding something shows `CutShort` for covers
  (`cover_fetch.rs:229-234`) and `Found` for photos (`artist_photos.rs:252-257`).
  This may be on purpose; one shared mapping would make it a visible choice.

**Simpler form**

- Integrations: a small `Latest` type for "the running search's stop flag"
  (`begin() -> Arc<AtomicBool>`, `cancel()`), and one generic worker started with
  two closures (search, download). `CoverPicker` and `PhotoPicker` become thin
  wrappers that pass their query type and event type.
- App: a generic `OnlinePicker<K>` holding the counter, keys, tiles, chosen key,
  `close`, `next_search` and the stale-answer check; `CoverFetch` and `ArtistPhotos`
  keep only what differs. `decode` moves to the shared helper of finding 3.
- Slint: one `ImagePicker` component with the texts, the extra status and
  `round-tiles` as properties; `PhotoKeyRow` stays where it is.

**What differs and must stay**: cover search takes the tagged release and reports
plain `String` errors; photo search takes the personal key and reports
`SearchError::Refused`. `CoverFetch` also has `remove`, `on_cover_changed` and the
Discord cover lookup; `ArtistPhotos` has an optional source (no key in the build),
the personal key and "Choose photo...".

**Tests**: inline tests in `musicbrainz.rs` and `fanart.rs`;
`crates/vcmp-app/tests/cover_fetch.rs` and `artist_photos.rs`.

## 2. Background-message plumbing copied in three services

`LibraryService`, `TagEditor` and `AgentService` each carry the same machinery for
messages from a worker thread:

- the `Delivery`-to-`notify` closure that posts `drain` to the event loop
  (`library_service.rs:81-91`, `tag_editor.rs:123-133`, `agents.rs:106-116`);
- `drain` (`library_service.rs:273`, `tag_editor.rs:760`, `agents.rs:739`);
- the `recv_timeout` loop for tests without an event loop
  (`library_service.rs:260-271`, `tag_editor.rs:312-323`, `agents.rs:406-417`);
- the `send = |message| { sender.send(message); notify() }` closure in each worker
  (`library_service.rs:229-232`, `399-401`, `tag_editor.rs:664-667`, `731-733`).

A related pair repeats in four more files: a `thread_local! { static X:
RefCell<Weak<T>> }` and a `post` that upgrades it inside `invoke_from_event_loop`
(`player_service.rs:1254`, `cover_fetch.rs:128`, `artist_photos.rs:163`,
`lastfm_service.rs:204`).

**Simpler form**: an `Inbox<M>` next to `Delivery` with `poster()` (a cloneable,
`Send` handle whose `post(message)` sends and notifies), `drain(apply)` and
`wait_while(busy, timeout, apply) -> bool`. One generic
`post_to(&'static LocalKey<RefCell<Weak<T>>>, action) -> bool` replaces the four
`post` bodies.

**What differs and must stay**: the wait predicates (a scan running; writing or
releasing; writing) and the message types. The per-service `thread_local` stays,
because the notify closure needs a path to the service that is `Send`.

**Tests**: `crates/vcmp-app/tests/library_service.rs`, `tag_editor.rs` and
`agents.rs` run with `Delivery::Manual`.

## 3. Decoding an image into a Slint RGBA buffer, 7 times

The same four lines (`thumbnail(px, px).into_rgba8()`, then
`SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(...)`) appear at:

- `cover_fetch.rs:330-339` and `artist_photos.rs:379-388` (identical functions);
- `tag_editor.rs:1129-1138` (`preview`);
- `albums.rs:412-422` (album grid);
- `player_service.rs:1002-1013` (large cover) and `1183-1192` (queue covers);
- `moving_pictures.rs:232-236` (the buffer step only).

The reason for RGBA over RGB is written down once, at `albums.rs:415-416`; the other
sites rely on it without saying so.

**Simpler form**: a `pictures` module in `vcmp-app` with `rgba_buffer(&RgbaImage)`,
`thumbnail_buffer(DynamicImage, px)` and `decode_thumbnail(&[u8], px) ->
Option<Image>`, carrying that comment.

**What differs and must stay**: threads must return the `SharedPixelBuffer` (it is
`Send`), not the `Image`; only UI-thread callers use `decode_thumbnail`.

## 4. Queue origin keys built and parsed in three places

The queue's origin is a string key such as `"album:42"`. It is:

- built with `format!` in `play_artist`, `play_folder`, `play_playlist`,
  `play_label` and `play_album` (`player_service.rs:736`, `760`, `793`, `829`,
  `1373`);
- built again by `origin()` (`agent_playback.rs:375-386`);
- parsed by `context()` (`agent_playback.rs:389-406`);
- parsed a second time, by hand, in `show_queue` (`player_service.rs:1076-1112`).

Adding a kind of context, or changing one key, needs the same edit in three places,
and nothing checks that the two parsers agree.

**Simpler form**: an app-level `Origin` enum with one `to_key` and one
`from_key`, next to `SEARCH_ORIGIN` and `SONGS_ORIGIN`. The `play_*` methods build
an `Origin`, `show_queue` matches on `Origin::from_key` to find the name, and
`agent_playback` maps `vcmp_mcp::Context` to and from `Origin`. Keeping the MCP
type out of the window code keeps the UI independent of the MCP crate's API.

**Tests**: `crates/vcmp-app/tests/player.rs`, `crates/vcmp-app/src/saved_queue.rs`
(saved queues hold these keys, so the format must not change).

## 5. `play_*` methods repeat the shuffle and start logic

`play_artist` (`player_service.rs:725`), `play_playlist` (`768`), `play_label`
(`815`) and `play_album` (`1351`) each turn shuffle on and then send
`start: if shuffle { None } else { start }`. Their guards differ:

- `play_label` returns when there are no entries or the start is past the end;
- `play_playlist` returns when the start is past the end;
- `play_album` and `play_artist` send whatever they have.

**Simpler form**: one `play_context(entries, origin, start, shuffle)` that applies
one rule for empty lists and out-of-range starts. Pair it with finding 4 so it takes
an `Origin`.

## 6. The weak-upgrade callback closure, 132 times

Every Slint callback is wired as

```rust
state.on_x({
    let weak = weak.clone();
    move |arg| {
        if let Some(service) = weak.upgrade() {
            service.x(arg);
        }
    }
});
```

`grep "= weak.upgrade()"` finds 132 of them in `vcmp-app/src` (24 in
`player_service.rs`, 19 in `labels.rs`, 17 in `library_view.rs`). Two files already
wrote a local helper for the zero-argument case (`player_service.rs:322`,
`lastfm_service.rs:182`).

**Simpler form**: one crate-level helper. Slint callbacks differ in arity, so a
small macro fits best, for example
`state.on_seek(bind!(weak, |service, ms| service.seek(...)));`. It removes several
hundred lines. Callbacks that return a value need a variant with a default.

This one is optional: the long form is plain Rust and easy to read. It is listed
because the two local `with` helpers show the need was already felt.

## 7. "Everything by an artist" assembled twice

The artist's albums' tracks followed by their appearances is built in
`LibraryView::shown_artist` (`library_view.rs:420-442`) and again in the MCP crate's
`artist_tracks` (`vcmp-mcp/src/playback.rs:481-488`). Inside the app,
`open_artist` (`library_view.rs:684-711`) and `reload_artist` (`715-742`) both read
albums and appearances into an `OpenArtist`; the first logs read errors, the second
drops them with `unwrap_or_default`.

**Simpler form**: `Library::artist_tracks(id) -> Result<(Vec<Track>, usize)>`, the
number being where appearances start, used by both crates; and one
`read_artist(&self, artist) -> OpenArtist` in `LibraryView` with one error
behaviour.

**Tests**: `crates/vcmp-app/tests/artists.rs`, `crates/vcmp-mcp/tests/server.rs`.

## 8. Stored pictures repeat each other in the library

`set_artist_photo` (`vcmp-library/src/artist_photos.rs:89-138`) and
`set_playlist_image` (`playlists.rs:176-205`) do the same steps: decode, bound,
encode a JPEG at quality 88, name it by a blake3 hash, create the folder, write the
JPEG, write an animated original beside it, update the row, remove the old file.
The "bound and encode a JPEG" step also appears twice in `artwork.rs` (`69-76`,
`110-116`).

In `artwork.rs`, `is_folder_image` (`136-144`) and the `matches` closure inside
`folder_image` (`152-163`) both lowercase a stem and an extension and test them
against the same lists.

**Simpler form**: `encode_jpeg(&RgbImage, quality)` and
`store_picture(dir, name_prefix, data, max_px) -> Result<String>` for the two
owner-chosen pictures; `folder_image` filters with `is_folder_image` and then picks
by `FOLDER_IMAGE_NAMES` order.

**What differs and must stay**: sizes (800 and 600 px), the playlist id in the
playlist file name, quality 85 for the cache, `write_atomically` for the cache only,
and the error variants each caller returns.

Also here: `set_artist_photo` and `clear_artist_photo` repeat the artist-name query
(`95-103`, `142-149`), and a missing artist is reported as
`Error::UnreadableImage("artist N")`, which reads as a bad image.

**Tests**: `crates/vcmp-library/tests/animated_pictures.rs`, inline tests in
`artwork.rs`, `crates/vcmp-app/tests/playlists.rs` and `artist_photos.rs`.

## 9. MusicBrainz `search` and `candidates` repeat their set-up

`Searcher::search` (`musicbrainz.rs:242-291`) and `Searcher::candidates`
(`302-384`) build the same simplified query and list of tries (`246-254`,
`324-332`), filter releases the same way (`258-263`, `338-343`) and filter release
groups the same way (`280-283`, `369-372`).

**Simpler form**: `fn tries(album) -> (Vec<AlbumQuery>, AlbumQuery)` and
`fn matching_releases(&mut self, query)` / `matching_groups(&mut self, simple)`
used by both. The loops that differ (first match versus every match with a preview)
stay in each method.

## 10. Boolean settings parsed and written by hand

`"true"` / `"false"` settings are read and written at 14 sites:
`vcmp-library/src/lib.rs:139`, `143`, `agents.rs:158`, `167`, `426`, `443`,
`agent_playback.rs:128`, `preferences.rs:44`, `46`, `89`, `player_service.rs:308`,
`1860`, `vcmp-app/src/updates.rs:63`, `90`.

**Simpler form**: `Library::flag(key, default) -> Result<bool>` and
`Library::set_flag(key, on)`. `auto_update` becomes `flag(SETTING_AUTO_UPDATE,
true)`. Defaults differ (true for automatic updates, Discord and the update check;
false for agents and the queue panel), so the default stays at each call.

## 11. HTTP transports

`musicbrainz::Http` (`musicbrainz.rs:127-193`), `lastfm::Http` (`lastfm.rs:209-246`)
and `updates::fetch` (`updates.rs:99-118`) end the same way: status plus body read
to a string, errors mapped to `String`. `USER_AGENT` is defined in
`musicbrainz.rs:23` and imported from there by `lastfm.rs:28` and `updates.rs:18`.

**Simpler form**: an `http` module in `vcmp-integrations` holding `USER_AGENT`,
`agent(timeout)` (status-as-error off) and `read_text(response)`.

**Where it stops**: the agents differ on purpose (15 and 20 second timeouts, no
redirects for MusicBrainz, five redirects and 30 seconds for images), so only the
shared parts move. `vcmp-mcp` builds its own agent (`tools.rs:893`) with
`ureq` on `default-features = false` and does not depend on `vcmp-integrations`;
adding that edge for one constant is not worth it. Its User-Agent,
`VCMP/<version>`, lacks the project URL the integrations send; copying the full
string there is enough.

## 12. Four lists of image extensions

| Constant | File | Extensions |
|---|---|---|
| `PHOTO_EXTENSIONS` | `vcmp-app/src/artist_photos.rs:21` | jpg jpeg png webp gif |
| `PICTURE_EXTENSIONS` | `vcmp-app/src/playlists.rs:24` | jpg jpeg png webp gif bmp |
| `COVER_EXTENSIONS` | `vcmp-app/src/tag_editor.rs:43` | jpg jpeg png gif webp |
| `FOLDER_IMAGE_EXTENSIONS` | `vcmp-library/src/artwork.rs:20` | jpg jpeg png webp gif |

`vcmp-app` enables only the `jpeg`, `png` and `webp` features of `image`. GIF (and
BMP) previews decode in the app only because Cargo unifies the features with
`vcmp-library`'s `image`, which enables `bmp` and `gif`.

**Simpler form**: one exported `IMAGE_EXTENSIONS` in `vcmp-library` used by the
three dialogs, and `gif` listed in the app's `image` features so the app does not
depend on unification. Whether BMP is accepted everywhere or nowhere is a decision
for the owner.

## 13. Slint focus boilerplate

`focus-gained(reason) => { if reason == FocusReason.tab-navigation {
Theme.keyboard-navigation = true; } }` appears 27 times in 12 `.slint` files, usually
with `clicked => { Theme.keyboard-navigation = false; ... }`, Return and Space
handling, and an `if focus.has-focus: FocusRing { ... }` (24 uses). `LabelLine`
(`label-page.slint:23-104`) is a typical example.

**Simpler form**: a `Pressable` component in `components.slint` (touch area, focus
scope, Return and Space, focus ring, `activated` callback, `@children`) for the rows
and cards that only activate. Components with their own key handling (grids, lists,
sliders) keep theirs. `docs/ui.md` owns the keyboard rules, so this is a UI change to
agree first.

## 14. Small items

- `file_name(&Path) -> String` is defined twice (`agents.rs:1168`,
  `tag_editor.rs:941`) and written inline elsewhere (`player_service.rs:1438-1440`).
- The doc comment "One card per album, with its cover thumbnail." at
  `albums.rs:309` belongs to `cards` (`albums.rs:320`) but sits above
  `album_initial`, so `cards` has no doc and `album_initial`'s doc starts with the
  wrong sentence.

## Left as is

- The one-method traits with forwarding impls (`CoverSource`, `PhotoSource`,
  `PresenceSink`, `MediaSink`, `CoverSearch`, `PlayerControl`) look repetitive but
  are the test seams; `CoverSource` and `PhotoSource` merge on their own with
  finding 1.
- One named thread per background job is clear and cheap at this scale; a shared
  pool would add more than it saves.

## Status: 2026-11-11

Branch `feature/review-fixes`:

- **4 (origin keys): done.** `crate::origin::Origin` builds and parses every key
  (`key`, `from_key`), keeping the strings saved queues and play history hold.
  `show_queue`, `agent_playback`'s `origin` and `context`, and the Library check
  use it; a unit test pins each key's form.
- **5 (`play_*`): done.** `PlayerService::play_context` takes the tracks, an
  `Option<Origin>`, the start and shuffle, and is the one place for the empty
  list, the start past the end, shuffle and moving the start past tracks the queue
  leaves out. Album and artist pages no longer send an empty context.
- **1 to 3 and 6 to 14: not started.** Each is a larger refactor best done on its
  own branch.
