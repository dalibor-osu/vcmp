# Lyrics offset and matching

Status: agreed
Date: 2026-11-19
Area: vcmp-integrations (LRCLIB client: search fallback, candidate search),
vcmp-library (lyrics priority, lookup cache, hand-picked lyrics, offsets, schema
version 18), vcmp-app (player service, now playing lyrics mode, the lyrics picker);
all three platforms

## Context of the change

Lyrics shipped with the agreed spec
[2026-11-07-lyrics-in-now-playing.md](2026-11-07-lyrics-in-now-playing.md) and its
notes in [docs/impl/2026-11-07-lyrics-in-now-playing.md](../impl/2026-11-07-lyrics-in-now-playing.md).
How they work today ([library.md, Lyrics](../library.md#lyrics)):

- **Sources and priority.** `read_local_lyrics` (`crates/vcmp-library/src/lyrics.rs`)
  reads the `.lrc` file and the tag when a track starts. `choose_lyrics` picks synced
  before plain, and within each the `.lrc` file, then the tags, then LRCLIB.
- **LRCLIB.** `crates/vcmp-integrations/src/lrclib.rs` uses only LRCLIB's exact
  `/api/get` with the raw tag title, artist, album and duration. LRCLIB matches the
  duration within 2 seconds, so a file that is a different version of the song
  (an intro more or less, a radio edit) gets a 404. The module says why: "lyrics of
  another song or version are worse than none".
- **The cache.** `lyrics_lookups` (schema version 16) stores each answer, "not
  found" included, with the question it answered. An answer counts only for that
  question (`lyrics_lookup_due`), and a "not found" is asked again after 30 days.
- **Timing.** The LRC parser applies the text's own `[offset:]` (positive shows lines
  earlier). There is no offset the user can set; the 2026-11-07 spec listed "a manual
  sync offset control" as out of scope, together with "a 'not these lyrics' action"
  and "choosing between several LRCLIB candidates".
- **The view.** Lyrics mode of the now playing view (`crates/vcmp-app/ui/lyrics.slint`,
  `now-playing.slint`) highlights the line playing; a click or Enter on a line seeks.
  A track without lyrics hides the in-view toggle and greys the player bar's lyrics
  button. The view has a track menu (three dots) for every track.

LRCLIB also has `/api/search` (by `track_name` and `artist_name`, or a free `q`),
which returns up to 20 records, each with its id, title, artist, album, duration,
`instrumental` flag and its plain and synced lyrics. Its order is not documented.
LRCLIB asks for sequential requests 200 to 500 ms apart and for `Retry-After` to be
honoured.

Issue #104 reports that well-known songs often get no lyrics because the file is a
slightly different version, and that lyrics are sometimes a little out of sync with
no way to fix it. This change reverses the three exclusions above.

## User request

Brainstorm the feature described in issue #104, "Lyrics improvements", on a new
branch and worktree made from `main`. The issue asks for two things: a way for the
user to set a per-song offset for lyrics, because they can be a little out of sync;
and better lyrics matching and fetching, because VCMP sometimes finds no lyrics even
for well-known songs only because the file is a slightly different version of the
song (for example with or without an intro).

## Decisions

### Issue 104 has two parts: a per-song lyrics offset and better LRCLIB matching for other versions of a song. What does this change include?

| Option | What it means |
|---|---|
| Both, one spec (Recommended) | One change covers both. The offset fixes the constant shift a looser match to another version brings. Bigger PR. |
| Offset first | Only the per-song offset now; matching comes in a later spec. Small and safe, but tracks that get no lyrics today still get none. |
| Matching first | Only better matching now; the offset comes later. More tracks get lyrics, but the looser matches arrive with no way to fix their sync. |

**Answer: Both, one spec**

### Today only LRCLIB's exact lookup is used (title, artist, album, duration within 2 s), on purpose: 'a wrong match is worse than none'. Which looser matches should VCMP accept automatically when that lookup finds nothing?

| Option | What it means |
|---|---|
| Search fallback (Recommended) | After a miss, LRCLIB's search by title and artist; the best candidate with the same artist and title (version words like 'Remastered' ignored), duration within a wider window (around 15 s) wins, synced first. Fixes intro cases; some wrong versions slip in. |
| Cleaner exact lookup | Still exact get only, but retried with the title stripped of version words and without the album. Never a wrong duration, but the with/without intro case from the issue stays unsolved. |
| No automatic fallback | Exact lookup stays as is; other versions are found only when the user asks (a manual search and pick in the app). No wrong matches ever; every miss costs the user a few clicks. |

**Answer: Search fallback**

### With the search fallback, lyrics of a wrong version (live, remix, another song with the same title) can now land automatically. What can the user do about lyrics that are wrong?

| Option | What it means |
|---|---|
| Pick from candidates (Recommended) | An action on online lyrics opens LRCLIB's candidates for the track (title, album, duration, synced or not) to choose one, or 'none'. The choice is remembered. Fixes any miss or mismatch; one new dialog. |
| Reject only | A 'Not these lyrics' action hides them and stops looking that track up. Small UI; a track with wrong lyrics ends up with none, not the right ones. |
| Nothing in the app | As today: fix the tags or add an .lrc file, which wins. No new UI; with looser matching, wrong lyrics are more common and harder to get rid of. |

**Answer: Pick from candidates**

### Where is the candidate picker opened from? Today a track without lyrics hides the in-view lyrics toggle and greys the bar's lyrics button, so a track with no lyrics has no lyrics UI at all.

| Option | What it means |
|---|---|
| Now playing menu (Recommended) | A 'Choose lyrics...' item in the now playing view's three-dots menu, there for every track, with or without lyrics. One place, reaches misses too; only for the track playing. |
| Lyrics mode only | A button inside lyrics mode. Close to the lyrics; unreachable for tracks where nothing was found, which are the issue's main case. |
| Menu and track lists | Now playing menu plus the track context menu on every page. Any track, playing or not; more surface, and a picker for a track not playing can't be checked against playback. |

**Answer: Now playing menu**

### Can the user change what the picker searches for?

| Option | What it means |
|---|---|
| Editable search (Recommended) | Opens with the track's title and artist filled in; the user can edit them and search again. Handles bad tags and odd titles; a small form in the dialog. |
| Fixed search | Shows the candidates for the track's own title and artist only. Simpler; when the tags are wrong, nothing useful shows. |

**Answer: Editable search**

### The picker sends a search to lrclib.net. Does it work while 'Find lyrics online' is off?

| Option | What it means |
|---|---|
| Only with switch on (Recommended) | With the switch off the item is hidden or says to turn it on. One rule: nothing goes to LRCLIB unless allowed; one more step for people who keep it off. |
| Always, user asked | A manual search counts as consent for that one request. Handy for people who want online lyrics only on demand; the switch no longer means 'never'. |

**Answer: Only with switch on**

### What does a per-song offset belong to? A track's lyrics can change under it: an .lrc file added later wins over LRCLIB, a tag edit asks LRCLIB again, and the picker swaps the online lyrics.

| Option | What it means |
|---|---|
| Track and source (Recommended) | The offset is saved for the track together with which lyrics it was set on (.lrc, tags, or one LRCLIB record); other lyrics start at 0. Never shifts text it wasn't tuned for; re-tuning needed after a swap. |
| The track | One offset per track, whatever lyrics show. Simplest; after a swap the old offset shifts lyrics that may have been in sync. |
| Online lyrics only | Only LRCLIB lyrics can be shifted; .lrc and tag lyrics keep their own [offset:]. Covers the version-mismatch case; out-of-sync local files stay unfixable in the app. |

**Answer: Track and source**

### How does the user set the offset of synced lyrics in lyrics mode? The search fallback can bring shifts of up to around 15 s (an intro), while the issue's 'a little out of sync' is usually under a second.

| Option | What it means |
|---|---|
| Steps and sync-to-line (Recommended) | Earlier/later buttons in 0.5 s steps with the value and a reset, plus 'Sync this line now': the selected line is set to start at the current position. Fine and coarse fixes both quick; two controls to design. |
| Steps only | Earlier/later buttons in 0.5 s steps, the value shown, a reset. Simple and familiar; a 12 s intro takes 24 presses. |
| Sync-to-line only | Select a line and press 'Sync this line now' while it is sung. One action for any shift; hard to hit within a fraction of a second, no fine nudge. |

**Answer: Steps and sync-to-line**

### The cache already holds 'not found' answers from the exact-only lookup, each asked again only after 30 days. What happens to them when this change lands?

| Option | What it means |
|---|---|
| Ask again on next play (Recommended) | The migration marks them as asked by the old matcher, so each is looked up again (with the fallback) the next time it plays. Fixes the issue's tracks at once; one request per old miss, spread over normal listening. |
| Wait out 30 days | Old misses stay until their retry. No migration work; the tracks from the issue keep showing nothing for up to a month unless picked by hand. |

**Answer: Ask again on next play**

### Should lyrics found by the search fallback (not an exact match) be marked as such in lyrics mode?

| Option | What it means |
|---|---|
| Quiet note (Recommended) | A small muted line such as 'Lyrics from LRCLIB, matched by search' with the 'Choose lyrics...' action next to it. Tells why sync may be off and where to fix it; one more line of text. |
| No marking | Fallback lyrics look like any other. Cleanest view; a shifted or wrong version gives no hint that the picker or offset can help. |
| Always show source | Every lyrics mode shows its source (.lrc, tags, LRCLIB exact, LRCLIB search). Most transparent; clutter on the common case where the lyrics are right. |

**Answer: Quiet note**

### The priority today is .lrc, embedded synced, LRCLIB synced, embedded plain, LRCLIB plain. When the user picks an LRCLIB candidate by hand, does it win over the track's local lyrics?

| Option | What it means |
|---|---|
| Pick wins (Recommended) | A hand-picked candidate shows over .lrc and tag lyrics, until the user picks 'Use the file's lyrics' in the picker. Fixes wrong embedded lyrics without editing files; local lyrics no longer always win. |
| Local still wins | The pick only replaces the online answer; local synced lyrics keep winning. Priority stays simple; wrong embedded synced lyrics can only be fixed by editing the file. |

**Answer: Pick wins**

### Which offset step size should the spec use? (Asked after the first draft: 0.5 s steps can't fix a shift under 0.5 s or the reaction-time error of "Sync this line now".)

| Option | What it means |
|---|---|
| 0.1 s plus 1 s | A fine step for small drift and a coarse step for bigger shifts. Covers both cases; two step sizes to offer in the controls. |
| Keep 0.5 s | One step as first chosen. Simplest; small drift and the sync-to-line error stay unfixable. |

**Answer: 0.1 s plus 1 s**

## High-level plan

One change in four workstreams. The first two have no UI and are tested on their
own; the view needs all three before it.

**1. Matching (vcmp-integrations, vcmp-library).** LRCLIB's search in the client, the
candidate scoring that the automatic fallback uses after a 404 from the exact
lookup, and a "how it was matched" mark on each cached answer. The cache migration
makes every "not found" stored by the old matcher due on the next play. Tests run
against the fake transport the client already has. Depends on nothing.

**2. Hand-picked lyrics and offsets (vcmp-library).** Storage for a track's
hand-picked LRCLIB record (or "none"), and for an offset tied to the track and the
lyrics it was set on. The priority function learns that a pick wins over
everything. Depends on the model from 1 only for the LRCLIB record id.

**3. Wiring (vcmp-app, player service).** The offset applied to the timed lines the
current-line computation uses; "sync this line now" turned into an offset; the
picker's searches sent through the existing one-at-a-time worker; a pick or an
offset change applied to the track playing at once. Depends on 1 and 2.

**4. The UI and the docs (vcmp-app UI).** The offset controls and the quiet note in
lyrics mode, the "Choose lyrics..." item in the now playing menu, and the picker
dialog. Then `docs/library.md`, `docs/ui.md`, `docs/manual-testing.md` (real
LRCLIB searches, sync by ear), DESIGN.md for the owner's review of the new controls,
and screenshots in light and dark themes. Depends on 3.

## Architecture decisions

### Matching

- **Exact first, search second.** The exact `/api/get` stays the first question,
  unchanged. Only its 404 (not an instrumental record, not a failure) leads to one
  `/api/search` by title and artist, without the album (an album name is the most
  common mismatch between versions). The title sent is the normalised one (see
  below), so "Song (2011 Remaster)" searches for "Song". Reason: exact answers keep their precision,
  and a miss costs one extra request, sent through the same sequential worker with
  the same spacing and `Retry-After` handling.
- **VCMP scores the candidates itself**, since LRCLIB's order is undocumented. A
  candidate counts only when it is not instrumental, has lyrics, its artist equals
  the track's and its title equals the track's after normalisation, and its
  duration is within a window of about 15 s. Among those: synced before plain, then
  the smallest duration difference, then the same album. Reason: the issue's case
  is a version that differs in length, not in name.
- **Normalisation folds case, accents and spacing**, and drops bracketed or dashed
  version words that don't change the words sung (remaster, remastered with a year,
  mono, stereo, explicit, clean, album or single version). Words that change them
  (live, remix, acoustic, instrumental, demo) are kept, so those versions never
  match a studio track by accident. Reason: widening the duration window is safe only
  if the name still has to match.
- **Each cached answer records how it was found**: exact, search, or the old exact-
  only matcher (all rows that exist before the migration). A "not found" from the old
  matcher is due at once; the 30-day retry applies to misses of the new matcher.
  Reason: the owner's "ask again on next play", and a later matcher change can use
  the same mark.

### Hand-picked lyrics

- **The picker is a dialog opened from "Choose lyrics..." in the now playing track
  menu**, for the track playing, with or without lyrics. It opens with the track's
  title and artist in editable fields and searches LRCLIB with them; the user can
  edit and search again. Each candidate shows title, artist, album, duration (and
  its difference from the track's) and whether it is synced. Its choices: a
  candidate, "No online lyrics" (none), and "Automatic" (remove the pick; named
  "Use the file's lyrics" when the track has local lyrics).
- **The picker works only while "Find lyrics online" is on.** With the switch off,
  nothing reaches LRCLIB from the picker either. Whether the item is hidden or
  explains the switch is settled in the UI work.
- **A pick is stored apart from the lookup cache**, keyed source-neutrally like
  `lyrics_lookups` (source and track id), with the LRCLIB record id, its lyrics
  text, and when. It is not tied to the question, so a tag edit or rescan keeps it.
  Reason: the cache is the automatic answer and is replaced when the tags change;
  the user's explicit choice must survive that.
- **Priority with a pick:** a picked record wins over every source, `.lrc` and tags
  included (owner's "pick wins"); "none" removes online lyrics and stops automatic
  lookups for the track, while local lyrics still show; "Automatic" restores
  today's order. Without a pick the order is unchanged. The priority stays one pure
  function, now also given the pick.
- **The quiet note** shows in lyrics mode only for lyrics found by the search
  fallback: a muted line saying they were matched by search, beside the "Choose
  lyrics..." action. Exact and hand-picked lyrics show no note.

### Offsets

- **An offset belongs to a track and the lyrics it was set on.** It is stored with
  the track's key and an identity of the lyrics text shown (a hash of the text
  before any offset), so an `.lrc` added later, a new LRCLIB answer or a new pick
  starts at 0, and going back to the same text brings its offset back. Reason: the
  owner's "track and source"; the hash needs no list of source kinds and covers an
  edited `.lrc` the same way.
- **Stored in SQLite, never in files.** Nothing in the lyrics code writes a music
  file or an `.lrc`, and the 2026-11-07 spec kept LRCLIB results out of files.
- **One sign convention:** the same as LRC's `[offset:]`, positive shows lines
  earlier. The user offset is added to whatever `[offset:]` the text carries, and
  applied once, where the timed lines reach the current-line computation and the
  click-to-seek times. Plain lyrics have no offset.
- **Two controls in lyrics mode for synced lyrics:** earlier and later in 0.1 s
  and 1 s steps with the value shown and a reset, and "Sync this line now", which sets the
  offset so the selected line starts at the current playback position. Both are
  keyboard operable and their strings go through `@tr()`.

### Invariants a test enforces

- No search request after an exact hit, an instrumental answer or a failure; one
  after a 404.
- Candidate scoring: wrong artist, a kept version word (live, remix), an
  instrumental record and a duration outside the window never win; synced beats
  plain; closer duration beats farther.
- Normalisation drops the listed version words and keeps the others.
- Migration: an old-matcher "not found" is due; an old found answer is kept.
- Priority over every combination of pick (record, none, absent), local and online.
- Offsets: applied with the right sign on top of `[offset:]`; a different text starts
  at 0; the same text brings its offset back; "sync this line now" puts the line at
  the position.
- With the switch off, neither the lookup nor the picker sends anything.
- The existing UI invariants (theme tokens, `@tr()`, contrast, keyboard) cover the
  new controls and the dialog.

## Weaknesses and risks

| Risk | Likelihood | Cost | Mitigation |
|---|---|---|---|
| The fallback picks a wrong version with the same name and a similar length (a re-recording, a live take titled without "live") | Medium | Low-medium: wrong or shifted lyrics | Name must still match, 15 s window, the quiet note, the picker and the offset |
| The version-word list is too narrow (misses stay) or too wide (wrong versions match) | Medium | Low | Kept as one list with tests per word; adjusted from real misses in use |
| The 15 s window is wrong for long intros or short edits | Medium | Low | A constant; the picker covers what it misses |
| A pick hides a better `.lrc` added later, since a pick wins over everything | Low | Low | "Use the file's lyrics" in the picker; the quiet note doesn't apply, so the docs and manual tests mention it |
| Old misses are asked again at once after the update | Certain | Very low: one extra request per old miss, only when it plays | Spread over normal listening by design |
| The search returns up to 20 records with full lyrics, a few hundred KB | Medium | Very low | One response per miss or per picker search |
| "Sync this line now" is off by a human reaction time (200 to 300 ms) | High | Very low | The 0.1 s steps fine-tune it |
| An offset keyed by a text hash resets when the same lyrics are re-saved with a small change | Low | Very low: set it again | Accepted for one uniform rule |
| Lyrics mode gets crowded by the offset controls and the note, mostly in narrow windows | Medium | Low-medium | Settled in the UI work with screenshots and the owner's review in DESIGN.md |
| Schema version clash: `feature/vcmp-streaming-server` also changes `schema.rs` | Medium | Low: a renumbered migration at merge | Whichever merges second renumbers its migration |
| LRCLIB changes or drops its search endpoint | Low | Low: back to exact only | Failures are handled as today |

**Questions deliberately left open**

- The exact version-word list and the window size are tuned during implementation
  against real misses, within the rules above.
- How loosely LRCLIB's search matches the title is not documented. Whether the
  stripped title alone finds the plain version, or a second search with the raw
  title is worth its request, is checked against the real API with failing tracks
  before the scoring is fixed.
- With the switch off, whether "Choose lyrics..." is hidden or explains the switch.
- The look of the offset controls (always visible or behind an "Adjust sync"
  toggle), how a line is selected for "Sync this line now" without seeking to it,
  and whether the picker previews a candidate's lyrics: UI work and the owner's
  review.

## Out of scope

- Writing an offset or picked lyrics into `.lrc` files or tags.
- Opening the picker from track lists or for a track not playing.
- An offset for all tracks at once, and estimating an offset automatically (for
  example from the duration difference).
- Other lyrics providers, word-by-word sync, editing lyrics text.
- MCP tools for agents to read or set picks and offsets.
- Spotify tracks (the keys stay source-neutral for #86).
